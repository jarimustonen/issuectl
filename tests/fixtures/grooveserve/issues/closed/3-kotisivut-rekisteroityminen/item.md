---
created: 2026-04-25
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
epic: 5
labels: [web, auth]
closed: 2026-04-26
commits:
  - hash: 526d809
    summary: "feat: architecture plan and image generation tooling"
  - hash: d1ff7ef
    summary: "feat: landing page, registration API, and deployment config"
  - hash: 3006afe
    summary: "feat: deploy and test full registration flow"
  - hash: 796be8a
    summary: "chore: add test-results to gitignore"
---

# 3. Kotisivut ja rekisteröityminen

_Source: käyttäjähallinta_

## Description

Grooveserve.com-kotisivut joilla palvelua esitellään ja joiden kautta käyttäjä voi rekisteröityä palvelun käyttäjäksi.

## Scope

- [x] Laskeutumissivu (palvelun esittely)
- [x] Rekisteröitymislomake
- [x] Käyttäjätietojen tallennus tietokantaan
- [x] Sähköpostivahvistus
- [x] Tervetuloviesti assistant@grooveserve.com -osoitteesta
- [x] Deployment-konfiguraatio (DNS, Ansible, nginx)
- [x] Deploymentin suorittaminen palvelimelle
- [x] Privacy policy ja käyttöehdot
- [x] Playwright E2E -testit (14 testiä)
