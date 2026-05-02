---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26", "#4"]
labels: [email, auth]
---

# 28. Email alias / plus-addressing -käsittely

_Source: #26 LLM review_

## Description

Sähköpostin plus-osoitteet (esim. `matti+kuitit@firma.fi` vs `matti@firma.fi`) käsitellään tietokannassa erillisinä osoitteina. Tämä voi aiheuttaa ongelmia käyttäjän tunnistuksessa jos käyttäjä lähettää viestejä eri plus-varianteilla.

MVP:ssä tämä dokumentoidaan tietoisena rajoitteena. Myöhemmin voidaan harkita normalisointia.

## Scope

- [ ] Dokumentoida MVP-rajoitus: plus-osoitteet ovat erillisiä
- [ ] Harkita normalisointia: stripataanko `+tag` ennen tallennusta/hakua
- [ ] Huomioida provider-kohtaiset erot (Gmail vs. O365 vs. muut)
- [ ] Mahdollinen varoitus admin-portaalissa kun kutsussa on plus-osoite
