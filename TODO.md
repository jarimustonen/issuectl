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

**Tila (2026-08-17, ilta — kaksi rupeamaa samana päivänä valmiina):** `main` **vihreä**
(1082 testiä + integraatiot, fmt/clippy/doc puhtaat, orkestroija verifioi mergien jälkeen),
pushattu. **Live-release `v0.15.0` kaikissa kanavissa** (crates.io ×2, GitHub Release 12
assetia — pudotus 15:stä on Windows-drop, tap 0.15.0; kaikki backstop-verifioitu). Paikallinen
binääri päivitetty (`cargo install --path`). Ei ajossa olevia workereita eikä worktreitä.

**Rupeama 1 (arkkitehtuurikatselmus):** @split-main-rs (kolme hot-tiedostoa pilkottu puhtaana
siirtona: `main.rs` 9278→5 riviä → `cmd/`-perheet, `doctor.rs` → `doctor/`, `mutate/mod.rs`
7319→906 → verbitiedostot; hot-file-sääntö nyt per-perhemoduuli) + @cli-verb-surface
(ADR `docs/decisions/0004`: `update` ainoa valikoiva mutaatioverbi, foldit + deprekointi-ikkuna
valmistelu → piilotetut aliakset + varoitukset → poisto; uusi ylätason verbi vaatii
ADR-amendmentin). Lisäksi: `issuectl archive` käyttöön (60 issueta arkistoon), AGENTS.md
sääntökirjaksi (perustelut `docs/decisions/0002–0003` + `docs/design/`), ossctl 0.7.0 +
`distribution:`-blokki OSS-RELEASE.md:hen.

**Rupeama 2 (0.15.0):** kolme workeria rinnakkain, kaikki landattu —
1. @intake-bug-issuectl-bad8e7d6118a: `skill install --force` säilyttää repo-kirjoitetun
   `issues/AGENTS.md`:n; uusi `--force-scaffold` regeneroi eksplisiittisesti.
2. @intake-bug-issuectl-bf2580033c3a: henkilökohtaiset infra-viittaukset pois lähteestä.
3. @intake-queue-legacy-mismatch: jono status-strict + legacy-varoitus + `intake migrate
   --apply` -polku. → @deprecate-triage-inbox vapautui blockeristaan.
4. @intake-feature-issuectl-77792e73735b: `apply --help` näyttää `body_ops`-shapet
   (parser-pinnattu esimerkki). 5. @intake-bug-issuectl-fab0edad2e42: dag-järjestyspolitiikka
   näkyviin. Intake-triage: 5 kohdetta (1 suljettu obsoletena, loput lanetettu).

**RELEASE-HAVAINTO (0.15.0 cut, ossctl 0.7.0):** engine julkaisi cratet ja delegoi GitHub
Releasen oikein (0.6.1-bugit todistetusti korjattu; @ossctl-cut-no-publish suljettu). MUTTA
cut päättyi exit-2:een: ossctl ajaa **oman** homebrew-leginsä vaikka cargo-distin
`publish-jobs=["homebrew"]` omistaa tapin — legi kaatui transienttiin GitHub 503:een, verify-
vaihe jäi ajamatta ja run kirjautui failed-tilaan vaikka kaikki kohteet toimitettiin
(false-red). Filattu ossctl:ään: `@cut-runs-own`. Kunnes se on korjattu: **failed-cutin
jälkeen aja backstop-tarkistukset ennen johtopäätöksiä** (crates.io API, `gh release view`,
tap-formula) — dist-vaiheen kaatuminen ei tarkoita että julkaisu epäonnistui.

**Seuraava askel:**
- `verb-surface`-lane: @intake-bug-issuectl-d5b6669a98fe (update:n `--json`-echo ei raportoi
  juuri asetettuja lane-kenttiä; vahvistettu koodista) → @update-canonical-forms (ADR:n
  valmisteluslice: `update --patch-file`, `--query`, parity-testit) → @deprecate-triage-inbox.
  Sarjassa (sama echo/write-pinta).
- `skills`-lane: @intake-feature-issuectl-42403ae544e3 (skill dokumentoi lane-liput).
- `intake`-lane: @intake-feature-issuectl-4f9dbc60a05e (deferred-labelin eläköinti +
  doctor-check).
- @purge-telegram-surfaces — nyt vapaana ajettavaksi (skills- ja intake-laneissa ei ajossa
  mitään; törmää templateihin, joten älä rinnakkain 42403:n kanssa).
- ADR:n deprekointi-release (piilotetut aliakset + varoitukset + canonical-only skillit)
  filataan kun @update-canonical-forms on landannut.

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
agentti voi jatkaa komennolla `/skill:stint-start`. Pidä `main` puhtaana rinnakkaisten
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

## Piialiisan bugiraportit

- [ ] 🐛 Piialiisan bugiraportti: issuectl apply cannot read a patch from stdin — jari via Telegram ([`intake-feature-issuectl-0b1bf129b13b`](issues/intake-feature-issuectl-0b1bf129b13b/item.md))
