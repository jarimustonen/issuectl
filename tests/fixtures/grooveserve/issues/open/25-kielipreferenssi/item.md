---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#24"]
labels: [email, i18n]
---

# 25. Käyttäjän kielipreferenssin tunnistaminen ja tallentaminen

_Source: services/email_

## Description

Järjestelmän tulee vastata sillä kielellä, jota käyttäjä käyttää. Tällä hetkellä:
- System prompt ohjaa vastaamaan suomeksi, ellei käyttäjä kirjoita muulla kielellä
- HTML-template käyttää kovakoodattua `lang="fi"`
- Footer ja virheviestit ovat kiinteitä (suomi/englanti)

Tarvitaan mekanismi jolla:
1. Tunnistetaan käyttäjän kieli ensimmäisestä viestistä
2. Tallennetaan kielipreferenssi käyttäjäprofiiliin (DB)
3. Käytetään tallennettua kieltä vastauksissa ja templateissa
4. Käyttäjä voi vaihtaa kieltä milloin tahansa

## Vaikutukset

- `templates.rs`: `wrap_html` tarvitsee `lang`-parametrin
- `agent.rs`: system prompt voi ohjeistaa kielivalinnan perusteella
- `db.rs`: käyttäjäprofiiliin kielikenttä
- Footer/virheviestien lokalisointi
