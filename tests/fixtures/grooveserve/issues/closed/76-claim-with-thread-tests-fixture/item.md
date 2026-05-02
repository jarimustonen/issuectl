---
created: 2026-04-30
updated: 2026-05-01
closed: 2026-05-01
type: bug
reporter: jari
assignee: jari
status: fixed
priority: normal
epic: 56
related: ["#33", "#38", "#78"]
labels: [tests, devex]
commits:
  - hash: 9fac27e
    summary: "refactor(ops/server): relocate ingest DB lifecycle to ops::ingest::* (#78 A5a, #76)"
---

# 76. crates/server/tests/claim_with_thread.rs — 11 testiä punaisina mainissa

_Source: C2-receipt-revision-history -worktree (`#56` Worktree-loki, 2026-04-30). Pre-existing failure, ei C2:n aiheuttama._

## Description

`crates/server/tests/claim_with_thread.rs`:n 11 testiä epäonnistuu mainissa: testifixture ei honoroi A3:n migraatiopaketin lisäämää `tenants.slug NOT NULL`-rajoitusta. Yhteenveto C2-worktree-lokirivissä:

> Pre-existing failure: `crates/server/tests/claim_with_thread.rs` 11 testiä punaisina mainissa (`tenants.slug NOT NULL` ei honoroitu fixturessa) — ei C2:n aiheuttamaa, dokumentoitu tähän etteivät jää huomaamatta.

## Reproduction

```bash
cargo test -p grooveserve-server --test claim_with_thread
```

11 testin pitäisi epäonnistua "null value in column 'slug' violates not-null constraint" -virheellä (tai vastaavalla `tenants`-fixture-kontekstilla).

## Scope

- Tunnista fixture-helper joka luo testitenantin ilman `slug`:ia (todennäköisesti `crates/server/tests/common/`-moduulissa tai testien sisällä)
- Lisää `slug: Option<&str>` -parametri ja oletusarvona `format!("test-{uuid}")` -slug
- Aja `cargo test --workspace` puhtaana

## Acceptance

- `cargo test -p grooveserve-server --test claim_with_thread` 11/11 vihreänä
- `cargo test --workspace` puhtaana mainissa
- Lokaalissa worktreessä `cargo test` ei näytä punaisia rivejä jotka piilottavat oikeita regressioita

## Why this matters for #33 (local-dev)

Lokaali kehittäjä ajaa `cargo test --workspace` säännöllisesti — 11 punaista riviä piilottaa todelliset regressiot. Tämä on local-dev-DX:n ongelma vaikka itse korjaus on `crates/server`-puolella.

## Related

- A3 migraatiot 001–018 (`tenants.slug NOT NULL` lukittu)
- C2 (#38) landing-notes 2026-04-30: ensimmäinen havainto

## Resolution (2026-05-01, niputettu A5a:n kanssa)

Korjaus oli laajempi kuin pelkkä `slug`-puute: tarkempi diagnoosi
näytti että koko `users`-fixture oli pre-A3-shapesta (tenant_id /
email / role -sarakkeet jotka A3 poisti), ja `db::resolve_user` oli
sekin rikki post-A3:ssa. Niputettiin A5a:n kanssa koska A5a koskettaa
samoja testitiedostoja import-päivitysten yhteydessä.

Korjattu kolmessa testitiedostossa
(`crates/server/tests/{claim_with_thread, unknown_sender,
extraction_rescue}.rs`):

- `create_tenant`: `slug TEXT NOT NULL UNIQUE` -arvo
  (`format!("test-{name}-{uuid}")`) + `status = 'active'`
  (default `'pending_verification'` ei läpäise A5a:n
  uutta agent-puolen status-filteröintia).
- `create_user`: post-A3 -kanoninen kolmen taulun fixture
  (`users` + `tenant_users` + `user_emails` -insertit).

Server-puolelta:

- `crates/server/src/ingest/db.rs::resolve_user` delegoi nyt
  `grooveserve_ops::user::find_user_by_email`-pintaan; A3:n yhteydessä
  rikkoutunut `SELECT tenant_id, id FROM users WHERE email = $1`
  -kysely oli ainoa polku jonka kautta runner.rs (ja testit)
  resolvoivat lähettäjän → tämä oli silent breakage joka näkyi vasta
  A5a:n testiajossa. Resolve-pinta enforcaa nyt `email_verified &&
  membership_status='active' && tenant_status='active'` -tarkistuksen
  agent-puolelle (canonical `find_user_by_email` palauttaa rivit
  kaikissa tiloissa, koska web-login tarvitsee disabled vs. unknown
  -erottelun).

Acceptance:

- `cargo test -p grooveserve-server --test claim_with_thread` 11/11 vihreänä
- `cargo test --workspace` puhtaana paitsi yksi pre-existing
  `tools_snapshot::anthropic_tools_matches_snapshot` -failure
  (verifioitu ettei A5a:n aiheuttama `git stash`-testillä baseline-commitilla
  `da10dc3`).
- Lokaalissa worktreessä `cargo test` ei näytä punaisia rivejä jotka
  piilottavat oikeita regressioita (tämä `tools_snapshot` -rivi siirtyy
  omaan issueeseen kun se tutkitaan).
- Lisätty 5 anti-regression-testiä `unknown_sender.rs`:iin
  (`invited_user_does_not_resolve`, `disabled_user_does_not_resolve`,
  `unverified_email_does_not_resolve`,
  `pending_verification_tenant_does_not_resolve`,
  `multi_tenant_user_surfaces_as_invalid_input`,
  `known_sender_signal_rejects_inactive_states`) jotka kiinnittävät
  identity-filterin ettei se rapistu.
