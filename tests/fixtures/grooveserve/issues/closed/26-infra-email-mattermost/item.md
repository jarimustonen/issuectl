---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: done
closed: 2026-04-26
priority: normal
labels: [infra, email, mattermost]
commits:
  - hash: ebb914b
    summary: "feat: add Mattermost, gsinfra CLI, jari@ email account, Roundcube SSL fix"
  - hash: 9ac5b33
    summary: "feat(email): add retry queue with backoff and Mattermost alerting"
---

# 26. Infrastructure: jari@grooveserve.com email + Mattermost alerting

_Source: operations_

## Description

Two infrastructure tasks:

### a) Create jari@grooveserve.com email account — DONE

- Created via `gsinfra mail create jari`
- Password stored in SOPS (`stalwart.enc.yaml`)
- Verified: IMAP, SMTP, Roundcube login all work
- Fixed Roundcube SSL peer verification (cert CN mismatch with `host.containers.internal`)

### b) Set up Grooveserve Mattermost — DONE

Deployed on the grooveserve server (204.168.196.71) instead of using Frondeo's instance.

- Mattermost at https://mattermost.grooveserve.com
- Podman container, nginx reverse proxy, Let's Encrypt TLS
- Uses existing PostgreSQL (new `mattermost` database)
- Grooveserve team, Alerts channel, incoming webhook
- Webhook posts as `grooveserve-bot`
- Admin credentials and webhook URL in SOPS (`mattermost.enc.yaml`)
- `gsinfra mattermost` CLI for user/team/webhook management
- DNS: `mattermost.grooveserve.com` A record in Cloudflare
