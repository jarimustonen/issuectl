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

**Tila (2026-08-10, iltapäivän rupeama):** **Iso linjaus: web/selain-UI poistetaan issuectl:stä.**
Tässä rupeamassa ei koodattu eikä releasetty — tämä oli **issue-kannan siivous- ja
linjausrupeama**. `main` == origin, työpuu puhdas, **v0.8.0** yhä uusin release (koko ketju
vihreä edellisestä rupeamasta: GitHub Release + Homebrew + crates.io).

**Mitä tässä rupeamassa tehtiin:**
- **Phantom-bug suljettu:** `@json-show-omits-blocked-by` → `cannot-reproduce` (oli jo korjattu
  0.7.2:ssa `project_blocked_by`-projektiolla; verifoitu nykykoodilla).
- **Web-UI:n poisto linjattu.** Kaikki selain/kanban-featuret suljettu `obsolete` (13 kpl, ks.
  alla), ja luotu portitettu poisto-issue **`@remove-web-ui`** (chore, deferred). **⛔ Gate:
  poistoa EI tehdä ennen kuin web-toiminnot on arvioitu seuraaja-ohjelmaa varten** (luonnos
  tekeillä toisessa repossa; gate hoidetaan käsin — un-defer + start vasta kun luonnos valmis).
- **Kaksi web-issueä säilytettiin ja re-scopattiin CLI-only + nimettiin kunnon slugilla:**
  `massively-periodic-surprise` → **`@issue-graph-view`** (`issuectl graph` -moottori + lensit;
  lens 2 osin jo `issuectl dag`), `needlessly-slippery-pan` → **`@epic-tree-view`**
  (`issuectl epic tree`). Molemmat pysyvät `deferred`.

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

**Iso linjaus (PÄIVITETTY tässä rupeamassa):** **web/selain-UI POISTETAAN** (ei enää vain
holdissa). issuectl → puhdas AI-first CLI. Kaikki kanban/web-featuret suljettu `obsolete`;
poisto ajetaan `@remove-web-ui`:n kautta **vasta kun seuraaja-ohjelman luonnos on arvioitu**
(gate käsin, ks. Tila-blokki). Fokus CLI:ssä.

**Seuraava askel:** **aktiivinen (ei-deferred) työjono on käytännössä tyhjä** — ainoa on
`@awfully-courageous-attempt` (low, build-only-if — vain jos Codex-käyttö todistuu). Kaikki muu
on `deferred`: portitettu `@remove-web-ui` (odottaa seuraaja-ohjelman luonnosta), CLI-only
re-scopatut `@issue-graph-view` + `@epic-tree-view`, iso strateginen `@focus-areas` (ADR 0001
valmis, koskee `schema.rs`), `@wire-oss-release-as-release-path` (blocked upstream-ossctl:llä),
`@somewhat-heady-zephyr` (events.jsonl, build-only-if). Uusi rupeama alkaa siis backlogista
nostamalla (todennäk. `@focus-areas`), uudesta intake-issuesta, tai — kun seuraaja-luonnos
valmistuu — `@remove-web-ui`:n un-deferillä.

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
GLOBAL HEAD-OF-LINE: dag-inprogress-spawnable
LANE A — issuectl dag (dag.rs + schema.rs + cmd_dag in main.rs) — "dag-polish", yksi worktree tekee kaikki 3 sarjassa
  ▶ dag-inprogress-spawnable       bug · dag raportoi spawnable=true vaikka status in-progress → kaksoisspawn-riski
    dag-stable-intralane-order     feature · valinnainen lane_seq:<int> intra-lane-sort (ennen slug-tie-breakia); schema.rs-kenttä
    dag-unlaned-parallel-sentinel  feature · ensiluokkainen confirmed-parallel-safe -merkki (varattu lane: unlaned) vs. puuttuva lane; schema.rs-kenttä
LANE B — skill install + templates/ + skill.rs (skill distribution)
    codex-prompt-variants  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle. Build-only-if: vain jos Codex-käyttö todistuu.
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl) — disjoint LANE A:sta, rinnakkaisturvallinen
  ▶ wire-oss-release-as-release-path  feature · siirrä julkaisu /oss-release-polulle; ossctl valmis. Korjaa myös crates.io-manuaalitriggerin.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: siivous-/linjausrupeaman jälkeen homebase-dogfood filasi 3 `dag-*`-gappia (committi
`03ab843`) jotka hiovat 0.8.0:n `issuectl dag`-komentoa → **LANE A "dag-polish"** (bug + 2 schema-kenttää,
yksi worktree sarjassa). ossctl valmistui → **`@wire-oss-release-as-release-path` un-deferoitu** LANE C:hen
(rinnakkainen). Molemmat production-koodia → `/llm-review` ennen mergeä. `codex-prompt-variants` (ent.
awfully-courageous-attempt) pysyy LANE B:ssä build-only-if-parkissa.

---

## Adjacent backlog (deferred — DAG:n ulkopuolella, ei ajossa)

Kaikki alla on labeloitu `deferred` issuectl:ssä (2026-08-04), joten ne eivät ole DAG-lanella
eivätkä laukaise drift-checkiä. Poista `deferred`-label kun otat takaisin peliin.

**Web/selain-UI: POISTETAAN (portitettu):**
`@remove-web-ui` (chore, deferred) — poistaa `issuectl serve` + web-server + `/api` + kanban-
frontend + web-only watcher. **⛔ Gate:** ei ennen kuin web-toiminnot on arvioitu seuraaja-
ohjelmaa varten (luonnos tekeillä toisessa repossa; hoidetaan käsin). Kaikki entiset kanban/web-
enhancement-issuet (13 kpl) suljettu `obsolete` tässä rupeamassa (2026-08-10): intensely-teeny-ink,
genuinely-cloistered-current, truly-somber-payment, needlessly-flimsy-scarecrow, partially-nasty-sack,
fiercely-juicy-kettle, almost-homely-decision, somewhat-flawless-letter, perfectly-white-linen,
needlessly-mysterious-volcano, practically-truculent-music, supremely-accurate-body,
fiercely-colossal-rabbits.

**Web-issueistä CLI-only re-scopatut (SÄILYVÄT):**
`@issue-graph-view` (ent. massively-periodic-surprise) — `issuectl graph` -moottori + lensit
(deps / worktree-planning / epic-rollup); lens 2 osin jo `issuectl dag`:ssä.
`@epic-tree-view` (ent. needlessly-slippery-pan) — `issuectl epic tree <slug>`.

**Build-only-if (älä rakenna ennen kuin tarve todistuu):**
`@somewhat-heady-zephyr` (events.jsonl — vain jos git-metriikat ei riitä).

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
