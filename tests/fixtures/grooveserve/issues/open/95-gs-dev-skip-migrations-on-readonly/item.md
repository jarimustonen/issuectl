---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#74", "#56"]
labels: [devex, gsdev, dev-cli, safety]
---

# 95. `gs-dev` ajaa migraatiot myös read-only-komennoille

_Source: B3 (`#74`) `/llm-review` round 1 — OpenAI #1, vahvistettu DeepSeek + Claude round 2._

## Description

`crates/dev-cli/src/main.rs:174-177` ajaa `grooveserve_ops::MIGRATOR.run`
**ehdoitta** ennen subcommand-dispatchia. Tämä koskee jokaista `gs-dev`-
komentoa, mukaan lukien dokumentaatiossa "read-only"-väitteen kantavia
`dev trace` ja `dev history`.

Vaikutus: jos kehittäjä ajaa epähuomiossa esim.

```bash
DATABASE_URL=postgresql://...prod-snapshot... gs-dev dev history --user X
```

binäärin yritys ajaa ajamattomia migraatioita ennen SELECTiä mutaatioi
schemaa. AGENTS.md varoittaa "never point gs-dev at a prod database",
mutta yksittäisten subkomentojen "read-only"-leima on silti epärehellinen.

## Päätös

Ratkaisuvaihtoehdot:

**A) Skip migrations for inspection commands.** Lisää `command_needs_migrations()`-
predikaatti `main()`:iin: `SetupTenant`, `Dev::Send`, `Dev::Tool`,
`Dev::ParseEml` → migrate; `Dev::Trace`, `Dev::History` → skip.

**B) Document and accept.** Päivitä AGENTS.md sanomaan että jokainen
`gs-dev`-kutsu ajaa migraatiot startup-time, eikä yhdenkään komennon
voi luvata olevan read-only schema-tasolla.

**C) Hybrid.** Lisää `--no-migrate`-lippu globaaliksi, jonka caller
voi asettaa varmuuden vuoksi.

Suositus: **A** + päivittää AGENTS.md. Hyödyt: subkomentojen
read-only-väitteet pitävät paikkansa; haitat: `gs-dev dev history`
fresh DB:llä (ennen `setup-tenant`:ia) ei enää ole one-shot — vaatii
manuaalisen `gs-dev migrate` tai vastaavan. Tämä on tuskin oikea
käyttötapaus inspection-komennoille (DB:n pitää olla jo populated
jotta history-kutsulla on jotain palautettavaa), joten kustannus
hyväksyttävä.

## Acceptance

- `gs-dev dev history` ja `gs-dev dev trace` eivät aja migraatioita
  ennen SELECTiä.
- Jos schema on vanhempi kuin migrator odottaa (puuttuva sarake),
  read-only-komento joko (a) palauttaa selvän virheen "schema older
  than expected — run `gs-dev setup-tenant` against a fresh DB" tai
  (b) palauttaa rivit jotka silti ovat luettavissa (parempi UX).
- AGENTS.md (`crates/dev-cli/AGENTS.md`) päivitetty kuvaamaan
  kumpi subkomento ajaa migraatiot ja kumpi ei.

## Related

- `#74` (B3) /llm-review-tulos joka nosti löydöksen
- `#56` epic (gs-dev:n architecture)
- A4a-design-loki: alkuperäinen "always-migrate" -päätös (DX-syyt
  fresh `gsdev instance ensure` -virrassa)
