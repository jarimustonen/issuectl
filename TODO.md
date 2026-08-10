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

**Tila (2026-08-10):** **0.7.2 CUTATTU + PUSHATTU.** Tag **v0.7.2** origin:ssa (`b9b3cf2`),
`main` == v0.7.2 (release-commit `release: 0.7.2`, työpuu puhdas). Release-flow käynnistetty:
cargo-dist (`release.yml`) käynnissä push-hetkellä, ja **taustawatcheri triggeröi crates.io-
julkaisun automaattisesti kun `release.yml` vihertyy** (`gh run watch … && gh workflow run
"Publish to crates.io"`). **VERIFOI seuraavalla resumella:** `gh run list --limit 5`,
`gh release view v0.7.2`, ja crates.io — jos watcheri ehti kaatua ennen crates.iota, aja
`gh workflow run "Publish to crates.io" --ref main` käsin.

**⚠️ Autonomy-muutos (2026-08-10):** `AGENTS.md`:n operating-faktoihin kirjattu että konduktööri
saa nyt tehdä releaset **JA päättää niistä autonomisesti** (ei enää go/no-go). Tämä 0.7.2-release
cutattiin sen nojalla. (Committi `docs(agents): grant conductor autonomous release authority`.)

**0.7.2:n sisältö (4 korjausta, CHANGELOG `[0.7.2]`):**
- **@show-json-omits-blocked-by + @json-blocked-by-null-top-level (bugs, fixed)** — `blocked_by`
  surfataan top-leveliin (kanoninen `@`-lista + johdettu `blocks` show:ssa) `show`/`ls`/`search
  --json`illa jaetun `project_blocked_by`-projektion kautta; ennen top-level `.blocked_by` oli aina
  `null` (arvo hautautui `.extra`an) — nyt yksi wire-esitys joka polulla. Worker piti kentän
  `extra`ssa (ei typettänyt: se on jo `canonical_hash`issa raakana → typetys rikkoisi version-tokenit).
- **@intensely-blushing-galley (improvement, done)** — typed `closed_by`-kenttä: `canonical_hash`,
  skeema, `doctor`-heal, näkyy `show --json` + human.
- **@close-comment (feature, done)** — `close --comment/--note` liittää aikaleimatun `## Resolution`
  -lohkon samassa atomisessa kirjoituksessa.

**⚠️ crates.io-caveat (ennallaan, muista JOKA release):** tag-push laukaisee cargo-dist
(GitHub Release + Homebrew) mutta **crates.io-julkaisu vaatii manuaalisen triggerin**
(`gh workflow run "Publish to crates.io"`) — `GITHUB_TOKEN` ei laukaise `publish-crates.yml`:ää.
0.7.2:ssa tämä hoidettiin taustawatcherilla joka triggeröi sen `release.yml`:n vihertyessä (ks.
Tila yllä — verifoi että meni läpi). (Korjaus odottaa `@wire-oss-release-as-release-path`ia.)

**Dogfood:** Käyttäjä dogfoodaa issuectl:ää omissa projekteissaan. Asennus: `cargo install
issuectl` (crates.io), `brew upgrade jarimustonen/issuectl/issuectl`, tai `cargo install
--path crates/issuectl`. 0.7.1:stä skillit `/issue`, `/issue-new`, `/issue-intake` tulevat
kaikki `issuectl skill install`ista; bugit/feature-pyynnöt sisään `issuectl intake file`lla,
`/issue-intake` (tai `/stint-start`) nostaa jonon seuraavan kerran.

**OSS-init (ennallaan):** `OSS-RELEASE.md` **approved** (`mvp`). cargo-dist pysyy
release-engineinä — `/oss-release-cut` EI regeneroi `release.yml`:ää.

**Iso linjaus (ennallaan):** **kanban/web-board holdissa**. Kaikki kanban/web-issuet +
build-only-if + `@focus-areas` `deferred` → **Adjacent backlog**. Fokus CLI:ssä.

**Seuraava askel:** ensin **verifoi 0.7.2-release meni vihreäksi loppuun** (ks. Tila yllä —
`gh run list`, `gh release view v0.7.2`, crates.io). Sitten backlogin kärjessä on kaksi uutta
**homebase-research**-featurea (2026-08-10): **@dag-scheduling-view** (normal — `lane`/`collision`-
frontmatter-kentät + `issuectl dag`-view, koskee `schema.rs` + `main.rs`, design-first) ja
**@default-slug-from-title** (normal — johda default-slug otsikosta satunnaisen sijaan, koskee
`main.rs`). Molemmat koskevat `main.rs`:ää → **samassa lanessa, sekvensoitu**. @awfully-courageous-
attempt (low, build-only-if) pysyy parkissa. Molemmat uudet ovat design-first-kokoisia — harkitse
`/worktree-code` (ihmisen review) tai design-first-spinoff. Ei kiirettä; feedback-issuet edelle.

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
GLOBAL HEAD-OF-LINE: dag-scheduling-view   ← normal-feature (homebase-research); design-first; toinen kärki default-slug-from-title samassa lanessa (jaettu main.rs → sekvensoitu)
LANE A — main.rs (cmd_* + clap) + schema.rs
  ▶ dag-scheduling-view      normal · feature; `lane`/`collision`-frontmatter-kentät (schema.rs) + `issuectl dag`-view-komento (main.rs): join lane+order+blocked_by+live status, head-of-line ON READ, --json. Pidä orkestraattori-agnostinen (reservations valinnaisena inputtina). collision: schema.rs. Design-first.
    default-slug-from-title  normal · feature; johda `new`:n default-slug otsikosta (2–3 sanan kebab) satunnaisen intensifier-adjective-noun sijaan; dedupe numeroliitteellä + --slug-random/-fallback sensitiivisille otsikoille. main.rs (do_new/claim). Sekvensoi dag-scheduling-viewn jälkeen (jaettu main.rs).
LANE B — skill install + templates/ + skill.rs (skill distribution)
    awfully-courageous-attempt  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle (frontmatter-strip kuten /issuella). Build-only-if: vain jos Codex-käyttö todistuu.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: tässä rupeamassa landasi **`json-blocked-by-null-top-level`** (normal bug,
`blocked_by` top-leveliin `show`/`ls`/`search --json`illa) ja **0.7.2 cutattiin** (4 korjausta,
ks. Tila-blokki). Kaikki 0.7.2:n issuet terminal → pudotettu DAG:sta. Tilalle **kaksi uutta
homebase-research-featurea** (2026-08-10): **`dag-scheduling-view`** (normal — DAG-mallinnus
issuectl:ään, `lane`/`collision`-kentät + `dag`-view) ja **`default-slug-from-title`** (normal —
otsikkojohdettu default-slug). Molemmat koskevat `main.rs`:ää → LANE A, sekvensoitu.
**`awfully-courageous-attempt`** (low, build-only-if) pysyy LANE B:ssä parkissa.

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
