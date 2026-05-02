---
created: 2026-04-30
updated: 2026-04-30
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#78", "#56"]
labels: [foundation, workspace, tech-debt]
---

# 79. Ops-crate split planning — `ops-core` / `ops-identity` / `ops-finance` / `ops-ingest`

_Source: B2-worktreen `/llm-review` #78:lle (2026-04-30). Anthropic-reviewerin nostama, validoitu Gemini + DeepSeek -kierroksella._

## Description

`crates/ops` on alkanut kasaantua. Tällä hetkellä **21 moduulia**:

```
agent_trace, attachments, audit, auth, context, email, error, extractions,
ingest/, invitation, locale, password, password_reset, receipts/,
schema_constraints, session, tenant, token, user, validate
```

A5 (#78) lisää 4 `ingest/`-alimoduulia (`lifecycle`, `threading`, `conversation`, `session_context`) → **25 moduulia**. C/D-aaltojen jatkot vievät 30+:aan vuoden lopussa.

Neljä loogista domainia ovat sekoittuneet yhteen crateen:

| Domain | Nykyiset moduulit |
|--------|-------------------|
| **identity** | auth, session, password, password_reset, invitation, user, tenant, token, validate, locale |
| **finance** | receipts, attachments, extractions (+ tulevat expenses, currency, tax) |
| **ingest** | ingest/, agent_trace |
| **infra** | audit, context, error, email, schema_constraints |

## Konkreettiset oireet (jotka näkyvät myöhemmin)

1. **Build-aika kasvaa.** Cargo:n yksikkö on crate, ei moduuli. Yhden moduulin muutos uudelleenkääntää koko cratin. 30 moduulin kohdalla `cargo build -p grooveserve-ops` kylmänä on 30+ s. Iteraationopeus kärsii etenkin sqlx-testien ajossa.
2. **Test-suite paisuu yhdeksi**. `cargo test -p grooveserve-ops` ajaa kaikki sqlx-testit (nyt ~100, projektion mukaan 200+). Identity-testin debugaaminen kärsii rinnakkaisten finance- ja ingest-testien hitaudesta.
3. **Riippuvuusgraafi on implisiittinen.** Yksi-cratin sisällä kaikki on `pub(crate)`-näkyvyydellä saatavilla. Domain-rajat eivät pakota puhtautta — `ops::receipts` voi vahingossa importata `ops::auth`:in apufunktion ilman että `Cargo.toml`-riippuvuus näyttää sen.
4. **PR-konfliktit kasvavat.** Kaksi rinnakkaista worktreeta (esim. C3 monivaluutta ja A5b conversation-pinta) muokkaavat `crates/ops/src/lib.rs`:n re-exporteja → triviaali merge-konflikti aina.
5. **CI test-matriisi ei skaalaudu.** Per-domain test-suiten ajo vaatii `cargo test -p ops --test foo` -yksityiskohtia; per-crate-jako olisi `cargo test -p ops-identity` puhtaasti.

## Ehdotettu jako

```
crates/
├── ops-core/        # OpContext, OpError, audit, validate, schema_constraints, locale, email
├── ops-identity/    # auth, session, password, password_reset, invitation, user, tenant, token
├── ops-finance/     # receipts, attachments, extractions, (expenses)
└── ops-ingest/      # ingest/, agent_trace
```

`ops-core` on jaettu pohja, jonka kolme muuta riippuu siitä. `crates/server` riippuu kaikista neljästä. `crates/dev-cli` riippuu tarvittavista (todennäköisesti kaikista neljästä).

**Hyödyt:**
- Identity-muutos ei rebuildaa finance-moduuleja (ja vice versa)
- Riippuvuussuunnat eksplisiittisinä `Cargo.toml`:issa
- `pub`-näkyvyydet pakotetaan crate-rajoilla
- Test-suiten voi ajaa per-crate kohdistetusti
- Migraatiot pysyvät yhdessä paikassa (`ops-core::MIGRATOR`) ettei skemaversioita synny

**Haasteet:**
- Kaikkien call-siten import-polut muuttuvat (`grooveserve_ops::receipts::*` → `grooveserve_ops_finance::receipts::*`)
- Workspace `Cargo.toml` + per-crate `Cargo.toml` -tiedostojen orkestrointi
- Cyclic-dependency-riski jos esim. `audit` haluaa kutsua `user`-cratea ja toisin päin (ratkaisu: audit jää `ops-core`:hin, user antaa pelkän id:n)

## Triggerit — milloin tehdä

**EI tee nyt.** Kustannus on iso (kaikki call-sitet päivittyvät), arvo materiaalistuu vasta kun haitat osuvat.

Aja kun **JOMPIKUMPI** triggeristä toteutuu:

- `cargo build -p grooveserve-ops` kylmänä > 30 s, **TAI**
- `crates/ops/`:ssa on yli 30 moduulia, **TAI**
- Yhdessä viikossa on syntynyt > 1 PR-konflikti `crates/ops/src/lib.rs`:n re-exporteissa, **TAI**
- C/D-aaltojen worktree-spawnit estyvät rinnakkain koska kaikki koskettavat `crates/ops/`:ia.

## Out of scope

- Migraatioiden jakaminen useaan crateen — pidetään yksi `ops-core::MIGRATOR` kaikkien yli, jotta skema-versionumerointi pysyy yksittäisenä lähteenä. Migraatiotiedostot voivat olla `ops-core/migrations/`:ssä ja muut cratet sqlx-yhteensopivuuden takia tuovat saman migrator:in.
- `crates/server`:n jako (server-http / server-ingest) — eri keskustelu, ei riippuvainen tästä.
- `crates/dev-cli`:n laajennukset — A5c:n jälkeinen erillinen aihe.

## Related

- **#78** A5 ops-ingest full-message-surface — lisää 4 alimoduulia ingest:iin, vauhdittaa tätä
- **#56** Foundation epic — tämä on Phase 1:n jälkeistä foundation-työtä, voisi olla A6 jos triggeri toteutuu MVP-vaiheessa
- B2 `/llm-review` -raportti `history/review-issue-75-ops-ingest-full-message-surface.md`

## Notes

Ei kuulu epiciin **#56** ennen kuin triggeri toteutuu — silloin lisätään Track A jatkona (esim. A6) tai tehdään standalone. Pidettiin epicistä erillään koska MVP-tavoite (kevät–kesä 2026) ei estä jos tätä ei tehdä; ja jos tehdään liian aikaisin, kustannus tulee ennen hyötyjä.
