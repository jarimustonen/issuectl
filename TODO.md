# TODO — issuectl

Työjono ja handoff `/stint`-työrupeamille. Tämä tiedosto on `/stint`:n
käynnistyspiste: lue ensin alla oleva handoff-block, sitten aja `issuectl dag`.
Issue-viittaukset ovat `@slug`-muodossa — koko backlog elää `issuectl`:ssä,
tämä tiedosto on vain kuratoitu handoff-näkymä.

Operating-faktat (deploy, green gate, hot files) ovat
[AGENTS.md](AGENTS.md#operating-policy-for-stint):ssä. Pysyvät opit eivät kuulu
tähän tiedostoon: päätökset → `docs/decisions/`, mekaniikka → `docs/design/`,
säännöt → `AGENTS.md`.

---

## 🔄 Continue here · ALOITA TÄSTÄ

**Tila (2026-08-17, arkkitehtuurikatselmus-rupeama valmis):** `main` **vihreä** (1078 testiä,
fmt + clippy + doc puhtaat, orkestroija verifioi itse mergen jälkeen), pushattu. Live-release
`v0.14.1` kaikissa kanavissa. Ei ajossa olevia workereita. **Molemmat rupeaman worktreet
valmistuivat ja mergasivat itsensä:**

- **@split-main-rs TEHTY:** kaikki kolme hot-tiedostoa pilkottu puhtaana siirtona —
  `main.rs` (9278 → 5 riviä) → `cmd/`-perhemoduulit, `doctor.rs` → `doctor/`
  (checks/apply/core/render + testit), `mutate/mod.rs` (7319 → 906) → verbitiedostot.
  Help-output byte-identtinen, green gate per vaihe. AGENTS.md:n hot-file-sääntö on nyt
  per-perhemoduuli: eri komentoperheet ovat rinnakkais-turvallisia.
- **@cli-verb-surface TEHTY:** `docs/decisions/0004-cli-verb-surface.md` — `update` on ainoa
  valikoiva mutaatioverbi (`set`/`assign`/`label`/`apply`/`bulk`/`close`/`depend` foldataan
  siihen), `note` kanoninen (`comment` alias), `stats`+`workload` → `metrics`, `burndown` →
  `cycle burndown`, `hooks`+`install-merge-driver` → `fmt`, `pick`/`new`/`ls` alias-then-remove,
  export JSON-only, intake ainoa vastaanotto. Deprekointi-ikkuna: 0.15.0 valmistelu → 0.16.0
  piilotetut aliakset + varoitukset → 0.17.0 poisto. Uusi ylätason verbi vaatii ADR-amendmentin.
  @deprecate-triage-inbox ratifioitu, gatettu @intake-queue-legacy-mismatch:n taakse.

**Tässä rupeamassa tehty (2026-08-17):**
1. `issuectl archive` otettu käyttöön omassa repossa: 60 suljettua issueta (>90 pv) siirretty
   `issues/archive/`-puuhun. Aja handoffissa jatkossa kun aktiivipuu kasvaa.
2. ossctl päivitetty 0.7.0:aan — molemmat 0.14.1-cutin sudenkuopat korjattu upstreamissa, ja
   `OSS-RELEASE.md` deklaroi nyt `distribution:`-blokin (cargo-dist-delegaatio + tap), joten
   engine **verifioi** GitHub Release -assetit ja tap-formulan ennen kuin cut on "complete".
   `@ossctl-cut-no-publish` pysyy auki seuraavan cutin verifiointiporttina.
3. AGENTS.md siivottu sääntökirjaksi (570 → ~300 riviä): perustelut siirretty
   `docs/decisions/0002` (canon-§22-hylkäykset), `0003` (frontmatter-kenttien tyypitys + DAG),
   `docs/design/pi-skill-mirror.md`, `docs/design/doctor-fix.md`. Release-osio kirjoitettu
   0.7.0:lle; henkilökohtaiset infra-nimet poistettu.
4. Filattu: @cli-verb-surface (ADR, ajossa), @deprecate-triage-inbox (blocked_by ADR),
   @purge-telegram-surfaces (cli-fixes, splitin jälkeen — henkilökohtaisen intake-kanavan
   nimi pois tuotepinnoista).
5. Vanha canon-review-worktree poistettu.

**Seuraava askel:**
- `cli-fixes`-lanen loput (@intake-feature-issuectl-77792e73735b, @intake-queue-legacy-mismatch,
  @purge-telegram-surfaces) ovat nyt ajettavissa — split on landattu, joten eri perhemoduuleihin
  osuvat voivat kulkea rinnakkain.
- `skills`-lane (@intake-bug-issuectl-bad8e7d6118a → @intake-bug-issuectl-bf2580033c3a,
  priority high) — spawnattavissa milloin vain, disjoint muista.
- ADR 0004:n toteutus: filaa fold-issuet (per fold, post-split `cmd/`-tiedostoihin) 0.15.0/0.16.0
  -aikataululla; @deprecate-triage-inbox odottaa @intake-queue-legacy-mismatch:ia.
- Splitti + ADR ovat sisäisiä — **ei release-tarvetta vielä**; seuraava cut 0.7.0-enginellä kun
  käyttäjälle näkyvää kertyy (resepti AGENTS.md; verify-barrieri tarkistaa, backstop kerran).

**Intake triagoitu (2026-08-17, 4 kohdetta):** @intake-bug-issuectl-fab0edad2e42 hyväksytty
(dag-järjestyspolitiikka näkyviin, lane help-docs) · @intake-feature-issuectl-769ae85ab662
suljettu obsoletena (`update --add-collision/--remove-collision` on jo olemassa) ·
@intake-feature-issuectl-42403ae544e3 hyväksytty (skill dokumentoi lane-liput, lane skills) ·
@intake-feature-issuectl-4f9dbc60a05e hyväksytty (deferred-labelin eläköinti + doctor-check,
lane intake).

**⚠️ Intake-detektio:** käytä provenanssi-agnostista hakua — `issuectl intake queue` TAI
`issuectl list --status open --label needs-triage`. Huomaa @intake-queue-legacy-mismatch:
jono voi listata legacy-kohteita joita `intake accept` ei käsittele.

**Dogfood:** `cargo install issuectl` / `brew upgrade jarimustonen/issuectl/issuectl` /
`cargo install --path crates/issuectl`. Skillit `/issue`, `/issue-new`, `/issue-intake`
tulevat `issuectl skill install`ista; uudet kohteet sisään `issuectl intake file`lla,
jonon nostaa `/issue-intake` (tai `/stint-start`).

---

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`,
`blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog
in this file. Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and
`issuectl dag` are the source of truth.

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin ja tarkistaa
`issuectl dag` -näkymän rupeaman lopussa. Committoi vain muuttuneet polut täsmällisesti
(`TODO.md` ja issue-tiedostot, jos niitä muutettiin) ennen `/wrap-up`:ia, jotta tuore
agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md). Aja samalla `issuectl archive --dry-run` ja
arkistoi jos kertymää on.

## Intake

Saapuneet bugiraportit ja feature-pyynnöt elävät `issuectl`:ssä, eivät tässä
tiedostossa. Uusi kohde sisään `issuectl intake file`lla; jonon nostaa
`/issue-intake` (tai `/stint-start`):

```bash
issuectl intake queue
issuectl ls --status open --label needs-triage
```

Hyväksytty kohde lanetetaan `issuectl`-frontmatteriin ja näkyy sen jälkeen
`issuectl dag`issa.

Huom: raportoivan pään filaus-flow (sisarrepon wrapper) appendaa tänne oman
checklist-osionsa joka kerta. Se ei tule issuectl:n binääristä eikä templateista;
poista osio triagen yhteydessä kunnes lähdepää on korjattu.

