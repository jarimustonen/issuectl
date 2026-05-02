---
created: 2026-04-29
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: normal
epic: 57
related: ["#56", "#57", "#58", "#26", "#60", "#82"]
labels: [agent, observability, audit]
commits:
  - hash: 486aa17
    summary: "feat(ops/agent_trace): add record_manual_run for Phase 4 manual runs"
  - hash: 3d3259b
    summary: "fix(ops/agent_trace): trim decision_type/actor_email + doc fixes from /llm-review"
---

# 62. Manuaalisen runin pseudoluonti (Phase 4 ennakointi)

_Source: #57 Phase 2, alaissue 5/5_

## Description

Lisää `AgentTraceWriter`:lle `record_manual_run(...)`-rajapinta, jolla
asiantuntijan UI (Phase 4) tai manuaaliset CLI-toiminnot voivat luoda
`agent_runs`-rivin joka edustaa **ei-LLM-suoritusta** (esim.
asiantuntijan undo, manuaalinen revert, käyttäjän web-UI:sta tehty
korjaus).

Tämä mahdollistaa että trace-näkymä Phase 4:ssä näyttää sekä
LLM-runit että manuaaliset toimenpiteet samassa aikajanassa, ilman
erillistä taulupintaa.

UI ja varsinaiset undo-toiminnot **eivät** ole tämän issuen scope —
vain rajapinta ja sen testaus.

Päätökset: `issues/open/57-…/analysis.md` §5.5 (suhde
`audit_events`:iin), `schema-draft.md` "audit_events-sidos" -osio.

## Scope

- `AgentTraceWriter::record_manual_run(...)`:
  - Parametrit: tenant_id, user_id, actor_user_id, message_id?,
    thread_id?, decision_type, decision_payload, linked_receipt_id?,
    linked_extraction_id?, linked_attachment_id?
  - Luo `agent_runs`-rivin: `model='manual:<actor_email>'`,
    `iterations=0`, `status='completed'`,
    `total_input_tokens=0`, `total_output_tokens=0`
  - Luo yhden `agent_steps`-rivin: `kind='decision'`,
    `decision_type` = annettu (esim. `'reverted'`, `'manual_correction'`,
    `'reprocess_requested'`)
  - Palauttaa `RunId` jotta kutsuja voi linkata sen
    `audit_events.metadata.agent_run_id`-kenttään
- Integraatiotesti joka demonstroi:
  - Manuaalinen revert luo run + step
  - Run näkyy samassa `WHERE message_id = ?`-haussa kuin LLM-runit
- Dokumentaatiokommentti rajapintaan: viittaus `audit_events`:iin
  (#26 §4.2) — miten kutsuja kytkee ne yhteen

## Decision_type-arvot jotka `record_manual_run` hyväksyy alkuun

- `reverted` — undo agentin teosta
- `manual_correction` — käyttäjän/asiantuntijan korjaus
- `reprocess_requested` — käyttäjän pyyntö uudelleenajosta

Lista laajenee kun Phase 4:n UI:n toiminnot tarkentuvat — älä lukitse
listaa CHECK-rajoituksella. (Schema-draft.md jättää
`decision_type`:in ilman CHECK:iä juuri tästä syystä.)

## Out of scope

- Asiantuntijan UI (Phase 4)
- Korjauskanavat (Phase 5)
- `audit_events`-rivien luonti — kuuluu kutsujalle (UI / endpoint)

## Riippuvuudet

- **Estyy:** #58 (writer-rajapinta)
- **Estää:** Phase 4 UI (vaihtoehtoisesti UI voi suoraan kirjoittaa
  agent_runs/steps-rivit, mutta `record_manual_run` on selkeämpi
  rajapinta — tämä issue siis ei ole pakollinen Phase 4:lle, vain
  hyvä ennakointi)

## Acceptance criteria

- `record_manual_run(...)` luo run + step-rivit oikealla statuksella
- Integraatiotesti vihreä
- Dokumentointi viittaa `audit_events`:iin ja näyttää esimerkkinä
  miten ne yhdistyvät metadatassa
