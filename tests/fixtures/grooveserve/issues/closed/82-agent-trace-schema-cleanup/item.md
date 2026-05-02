---
created: 2026-05-01
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: high
epic: 57
related: ["#56", "#57", "#58", "#62", "#60"]
labels: [agent, observability, schema, cleanup]
commits:
  - hash: TBD
    summary: "feat(agent_trace): add actor_user_id, nullable message_id (#82)"
---

# 82. agent_trace schema cleanup ennen Phase 4:ää

_Source: #62 LLM-review löydökset (1 + 2)_

## Description

`#62`:n implementoinnissa kaksi rakennetta päätyi paikkaan, joka
toimii tämän hetken testikäytössä mutta haittaa Phase 4:n trace-UI:ta
ja audit-konsumentteja:

1. **`agent_runs.model`-saraketta on ylikuormitettu**:
   `model = "manual:<actor_email>"` tunkee actor-identiteetin
   sarakkeeseen jonka semantiikka on "LLM-versio". Phase 4:n kuluttajat
   joutuisivat parsimaan `model.strip_prefix("manual:")`:n joka kerta,
   `SELECT DISTINCT model FROM agent_runs` -kyselyt vuotavat PII:tä
   raportteihin, eikä actor-perusteisiin kyselyihin ("kaikki Janen
   tekemät toimet") saa indeksiä ilman funktional-indeksiä.

2. **`agent_runs.message_id` on `NOT NULL`**: tämä pakotti `#62`:n
   syntetisoimaan `<manual-{trace_uuid}@grooveserve.local>`-
   placeholderin niille manuaalisille runeille joilla ei ole
   email-ankkuria. Placeholder roskaa `idx_agent_runs_message`-
   indeksin, rikkoo `LEFT JOIN thread_messages USING (message_id)`
   -joinit (silent NULL-join), ja vuotaa sisäisen `trace_id`:n
   sarakkeeseen jonka arvot ovat käyttäjälle näkyviä.

Molemmat löydökset ovat #62:n LLM-reviewin (gemini, claude, deepseek)
yhteinen kanta.

Referenssi review: `history/review-record-manual-run.md`,
`record_manual_run`-rajapinta `crates/ops/src/agent_trace.rs`:ssä,
schema migraatio `crates/ops/migrations/018_create_agent_trace.sql`.

## Scope

### Migraatio 021 — actor + nullable message_id

Yksi migraatio joka tekee molemmat kerralla, koska CHECK
`message_id IS NOT NULL OR actor_user_id IS NOT NULL` ankkuroi
nullable-message_id:n actor-saraketta vasten:

```sql
ALTER TABLE agent_runs
    ADD COLUMN actor_user_id BIGINT
        REFERENCES users(id);

-- Composite-FK:n muoto valittu siten että actor on samassa tenant_users
-- -kontekstissa. Yksinkertainen REFERENCES users(id) on minimaalinen;
-- jos halutaan tiukempi composite-FK, lisätään (actor_user_id, tenant_id)
-- -tuple ja vastaava ankkuri tenant_users:iin (vaihtoehto, päätös
-- migraation kirjoitusvaiheessa).

ALTER TABLE agent_runs
    ALTER COLUMN message_id DROP NOT NULL;

ALTER TABLE agent_runs
    DROP CONSTRAINT IF EXISTS agent_runs_message_id_check;
ALTER TABLE agent_runs
    ADD CONSTRAINT agent_runs_message_id_check
    CHECK (message_id IS NULL OR length(message_id) BETWEEN 3 AND 512);

ALTER TABLE agent_runs
    ADD CONSTRAINT agent_runs_anchor_check
    CHECK (message_id IS NOT NULL OR actor_user_id IS NOT NULL);
```

Olemassa olevat rivit ovat kaikki LLM-runeja → `actor_user_id IS NULL`
ja `message_id IS NOT NULL`, joten `agent_runs_anchor_check` läpäisee.

### Koodimuutokset

- `ManualRunInput`: pudotetaan `actor_email`-kenttä. Actor johdetaan
  `ctx.actor_user_id`:stä; kutsuja ei syötä sitä erikseen.
- `record_manual_run`:
  - `actor_user_id = ctx.actor_user_id` (uusi sarake).
  - `model = NULL` (tai `'manual'` — päätös toteutusvaiheessa, NULL on
    semanttisesti puhtaampi).
  - `message_id` ohjataan suoraan `Option<&str>`:nä SQL:lle ilman
    placeholder-syntetisointia.
  - `synthesized_message_id` -lohko poistuu kokonaan.
  - `trace_id` säilyy (ei kytköstä message_id:hen enää).
- `crates/ops/src/agent_trace.rs`:n module-rustdoc päivitetään: pois
  placeholder-kuvaus, pois `manual:` -prefix -konventio.
- Testit:
  - `manual_run_synthesizes_message_id_when_none` →
    `manual_run_with_no_message_id_stores_null`.
  - `manual_run_creates_completed_run_and_decision_step` rikkoutuu
    (model-tarkistus) — päivitetään.
  - `manual_run_trims_decision_type_and_actor_email_before_storage`:
    `actor_email`-trim-osa poistuu kun kenttä menee.
- `crates/ops/AGENTS.md`: agent_trace-osio päivittyy
  `actor_user_id`-saraketta ja `message_id`-nullable-tilaa kuvaamaan.

### Phase 4 -kuluttajien aggregointiohje

Sama doc-fix joka `#62`:ssa lisättiin `record_manual_run`-rustdociin
muotoon "filtteröi `WHERE model NOT LIKE 'manual:%'`" muuttuu muotoon
"filtteröi `WHERE actor_user_id IS NULL`" (= LLM-runit).

## Out of scope

- `KnownDecisionType`-enum — kuuluu `#60`:een (#60 owns the entire
  decision-row surface).
- Phase 4:n UI itse — tämä on infrastruktuuripuhdistus _ennen_ UI-
  työn alkua.

## Acceptance criteria

- [ ] Migraatio 021 lisätty + `crates/ops/migrations/`-numerointi
  oikein.
- [ ] `ManualRunInput.actor_email` poistettu, `record_manual_run`
  käyttää `ctx.actor_user_id`:tä.
- [ ] `agent_runs.message_id` voi olla NULL, placeholder-synteesi
  poistettu.
- [ ] `cargo test -p grooveserve-ops --tests` puhdas (testit päivitetty).
- [ ] `crates/ops/AGENTS.md` agent_trace-osio päivitetty.
- [ ] `record_manual_run`-rustdoc päivitetty (poistuu placeholder +
  `manual:`-prefix-kuvaus).

## Riippuvuudet

- **Estyy:** ei mikään (suora seuraaja `#62`:lle).
- **Estää:** Phase 4:n UI ja Phase 4:n trace-aggregointityö.

## Konteksti

`#62`:n review (`history/review-record-manual-run.md`) löysi nämä
arkkitehtuuriset ongelmat. `#62`:n issue-spec ohjasi nykyisen
toteutuksen (jätti schema-muutokset scope-outiksi), mutta review
osoitti että Phase 4:n vaatimukset eivät täytty placeholderilla +
prefix-kuvauksella. CLAUDE.md:n mukaisesti: "Päätökset ovat
ohjaavia, eivät sitovia" — review paljasti tilanteen jota #62:n
päätöksenteon hetkellä ei harkittu, joten päätös revisoidaan
tämän issuen kautta.
