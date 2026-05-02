---
created: 2026-04-25
updated: 2026-04-25
type: task
reporter: jari
assignee: jari
status: done
closed: 2026-04-25
priority: high
epic: 5
labels: [infra, email]
commits:
  - hash: 6b1951a
    summary: "feat: add infrastructure for grooveserve.com"
  - hash: db7bf2e
    summary: "feat: email processing service with Mailgun integration"
  - hash: 9afda0e
    summary: "feat: Stalwart mail server infrastructure"
  - hash: 89fd6fc
    summary: "feat: rewrite email service from Mailgun to IMAP/SMTP"
  - hash: fe84d17
    summary: "docs: Stalwart operational knowledge and email architecture"
  - hash: 8cbdf01
    summary: "feat: multi-account email monitoring (healthcheck@, assistant@, postmaster@)"
  - hash: d01e617
    summary: "docs: update issue #1 with commit history and scope progress"
  - hash: 1d560bb
    summary: "chore: Roundcube, loglevel, DKIM/DMARC verification, issue #13"
---

# 1. Sähköpostipalvelun pystytys

_Source: grooveserve.com infra_

## Description

Pystytetään sähköpostipalvelu grooveserve.com-domainille niin, että järjestelmä voi vastaanottaa ja lähettää sähköpostia.

## Scope

- [x] MX-tietueiden konfigurointi grooveserve.com:ille
- [x] SPF-tietue
- [x] DKIM-allekirjoitus
- [x] DMARC-policy
- [x] TLS pakotus vastaanottopäässä
- [x] Inbound-sähköpostin vastaanotto ja prosessointi ohjelmallisesti
