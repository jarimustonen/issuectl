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

**Tila (2026-08-17, arkkitehtuurikatselmus-rupeama):** `main` vihreä, live-release `v0.14.1`
kaikissa kanavissa. **Kaksi worktreetä ajossa** (ks. alla) — älä spawnaa mitään mikä koskee
`main.rs` / `doctor.rs` / `mutate/` ennen kuin split-run on mergannut.

**Ajossa olevat runit:**
- `01m07e2m4nxsmm6wqqtcdsybh5` (spinoff, `wt/01m07e2m4n-split-hot-files`) — @split-main-rs:
  kolmen hot-tiedoston pilkkominen (main.rs → `cmd/`-perheet, doctor.rs → `doctor/`,
  mutate/mod.rs → verbitiedostot) puhtaana siirtona, green gate per vaihe. Mergaa itsensä.
- `01m07e44ygjgf7vmdvjkbmfacm` (technical-decision, `wt/01m07e44yg-cli-verb-surface`) —
  @cli-verb-surface: verbipinnan konsolidointi-ADR (`docs/decisions/0004`). Ehdottaa
  toteutusissuet raportissaan; ne lanetetaan kun ADR on mergattu.

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

**Seuraava askel kun runit valmistuvat:**
- Split mergattu → kapea hot-file-lista on voimassa; `cli-fixes`-lanen loput
  (@intake-feature-issuectl-77792e73735b, @intake-queue-legacy-mismatch,
  @purge-telegram-surfaces) voidaan ajaa, osin rinnakkain perhemoduulien mukaan.
- ADR mergattu → lanetä sen ehdottamat toteutusissuet `verb-surface`-laneen;
  @deprecate-triage-inbox vapautuu blockeristaan.
- `skills`-lane (@intake-bug-issuectl-bad8e7d6118a → @intake-bug-issuectl-bf2580033c3a,
  priority high) on splitistä disjoint — spawnattavissa milloin vain.
- Kun `main` saa käyttäjälle näkyviä muutoksia: release 0.7.0-enginellä (resepti AGENTS.md);
  verify-barrieri hoitaa tarkistuksen, backstop-check silti kerran.

**Uutta intakea, triage tekemättä:** @intake-bug-issuectl-fab0edad2e42 (dag: priority ohittaa
lane_seq:n lanen sisällä hiljaisesti) ja @intake-feature-issuectl-769ae85ab662 (`collision`
dokumentoitu mutta `update` ei toteuta sitä — huom. tarkista, voi olla jo vanhentunut:
`update --add-collision` on olemassa). Aja `/issue-intake` tai lanetä suoraan; molemmat
osunevat `dag.rs`/`main.rs`-alueelle eli splitin jälkeen.

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
