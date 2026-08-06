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

**Tila (2026-08-06, myöhempi rupeama):** viimeisin **julkaistu** versio on **v0.7.1**
(GitHub Release + Homebrew + crates.io indeksissä). MUTTA `main` on nyt **7 committia
tagia edellä** — landattua mutta **JULKAISEMATONTA** työtä (3 close/show-polun korjausta,
alla). `main` ≠ julkaistu; seuraava release olisi **0.7.2** (tai 0.8.0). Työpuu puhdas.

**Tässä rupeamassa landannut mainiin (JULKAISEMATTA — CHANGELOG `[Unreleased]` päivitettävä
release-cutissa):**
- **@show-json-omits-blocked-by (bug, normal, fixed)** — `issuectl --json show` tulostaa nyt
  top-level `blocked_by`in (kanoninen, `@`-prefiksoitu) + johdetun käänteisen `blocks`-näkymän.
  Juurisyy: `blocked_by` asui `Issue::extra`-mapissa, plain serde upotti sen; cmd_show:n
  --json-polku nostaa sen top-leveliin ja strippaa raakakopion (yksi esitys johdolla).
  Regressio- + kanonisointitestit. 3/3 llm-review-konsensus, green gate ok.
- **@intensely-blushing-galley (improvement, low, done)** — `closed_by` nostettu `extra`-mapista
  ensiluokkaiseksi typed-kentäksi (`Issue.closed_by`): mukana `canonical_hash`issa (taaksepäin-
  yhteensopiva), skeemassa, `doctor`-heal varoittaa aktiivistatuksilla, näkyy `show --json` +
  human show ("Closed by:"), human `close` kaikuttaa "(by <author>)" `--as`illa. Legacy
  `extra["closed_by"]` migratoituu lukiessa.
- **@close-comment (feature, low, done)** — `close --comment/--note "<text>"` liittää
  aikaleimatun `## Resolution`-lohkon samassa atomisessa kirjoituksessa; komponoituu
  `--status/--as/--commit`in kanssa. 4-malli-llm-review, yksi oikea parser-fix sovellettu.

**Aiemmat julkaistut (0.7.1, taustaa):** @distribute-intake-skills (`skill install` jakaa
`/issue-new` + `/issue-intake`), @verify-intake-split-queue (§6 split queue verifioitu). Vain
Claude-Code-formaatti skilleillä (Codex-variantti = @awfully-courageous-attempt, build-only-if).

**Seuraava release-huomio:** kun 0.7.2 cutataan, päivitä CHANGELOG `[Unreleased]` yllä olevilla
3 kohdalla ennen tagia.

**⚠️ crates.io-caveat (ennallaan, muista JOKA release):** tag-push laukaisee cargo-dist
(GitHub Release + Homebrew) mutta **crates.io-julkaisu vaatii manuaalisen triggerin**
(`gh workflow run "Publish to crates.io"`) — `GITHUB_TOKEN` ei laukaise `publish-crates.yml`:ää.
0.7.1:ssä ajettiin käsin release.yml:n vihertymisen jälkeen ja onnistui. (Korjaus odottaa
`@wire-oss-release-as-release-path`ia.)

**Dogfood:** Käyttäjä dogfoodaa issuectl:ää omissa projekteissaan. Asennus: `cargo install
issuectl` (crates.io), `brew upgrade jarimustonen/issuectl/issuectl`, tai `cargo install
--path crates/issuectl`. 0.7.1:stä skillit `/issue`, `/issue-new`, `/issue-intake` tulevat
kaikki `issuectl skill install`ista; bugit/feature-pyynnöt sisään `issuectl intake file`lla,
`/issue-intake` (tai `/stint-start`) nostaa jonon seuraavan kerran.

**OSS-init (ennallaan):** `OSS-RELEASE.md` **approved** (`mvp`). cargo-dist pysyy
release-engineinä — `/oss-release-cut` EI regeneroi `release.yml`:ää.

**Iso linjaus (ennallaan):** **kanban/web-board holdissa**. Kaikki kanban/web-issuet +
build-only-if + `@focus-areas` `deferred` → **Adjacent backlog**. Fokus CLI:ssä.

**Seuraava askel:** aktiivinen backlog on käytännössä **tyhjä** — kaikki tämän rupeaman
DAG-työ landasi. Jäljellä DAG:issa vain **@awfully-courageous-attempt** (low, build-only-if:
vain jos Codex-käyttö todistuu). Kaksi realistista seuraavaa liikettä: (a) **cutata 0.7.2**
(3 julkaisematonta korjausta mainissa — päivitä CHANGELOG, tag, muista crates.io manual
trigger) — vaatii käyttäjän go/no-go:n; tai (b) odottaa lisää dogfood-feedbackia ja nostaa
uudet issuet DAG:iin. Ei kiirettä; feedback-issuet menevät edelle jos niitä tulee.

---

## Execution DAG (2026-08-06)

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
GLOBAL HEAD-OF-LINE: awfully-courageous-attempt   ← ainoa aktiivinen ei-deferred issue (low, build-only-if); LANE A tyhjeni (kaikki close-polun työ landasi)
LANE B — skill install + templates/ + skill.rs (skill distribution)
  ▶ awfully-courageous-attempt  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle (frontmatter-strip kuten /issuella). Build-only-if: vain jos Codex-käyttö todistuu.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: kolme close/show-polun issueta landasi tässä rupeamassa — **`show-json-omits-blocked-by`**
(bug), **`intensely-blushing-galley`** (typed `closed_by`) ja **`close-comment`** (`close --comment`);
kaikki pudotettu DAG:sta (terminal). LANE A tyhjeni kokonaan. Jäljellä vain LANE B:n
**`awfully-courageous-attempt`** (low, build-only-if), joka on nyt GLOBAL HEAD-OF-LINE muun puuttuessa.
Huom: nämä 3 ovat mainissa mutta **julkaisematta** — seuraava release cuttaa 0.7.2:n.

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
