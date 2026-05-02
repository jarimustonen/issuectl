---
created: 2026-04-26
updated: 2026-04-26
type: task
reporter: jari
assignee: jari
status: done
priority: high
labels: [infra, deployment]
closed: 2026-04-26
---

# 23. Cross-compile pipeline for email service

_Source: operations/infra/ansible/roles/email-service_

## Description

`cargo build --release` palvelimella (CX23, 4GB RAM) kuluttaa yli 4GB muistia ja OOM killer tappaa prosessin. Ansible-playbook failaa kohdassa "Build container image".

Ratkaisu: cross-compile paikallisesti (macOS Apple Silicon -> Linux x86_64) ja lähetä valmis binary palvelimelle.

## Solution

- `build-linux.sh` — Podman-pohjainen cross-compile (ARM64 Rust + x86_64 cross toolchain)
- OpenSSL vendored (rakennetaan lähdekoodista cross-compilessa)
- Runtime-only Dockerfile (debian:trixie-slim + binary, ~100MB)
- Ansible-rooli päivitetty: lähettää binaryn, ei yritä buildata palvelimella

## Files changed

- `services/email/build-linux.sh` (new)
- `services/email/Dockerfile.builder` (new)
- `services/email/Dockerfile` (updated — runtime-only)
- `services/email/Cargo.toml` (updated — vendored OpenSSL)
- `operations/infra/ansible/roles/email-service/tasks/main.yml` (updated)
- `services/email/AGENTS.md` (updated)
- `operations/AGENTS.md` (updated)
