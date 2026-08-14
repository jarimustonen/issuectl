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

**Tila (2026-08-14, jatkettu rupeama):** `main` **vihreä** (1038 testiä, 0 fail, 0 uutta clippyä,
fmt puhdas), työpuu puhdas. **Local `main` 22 committia edellä `origin/main`:ia (pushaamatta —
rinnakkaisworktree-politiikka).** **Live-release yhä `v0.10.0`**; mainissa ~17 julkaisematonta
feat/fix-committia (session 1:n 8 + tämän session 8) menossa 0.11.0:aan. `Cargo.toml == 0.10.0` (ei bumpattu).

**Tämä sessio: 9-issuen DAG TYHJENNETTY.** 8 yksikköä landasi mainiin, kaikki `/llm-review`+`/assess-findings`
läpi, 3 aaltoa (rinnakkain paitsi main.rs-kollisiot sarjassa):
- **Aalto 1:** `@flock-write-lock` (release-gaten flaky-testi vihreäksi — deflake fresh-fd try_lock retry, 0/80 @32 threads),
  `@dag-inprogress-is-spawnable` (ISO LINJAUS implementoitu), `@pi-mirror-hint-accuracy`.
- **Aalto 2:** `@dag-reservations-run-id-object-shape`, `@configsource-load-return-value` (load return-by-value),
  `@action-verb-json-echo-mutation`.
- **Aalto 3 (sarjassa, main.rs):** `@as-flag-strip-at-sign` (`--as @jari` strippaa yhden @:n), `@update-set-body-flag`
  (`update --body-file/--description`, stdin `-`).

**⚠️ RELEASE 0.11.0 EI CUTATTU — blocker `@changelog-trailers-never` (high, GLOBAL HEAD):** changelog on
trailer-vetoinen (`issuectl changelog` kokoaa `Fixes-Issue:`/`Refs-Issue:`-trailereista), mutta MIKÄÄN ei
injektoi niitä (`git_trailers.rs` vain PARSII; ei commit-hookia, ei worktree-merge-steppiä, CONTRIBUTING ei
dokumentoi) → 1/63 committia v0.10.0:n jälkeen kantaa trailerin → 0.11.0:n julkaisunootit tulisivat lähes
tyhjinä. **@jari valitsi OPTION 1:** korjaa juurisyy niin että trailer stampataan automaattisesti kun
run/worktree sulkee issuen (orchestratectl `run merge` ja/tai `issuectl close --commit`). Design-first.
**EI RELEASEA ennen kuin tämä landaa** (ei käsinkuratointia — @jari halusi juurisyyn).

**⚠️ PROSESSI-HAVAINTO — worktree-workerit eivät sulkeneet issueitaan:** 6:sta autonomisesta spinoffista
**4 mergesi koodin mutta jätti issuen `open`ksi** (dag-reservations, action-verb, as-flag, update-set-body).
Konduktori sulki ne itse verifioituaan sisällön mainista (+ kirjasi landing-commitin). **Sama juurisyy kuin
changelog-trailers:** mikään ei stamppaa "valmis"-metadataa merge-hetkellä. `@changelog-trailers-never`-korjaus
(close/merge → stamppaa) voi kattaa molemmat.

**SPIN-OFF-LAADUN TARKKAILU → NOSTETTU GEENERISEKSI SKILLISÄÄNNÖKSI:** projektikohtainen vahtisääntö on
siirretty yleiseksi standing-disciplineksi **`/stint-handoff`-skilliin (orchestratectl-repo)** — worktree
`spinoff-quality-watch-rule` (run `01m003xp0z`). Nyt voimassa **kaikissa** stinteissä. Tämän session havainto
piti: as-flag-workerin review 8/8 löydöstä → DROP; reviewit eivät tuottaneet turhaa hiomista tällä kierroksella.

**ISO LINJAUS (in-progress spawnable) IMPLEMENTOITU** — `@dag-inprogress-is-spawnable` landattu (poistettu
`!underway` + `IN_PROGRESS`-vakio dag.rs:stä, docstring/AGENTS/CHANGELOG päivitetty, testi lisätty). **Sisar
orchestratectl-repossa:** `stint-head-of-line-in-progress-eligible` — tarkista onko vielä auki, linjaa
head-of-line-konventio samaan.

**Seuraava askel:** **design + implementoi `@changelog-trailers-never` (option 1)** — se on GLOBAL HEAD ja
0.11.0:n release-blocker. Kun se landaa → cut 0.11.0 (bump 0.10.0→0.11.0 + caret-dep, CHANGELOG-finalisointi
trailereista, `release:`-commit, `ossctl release plan|cut`). Sitten jää vain `@ossctl-cut-no-publish`
(upstream-blocked). Kun molemmat LANE C -issuet ratkennut → ei enää tehtävää.

**⚠️ RELEASE-OPPI (yhä voimassa):** ossctl `release cut` EI julkaise oikeasti (`@ossctl-cut-no-publish`,
upstream-blocked) → seuraava release vaatii manuaalisen `cargo publish -p issuectl-core` → `-p issuectl`
→ tag → push -fallbackin. `ossctl release cut` julkaisee PUUN version, EI bumppaa → bump +
CHANGELOG-finalisointi + `release:`-commit ENNEN cutia.

**⚠️ Minor-bump-gotcha:** `crates/issuectl/Cargo.toml`:n sisäinen `issuectl-core = { …, version = "X" }`
on caret-vaatimus → bumppaa se vastaamaan uutta minoria samassa release-commitissa (0.10.0 → 0.11.0).
Vain minor/major-raja vaatii tämän, ei patch.

**⚠️ Autonomy:** deployt/releaset TÄYSIN autonomisia (ei go/no-go, ei output-reviewia) — @jarin ohje 2026-08-10.

**Dogfood:** `cargo install issuectl` / `brew upgrade jarimustonen/issuectl/issuectl` / `cargo install
--path crates/issuectl`. Skillit `/issue`, `/issue-new`, `/issue-intake` tulevat `issuectl skill install`ista;
bugit/feature-pyynnöt sisään `issuectl intake file`lla, `/issue-intake` (tai `/stint-start`) nostaa jonon.

**OSS-init:** `OSS-RELEASE.md` approved (`mvp`). cargo-dist release-engine — `/oss-release-cut` EI
regeneroi `release.yml`:ää.

---

## Execution DAG (2026-08-14)

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
GLOBAL HEAD-OF-LINE: changelog-trailers-never   ← blocks a quality 0.11.0 release; design-first (pick trailer-injection approach). ossctl-cut-no-publish still upstream-blocked. LANE A/B drained this round (7 units landed).
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl + git_trailers.rs / commit-hook)
    changelog-trailers-never  high · bug; nothing injects Fixes-Issue/Refs-Issue trailers → trailer-driven `issuectl changelog` compiles near-empty (1/63 commits since v0.10.0) → 0.11.0 release notes would be misleading. DESIGN-FIRST: prefer stamping the trailer at close/merge (option 1 — orchestratectl run merge and/or `issuectl close --commit`). Touches issuectl close (mutate/cmd_close) and/or git_trailers.rs + orchestratectl run merge. Until fixed, a release needs a HAND-CURATED CHANGELOG [Unreleased].
    ossctl-cut-no-publish  high · bug; ossctl release cut doesn't actually publish → manual cargo publish. BLOCKED upstream-ossctl; when fixed, remove AGENTS caveat + re-point releases to ossctl.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: **9-issuen DAG tyhjennetty tässä sessiossa** — 8 yksikköä landasi mainiin (flock-release-gate
vihreäksi, in-progress-spawnable-linjaus, dag-reservations, configsource-load-by-value, action-verb-json-echo,
as-flag-@-strip, update-set-body, pi-mirror-hint) 3 aallossa, kaikki `/llm-review` läpi. **Release 0.11.0 EI
cutattu:** blocker `changelog-trailers-never` (trailer-vetoinen changelog kokoaa tyhjää koska mikään ei injektoi
trailereita) — @jari valitsi option 1 (juurisyy: stamppaa trailer close/merge-hetkellä). DAG nyt **2 aktiivia,
molemmat LANE C:** `changelog-trailers-never` (GLOBAL HEAD, design-first, release-blocker) + `ossctl-cut-no-publish`
(upstream-blocked). Kun molemmat ratkennut → ei enää tehtävää. Spin-off-laadun vahtisääntö nostettu geneeriseksi
`/stint-handoff`-skillisäännöksi (orchestratectl-repo).

---

## Adjacent backlog (deferred — DAG:n ulkopuolella, ei ajossa)

Kaikki alla on labeloitu `deferred` issuectl:ssä (2026-08-04), joten ne eivät ole DAG-lanella
eivätkä laukaise drift-checkiä. Poista `deferred`-label kun otat takaisin peliin.

**Web/selain-UI: POISTETTU (`@remove-web-ui` done, 0.10.0).** Ei enää backlogissa. Kaikki entiset
kanban/web-enhancement-issuet (13 kpl) oli jo suljettu `obsolete` (2026-08-10).

**CLI-only visualisointi (deferred):**
`@issue-graph-view` (ent. massively-periodic-surprise) — `issuectl graph` -moottori + lensit
(deps / worktree-planning / epic-rollup); lens 2 osin jo `issuectl dag`:ssä. **EI kevyt** (koko graph-
moottori + mermaid/dot/svg); jos tarve konkretisoituu tee vain lens 1 (dep→mermaid).
(`@epic-tree-view` shipattiin tässä sessiossa — ei enää tässä eikä DAG:ssa.)

**Strateginen:** `@focus-areas` **suljettu `wontfix` (2026-08-10)** — ei nyt tarvetta. Ylätason
päätös (ADR 0001: `areas: []` skeemakenttä) on tallessa; reopen + kirjoita implementaatio-ADR jos
tarve palaa.

_(Review-cascaden 14 wontfix-spin-offia 2026-08-14: pi-korpuksen defensiiviset kovennukset
(`fd-relative-hardening`, `manifest-fsync-durability`, `mirror-atomic-writes`, `cross-tool-lock`,
`prune-digest-gate`, `owned-symlink-unmanaged-hidden`, `prune-report-inaccessible`, `status-check-exit`,
`status-shared-lock`), `dag-inprogress-schema-aware` (korvattu `dag-inprogress-is-spawnable`:lla),
`epic-tree-human-render-control-chars`, `epic-tree-view-filters`, `load-once-thread-schema`,
`new-and-update-blocked-by`. Perustelut issueiden `wontfix`-kommenteissa.)_

---

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin JA mergeää
Execution DAG:n (drop landed, add active, keep order) rupeaman lopussa, ja committaa ne
omana committinaan (`git add TODO.md issues/ && git commit`) ennen `/wrap-up`:ia — näin
tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md).

## Piialiisan bugiraportit

- [ ] 🐛 Piialiisan bugiraportti: issuectl new: accept --lane (set scheduling lane at creation) — jari via Telegram ([`intake-feature-issuectl-035722451473`](issues/intake-feature-issuectl-035722451473/item.md))
- [ ] 🐛 Piialiisan bugiraportti: issuectl update: add --blocked-by / --add-blocked-by (edit blocked_by v… — jari via Telegram ([`intake-feature-issuectl-d93eaa168c66`](issues/intake-feature-issuectl-d93eaa168c66/item.md))
- [ ] 🐛 Piialiisan bugiraportti: label: flag-style --remove silently no-ops with --json instead of error… — jari via Telegram ([`intake-bug-issuectl-d6947128f6c9`](issues/intake-bug-issuectl-d6947128f6c9/item.md))
