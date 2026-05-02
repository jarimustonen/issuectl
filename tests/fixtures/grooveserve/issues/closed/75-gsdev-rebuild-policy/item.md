---
created: 2026-04-30
updated: 2026-05-01
closed: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#33"]
labels: [devex, gsdev]
---

## Resolution (2026-05-01)

Implemented vaihtoehto A — mtime-pohjainen rebuild-tarkistus.

- New module `tools/dev/gsdev/build_cache.py` — `compute_cache_status()`
  vertailee `crates/dev-cli/src/**`, `crates/ops/src/**`,
  `crates/{dev-cli,ops}/Cargo.toml`, workspace `Cargo.toml`, ja
  `Cargo.lock` tiedostojen mtimea binäärin mtimea vasten. Status:
  `fresh` / `stale` / `missing`.
- `tools/dev/gsdev/mail.py::gs_dev_cli_path()` käyttää
  `compute_cache_status()`:ia ja kutsuu `_build_gs_dev()`:tä kun status
  on `stale` (tai `missing`).
- Quiet-flag: `GSDEV_NO_REBUILD=1` ohittaa staleness-tarkistuksen
  (binary-missing-haaraan se ei vaikuta — exec olisi muuten rikki).
  Ei per-komento `--no-rebuild`-flagia, koska gsdev:llä ei ole prior
  pattern globaaleille toggleille; env-var on AI-first-pinnalle (vrt.
  `GSADMIN_*`-env-varit) idiomaattisempi.
- `gsdev doctor` raportoi cache-statuksen (binary path, mtime,
  newest/oldest source + path, status, source count). `--json`-flagi
  AI-callerille (per AGENTS-AI-FIRST-CLI.md §2).
- Yksikkötestit `tools/dev/tests/test_build_cache.py` (9 testiä) —
  fake repo + mtime-mock kattaa fresh / stale / missing -haarat,
  Cargo.lock-trigger, ops-source-trigger, env-var-toggle, source-count
  invariantti.

**Mittaus**: 0.72 ms / `compute_cache_status()`-kutsu (40 sources).
Reilusti alle 100 ms warm-cache-budjetin.

**Acceptance** (issue § ja epic A): kaikki täytetyt.

# 75. gsdev rebuild policy — pre-built CLI cache jää vanhentuneeksi

_Source: B2-local-dev-analysis worktree, 2026-04-30. Toistuva ongelma — alunperin havaittu C1-worktreessä 2026-04-29 (`#56` Decision log)._

## Description

`gsdev` rakentaa `gs-dev`-binäärin (entinen `gs-email-cli`) erilliseen target-kansioon `~/.cache/gsdev/cli/release/gs-dev` välttääkseen kontentointia worktreen oman `target/`:n kanssa. `tools/dev/gsdev/mail.py::gs_dev_cli_path()` rakentaa binäärin **vain jos tiedosto puuttuu** — mtime-tarkistusta ei ole.

**Käyttötapaus joka rikkoutuu:** kehittäjä muokkaa `crates/dev-cli/src/main.rs`:ää (esim. uusi `dev tool` -aliaksi tai `ops::*`-funktion signature-muutos), ajaa `gsdev mail send`. Cache palauttaa vanhan binäärin → uutta logiikkaa ei aja → debugaaminen vie tunnin.

C1 löysi tämän verifioinnissa:
> `gsdev mail send` käytti vanhentunutta cachea (`~/.cache/gsdev/cli/release/gs-email-cli`), vaati manuaalisen rm+rebuildin

## Scope

**Vaihtoehto A — mtime-pohjainen rebuild-tarkistus (suositus):**
- `gs_dev_cli_path()` tarkistaa `crates/dev-cli/src/**`, `crates/ops/src/**`, ja `crates/dev-cli/Cargo.toml` mtime vs. binäärin mtime
- Jos lähde on uudempi → kutsu `_build_gs_dev()`
- Lisää quiet-flag `--no-rebuild` ohittaa tarkistuksen ad hoc -hätätilanteissa
- Kustannus: ~10 ms per `gsdev mail`-kutsu (rekursiivinen `os.walk` 200 tiedostolle), hyväksyttävä

**Vaihtoehto B — `cargo build`-poll joka kerta:**
- Cargo on idempotentti, lämpimällä cachella ~0,3 s
- Mahdollinen kontentointi `cargo watch`:n kanssa vaikka `CARGO_TARGET_DIR` on eri (sccache jakaa kompilaattorivälimuistia)
- Kalliimpi mutta yksinkertaisempi

**Vaihtoehto C — `gsdev doctor` raportoi stale-cachen:**
- Ei automaatiota, käyttäjä huomaa itse
- Heikompi DX, mutta riskittömin

## Acceptance

- `crates/dev-cli/src/main.rs`:n muokkaus → seuraava `gsdev mail send` ajaa uutta logiikkaa
- Lämpimällä cachella overhead < 100 ms
- `gsdev doctor` näyttää cache-status (binary mtime, oldest-source mtime, "stale"/"fresh")

## Related

- A4b A-decision log 2026-04-29: "gsdev mail send käyttää välimuistissa olevaa release-binääriä, ei worktreen omaa"
- C1-worktree (`#49` verify): manuaalinen rm+rebuild oli vaadittu

## Out of scope

- Cargo workspace -muutokset (kohta hallinnollisesti C/D-aallon piirissä)
- Sccache-integraatio (toimii jo)
