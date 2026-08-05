# TODO — issuectl

Työjono ja handoff `/stint`-työrupeamille. Tämä tiedosto on
`/stint`:n käynnistyspiste: lue ensin alla oleva handoff-block, sitten
Execution DAG. Issue-viittaukset ovat `@slug`-muodossa — koko backlog elää
`issuectl`:ssä (`issuectl list`), tämä tiedosto on vain kuratoitu näkymä
"mitä kannattaa tehdä ja missä järjestyksessä".

Operating-faktat (deploy, green gate, hot files) ovat
[AGENTS.md](AGENTS.md#operating-facts-for-stint):ssä.

---

## 🔄 Continue here · ALOITA TÄSTÄ

**Tila (2026-08-05):** `main` == **v0.7.0** — TÄYSIN JULKAISTU: GitHub Release live,
Homebrew-tap päivitetty, **crates.io `0.7.0` indeksissä** (varmistettu), CI vihreä. Työpuu
puhdas, integraatiobranch mergetty + siivottu. (Huom: rinnakkaissessio filasi `close-comment`in
mainiin tämän rupeaman jälkeen — mergetty DAG:iin alle.)

**0.7.0:ssa shipannut (CHANGELOG täydellinen):**
- **@standard-intake-flow (HIGH)** — koko `issuectl intake`-komentoryhmä (file/queue/
  show/accept/defer/need-info/reject/cannot-reproduce/duplicate/obsolete/retype/reopen/
  withdraw) + statukset `untriaged`/`deferred`/`needs-info` + kentät (provenance/
  disposition_reason/duplicate_of/source_ref/deferred_until) + `intake migrate`
  (dry-run-first) + intrinsic invariantit + type×status-check + täysi transitions-matriisi.
  Skillit **`/issue-new`** + **`/issue-intake`** (korvaa `/triage-bugs` → thin alias).
  Rakennettiin `/orchestrate`-kampanjana (4 yksikköä, kukin 4-malli-review). Toteutus =
  approved `docs/design/intake-flow.md`.
- **@close-as-flag-asymmetry** — `close --as <author>` → `closed_by` (näkyy `show --json`
  `.extra`:ssa). + edellisen rupeaman 3 fixiä (refs-issue-hint, mutation-not-found,
  new-body-flag) jotka odottivat releasea.

**⚠️ crates.io-caveat (ennallaan, muista JOKA release):** tag-push laukaisee cargo-dist
(GitHub Release + Homebrew) mutta **crates.io-julkaisu vaatii manuaalisen triggerin**
(`gh workflow run "Publish to crates.io"`) — `GITHUB_TOKEN` ei laukaise `publish-crates.yml`:ää.
0.7.0:ssa se ajettiin käsin ja onnistui. (Korjaus odottaa `@wire-oss-release-as-release-path`ia.)

**Dogfood:** Käyttäjä dogfoodaa 0.7.0:aa omissa projekteissaan. Asennus: `cargo install issuectl`
(crates.io), `brew upgrade jarimustonen/issuectl/issuectl`, tai `cargo install --path crates/issuectl`.
Uudet skillit `/issue-new` + `/issue-intake` asennettuina; bugit/feature-pyynnöt sisään
`issuectl intake file`lla, `/issue-intake` (tai `/stint-start`) nostaa jonon seuraavan kerran.

**OSS-init (ennallaan):** `OSS-RELEASE.md` **approved** (`mvp`). cargo-dist pysyy
release-engineinä — `/oss-release-cut` EI regeneroi `release.yml`:ää.

**Iso linjaus (ennallaan):** **kanban/web-board holdissa**. Kaikki kanban/web-issuet +
build-only-if + `@focus-areas` `deferred` → **Adjacent backlog**. Fokus CLI:ssä.

**Seuraava askel:** 0.7.0 ulkona ja dogfoodissa — odota käyttäjän palautetta
dogfoodauksesta. DAG:issa vain 2 low-prio LANE A -riviä (molemmat `close`-polkua):
**@intensely-blushing-galley** (closed_by → typed field) ja **@close-comment**
(`close --comment`). GLOBAL HEAD-OF-LINE = **@intensely-blushing-galley**. Ei kiireä
kumpikaan; feedback-issuet menevät edelle jos niitä tulee.

---

## Execution DAG (2026-08-05)

Scheduling PLAN — source of truth for lane + order; issuectl is authoritative for STATUS
(never copied here). Merge each round (drop landed, add active, keep existing order).
`▶` = head-of-line snapshot — RE-COMPUTE from issuectl at pick time.
`after <slug> (needs …)` = logical blocked_by mirror. `collision: <file>` = touches a
second lane's hot file (spawn-time exclusion).

Lanes = hot-file families (AGENTS.md): `main.rs` (clap + cmd_* handlers +
`fn main` error rendering + `parse_apply_patch`), `mutate/` (write paths),
`schema.rs`, skill templates, `hooks.rs`/`git_trailers.rs` (commit-hook).

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: intensely-blushing-galley   ← molemmat LANE A -riviä ovat low-prio nice-to-have; ei kiireä
LANE A — main.rs (show/close handlers) + mutate/ close path + schema/issue_fields  (sequenced — molemmat koskevat cmd_close/close_issue)
  ▶ intensely-blushing-galley   low · improvement; promote closed_by → typed Issue field (top-level in show) + doctor heal + human close output. Follow-up llm-reviewsta close-as-flag-asymmetrylle.
    close-comment                low · feature; `close` hyväksyy `--comment/--note` → kirjaa sulkemisen perustelun samassa stepissä (nyt manuaalinen 2-vaihe). Filattu rinnakkaissessiosta.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: intake-kampanja + close-as-flag landasivat ja **shippasivat 0.7.0:ssa**;
`standard-intake-flow` (done) ja `close-as-flag-asymmetry` (fixed) pudotettu DAG:sta.
`particularly-tart-spade` suljettu `wontfix` (väärä hälytys — closed_by ON show --json:issa
`.extra`:n alla). Jäljellä 2 low-prio LANE A -riviä: **`intensely-blushing-galley`** +
**`close-comment`** (rinnakkaissessiosta, drift-checkin nostama). Ei kiireä kumpikaan.

---

## Adjacent backlog (deferred — DAG:n ulkopuolella, ei ajossa)

Kaikki alla on labeloitu `deferred` issuectl:ssä (2026-08-04), joten ne eivät ole DAG-lanella
eivätkä laukaise drift-checkiä. Poista `deferred`-label kun otat takaisin peliin.

**Kanban / web-board (HOLD — ei käyttäjiä nyt):**
`@intensely-teeny-ink` (custom boards, oli high), `@genuinely-cloistered-current`
(multiple boards), `@truly-somber-payment` (priority-sort), `@needlessly-flimsy-scarecrow`
(card fields), `@partially-nasty-sack` (WIP-limit), `@fiercely-juicy-kettle` (copy-buttonit),
`@almost-homely-decision` (per-user view state), `@somewhat-flawless-letter`
(uncommitted-indikaattori), `@perfectly-white-linen` (undo), `@needlessly-mysterious-volcano`
(näppäimistö-a11y), `@massively-periodic-surprise` (dependency graph),
`@needlessly-slippery-pan` (epic-puu; CLI-osa relevantti jos kanban palaa),
`@fiercely-colossal-rabbits` (canonical_hash-cache — vain `/api/issues`-web-boardin perf).

**Build-only-if (älä rakenna ennen kuin tarve todistuu):**
`@supremely-accurate-body` (field-merge — vain jos 409-kitka todistuu),
`@somewhat-heady-zephyr` (events.jsonl — vain jos git-metriikat ei riitä),
`@practically-truculent-music` (watcher-race-observability — matala kiire).

**Strateginen (iso, aikatauluta kun on tilaa):**
`@focus-areas` — päätös tehty (ADR 0001: `areas: []` skeemakenttä), valmis
implementaatio-ADR:ää + rakentamista varten. Koskee `schema.rs`:ää.

**Blocked on tooling (ei voi tehdä ennen kuin ossctl valmis):**
`@wire-oss-release-as-release-path` — siirrä julkaisu `/oss-release`-polulle
(ossctl release-engine + cargo-dist backend, `publish-crates.yml` eläkkeelle).
Odottaa upstream-ossctl-muutoksia (työn alla). Sivutulos: paljasti että
crates.io-auto-julkaisu on nyt rikki (`GITHUB_TOKEN` ei laukaise
`publish-crates.yml`:ää) → crates.io vaatii manuaalisen triggerin joka releasessa
kunnes tämä tehty.

---

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin JA mergeää
Execution DAG:n (drop landed, add active, keep order) rupeaman lopussa, ja committaa ne
omana committinaan (`git add TODO.md issues/ && git commit`) ennen `/wrap-up`:ia — näin
tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md).
