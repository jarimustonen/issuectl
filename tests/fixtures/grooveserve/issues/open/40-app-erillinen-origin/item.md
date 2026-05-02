---
created: 2026-04-29
updated: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
labels: [architecture, web, multi-tenant]
related: ["#11", "#26"]
---

# 40. Käyttäjäsovellus erilliseen originiin (app./console./platform.)

_Source: in-app navigaation arkkitehtuuri_

## Description

Käyttäjien näkymä (kirjautuminen, sovellus, hallinta) on tällä hetkellä
samalla originilla kuin julkinen API ja samat HTML-sivut renderöidään
suoraan `services/api`-binäärin sisältä. Tuotantoarkkitehtuurissa
käyttäjäsovelluksen pitäisi olla **omalla aliverkkotunnuksella**,
esimerkiksi `app.grooveserve.com`, `console.grooveserve.com` tai
`platform.grooveserve.com`. Käytännössä se tarkoittaa että:

- Käyttäjäsovellus servataan omalta palvelimelta (oma deployment-yksikkö),
  ei `services/api`:n osana.
- API tarjoaa pelkän JSON-rajapinnan; HTML-renderöinti siirtyy pois
  api-palvelusta.
- Marketing-sivu (`grooveserve.com`) säilyy erillään myös käyttäjäpinnasta.

Tämä on edellytys monelle muulle työlle:

- Issue #11 (käyttäjälle näkyvä tositteiden web-UI) — tarvitsee oman
  origin/auth-pinon.
- Sessio-cookie-skooppi pitää olla `app.grooveserve.com`, jotta marketing
  ja API eivät jaa istuntoa.
- CORS- ja CSP-policy on yksinkertaisempi kun roolit ovat erillään.

## Scope (alustava)

- [ ] Päätä alustavalla tasolla: aliverkkotunnus (app vs console vs
      platform), jaettu-domain vs. eri TLD, cookie-domain.
- [ ] Erota käyttäjäsovellus omaksi paketikseen (todennäk.
      `sites/app/`) — todennäk. SPA samaan tapaan kuin `sites/www/`,
      mutta omilla auth-virroilla.
- [ ] `services/api` siivotaan: poistetaan `web.rs`,
      server-rendered admin-sivut, login-form, set-password-form jne.
      Niiden tilalle JSON-API:t joita SPA kuluttaa.
- [ ] Sähköposti-linkit (vahvistus, kutsu, salasanan vaihto) osoittavat
      `app.`-aliverkkoon, joka renderöi vastaavat sivut SPA-puolella.
- [ ] Local-dev: `gsdev`-templates lisätään kolmas palvelu (`app_port`,
      ensure-flow, mailpit-osoitteet).
- [ ] Tuotanto-deployment: oma palvelin / Cloudflare Pages -projekti
      `app.grooveserve.com`:lle, DNS-record `operations/cloudflare/config.yaml`,
      Ansible-playbook tai Pages-deploy-ohje.

## Out of scope

- Lopullinen päätös marketing-sivun migrointi pois Cloudflare Pagesista.
- Mobile-app, API-key-tokenit (eri issuet).

## Quick Test

Selain → `app.grooveserve.com`/`https://app.grooveserve.local` → SPA
latautuu, kirjautuminen onnistuu, sessio-cookie on skoopattu `app.`:iin
eikä vuoda marketing-sivulle.
