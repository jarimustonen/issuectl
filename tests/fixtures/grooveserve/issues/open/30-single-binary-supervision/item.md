---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#26"]
labels: [infra, reliability]
---

# 30. Single binary -vikaantumismalli ja supervision

_Source: #26 LLM review_

## Description

Grooveserve MVP käyttää yhtä binääriä sekä email-loopille (IMAP/SMTP) että HTTP-palvelimelle (axum). Tämä tarkoittaa, että yhden osan kaatuminen kaataa kaiken. Tarvitaan suunnitelma vikaantumisen hallintaan.

## Scope

- [ ] Graceful shutdown -mekanismi (tokio::select!, shutdown signal)
- [ ] Erillisten task-ryhmien supervision (HTTP ja email erillisinä tokio-taskeina)
- [ ] Health check -endpointti joka erottaa HTTP:n ja email-loopin tilan
- [ ] DB pool -konfiguraatio yhdistettyä kuormaa varten
- [ ] Deployment-strategia: miten päivittää ilman email-prosessoinnin katkeamista
- [ ] Dokumentoi milloin jakaa kahdeksi binääriksi (shared ops-crate)
