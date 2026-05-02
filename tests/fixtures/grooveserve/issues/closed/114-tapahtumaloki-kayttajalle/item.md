---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: fixed
priority: normal
epic: 56
related: ["#57", "#11"]
labels: [web, agent-trace, user-facing]
closed: 2026-05-01
commits:
  - hash: e8954bd
    summary: "feat(events): user-facing event log read surface and web UI (#114)"
  - hash: 672be02
    summary: "fix(events): apply #114 review findings — XSS, pagination, status derivation, SQL optimization"
---

# 114. Tapahtumaloki / agenttihistoria käyttäjälle ("mitä agentti teki viestilläni")

_Source: #56 Phase 3 UUSI-issuena, kattaa myös #57 Phase 3:n "Käyttäjän tapahtumaloki" -kohdat._

## Description

Käyttäjä lähettää sähköpostin liitteineen ja saa vastauksen, mutta agentin
työvaiheet (mitä työkaluja kutsuttiin, mitkä päätökset tehtiin, miksi joku
liite jätettiin pois, mitä tositteita syntyi) ovat tällä hetkellä
näkymättömissä. Tämä issue rakentaa **käyttäjän web-UI:n näkymän** joka
kertoo selkokielisesti agentin toimet per inbound-viesti.

Tietolähteet ovat valmiina:
- `agent_runs` + `agent_steps` -taulut (D-aalto, #58–#62)
- `email_processing` per inbound-message status
- `extractions` ja `receipts` per attachment
- `pending_admin_actions` (jos hallinto-operaatio kanavasta)

## Scope

- [x] **Lista "viimeisimmät viestini"** käyttäjän omilla viesteillä
  (`/me/events` tai `/events` user-routessa). Pagination.
- [x] **Yksittäisen viestin näkymä** (`/me/events/:message_id` tai
  `/events/:message_id`):
  - Otsikko + lähetysaika + status (`reply_sent` / `unknown_sender` /
    `spam_skip` / `aborted_*` / `failed_*` / `pending_admin_confirmation`)
  - Selkokielinen yhteenveto: "Saimme viestin → tunnistettiin 5 liitettä →
    luettiin 4 kuitiksi → 1 ohitettu (ei kuitti)"
  - Linkit syntyneisiin tositteisiin (vie #11:n tositenäkymään)
  - Aikajana `agent_steps`-pohjalta (LLM-kutsut, tool-kutsut, decision-
    rivit) — tiivistettynä, ei kaikkia stepejä raakana
  - "Pyydä korjausta" -nappi joka avaa tositteen muokkaussivun (#115)
- [x] **i18n** (en/fi/sv) — kuten muutkin Phase 3 -näkymät
- [x] **Auth-rajaus**: `(ctx.tenant_id, ctx.actor_user_id)`-skooppinen
  luenta. #67 v1.1 -policy lukitsee — emme näytä toisen käyttäjän viestejä
  edes saman tenantin sisällä.

## Out of scope

- Asiantuntijan ristiin-näkymä (#57 Phase 4) — eri rooli (`#86` aka
  data-visibility -policyssa "expert reviewer").
- Kuittien muokkaus webistä — eriytetty issue **#115**.
- Agent-tason raakana näkyvät prompit / LLM-tokenit — käyttäjälle
  selkokielistettyä, ei diagnostiikkaa.

## Files to Examine

- `crates/ops/src/agent_trace.rs` — writer-pinta (D2:sta), reader-pinta
  on tässä lisättävä — todennäköisesti `crates/ops/src/agent_trace/view.rs`
  uudella moduulilla
- `crates/server/src/http/routes/` — uusi `me/events` -submoduuli
- `crates/server/src/http/routes/receipts.rs` — viite jo existing
  user-route-pinnasta
- `issues/open/57-auditoitavuus-asiantuntijanakyma/item.md` Phase 3 -lista
- `issues/open/57-auditoitavuus-asiantuntijanakyma/schema-draft.md` —
  agent-trace-schema
- `issues/open/56-toimiva-testattava-perusta/item.md` Phase 3
- `crates/ops/AGENTS.md` ja `crates/server/AGENTS.md`

## Acceptance criteria

- `GET /me/events` näyttää käyttäjän viimeisimmät viestit pagination:lla
  (50/sivu).
- `GET /me/events/:message_id` näyttää selkokielistetyn aikajanan +
  linkit tositteisiin.
- 8–12 sqlx-/integration-testiä: happy path, cross-tenant reject,
  cross-user same-tenant reject (per #86 policy), useita liitteitä,
  permanent_skip-rivien näkyminen, `aborted_max_iterations`-statuksen
  näyttö.
- i18n-stringit en/fi/sv.
- `cargo test --workspace --tests` puhdas.
- AGENTS.md:t päivitetty.

## Notes

Tämä on **#57 Phase 3:n** käytännön toteutus #56-puolen UI-rungon
päälle — eli sama toiminnallinen issue kahdesta kulmasta. Pidetään
yhdessä alaissueena.
