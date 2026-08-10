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

**Tila (2026-08-10, iltapäivän rupeama):** **0.8.1 JULKAISTU JA VARMISTETTU — kaikki 3 kanavaa vihreä.**
crates.io molemmat cratet `200`, tag `v0.8.1` originissa, `release.yml`-run `31395041120`
**completed/success** (binäärit + shell-installer + Homebrew). `main` == origin (`0b49850`), työpuu
puhdas. Rupeama alkoi siivous-/linjausrupeamana, laajeni: dag-polish (2 featurea + 1 bugfix) + wire-oss
landasivat ja **0.8.1 cutattiin — mutta manuaalisesti**, koska `ossctl release cut` osoittautui rikki.

**⚠️ RELEASE-OPPI (tärkein): ossctl `release cut` EI julkaissut oikeasti → 0.8.1 tehtiin `cargo publish`illa.**
Real cut ei uploadannut crates.io:hon (raportoi 300s "index visibility" -timeoutin cratelle jota se ei
koskaan uploadannut; crate 404:ssä 9 min, ei receiptejä). Juurisyy todennäk. ossctl:n publish-adapterin
bug (dry-run/no-op oikeassa cutissa), jäi kiinni koska `@wire-oss-release-as-release-path` verifioi vain
`release plan`-dry-runilla. 0.8.1 pelastettiin manuaalilla `cargo publish -p issuectl-core` → `-p issuectl`
→ tag → push. **EI jätetä workaroundiksi:** bug filattu upstream-ossctl-reppuun **`release-cut-publish-noop`**
(high) + downstream-tracker tässä **`@ossctl-cut-no-publish`** (high, DAG LANE C). AGENTS.md kuvaa taas
`ossctl release cut`:n ainoana polkuna (workaround-stepit poistettu käyttäjän pyynnöstä — korjataan, ei
kierretä). **Toinen 0.8.1-opetus (AGENTS.md:ssä):** `ossctl release cut` julkaisee PUUN version — se EI
bumppaa; siksi bump + CHANGELOG-finalisointi + `release:`-commit tehdään ENNEN cutia.
(Sekundäärifriktio: stale-lock esti `release abandon`in tapetun cutin jälkeen → ossctl
`@release-abandon-break-stale-lock`, oli jo filattu, lisätty repro-kommentti.)

**Mitä tässä rupeamassa tehtiin:**
- **0.8.1 sisältö (2 featurea + 1 bugfix, CHANGELOG `[0.8.1]`):** dag-polish — `issuectl dag` ei enää
  raportoi `spawnable=true` in-progress-issuelle (`@dag-inprogress-spawnable`, fixed); uusi valinnainen
  `lane_seq:<int>` intra-lane-sort + `update --lane-seq` (`@dag-stable-intralane-order`, done); `lane:
  unlaned`-sentinel confirmed-parallel-safe (`@dag-unlaned-parallel-sentinel`, done). Molemmat schema-
  kentät `closed_by`-precedentillä (canonical_hash vain kun set, ei schema-bumppia). + wire-oss:
  `publish-crates.yml` eläkkeelle, docs `ossctl release`-polulle (`@wire-oss-release-as-release-path`,
  done — mutta polku osoittautui rikki, ks. yllä). Molemmat 2 worktreeta landasivat, green gate vihreä.
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

**⚠️ Autonomy (VAHVISTETTU tässä rupeamassa):** `AGENTS.md`:n operating-faktoissa deployt/releaset
ovat **TÄYSIN autonomisia — ei go/no-go, ei output-reviewia** (käyttäjän eksplisiittinen ohje 2026-08-10:
"deployt ja releaset saa tehdä automaattisesti"). 0.8.1 cutattiin sen nojalla.

**Aiemmat releaset (pointeri, ei toistoa):** 0.8.0 = `@dag-scheduling-view` (lane/collision-kentät +
`issuectl dag`-view) + `@default-slug-from-title` (otsikkojohdettu default-slug). Täydet muutokset
CHANGELOG `[0.8.0]`/`[0.8.1]`:ssä; ei toisteta tässä.

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

**Seuraava askel:** aktiivinen (ei-deferred) jono: **`@ossctl-cut-no-publish`** (high, bug — mutta
juurisyy upstream-ossctl:ssä; ei koodattavaa täällä ennen kuin ossctl korjaa, sitten poista AGENTS-
caveat + re-point), **`@warn-reserved-notes-section`** (low, homebase-filaama feature — varoita kun
issue-body käyttää varattua `## Notes`-sektiota), **`@codex-prompt-variants`** (low, build-only-if).
Deferred: portitettu `@remove-web-ui` (odottaa seuraaja-luonnosta), CLI-only `@issue-graph-view` +
`@epic-tree-view`, `@events-jsonl-log` (build-only-if). `@focus-areas` suljettu `wontfix` (ei tarvetta;
ADR 0001 tallessa). Uusi rupeama nostaa jonosta (warn-reserved-notes-section on ainoa selkeästi
koodattava) tai uudesta intakesta.

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
GLOBAL HEAD-OF-LINE: warn-reserved-notes-section   ← ainoa selkeästi koodattava; ossctl-cut on upstream-blocked
LANE A — main.rs (cmd_* + clap) + mutate/ + parser/body_sections
    warn-reserved-notes-section  low · feature; varoita authoring-aikaan kun issue-body käyttää varattua ## Notes -sektiota (homebase-filaama)
LANE B — skill install + templates/ + skill.rs (skill distribution)
    codex-prompt-variants  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle. Build-only-if: vain jos Codex-käyttö todistuu.
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl)
    ossctl-cut-no-publish  high · bug; ossctl release cut ei julkaise oikeasti → manuaalinen cargo publish. BLOCKED upstream-ossctl:llä; kun korjattu, poista AGENTS-caveat + re-point releaset ossctliin.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: rupeama alkoi siivouksena, laajeni: **dag-polish** (3 dag-gappia yhtenä worktreena) +
**wire-oss** landasivat, **0.8.1 cutattiin** — mutta ossctl-release-polku osoittautui rikki, joten
0.8.1 julkaistiin manuaalisesti `cargo publish`illa (bug `@ossctl-cut-no-publish`, high, upstream-
blocked). dag-trio + wire-oss terminal → pudotettu DAG:sta. Tilalle nousi 2 uutta homebase-filaamaa:
`warn-reserved-notes-section` (low, koodattava) + `ossctl-cut-no-publish` (high, upstream-blocked).
`codex-prompt-variants` pysyy LANE B:ssä build-only-if-parkissa.

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
`@events-jsonl-log` (ent. somewhat-heady-zephyr; events.jsonl — vain jos git-metriikat ei riitä).

**Strateginen:** `@focus-areas` **suljettu `wontfix` (2026-08-10)** — ei nyt tarvetta. Ylätason
päätös (ADR 0001: `areas: []` skeemakenttä) on tallessa; reopen + kirjoita implementaatio-ADR jos
tarve palaa.

_(`@wire-oss-release-as-release-path` on suljettu `done` — mutta paljasti `@ossctl-cut-no-publish`in:
ossctl release cut ei julkaise oikeasti → releaset manuaalisesti, ks. Tila-blokki. Se bug on DAG:n
LANE C:ssä, ei tässä.)_

---

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin JA mergeää
Execution DAG:n (drop landed, add active, keep order) rupeaman lopussa, ja committaa ne
omana committinaan (`git add TODO.md issues/ && git commit`) ennen `/wrap-up`:ia — näin
tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md).
