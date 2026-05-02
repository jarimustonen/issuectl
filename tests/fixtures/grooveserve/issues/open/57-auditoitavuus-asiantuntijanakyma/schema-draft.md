---
created: 2026-04-29
updated: 2026-04-29
type: schema-draft
owner: jari
status: draft
related: ["#56", "#57", "#38"]
---

# D1 — Agent-trace schema-luonnos

Tämä on **luonnos**. Päätökset perusteltu `analysis.md`:ssä.
Varsinainen `.sql`-migraatio ajetaan #57 Phase 2:ssa, kun #56 Phase 1
on tuonut yhteisen DB:n.

Migraationumero on placeholder (`NNN`) — Phase 2 valitsee oikean
numeron sen hetken `migrations/`-kansiosta.

---

## Taulujen yleiskatsaus

```
agent_runs                       (yksi rivi per viestin agent-suoritus)
└─ agent_steps                   (N riviä per run: LLM-iteraatiot, tool_uset, päätökset, virheet)
   └─ FK:t: attachments, extractions, receipts, thread_messages

receipt_revisions  (#38, eri issue)
└─ created_by_run_id → agent_runs.id   (linkki: mikä agent-suoritus loi tämän revision)

audit_events       (#26 §4.2, eri issue)
└─ metadata.agent_run_id → agent_runs.id  (linkki: mitä agent-suoritusta manuaalinen toiminto kosketti)
```

---

## `agent_runs`

Yksi rivi per `process_with_tools`-kutsu.

```sql
CREATE TABLE IF NOT EXISTS agent_runs (
    id                   BIGSERIAL PRIMARY KEY,
    -- Stable, externally referenceable id (UI-linkit, lokien grep).
    -- BIGSERIAL on ensisijainen, run_uuid sekundaarinen — UI:ssa
    -- näytetään uuid jotta sequential id ei vuoda käyttäjille.
    run_uuid             UUID NOT NULL DEFAULT gen_random_uuid(),

    -- Owner triple — sama politiikka kuin muilla user-data-tauluilla.
    tenant_id            BIGINT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_id              BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Mihin viestiin suoritus liittyy. Sama Message-Id kuin
    -- thread_messages.message_id ja email_processing.message_id.
    -- Ei FK email_processing:iin koska se on per (recipient, message_id),
    -- mutta thread_messages:iin voidaan FK:ttaa per (tenant, user, message_id).
    message_id           TEXT NOT NULL CHECK (length(message_id) BETWEEN 3 AND 512),
    thread_id            BIGINT REFERENCES threads(id) ON DELETE SET NULL,

    -- Sama UUID kuin tracing-spaniin (main.rs:n msg_span trace_id) —
    -- yhdistää lokirivit ja DB-rivit.
    trace_id             UUID NOT NULL,

    -- Käytetty malli (esim. claude-sonnet-4-6).
    model                TEXT NOT NULL,

    -- Suoritetun loopin lopputulos. Värivaihtoehdot peilaa nykyisiä
    -- code paths:eja (agent/mod.rs ja main.rs).
    status               TEXT NOT NULL
        CHECK (status IN (
            'running',                   -- vain pre-finalize, ei pitäisi jäädä DB:hen pysyvästi
            'completed',                 -- StopReason::EndTurn / StopSequence
            'truncated_max_tokens',      -- StopReason::MaxTokens (#49)
            'failed_transient',          -- AgentError::Transient (retry queueta varten)
            'failed_permanent',          -- AgentError::Permanent
            'failed_database',           -- AgentError::Database
            'aborted_max_iterations',    -- MAX_TOOL_ITERATIONS exceeded
            'aborted_wall_clock'         -- MAX_AGENT_WALL_CLOCK exceeded
        )),

    -- Aggregate-metriikat per run (samat kuin AgentReply struct).
    iterations           INTEGER NOT NULL DEFAULT 0 CHECK (iterations >= 0),
    total_input_tokens   INTEGER NOT NULL DEFAULT 0 CHECK (total_input_tokens >= 0),
    total_output_tokens  INTEGER NOT NULL DEFAULT 0 CHECK (total_output_tokens >= 0),

    -- Lyhyt error-class + täysi viesti. error_class on stable koodi
    -- ("transient", "max_tokens", "max_iterations", ...) jonka päälle
    -- voi tehdä WHERE-suodatuksia, error_message on ihmisluettava.
    error_class          TEXT,
    error_message        TEXT,

    started_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    finished_at          TIMESTAMPTZ,
    duration_ms          INTEGER GENERATED ALWAYS AS (
        CASE
            WHEN finished_at IS NULL THEN NULL
            ELSE EXTRACT(EPOCH FROM (finished_at - started_at)) * 1000
        END
    ) STORED,

    UNIQUE (run_uuid)
);

-- Yleisin haku: "tämän käyttäjän viimeisimmät runit"
CREATE INDEX IF NOT EXISTS idx_agent_runs_tenant_user_started
    ON agent_runs (tenant_id, user_id, started_at DESC);

-- Asiantuntijanäkymä: "kaikki epäonnistuneet runit ajalta X..Y"
CREATE INDEX IF NOT EXISTS idx_agent_runs_status_started
    ON agent_runs (status, started_at DESC)
    WHERE status NOT IN ('completed', 'running');

-- Viestin trace-haku: "mikä on viimeisin run tälle Message-Id:lle"
CREATE INDEX IF NOT EXISTS idx_agent_runs_message
    ON agent_runs (tenant_id, user_id, message_id, started_at DESC);

-- Threadin koko trace-historia
CREATE INDEX IF NOT EXISTS idx_agent_runs_thread
    ON agent_runs (thread_id, started_at DESC)
    WHERE thread_id IS NOT NULL;

-- Lokin ja DB:n yhdistely tracing-spanin trace_id:llä (debug)
CREATE INDEX IF NOT EXISTS idx_agent_runs_trace_id
    ON agent_runs (trace_id);
```

**Huomiot:**

- `run_uuid` on UI:n näytettävä id — sekvenssipohjainen `id` jää
  sisäiseen käyttöön. Tämä on sama kuvio kuin `audit_events` (#26).
- `error_class` ei ole CHECK-rajoitettu enum vaan vapaa string —
  uusia error-luokkia voi syntyä Phase 2:ssa ilman migraatiota.
- `duration_ms` on STORED generated column → indeksoitavissa ilman
  että sovellus joutuu laskemaan sen.
- Ei FK `message_id`:lle `email_processing`-tauluun: `email_processing`
  on per `(recipient, message_id)` ja sama Message-Id voi olla useassa
  tilissä (CC, alias). FK puuttuu tarkoituksella — `message_id` on
  vain pointer, ei rajoite.

---

## `agent_steps`

Yksi rivi per LLM-iteraatio / tool_use / päätös / virhe.

```sql
CREATE TABLE IF NOT EXISTS agent_steps (
    id                   BIGSERIAL PRIMARY KEY,
    run_id               BIGINT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,

    -- Juokseva järjestys runin sisällä. Aloitetaan 1:stä, monotoninen.
    seq                  INTEGER NOT NULL CHECK (seq >= 1),

    kind                 TEXT NOT NULL
        CHECK (kind IN ('llm_call', 'tool_use', 'decision', 'error')),

    -- LLM-iteraation numero (sama kuin agent/mod.rs:n iterations-counter).
    -- llm_call: 1..N. tool_use: minkä iteraation tool. decision/error:
    -- iteraation aikana joka oli viimeisenä — voi olla NULL jos päätös
    -- syntyi ennen agent-loopia (esim. policy_reply ennen LLM:ää).
    iteration            INTEGER CHECK (iteration IS NULL OR iteration >= 1),

    -- ── llm_call-spesifiset kentät ─────────────────────────────────
    stop_reason          TEXT
        CHECK (stop_reason IS NULL OR stop_reason IN (
            'end_turn', 'stop_sequence', 'max_tokens', 'tool_use', 'unknown'
        )),
    input_tokens         INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    output_tokens        INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    -- Anthropic-API:n request_id, jos saatavilla. Auttaa Anthropic-
    -- supportin kanssa korreloitua latenssi-/quota-tutkintaa.
    api_request_id       TEXT,

    -- ── tool_use-spesifiset kentät ─────────────────────────────────
    tool_name            TEXT,
    -- Anthropic-API:n tu_*-id. Sama id näkyy sekä llm_call-vastauksen
    -- ContentBlock::ToolUse:ssa että seuraavan iteraation
    -- ContentBlock::ToolResult:ssa.
    tool_use_id          TEXT,
    -- Tool-input agentin näkemällä tavalla (Anthropic-API:n input).
    tool_input           JSONB,
    -- Tool-output (cap_tool_result_json:in jälkeen sama JSON joka
    -- agentille palautettiin). Voi olla NULL jos error tai timeout.
    tool_output          JSONB,
    tool_ok              BOOLEAN,
    tool_is_error        BOOLEAN,

    -- ── decision-spesifiset kentät ─────────────────────────────────
    -- Päätösten taksonomia — laajenee, mutta nämä ovat MVP-kategoriat.
    -- decision_type ei ole CHECK-rajoitettu jotta uusia päätöstyyppejä
    -- voi lisätä koodissa ilman migraatiota.
    decision_type        TEXT,
        -- Esim:
        -- 'spam_skip'           — message.skipped pre-LLM
        -- 'unknown_sender'      — #43-politiikka
        -- 'policy_reply'        — liitteitä liikaa, templated reply (#46)
        -- 'permanent_skip'      — extraction.permanent_skip (single attachment)
        -- 'extraction_policy_skip'  — koko message-tason policy (#46)
        -- 'reply_sent'          — successful agent reply
        -- 'reply_truncated'     — MaxTokens, partial reply sent (#49)
        -- 'reprocess_requested' — asiantuntijan / käyttäjän pyyntö (Phase 5)
        -- 'reverted'            — asiantuntijan undo (Phase 4)
    decision_payload     JSONB,
        -- Esim. policy_reply:lle: {"max_attachments": 15, "actual": 22}.
        -- Reply_sent:lle: {"reply_message_id": "<...>"}.

    -- ── Linkit muihin tauluihin (kontekstuaalinen evidenssi) ───────
    attachment_id        BIGINT REFERENCES attachments(id) ON DELETE SET NULL,
    extraction_id        BIGINT REFERENCES extractions(id) ON DELETE SET NULL,
    receipt_id           BIGINT REFERENCES receipts(id) ON DELETE SET NULL,
    -- Liittyvä thread_messages-rivi. Käyttäjälle lähetetty vastaus
    -- on tunnistettavissa (direction='outbound') ja taustaviittaus
    -- agentin vastaukseen syntyy luonnollisesti.
    thread_message_id    BIGINT REFERENCES thread_messages(id) ON DELETE SET NULL,

    -- ── Yhteiset kentät ────────────────────────────────────────────
    duration_ms          INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    error                TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Per-run unique sequence — estää race condition jos kirjoittaja
    -- vahingossa toistaa saman seq:n.
    UNIQUE (run_id, seq),

    -- Soft consistency: jos kind = 'tool_use', tool_name on pakollinen.
    -- Käytämme CHECK:iä eikä erillisiä tauluja koska kategorioita on
    -- vain neljä ja niiden yhteiset kentät dominoivat.
    CHECK (
        (kind <> 'tool_use')
        OR (tool_name IS NOT NULL AND tool_use_id IS NOT NULL)
    ),
    CHECK (
        (kind <> 'llm_call')
        OR (stop_reason IS NOT NULL)
    ),
    CHECK (
        (kind <> 'decision')
        OR (decision_type IS NOT NULL)
    )
);

-- Stepin haku runin sisällä järjestyksessä — pääkäyttötapaus UI:ssa.
CREATE INDEX IF NOT EXISTS idx_agent_steps_run_seq
    ON agent_steps (run_id, seq);

-- Päätös-streamin haku ("kaikki tämän runin päätökset"):
CREATE INDEX IF NOT EXISTS idx_agent_steps_run_decisions
    ON agent_steps (run_id, seq)
    WHERE kind = 'decision';

-- Asiantuntijanäkymä: "kaikki epäonnistuneet tool_use-rivit ajalta X..Y"
CREATE INDEX IF NOT EXISTS idx_agent_steps_tool_errors_created
    ON agent_steps (created_at DESC)
    WHERE kind = 'tool_use' AND tool_is_error = true;

-- Tool-spesifinen haku: "kaikki save_receipt-kutsut joiden vendor on X"
-- — JSON-haku, ei rajoittavaa indeksiä Phase 1:ssä, lisätään vasta jos
-- konkreettinen kysely on toistuva. Esimerkki vasten tulevaisuutta:
--
--   CREATE INDEX idx_agent_steps_tool_input_path
--       ON agent_steps USING gin (tool_input jsonb_path_ops)
--       WHERE kind = 'tool_use';
--
-- Pidetään pois nyt — premature optimization.

-- Linkki receipts-tauluun: "mikä step loi tämän kuitin"
CREATE INDEX IF NOT EXISTS idx_agent_steps_receipt
    ON agent_steps (receipt_id)
    WHERE receipt_id IS NOT NULL;
```

**Huomiot:**

- Ei erillistä `agent_decisions`-taulua — `kind = 'decision'` riittää.
  Perustelu `analysis.md` §5.1.
- `tool_input` ja `tool_output` ovat JSONB. PII-katselu rajataan
  pääsynhallinnalla, ei sanitizationilla MVP:ssä. Cap noudattaa
  `cap_tool_result_json`:in 64 KB rajaa.
- `iteration` on NULL kun decision/error syntyy ennen LLM:ää (esim.
  spam_skip, policy_reply ennen agentin kutsua).
- `thread_message_id` linkki: kun agentti tuottaa vastauksen joka
  lähetetään, decision-rivi (`decision_type = 'reply_sent'`) viittaa
  `thread_messages`-riviin — UI saa kierrettyä trace → näkyvä viesti.

---

## Mitä **ei** ole muutettava (vain pointer)

Olemassa olevat taulut pysyvät ennallaan. Trace viittaa niihin:

- `attachments.id` — agent_steps.attachment_id
- `extractions.id` — agent_steps.extraction_id (sis. permanent skip
  stub-rivit `content_type='extraction_skipped'`)
- `receipts.id` — agent_steps.receipt_id
- `thread_messages.id` — agent_steps.thread_message_id
- `email_processing` — *ei* FK; pelkkä `message_id` pointer

---

## #38:n sidos (sub-issuena, ei tässä migraatiossa)

Kun #38 (receipt-revision-history) toteutetaan, sen `receipt_revisions`
-tauluun lisätään:

```sql
ALTER TABLE receipt_revisions
    ADD COLUMN created_by_run_id BIGINT REFERENCES agent_runs(id) ON DELETE SET NULL,
    ADD COLUMN created_by_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    ADD CONSTRAINT receipt_revisions_creator_check
        CHECK (
            (created_by_run_id IS NOT NULL AND created_by_user_id IS NULL)
            OR (created_by_run_id IS NULL AND created_by_user_id IS NOT NULL)
        );
```

**Tämä on #38:n migraatio**, ei #57:n. Mainittu tässä että suunnitelma
on tiedostossa ja #38:n omistaja näkee sidoksen.

---

## #26:n `audit_events`-sidos (Phase 4:ssä)

Kun asiantuntija peruuttaa agentin tekemän muutoksen UI:sta, muodostuu:

- Yksi `audit_events`-rivi (`action='revert_save_receipt'`,
  `metadata={"agent_run_id": <run_id>, "agent_step_id": <step_id>,
   "receipt_id": <receipt_id>}`)
- Yksi `agent_steps`-rivi `kind='decision'`, `decision_type='reverted'`
  joka kuuluu **uuteen pseudo-runiin** (esim. `model='manual-revert'`,
  `iterations=0`) jotta sama trace-näkymä kantaa myös manuaaliset
  muutokset

**Tämä on Phase 4:n yksityiskohta** — ei vaadi schema-muutoksia tässä
migraatiossa, mutta `agent_runs.model` ei ole CHECK-rajoitettu siksi
että pseudo-runit voivat käyttää erikoisarvoja.

---

## Migraatio-luonnos (käytetään Phase 2:ssa)

```sql
-- migrations/NNN_agent_trace.sql
-- Numero valitaan Phase 2:ssa kun #56 Phase 1:n yhdistetty schema on
-- olemassa.

-- pgcrypto extension tarvitaan gen_random_uuid:lle. #26 §4.x:n
-- alustaminen voi jo lisätä tämän — silloin tämä on no-op.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Lisää tähän taulujen CREATE-statementit yllä olevassa järjestyksessä:
--   1) agent_runs
--   2) agent_steps
--   3) indeksit kummallekin
```

---

## Esimerkki: yhden viestin trace

Demo-viesti, 5 liitettä, 1 hylätty (permanent skip), 4 tallennettu.

**`agent_runs` (1 rivi):**

| id | run_uuid | message_id      | model            | status     | iterations | total_input_tokens | total_output_tokens |
|----|----------|-----------------|------------------|------------|------------|--------------------|---------------------|
| 42 | 7f8c…   | `<abc@...>`     | claude-sonnet-4-6 | completed | 6          | 12450              | 3210                |

**`agent_steps` (esim. 13 riviä):**

| seq | kind      | iteration | tool_name           | decision_type      | attachment_id | extraction_id | receipt_id | duration_ms |
|-----|-----------|-----------|---------------------|--------------------|---------------|---------------|------------|-------------|
| 1   | llm_call  | 1         |                     |                    |               |               |            | 2300        |
| 2   | tool_use  | 1         | save_receipt        |                    | 101           | 201           | 301        | 45          |
| 3   | tool_use  | 1         | save_receipt        |                    | 102           | 202           | 302        | 38          |
| 4   | tool_use  | 1         | save_receipt        |                    | 103           | 203           | 303        | 41          |
| 5   | tool_use  | 1         | save_receipt        |                    | 104           | 204           | 304        | 39          |
| 6   | decision  | 1         |                     | permanent_skip     | 105           | 205           |            |             |
| 7   | llm_call  | 2         |                     |                    |               |               |            | 1800        |
| 8   | tool_use  | 2         | add_expense         |                    |               |               | 301        | 22          |
| ... | ...       | ...       | ...                 | ...                | ...           | ...           | ...        | ...         |
| 13  | decision  | 6         |                     | reply_sent         |               |               |            |             |

UI voi rakentaa selkokielisen kuvauksen tästä JOIN-tauluttamalla.

---

## Avoimet kohdat schema-luonnoksessa

Nämä kysymykset on toistettu `analysis.md` §7:ssä — vastaukset siellä
hyväksytään, sitten Phase 2 kirjoittaa konkreettisen `.sql`:n.

1. Hashataanko `tool_input.email_address` -tyyppisiä kenttiä? **Ei
   MVP:ssä** — pääsynhallinta hoitaa.
2. `api_request_id` Anthropic-API:sta — tallennetaanko jos saatavilla?
   **Suositus: kyllä, sarakkeena valmiina.**
3. JSON GIN-indeksit — **ei MVP:ssä**, lisätään kun konkreettinen
   kysely on toistuva.
