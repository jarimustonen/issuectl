---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#3", "#4", "#6", "#26"]
labels: [web, auth, admin]
---

# 22. Käyttäjähallinta — pääkäyttäjän ylläpitonäkymä

_Source: service.grooveserve.com_

## Description

Pääkäyttäjä (yrityksen admin) voi hallita organisaationsa käyttäjiä: lisätä, muokata, poistaa ja hallita rooleja. Tämä on osa service.grooveserve.com -sovellusta.

## Scope

- [ ] Käyttäjälistanäkymä (pääkäyttäjälle)
- [ ] Käyttäjän lisääminen (nimi, sähköposti, rooli)
- [ ] Käyttäjän muokkaaminen (tiedot, rooli, aktiivisuus)
- [ ] Käyttäjän poistaminen / deaktivointi
- [ ] Roolien hallinta (admin, käyttäjä, hyväksyjä)
- [ ] Organisaation asetukset (nimi, laskutusosoite, hyväksyntäkierto)
- [ ] Pending-operaatiot: sähköpostiagentti-kanavasta tehdyt admin-operaatiot voivat vaatia web-vahvistuksen
  - [ ] Pending admin action -taulun suunnittelu (action_type, payload, token, status, expires_at)
  - [ ] Kanavakohtainen politiikka: mitkä operaatiot vaativat vahvistuksen sähköpostikanavasta
  - [ ] Web-vahvistussivu: admin klikkaa linkkiä → kirjautuu → vahvistaa operaation
  - [ ] Agentti palauttaa "vahvistuslinkki lähetetty" -viestin pending-operaatioista

## Notes

- Liittyy käyttäjän tunnistamiseen (#4) ja onboarding-flowiin (#6)
- Pääkäyttäjä on ensimmäinen rekisteröitynyt käyttäjä tai erikseen nimetty
- MVP:lle riittää yksinkertainen CRUD-näkymä
