# #33 — Status & handoff (2026-04-28)

Kompakti tilannekatsaus. Kanonen suunnittelu on edelleen [`design.md`](./design.md),
mutta sen schema/algoritmi-osioita on muutettu LLM-reviewin (commit `da5ab4f` →
`19476d0`) jälkeen — katso §"Poikkeamat design.md:stä" alta ennen toteutusta.

## Mitä on valmista

**Vaiheet 1–8** (skeema + DB + parsinta + agentin kerrostettu prompt + USER.md
+ thread-skooppattu pysyvyys) ovat koodissa. 183 unit-testiä + 11
integraatiotestiä menevät läpi.

| Vaihe | Tila | Commit |
|-------|------|--------|
| Migraatio 009 (threads, thread_messages, conversations.thread_id+tenant_id+user_id, email_processing.thread_id, user_profiles.{language, notes_md}) | done | `da5ab4f`, `662e7a0` |
| `email.rs` In-Reply-To + References parsinta | done | `da5ab4f` |
| `db.rs` thread-aware funktiot (`resolve_thread`, `claim_with_thread`, `record_thread_message_tx`, `load_conversation_by_thread`, `persist_successful_reply`) | done | `da5ab4f`, `52cf2d2` |
| `scripts/009_backfill_threads.sql` legacy-thread backfill (multi-tenant precondition) | done | `da5ab4f`, `662e7a0` |
| Pikkukorjaukset (`from_domain` doc, `strip_reply_prefix`, JSON error propagation, deterministic fallback Message-ID) | done | `df67df5` |
| Integraatiotestit `claim_with_thread` | done | `19476d0` |

Promptit `services/email/prompts/{SOUL,AGENTS,SALIENCE}.md` ovat repossa ja
ne kootaan Block 1:ksi `agent::prompts::block1_persona_rules()` -funktiolla
(cache-control: ephemeral).

## Mitä on jäljellä

1. **Vanhan polun poisto** (compatibility-fallback): `try_claim_message`
   muille kuin assistant-tilille on käytössä; `load_conversation(sender)` ja
   `save_conversation_messages` ovat fallbackeja vanhoille (thread_id=NULL)
   keskusteluille ja retryille. Suositeltava poisto-PR seuraavalla
   release-syklillä kun vanha conversations-rivit on backfilattu (design.md §11
   vaihe 5).
2. **Migraatio 5**: `conversations.thread_id NOT NULL` -tiukennus
   (design.md §11 vaihe 5) jää myöhemmälle PR:lle.
3. **Behavioural evals (§13.2)**: ei estä mergeä, mutta hyvä ajaa kun ENV-koe
   on käytettävissä.

## Tämän PR:n koodimuutokset

| Tiedosto | Muutos |
|----------|--------|
| `services/email/src/agent/mod.rs` | Refaktoroitu kerrostettuun systemiin: Block 1 (cached) + USER.md + Session, USER.md re-render per iteraatio. `_sender`-parametri poistettu. |
| `services/email/src/agent/prompts.rs` | Uusi: `include_str!` SOUL/AGENTS/SALIENCE → `block1_persona_rules()`. |
| `services/email/src/agent/user_memory.rs` | Uusi: `render_user_md()`, `render_session_context()`, BCP-47-arvojen YAML-renderöinti, 16 kB hard cap. |
| `services/email/src/db.rs` | Uudet helperit `load_thread_meta()`, `draft_summary_brief()` (Block 3:lle). |
| `services/email/src/tools/mod.rs` | `ToolContext.thread_id`-kenttä lisätty. |
| `services/email/src/tools/definitions.rs` | `update_user_preferences` saa `language`-kentän, `update_user_notes` -tool lisätty (yhteensä 11 toolia). |
| `services/email/src/tools/handlers.rs` | `update_user_preferences` validoi BCP-47:n, `update_user_notes` strippaa frontmatterin + 16 kB cap. |
| `services/email/src/main.rs` | `process_message_inner` haarautuu assistantilla `claim_with_thread`-polkuun; `process_assistant_reply` käyttää `load_conversation_by_thread` + `persist_successful_reply`; retry-polku samoin (legacy fallback thread_id=None retryille). |
| `services/email/src/bin/gs_email_cli.rs` | Päivitetty `agent::process_with_tools` -kutsu. |

## Poikkeamat design.md:stä review-korjausten jälkeen

Lue nämä ennen kuin nojaudut design.md:n schema- tai algoritmikoodiin. Itse
suunnittelu (kerrostettu prompt, USER.md, SALIENCE-säännöt) pätee sellaisenaan.

| design.md kohta | Mitä muutettiin | Miksi |
|----|----|----|
| §3.1 `thread_messages.UNIQUE` | `(tenant_id, message_id)` → `(tenant_id, user_id, message_id)` + composite FK `(thread_id, tenant_id, user_id)` → `threads(id, tenant_id, user_id)` | Sama Message-ID voi tulla legitiimisti monelle käyttäjälle samassa tenantissa (CC, alias, internal forward). Vanha UNIQUE pudotti hiljaa toisen rivin → toisen käyttäjän thread fragmentoitui. |
| §3.1 `conversations` | Lisätty `tenant_id` + `user_id` (nullable) | Defense-in-depth: vaikka `thread_id` on globaalisti uniikki, väärän thread_id:n ohjaaminen ei saa vuotaa cross-tenant. |
| §3.1 `user_profiles.language` CHECK | `^[a-z]{2,3}(-[A-Z]{2})?$` → `^[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*$` | Vanha hylkäsi `zh-Hant`, `es-419`, `sr-Latn-RS` ym. validit BCP-47-tagit; backfill tuhosi ne hiljaa. |
| §3.1 `language`-backfill | Kaksi UPDATE:a → yksi atominen (poistaa JSON-avaimen vain jos kopio onnistui) | Vanha versio tuhosi non-konformit arvot. |
| §2.2 `resolve_thread` | Per-candidate query → yksi `ANY($cands)` + cap 50; cross-user-osuma EI poison-pill, vaan continue → seuraava candidate; `t.status = 'closed'` filtteröidään | (1) DoS-suoja pitkille References-listoille. (2) Vanha tappoi haun ensimmäiseen huonoon kandidaattiin → validit ancestorit hukkuivat. (3) Closed-thread ei revive. |
| §2.2 cross-user spoof | "→ New thread + warning" → primary lookup on per-user-skoopattu (ei poison); cross-user osumat erillisellä telemetria-queryllä | Per-user UNIQUE sallii saman ID:n monelle käyttäjälle, joten cross-user-tarkistus ei voi toimia entiseen tapaan. |
| §3.1 `email_processing.thread_id` FK | `REFERENCES threads(id)` → `... ON DELETE SET NULL` | Säilyttää audit-jäljen jos thread joskus poistetaan. |
| §4.1 `last_activity_at` | Päivittyy vain `persist_successful_reply`:ssä → päivittyy myös `claim_with_thread`:n inbound-recordauksessa | Inbound-only-aktiivisuus (failed processing, suspicious) ei jäätyttänyt threadin viimeistä aktiivisuutta → 91 vrk:n päästä thread fragmentoitui väärin. |
| §6.2 `record_thread_message_tx` | `ON CONFLICT DO NOTHING` → insert-or-validate (lokita warn jos `thread_id`/`direction` poikkeaa, telemetria cross-user-osumalle) | Hiljainen no-op piilotti consistency-virheet ja forged-ID-misroute-tilanteet. |
| §2.1 Message-ID-käsittely | Strict equality | `email.rs::normalize_msg_id`: trim + lowercase + `<…>`-wrap. Käytännön mailerit muuttavat case:a quote-replyssä → strict equality hukkasi threadeja. |
| `email.rs::generate_fallback_id` | UUID v4 → SHA-256 raw bytes | Determinismi pakollinen idempotenssille: sama ID:tön viesti retryssä = sama fallback-ID. |
| `id_list("")` | `vec!["<>"]` → `vec![]` | Tyhjä header tuotti hyödytöntä `<>`-kandidaattia. |

## Concurrency-invariantti (säilytetty)

Sarjallinen käsittely per IMAP-tili pätee ennallaan (`main.rs::run_imap_loop`,
`for uid in &uids`). `assistant@` on yksi tili → kaikki agentin kirjoitukset
ovat keskenään serial. Tämä on ainoa syy miksi `claim_with_thread` toimii ilman
`FOR UPDATE SKIP LOCKED`-lukitusta. Jos arkkitehtuuria horizontaali-skaalataan,
lisätään advisory lock per `(tenant, user)` — design.md §14.3.

## Avoimet jatkokehityskohteet

- **#37** — `tool_result` content size cap conversation history:ssa. Ei kriittinen
  nykytooleilla (ei base64-paluuta), mutta korjattava ennen agent-cutoveria
  jotta `list_expenses` yms. eivät paisuta `conversations.content_json`:ia.
- **#34** — `report_suspicious_message`-työkalu (toinen issue). Ei kuulu tämän
  scopen, mutta SALIENCE.md viittaa siihen.
- **#35** — Prompt-cache-strategia. Toteutuksen jälkeen telemetrian kanssa.
- **#36** — Privacy-review (audit-taulu, GDPR-endpointit, retention).

## Mistä etsiä

- **Suunnittelu**: `design.md` (39k tokenia — lue ne osiot joita aiot toteuttaa)
- **Schema**: `services/email/migrations/009_conversation_threads.sql`
- **Backfill**: `services/email/scripts/009_backfill_threads.sql` (manuaalinen, ei auto-applied)
- **Promptit**: `services/email/prompts/{SOUL,AGENTS,SALIENCE}.md` (valmiit, integroimatta)
- **DB-funktiot**: `services/email/src/db.rs` (etsi `resolve_thread`, `claim_with_thread`, `record_thread_message_tx`, `load_conversation_by_thread`, `persist_successful_reply`)
- **Sähköpostin parsinta**: `services/email/src/email.rs` (`normalize_msg_id`, `generate_fallback_id`)
- **Käyttäytymisen kanonen kuvaus**: `services/email/tests/claim_with_thread.rs` (11 integraatiotestiä)
- **Review-raportti**: `history/review-da5ab4f-thread-model-scaffold.md` (ei track:issa)

## Testien ajaminen

```bash
# Yksikkötestit (ei tarvitse DB:tä)
cargo test --lib

# Integraatiotestit (vaatii Postgres + CREATEDB-oikeudet)
DATABASE_URL=postgres://USER@localhost/postgres \
  cargo test --test claim_with_thread
```

`#[sqlx::test]` luo per-test-tietokannan ja ajaa migraatiot
(`migrations/`-hakemistosta) automaattisesti.
