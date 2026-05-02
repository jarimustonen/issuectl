---
created: 2026-05-01
updated: 2026-05-01
type: task
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#73", "#26", "#38", "#57"]
labels: [audit, security, compliance]
---

# 81. Käyttäjän actioneista lähtevän audit trailin tarkastus ja varmistus

_Source: #73 `/llm-review` -kierroksen DISCUSS-päätös (2026-05-01)._

## Description

#73:n yhteydessä nousi DISCUSS-keskustelu siitä, pitäisikö
`AddExpenseInput.message_id` olla pakollinen (kuten
`SaveReceiptInput.message_id`). Päätös: **valinnainen pysyy**, koska
expenses voi tulla useasta lähteestä eikä synteettistä message_id:tä
tarvitse mintata kun jokaisella kanavalla on muu tunnistustapa.

Mutta tämä paljastaa laajemman kysymyksen: **onko meillä riittävä
audit trail siitä, mistä jokainen domain-mutaatio (receipt /
expense / user-profile / tenant-asetus) on alunperin lähtöisin?**
Tieto pitää olla rekonstruoitavissa myös ilman message_id:tä —
web-action, agent-toolikutsu, gsadmin-CLI, manuaalinen DB-edit.

Käyttäjän maininta: "Sellainen käsittääkseni jo jollakin tasolla
on, mutta se tulee verifioida ja tarkastaa."

## Scope

### Inventaari

Kartoita mitä audit-rakenteita tällä hetkellä on käytössä:

- `crates/ops/src/audit.rs` — `audit_events`-taulu + `record` /
  `record_with_email`. Nykyinen käyttö (grep `audit::record`):
  - `password_reset.rs`
  - `tenant.rs`
  - `auth.rs`
  - `user.rs`
  - `invitation.rs`
  
  **Eli identity-domain on katettu** (login, role-changes, kutsut,
  password-reset). Receipts / expenses / extractions / attachments
  **eivät kirjoita audit_eventseihin tällä hetkellä.**

- `crates/ops/src/agent_trace.rs` — `agent_runs` / `agent_steps`
  (#57 D-aalto). Kattaa LLM-loopin kutsut: jokainen
  agent-tool_use-kutsu (esim. `save_receipt`, `add_expense`)
  saa `tool_use`-rivin agent_steps-tauluun. Tämä antaa
  email-kanavan domain-mutaatioille trace:n mutta **ei** kata
  web-handler-kutsuja eikä gsadmin-CLI-kutsuja.

- `crates/ops/src/receipts/revision.rs` — `receipt_revisions`-
  taulu (#38, C2). Kantaa jokaisen kuittirivin pre-state-
  snapshotin + `captured_by_message_id` + `captured_by_tool`
  -kentät. **Tämä on revision-historia, ei audit trail** — kertoo
  *mitä* muuttui, mutta ei *miksi* tai *kuka* (vain kuva
  edellisestä tilasta).

### Gap-analyysi

Mitä **puuttuu** kun verrataan "user-actioneista lähtevä audit trail"
-vaatimukseen:

1. **Receipts / expenses / extractions web-päivitykset** ilman
   email-kanavan trace:a — esim. käyttäjä avaa web-UI:n (Phase 3)
   ja korjaa kuittinsa. Mistä polusta tieto on peräisin? Onko
   audit-rivi joka kertoo "alice korjasi kuitin #42 weballa
   2026-05-01 14:23"?
2. **gsadmin-CLI-kutsut** — admin manipuloi käyttäjätietoja tai
   tenant-asetuksia. Tällä hetkellä `gsadmin password-reset` ja
   tenant-luonti tallentavat audit_events-rivit, mutta esim.
   manuaaliset SQL-korjaukset (jos sellaisia tehdään) eivät
   jätä jälkeä.
3. **`OpContext.channel`** — neljä kanavaa (`Web`, `EmailAgent`,
   `EmailIngest`, `Internal`). Audit-rivit tallentavat tämän,
   mutta domain-mutaatiot (receipt-write yms) eivät tällä
   hetkellä kirjoita audit-riviä lainkaan, joten kanava-tietoa
   ei näy mistään.

### Tehtävät

1. **Inventaari-dokumentti** `analysis.md` — mitä audit-mekanismeja
   on käytössä, mitä taulua kukin kirjoittaa, mitä kanavia kukin
   kattaa.
2. **Vaatimusten kirjaus** — mistä toiminnoista *pitäisi* jäädä
   audit-jälki? Verohallinto-vaatimukset, GDPR-vaatimukset, oma
   policy. Vrt. #36 (privacy-review) ja #67 (data-visibility).
3. **Gap-listaus** — eroittele FIX-tason puutteet (esim.
   "save_receipt ei kirjoita audit-riviä lainkaan") ja
   nice-to-have -lisäykset.
4. **Toteutus-ehdotus** — yksittäinen issue per FIX-tason puute,
   tai kokoava epic jos puutteita on useita.

## Out of scope

- `agent_trace`-kerroksen toteutus (#57 D-aalto, käynnissä).
- `receipt_revisions`-taulun laajennus (#38, valmis).
- `expense_revisions`-historia (jos tarvetta — oma issue).

## Dependencies

- **#26** (multi-tenant käyttäjähallinta) — `OpContext.channel`-
  semantiikka on tästä peräisin; auditin kanavakohtainen politiikka
  pohjautuu #26:n design.md §2:een.
- **#38** (receipt-revision-history) — kuittien revision-historia
  *ei* korvaa audit-trailia, mutta on toinen kerros samaa
  rekonstruktio-pinnasta.
- **#57** (auditoitavuus / asiantuntijanäkymä) — agent_trace
  kattaa LLM-loopin osuuden audit-pinnasta. Tämä issue kattaa
  *muut* polut (web, gsadmin, internal).
- **#67** (data-visibility / access-control) — sukulaisteema:
  audit kertoo *mitä tehtiin*, access-control kertoo *kuka saa
  nähdä mitä*.

## Acceptance

- [ ] `analysis.md` valmis — inventaari + gap-lista
- [ ] Päätös: yksi kokoava epic vai monta pientä issuea
  (riippuu gap-listan koosta)
- [ ] Vähintään `save_receipt` / `update_receipt` /
  `add_expense` / `update_expense` -auditpolku selvitetty
  (kirjoittaako, mihin, mitä kanavaa varten)
