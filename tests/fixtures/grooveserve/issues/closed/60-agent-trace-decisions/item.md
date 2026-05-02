---
created: 2026-04-29
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: high
epic: 57
related: ["#56", "#57", "#58", "#59", "#46", "#49"]
labels: [agent, observability]
commits:
  - hash: 1611548
    summary: "feat(agent_trace): pre-/post-LLM decision rows on the seam (#60)"
  - hash: 53f1ed0
    summary: "fix(agent_trace): apply review fix-up bundle (#60)"
---

# 60. agent_trace decision-rivit pre-LLM + post-LLM reunoilla

_Source: #57 Phase 2, alaissue 3/5_

## Description

Lisää `agent_steps.kind = 'decision'` -rivit kaikkiin niihin
`process_message_inner`:n / `process_assistant_reply`:n /
`extraction.rs`:n reunoihin joissa syntyy nimenomainen päätös.
Tämä on **asiantuntijan UI:n perusta** (Phase 4) — ilman näitä rivejä
trace ei vastaa kysymykseen "mikä päätös tämän käsittelyn aikana
tehtiin".

Decision-tyyppien taksonomia: `issues/open/57-…/schema-draft.md`
`agent_steps.decision_type`-sarakkeen kommentit.

## Scope

Lisättävät päätös-rivit, sijainnit ja `decision_type`-arvot:

| Sijainti                                                          | decision_type             | Konteksti |
|-------------------------------------------------------------------|---------------------------|-----------|
| `process_message_inner`, ennen LLM:ää, hard-spam (DMARC reject)   | `spam_skip`               | #46:n spam-haara |
| `process_message_inner`, unknown sender (#43-politiikka)          | `unknown_sender`          | `skip_unknown_sender` haara |
| `process_message_inner`, `extraction.policy_skip`-haara `CanReply`:llä | `policy_reply`        | #46 round-3: liitteitä liikaa, templated reply |
| `extraction.rs::persist_permanent_skip`                           | `permanent_skip`          | #46 round-3: per liite (size, MIME, Anthropic 4xx) |
| `process_assistant_reply`, onnistuneen replyn jälkeen             | `reply_sent`              | normaali happy path |
| `process_with_tools` MaxTokens-haara                              | `reply_truncated`         | #49 |

Kaikki decision-rivit kantavat:
- `attachment_id` / `extraction_id` / `receipt_id` /
  `thread_message_id` -linkit kun relevantti (esim. permanent_skip
  → `attachment_id` + `extraction_id` stub)
- `decision_payload` JSONB:nä jos lisätieto auttaa UI:ta (esim.
  `policy_reply`: `{"max_attachments": 15, "actual": 22}`)

## Erikoiskäsittely: pre-LLM-päätökset

Jos viesti hylätään ennen LLM:ää (spam_skip, unknown_sender,
policy_reply), `agent_runs`-rivi luodaan silti — `model='none'`,
`iterations=0`, `status='completed'`. Tämä pitää trace-näkymän
yhtenäisenä: jokainen viesti kantaa run-rivin, vaikka agentti ei
ajanutkaan.

## Out of scope

- Error/abort-statukset — #61
- Manuaaliset undot — #62
- UI Phase 4

## Riippuvuudet

- **Estyy:** #58, #59
- **Estää:** Phase 4 (asiantuntijan UI)

## Acceptance criteria

- Demoa ajaessa (#46 reproduktio) syntyy:
  - `decision_type='permanent_skip'` rivi per skipattu liite
  - `decision_type='policy_reply'` rivi kun rajat ylittyvät
  - `decision_type='reply_sent'` rivi happy path:lla
- Pre-LLM-hylkäykset (spam_skip, unknown_sender) tuottavat
  `agent_runs`-rivin statuksella `completed`, `iterations=0`
- Asiantuntijakysymys "kaikki tämän viikon permanent_skipit"
  vastattavissa yhdellä SQL-kyselyllä:
  `SELECT * FROM agent_steps WHERE kind='decision' AND decision_type='permanent_skip' AND created_at > '...'`
