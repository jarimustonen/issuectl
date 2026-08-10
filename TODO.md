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

**Tila (2026-08-10):** **0.8.0 JULKAISTU JA VARMISTETTU — koko ketju vihreä.** Tag **v0.8.0**
origin:ssa (`90cb695` = `release: 0.8.0`). Kaikki kolme julkaisukanavaa vahvistettu vihreiksi
tässä rupeamassa: **GitHub Release** v0.8.0 (ei draft, 12 assettia — binäärit + installerit),
**Homebrew** (cargo-dist push vihreän `release.yml`:n kautta), **crates.io** (`Publish to
crates.io` -run `31364623070` `completed/success` — taustawatcheri triggeröi sen automaattisesti
`release.yml`:n vihertyessä). `main` == origin, työpuu puhdas.

**⚠️ Minor-bump-gotcha (UUSI, muista seuraavassa minor-releasessa):** 0.8.0-cut paljasti että
`crates/issuectl/Cargo.toml`:n sisäinen dep `issuectl-core = { path = "…", version = "0.7.0" }`
on caret-vaatimus (`^0.7.0` = `<0.8.0`) → **rikkoi buildin** kun workspace-versio nousi 0.8.0:aan.
Korjaus: bumppaa tuo `version =` vastaamaan uutta minoria (0.7.0 → 0.8.0) samassa release-commitissa.
Patch-bumpeissa (0.8.0 → 0.8.1) ei tarvita; vain minor/major-rajan ylitys vaatii tämän.

**⚠️ Autonomy (ennallaan):** `AGENTS.md`:n operating-faktoissa: konduktööri saa tehdä releaset
**JA päättää niistä autonomisesti** (ei go/no-go). 0.8.0 cutattiin sen nojalla.

**0.8.0:n sisältö (2 homebase-research-featurea, CHANGELOG `[0.8.0]`):**
- **@dag-scheduling-view (feature, done)** — kaksi valinnaista per-issue-scheduling-kenttää `lane`
  + `collision` (frontmatter, `closed_by`-precedentti: absent-by-default, `canonical_hash`iin vain
  kun asetettu → olemassa olevien issueiden version-tokenit EIVÄT muutu; verifoitu frozen-hash-
  testillä; `SUPPORTED_SCHEMA_VERSION` EI noussut) + uusi `issuectl dag [--json] [--reservations
  <file|-|json>]`-komento (`crates/issuectl-core/src/dag.rs`): join lane+order+blocked_by+live
  status, head-of-line & spawnability ON READ (mitään ei talleteta), orkestraattori-agnostinen
  (reservations caller-input). 4-model /llm-review sovellettu. → Consumerit voivat korvata käsin-
  ylläpidetyn markdown-`## Execution DAG`-blockin lasketulla näkymällä.
- **@default-slug-from-title (feature, done)** — `issuectl new "<title>"` ilman `--slug`:ia johtaa
  nyt 2–3 sanan kebab-slugin otsikosta (stop-wordit pois, apostrofit, vain ASCII) satunnaisen
  `intensifier-adjective-noun`in sijaan; dedupe numeroliitteellä (`base-2`…99). Satunnaismuoto
  tavoitettavissa uudella `--slug-random`-flagilla + auto-fallbackina kun otsikko ei slugautu.
  `--slug <x>` ennallaan. Intake + toistuvat pitävät satunnaismuodon tarkoituksella. AGENTS.md-
  konventionootti ("CLI default is random") päivitetty + `/issue`-skilltemplatet synkattu. 4-model
  /llm-review sovellettu.

**⚠️ crates.io-caveat (ennallaan, muista JOKA release):** tag-push laukaisee cargo-dist
(GitHub Release + Homebrew) mutta **crates.io-julkaisu vaatii manuaalisen triggerin**
(`gh workflow run "Publish to crates.io"`) — `GITHUB_TOKEN` ei laukaise `publish-crates.yml`:ää.
0.8.0:ssa (kuten 0.7.2:ssa) tämä hoidettiin taustawatcherilla joka triggeröi sen `release.yml`:n
vihertyessä. (Korjaus odottaa `@wire-oss-release-as-release-path`ia.)

**Dogfood:** Käyttäjä dogfoodaa issuectl:ää omissa projekteissaan. Asennus: `cargo install
issuectl` (crates.io), `brew upgrade jarimustonen/issuectl/issuectl`, tai `cargo install
--path crates/issuectl`. 0.7.1:stä skillit `/issue`, `/issue-new`, `/issue-intake` tulevat
kaikki `issuectl skill install`ista; bugit/feature-pyynnöt sisään `issuectl intake file`lla,
`/issue-intake` (tai `/stint-start`) nostaa jonon seuraavan kerran.

**OSS-init (ennallaan):** `OSS-RELEASE.md` **approved** (`mvp`). cargo-dist pysyy
release-engineinä — `/oss-release-cut` EI regeneroi `release.yml`:ää.

**Iso linjaus (ennallaan):** **kanban/web-board holdissa**. Kaikki kanban/web-issuet +
build-only-if + `@focus-areas` `deferred` → **Adjacent backlog**. Fokus CLI:ssä.

**Seuraava askel:** **triage `@json-show-omits-blocked-by` ensin.** Tämä on `from-homebase`-
filattu bug (2026-08-10) joka väittää `--json show`:n jättävän `blocked_by`:n pois top-leveliltä
— MUTTA tämä on jo korjattu 0.7.2:ssa (`@show-json-omits-blocked-by` / `@json-blocked-by-null-
top-level`, ks. AGENTS.md-nootti). Verifoitu nykykoodilla: `issuectl depend add y --blocked-by x;
issuectl --json show y | jq 'has("blocked_by")'` → **true**. Raportti syntyi ennen 0.7.2-fixin
tuloa. **Suositus: sulje `cannot-reproduce`** (`issuectl close json-show-omits-blocked-by
--status cannot-reproduce --comment "already fixed in 0.7.2 via project_blocked_by"`). Bug-intake
on decoupled → jätetty käyttäjän/`/stint`:n päätökseksi, EI suljettu autonomisesti.

Sen jälkeen **aktiivinen työjono on tyhjä** paitsi parkissa oleva `@awfully-courageous-attempt`
(low, build-only-if — vain jos Codex-käyttö todistuu). Backlog (Adjacent) on kaikki `deferred`:
kanban/web holdissa, `@focus-areas` iso strateginen (ADR 0001 valmis, koskee `schema.rs`),
`@wire-oss-release-as-release-path` blocked upstream-ossctl:llä. Uusi rupeama alkaa siis
todennäköisesti backlogista nostamalla tai uudesta feedback/intake-issuesta.

---

## Execution DAG (2026-08-10)

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
GLOBAL HEAD-OF-LINE: json-show-omits-blocked-by   ← TRIAGE FIRST — from-homebase-bug, jo korjattu 0.7.2:ssa, suositus close cannot-reproduce (ei ajettavaa työtä; bug-intake on käyttäjän päätös)
LANE A — main.rs (cmd_* + clap) + schema.rs
    json-show-omits-blocked-by  bug · from-homebase; väittää `--json show`:n jättävän `blocked_by`:n pois top-leveliltä. JO KORJATTU 0.7.2:ssa (`project_blocked_by`-projektio) — verifoitu nykykoodilla has("blocked_by")=true. EI ajettavaa fixiä → triage/close cannot-reproduce. Ei worktreetä.
LANE B — skill install + templates/ + skill.rs (skill distribution)
    awfully-courageous-attempt  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle (frontmatter-strip kuten /issuella). Build-only-if: vain jos Codex-käyttö todistuu.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: tässä rupeamassa landasi **kaksi homebase-research-featurea** — `dag-scheduling-view`
(lane/collision-kentät + `issuectl dag`-view, orkestraattori-agnostinen) ja `default-slug-from-title`
(otsikkojohdettu default-slug) — ja **0.8.0 cutattiin + varmistettiin** (kaikki 3 kanavaa vihreää,
ks. Tila-blokki). Molemmat featuret terminal (`done`) → pudotettu DAG:sta. Tilalle nousi vain yksi
aktiivinen: **`json-show-omits-blocked-by`** (from-homebase-bug, jo korjattu 0.7.2:ssa → suositus
close). **`awfully-courageous-attempt`** (low, build-only-if) pysyy LANE B:ssä parkissa. Aktiivinen
työ on käytännössä loppu — seuraava rupeama nostaa backlogista tai uudesta intake-issuesta.

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
