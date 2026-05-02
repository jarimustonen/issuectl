---
created: 2026-04-26
updated: 2026-04-26
type: chore
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
labels: [infra, devops]
---

# 23. Versiointi ja release-prosessi sisäisille komponenteille

_Source: operations, services_

## Description

Monorepon palveluilla ja komponenteilla ei ole versionumerointia eikä release-prosessia. Kaikki deployataan suoraan mainista. Tarvitaan järjestelmällinen tapa seurata mitä versiota palvelimella pyörii, mahdollistaa rollback, ja pitää changelog ajan tasalla.

## Scope

- [ ] Versionumerointistrategia (semver, calver, git-tag-pohjainen?)
- [ ] Komponentit joita versioidaan: email-service, www-sivusto, API (myöhemmin), Ansible-roolit
- [ ] Git-tagit releaseille (esim. `email-v0.1.0`, `www-v0.1.0`)
- [ ] Version embedding buildiin (binary tai env muuttuja kertoo version)
- [ ] Deployment tracking: miten tiedetään mikä versio on palvelimella
- [ ] Rollback-mekanismi (edellisen binaryn/imagen säilytys)
- [ ] Changelog-generointi (git log → changelog, tai manuaalinen)

## Vaihtoehtoja

1. **Git tag + Cargo.toml version** — yksinkertainen, semver, `git tag email-v0.1.0`
2. **Calver** (2026.04.1) — helppo, ei tarvitse miettiä breaking changes
3. **Git SHA -pohjainen** — automaattinen, mutta ei ihmisluettava
4. **release-please / semantic-release** — automaattinen versiointi commit-viesteistä

## Notes

- MVP:lle riittää git-tagit + Cargo.toml version. Monimutkaisempi automatiikka myöhemmin.
- Cross-compile pipeline (`build-linux.sh`) voisi embedata version binaryyn
- Palvelimella `grooveserve-email --version` kertoisi version
