---
created: 2026-04-30
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: high
epic: 56
related: ["#74", "#56", "#59", "#60", "#66", "#76"]
labels: [foundation, ops, ingest]
commits:
  - hash: 9fac27e
    summary: "refactor(ops/server): relocate ingest DB lifecycle to ops::ingest::* (#78 A5a, #76)"
---

> **Status 2026-05-01:** A5 valmis kokonaan. A5a + A5b + A5c kaikki
> landanneet. A5c (`a5c-gs-dev-parse-eml` -worktree) lisäsi `gs-dev dev
> parse-eml` -komennon ja kytki `gsdev mail send-eml` -wrapperin
> oikeaan toteutukseen → **#74:n send-eml-osa suljettu**. Tämä issue
> suljetaan `done`-tilaan, `git mv → closed/`.

# 78. ops-ingest full-message surface (A5 — A4-jatkona)

_Source: B2-worktreen `/llm-review` -tuotos 2026-04-30. Tämä versio kirjoitettu uudelleen toisen review-kierroksen jälkeen — kts. `history/review-issue-75-ops-ingest-full-message-surface.md`._

## Description

A4b siirsi `ops::ingest::process_message` -pinnan **kapeasti** sender-resolutionia varten. Loput per-viesti-DB-vaiheet (claim, spam_verdict, status_update, retry-state, threading, conversation persist) jäivät `crates/server/src/ingest/db.rs`:ään (1714 riviä, ~30 funktiota) ja `runner.rs::process_message_inner`:iin.

**A5 jakaa orkestroinnin ja persistoinnin:**

- **`crates/ops/src/ingest/`** kasvaa kattamaan kaikki DB-tilatransitiot puhtaina funktioina (`claim`, `record_spam_verdict`, `update_status`, retry-tilatransitiot, threading-SQL, conversation-row-fetch+persist).
- **`crates/server/src/ingest/pipeline.rs` (UUSI)** omistaa orkestroinnin: kutsuu `handler::decide` + `spam::evaluate` + LLM-clientin + ops-funktiot + SMTP:n. Palauttaa `PipelineEffect`-enumin (3 varianttia) jonka `runner.rs` toteuttaa IMAP/SMTP-kutsuilla.
- **`runner.rs`** kutistuu ≤ 250 riviin — ei DB-kutsuja, ei `ops::*`-importteja, IMAP-sessio ei ylitä pipeline-rajaa.

Tämä unblockaa #74:n (`gs-dev dev parse-eml` rakentuu pipelinen päälle ilman GreenMail-stack:ia) ja luo paikat #60:n decision-riveille (D-aalto kirjoittaa pipelinen seamien lävitse).

### Konteksti — miksi tämä on omana A-track-vaiheena

A4b:n decision log lukitsi tulkinnan että "D-aalto laajentaa kun siirtää sinne agent_runs-kirjoitukset." B2:n `/llm-review` paljasti tulkinnan vääräksi: D-aalto (#58–#62) on puhtaasti agent-trace-instrumentointia, ei pipeline-laajennus. Refaktori on erillistä foundation-työtä joka kuuluu A-trackiin.

## Architecture

### Pipeline boundary

```
runner.rs (IMAP IDLE + retry polling task)
    ↓
    pipeline::process_message(pool, ai_client, &parsed) → PipelineEffect
    ↓
    [pipeline.rs orkestroi:]
        ops::ingest::resolve_inbound_sender
        ops::ingest::lifecycle::claim_*
        spam::evaluate (server-side, AR-headers + known_sender)
        ops::ingest::lifecycle::record_spam_verdict
        handler::decide (server-side, ParsedEmail+SpamVerdict → Decision)
        extraction::process_attachment (server-side, vision-OCR)
        ops::extractions::record_extraction
        agent::process_with_tools (server-side, LLM tool loop)
        ops::ingest::conversation::persist_reply
    ↓
runner.rs match {
    PipelineEffect::MoveTo(folder)        → imap::move_message(...)
    PipelineEffect::LeaveInInbox          → imap::mark_seen(...)
    PipelineEffect::ReplyThen(reply, fld) → smtp::send_reply(...) + imap::move_message(...)
}
```

### PipelineEffect (control-flow type, server)

```rust
pub enum PipelineEffect {
    MoveTo(ImapFolder),                  // Processed | Skipped | Junk
    LeaveInInbox,                        // mark_seen, retry next IDLE
    ReplyThen(ReplyContent, ImapFolder), // body precomputed by pipeline-internal templates
}
```

3 varianttia kattavat handler.rs:n nykyiset 4 dispositiota (Processed/Skipped/Junk/LeaveInInbox).

### ProcessRecord (struct, sisäinen + dev-CLI)

```rust
pub struct ProcessRecord {
    pub resolution: SenderResolution,
    pub claim: Option<ClaimResult>,
    pub spam_verdict: Option<SpamVerdict>,
    pub decision: Option<Decision>,
    pub extraction_outcome: Option<ExtractionOutcome>,
    pub reply: Option<ReplyContent>,
    pub final_status: ProcessingStatus,
}
```

Pipeline akkumuloi tämän sisäisesti; `gs-dev dev parse-eml --json` printtaa rakenteen. Ei käytetä runnerin control-flowiin (PipelineEffect on sitä varten).

### Dry-run semantics

**EI tx-rollbackia.** Wrapping pipeline tx:n sisään pitäisi tx auki 30–60 s LLM-kutsujen yli → pool-starvation, audit-rivien menetys, `agent_trace`-rivien menetys (D2:n tx-share-design-rikkomus), retry-tilan menetys.

Sen sijaan eksplisiittinen `dry_run: bool` lippu pipeline-kontekstissa:

```rust
pub struct PipelineMode {
    pub skip_db_writes: bool,   // dry-run: ops::* write-funktioita ei kutsuta
    pub mock_llm: bool,         // dry-run: vision-OCR + agent loop palauttavat fixturen
    pub mock_smtp: bool,        // dry-run: SMTP-effect ei suorita verkkokutsua
}
```

CLI:n `--dry-run` asettaa kaikki kolme `true`-arvoon. Tuotanto käyttää `false`/`false`/`false` -oletusta. Ei tx-käärityksiä.

## Scope — `db.rs` -symbolien disposition-taulukko

Tämä on A5:n todellinen scope-määrittely. Jokaiselle `crates/server/src/ingest/db.rs`-symbolille destinaatio:

| `db.rs` symboli | Destinaatio | Vaihe | Notes |
|-----------------|-------------|-------|-------|
| `try_claim_message`, `claim_with_thread`, `update_status` | `crates/ops/src/ingest/lifecycle.rs` | A5a | DB state transitions; **molemmat claim-polut säilyvät** (assistant-recipient saa thread-aware:n, muut basic:in). Refaktoroidaan ottamaan `db: impl PgExecutor<'_>` (drop sisäinen `pool.begin()`). |
| `schedule_retry`, `mark_failed`, `RetryableMessage` | `crates/ops/src/ingest/retry.rs` | A5a | DB state transitions vain. |
| `MAX_RETRIES`, `RETRY_DELAYS_SECS`, `retry_delay_secs`, `fetch_retryable_messages` polling task | **STAY IN** `crates/server/src/ingest/runner.rs` (+ uusi `retry_policy.rs`) | A5b | Retry-aikataulu (60s/300s/900s) + polling-loop on **server-policy**, ei DB-domain. |
| `resolve_thread`, `record_thread_message_tx`, `create_thread_tx`, `THREAD_REVIVE_MAX_AGE_DAYS` | `crates/ops/src/ingest/threading.rs` | A5a | DB-kysely + persist. |
| `ordered_reference_candidates`, `strip_reply_prefix`, `MAX_REFERENCE_CANDIDATES` | `crates/server/src/ingest/threading.rs` (UUSI) | A5a | Pure parse helpers, server-side. |
| `load_conversation`, `load_conversation_by_thread`, `save_conversation_messages*`, `persist_successful_reply`, `PersistArgs` | **SPLIT**: row-fetch + persist → `crates/ops/src/ingest/conversation.rs`; LLM-shape mapping → `crates/server/src/ingest/conversation.rs` (UUSI) | A5b | DB-shape (`ConversationRow`) on opsin pinta; `Vec<ConversationRow>→Vec<Message>` mapping ja `truncate_pair_aware` palaavat serveriin. Sama kuvio kuin `ops::extractions::load_extraction_summaries`. |
| `truncate_pair_aware`, `cap_tool_result_json`, `MAX_TOOL_RESULT_BYTES`, `MAX_HISTORY_ROWS` | `crates/server/src/ingest/llm/limits.rs` (UUSI) | A5b | LLM-API-concerns; ei opsiin. |
| `load_thread_meta`, `ThreadMeta`, `draft_summary_brief`, `DraftBrief` | `crates/ops/src/ingest/session_context.rs` | A5b | Block-3 reads. |
| `load_extraction_summaries` (db.rs:n wrapperi) | **DELETE** | A5a | Duplikaatti `ops::extractions::load_extraction_summaries`:n kanssa. |
| `record_profile_revision_tx`, `RevisionSource`, `diff_jsonb_top_level` | `crates/ops/src/user_profile/revision.rs` (siirrä `ingest`:n ulkopuolelle) | A5b | Ei ingest-spesifinen. |
| `is_known_sender` | `crates/ops/src/user::is_known_sender_in_tenant(pool, tenant_id, email)` | A5a | **KORJATA tenant-scope** — nykyinen `SELECT id FROM users WHERE email=$1` LIMIT 1 on cross-tenant-leak. Caller johtaa tenant_id:n viestin recipient-osoitteesta. |
| `connect`, `migrate` | **DELETE** | A5a | Server bootstrap hoitaa; `migrate` on stub joka bailaa. |
| `update_spam_verdict`, `mark_failed` | `crates/ops/src/ingest/lifecycle.rs` | A5a | DB state transitions. |
| `TERMINAL_STATUSES` const | `crates/ops/src/ingest/lifecycle.rs` | A5a | Co-locate state machine -vakioiden kanssa. |

**Acceptance:** `crates/server/src/ingest/db.rs` on **poistettu** A5b:n jälkeen. Yksikään yllä oleva symboli ei ole jäänyt orvoksi.

## Worktree-jako (A5a / A5b / A5c)

Sequential split — jokainen sub-PR vastaa yhteen rakennekysymykseen, on itsenäisesti reviewable, ja jättää koodin toimivaan tilaan.

### A5a — DB-pinnan siirto opsiin — VALMIS 2026-05-01

**Status:** ✅ landed `a5a-ops-ingest-and-fixture-fix` -worktreessä
2026-05-01. Yhteen niputettu #76:n fixture-fix (laajeni A3-skema-
fixture-rivin korjaukseksi). Worktree-loki #56-epicissä; commitit
listataan tämän issueen `commits:`-frontmatterissa kun worktree
mergataan mainiin.

**Tavoite:** kaikki DB-only-symbolit `db.rs`:stä → `ops::ingest::*` -taulukon mukaisesti. `db.rs` on kutistunut tai poistettu A5a:n päätteeksi.

Kohteet:
- `lifecycle.rs` (claim, status, spam_verdict, retry-state-transitions, TERMINAL_STATUSES)
- `retry.rs` (retry-state-tx vain; policy + polling jää serveriin)
- `threading.rs` (DB-puoli)
- `crates/server/src/ingest/threading.rs` (UUSI, parse-helpers `ordered_reference_candidates`+`strip_reply_prefix`)
- `is_known_sender` → `ops::user::is_known_sender_in_tenant` (tenant-scope-fix mukana)
- `claim_with_thread` ja `persist_successful_reply` refaktorointi → `db: impl PgExecutor<'_>` (drop sisäinen `pool.begin()`); kutsujapuoli (runner.rs) hoitaa tx-koordinaation
- Päivitä callsitet `runner.rs`:ssä → kutsuu `ops::ingest::*` suoraan
- `db.rs` deletoidaan tai jää korkeintaan tyhjäksi mod-tiedostoksi

**Ei vielä** pipeline.rs:ää, ei `PipelineEffect`-enumia, ei `gs-dev dev parse-eml` -CLI:tä, ei `ProcessRecord`-structia. Pelkkä relokaatio + tenant-scope-fix.

**Acceptance:**
- `cargo test --workspace` puhtaana
- `cargo build --workspace --tests` puhtaana
- `crates/server/tests/{claim_with_thread.rs, extraction_rescue.rs, unknown_sender.rs}` läpäisevät (importtien päivitys `db::` → `ops::ingest::`)
- `is_known_sender_in_tenant` ottaa `tenant_id`-parametrin; vanha cross-tenant-rajapinta ei jää käytäntöön
- `runner.rs::process_message_inner` ei kutsu omaa db-modulia (kaikki ops:n kautta)

### A5b — Pipeline-decoupling

**Tavoite:** `crates/server/src/ingest/pipeline.rs` (UUSI) omistaa orkestroinnin. `runner.rs` kutistuu ≤ 250 riviin.

Kohteet:
- `pipeline.rs::process_message(pool, ai_client, &parsed, mode: PipelineMode) → PipelineEffect`
- `PipelineEffect`-enum (`MoveTo` / `LeaveInInbox` / `ReplyThen`) ja `ReplyContent`-struct
- `ProcessRecord`-struct sisäiseksi akkumulaattoriksi (näytetään testeille + dev-CLI:lle)
- Konversaatio-pinta: `ops::ingest::conversation` + `crates/server/src/ingest/conversation.rs` (LLM-mapping)
- `crates/server/src/ingest/llm/limits.rs` (UUSI, `truncate_pair_aware` + `cap_tool_result_json`)
- `ops::ingest::session_context` (block-3 reads)
- `ops::user_profile::revision` (siirrä ulos ingest:istä)
- `runner.rs` kutistus: poista `process_message_inner` -orkestrointi → `pipeline::process_message` + `match` IMAP/SMTP-effekteille

**Acceptance:**
- `runner.rs ≤ 250 riviä`
- `runner.rs` ei sisällä `ops::*` tai `sqlx::*` -importteja
- `pipeline::process_message` toimii pelkän `&PgPool` + `Option<&AnthropicClient>` -panaaroilla — ei `&mut ImapSession`
- `cargo test --workspace` puhtaana

### A5c — `gs-dev dev parse-eml` -CLI

**Tavoite:** unblock #74 (`gsdev mail send-eml`).

Kohteet:
- `crates/dev-cli/src/main.rs` lisää `Commands::Dev::ParseEml { file: PathBuf, dry_run: bool, json: bool }`
- Lukee `.eml`-fixturen `mail_parser`:lla, kutsuu `pipeline::process_message` + mockattu LLM/SMTP `dry_run` -tilassa
- JSON-output: `ProcessRecord` kanonisena shape:na (esim. seuraava skema):
  ```json
  {
    "resolution": {"outcome": "resolved", "tenant_id": 1, "user_id": 5},
    "claim": {"outcome": "claimed", "thread_id": 42},
    "spam_verdict": {"verdict": "clean", "signals": [...]},
    "decision": {"mailbox": "Processed", "reply": "trigger"},
    "extraction_outcome": {"attachments": 2, "extractions": 2, "failures": 0},
    "reply": {"subject": "Re: ...", "body": "..."},
    "final_status": "replied"
  }
  ```
  (`gs-dev dev send`-komentoa päivitetään myös samaan JSON-konventioon yhdenmukaisuuden vuoksi.)
- Päivitä `tools/dev/gsdev/mail.py::cmd_send_eml` kutsumaan uutta CLI:tä — exit-2-stub poistuu, todellinen toteutus tilalle. Tämä sulkee #74:n send-eml-osan.
- Lisää `crates/dev-cli/tests/parse_eml.rs` smoke-testit fixtureilla (tunnettu lähettäjä, tuntematon, attachment).

**Acceptance:**
- `gs-dev dev parse-eml --file <fixture>` toimii lokaalisti ilman GreenMail-stack:ia
- `gsdev mail send-eml --file <fixture>` (Python-wrapperi) ei enää exit-2; ajaa pipelinen läpi
- `gs-dev dev parse-eml --dry-run` ei kirjoita DB:hen, ei kutsu Anthropic-rajapintaa, ei lähetä SMTP:tä
- `--json`-output noudattaa kanonista shape:a (yllä)
- `crates/server/tests/{claim_with_thread, extraction_rescue, unknown_sender}.rs` säilyvät vihreinä
- `cargo test --workspace` puhtaana

## Out of scope

- **#59** D3 (process_with_tools instrumentointi) — D-aaltoa, ei A5:ssä
- **#60** D4 (decision-rivit) — **BLOKATTU A5:llä** (tarvitsee A5b:n pipeline-seamit), spawnataan A5b:n landauksen jälkeen
- **#66** claim-with-thread spam-amplifikointi — pre-existing tech-debt. **A5 EI lukitse claim/spam-järjestystä ops-pinnalle** — pipeline.rs (server) päättää järjestyksen, ops vain rekisteröi tapahtumat. Caller controls. #66:n korjaus on erillinen issue.
- **Ops-crate split** (`ops-identity`/`ops-finance`/`ops-ingest`) — workspace-tason refaktori. Filed #79 (SPIN-OFF).
- **`gs-dev dev history`** — itsenäinen 5-rivin SELECT, voidaan tehdä erillisessä B-aallon issuessa (mahdollisesti yhdistetty A5c:hen jos worktree-budjetti antaa).
- **`gsdev mail send` body/attachment** -palautus — riippuu A5c:n JSON-pinnan vakautumisesta; käsittele A5c:n jälkeen erikseen #74-jatkona.

## Spawn-aikataulu

- **A5a** spawnataan **A4c:n + B2:n landauksen jälkeen**. Voi rinnakkain **D3:n** (#59 — eri tiedostot, agent-loop) kanssa.
- **A5b** spawnataan **A5a:n landauksen jälkeen** (sarjallinen — A5b koskettaa A5a:n moveja).
- **A5c** spawnataan **A5b:n landauksen jälkeen** (pipeline.rs olemassa).
- **D4 (#60)** **BLOKATTU A5b:llä** — tarvitsee pipeline-seamit decision-rivien kirjoituksiin. Päivitettävä `#56` Spawn-aikataulu.
- **C-aalto (C2/C3/C4)** — voi rinnakkain A5a:n kanssa **JOS** ei kosketa `crates/server/src/ingest/`-koodia. Tarkista per worktree.

Maksimi rinnakkainen 3 worktreetä edelleen voimassa.

## Why this is A-track (not B/D)

| Vaihtoehto | Arvio |
|------------|-------|
| **A-track (Foundation)** ✓ valittu | Refaktori on puhdas crate-rajan siirto + uusi server-puolen pipeline-moduuli. Lukko muiden trackien rinnakkaisen edistämisen taakse. |
| B-track (Dev-env) | Mahdollinen, koska #74 unblockaa B:lle. Mutta refaktorin laajuus ja vaikutus muihin trackeihin on liian iso B:hen. |
| D-track (#57) | A4b lukitsi tämän alunperin D:n alle. B2:n analyysi paljasti että D-aalto (#58–#62) on instrumentointi, ei pipeline-laajennus → väärä koti. |

## Open questions (need human judgment)

1. **`OpContext` / `trace_id` plumbing**: pitäisikö `ops::ingest::lifecycle::*`-funktioiden ottaa `OpContext`-parametri, vai vain `(tenant_id, user_id)`? D-aalto haluaa lopulta `agent_runs.run_uuid` `trace_id`-kenttään. Suositus: ota `OpContext`, jätä `trace_id` `Option`-kentäksi joka A5a kirjoittaa `Some(message_id)`-arvolla; D3:n landauksessa vaihdetaan `run_uuid`:hen.
2. **`agent_runs.message_id` FK** — nyt `TEXT`/`VARCHAR`, ei FK `email_processing.message_id`:hen. Skemavalinta jätettiin avoimeksi A3:ssa. Ei A5:n scope, mutta tieto kannattaa lisätä `crates/ops/AGENTS.md`:hen.
3. **Onko `claim_with_thread` ja `try_claim_message` jako oikein** vai pitäisikö A5a unifioida `claim()`-funktioksi joka ottaa `Option<ThreadingHints>`-parametrin? Suositus: säilytä jako, älä unifioi yhdellä parametrilla — kaksi nimettyä funktiota ovat luettavammat kuin yksi monimuotoinen.

## Related

- **A4b** Decision log 2026-04-30: "narrow surface" -tulkinta (jonka A5 oikaisee)
- **B2** `history/review-b2-local-dev-analysis.md` 2026-04-30: D-wave-väite virheellinen
- **B2** `history/review-issue-75-ops-ingest-full-message-surface.md` 2026-04-30: review tämän ticketin v2-kirjoituksesta
- **#74** gsdev mail commands post-A4 — odottaa A5c:tä
- **#79** ops-crate split (SPIN-OFF B2:sta) — workspace-rakenne kasvaa A5:n myötä; ei kiireellinen
- **#59** D3 process_with_tools instrumentointi — A5a:n kanssa rinnakkain OK
- **#60** D4 decision-rivit — **BLOKATTU A5b:llä**
- **#66** claim-with-thread spam-amplifikointi — A5:n out-of-scope; ops ei lukitse järjestystä
