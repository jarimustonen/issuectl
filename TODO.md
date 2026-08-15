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

**Tila (2026-08-15, triage + jononrakennus-sessio):** `main` viimeksi tunnettu **vihreä** (1038 testiä),
työpuu: vain tämän handoffin DAG-editit committaamatta (tämä commit hoitaa). **`origin/main` == `main`
(pushattu tämän session alussa — rebase+push, ei enää pushaamattomia committeja).** **Live-release yhä
`v0.10.0`**; mainissa ~17 julkaisematonta feat/fix-committia menossa 0.11.0:aan. `Cargo.toml == 0.10.0`
(ei bumpattu).

**Tämä sessio: EI koodia landattu — triage + uuden työjonon rakennus.** Kulku:
1. **Rebase+push:** `main` oli 23 edellä / 3 jäljessä originia. 3 uutta intake-issuea saapunut (agent-homebase-
   wrapup): `@intake-feature-issuectl-035722451473` (new --lane), `@intake-feature-issuectl-d93eaa168c66`
   (update --add-blocked-by), `@intake-bug-issuectl-d6947128f6c9` (label --remove --json no-op). Rebattu +
   pushattu → haara ehjä.
2. **Deep triage (`/triage-bugs`):** 8 open∧¬laned issuea. DAG oli **täysin tyhjä** (0 lanea) edellisen rupeaman
   jäljiltä.
3. **Uusi työjono rakennettu (@jarin pyyntö "kaikki tulee tehdyksi"):** 7 laned, 1 suljettu. `@issue-graph-view`
   **suljettu `obsolete`** (ydin toimitettu jo `issuectl dag`:na; jos mermaid-export tarvitaan, filaa tuoreena
   kapeana lens-1-issuena).

**Uusi DAG (frontmatter `lane:`/`lane_seq:`/`collision:` asetettu — `issuectl dag` ja tämä TODO nyt yhtäpitävät):**
- **LANE `release`:** `@changelog-trailers-never` (GLOBAL HEAD, high, 0.11.0-blocker).
- **LANE `cli-fixes` (sarjassa, `main.rs`-perhe, bugit ensin):** `@intake-bug-issuectl-d6947128f6c9` →
  `@list-status-done` → `@intake-feature-issuectl-d93eaa168c66` → `@intake-feature-issuectl-035722451473` →
  `@add-comment-alias`.
- **LANE `blocked-upstream` (PARKISSA — ei spawnata):** `@ossctl-cut-no-publish` (upstream-ossctl-blocked).

Molemmat aktiiviset lanet kantavat `collision: crates/issuectl/src/main.rs` → scheduler ei spawnaa kahta
`main.rs`:ää muokkaavaa yhtä aikaa (kaikki CLI-argumentit + dispatch ovat yhdessä `main.rs`:ssä).

**⚠️ RELEASE 0.11.0 EI CUTATTU — blocker `@changelog-trailers-never` (high, GLOBAL HEAD):** changelog on
trailer-vetoinen (`issuectl changelog` kokoaa `Fixes-Issue:`/`Refs-Issue:`-trailereista), mutta MIKÄÄN ei
injektoi niitä (`git_trailers.rs` vain PARSII; ei commit-hookia, ei worktree-merge-steppiä, CONTRIBUTING ei
dokumentoi) → 1/63 committia v0.10.0:n jälkeen kantaa trailerin → 0.11.0:n julkaisunootit tulisivat lähes
tyhjinä. **@jari valitsi OPTION 1:** korjaa juurisyy niin että trailer stampataan automaattisesti kun
run/worktree sulkee issuen (orchestratectl `run merge` ja/tai `issuectl close --commit`). Design-first.
**EI RELEASEA ennen kuin tämä landaa** (ei käsinkuratointia — @jari halusi juurisyyn).

**Seuraava askel:** **design + implementoi `@changelog-trailers-never` (option 1)** — GLOBAL HEAD ja 0.11.0:n
release-blocker. Rinnalla ajettavissa `cli-fixes`-lane (bugit ensin: `label`-json-no-op, `list --status done`),
mutta koska molemmat lanet jakavat `main.rs`-collisionin, scheduler sarjoittaa headit — käytännössä yksi
worktree kerrallaan. Kun `changelog-trailers-never` landaa → cut 0.11.0 (bump 0.10.0→0.11.0 + caret-dep,
CHANGELOG-finalisointi trailereista, `release:`-commit, `ossctl release plan|cut`). `@ossctl-cut-no-publish`
pysyy parkissa kunnes upstream-ossctl korjaa. Kun `cli-fixes` tyhjenee + changelog-trailers landaa + ossctl
ratkeaa → ei enää tehtävää.

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

## Execution DAG (2026-08-15)

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
GLOBAL HEAD-OF-LINE: changelog-trailers-never   ← highest value: 0.11.0 release-blocker, design-first (option 1 = stamp trailer at close/merge). cli-fixes lane spawnable but shares main.rs collision → serialized behind whatever main.rs worker is live. ossctl-cut-no-publish PARKED (upstream-blocked, do not spawn).
LANE release — release pipeline (git_trailers.rs / commit-hook + issuectl close (mutate) + orchestratectl run merge)
  ▶ changelog-trailers-never  high · bug; nothing injects Fixes-Issue/Refs-Issue trailers → trailer-driven `issuectl changelog` compiles near-empty (1/63 commits since v0.10.0) → 0.11.0 release notes would be misleading. DESIGN-FIRST: prefer stamping the trailer at close/merge (option 1 — orchestratectl run merge and/or `issuectl close --commit`). collision: crates/issuectl/src/main.rs
LANE cli-fixes — CLI surface (crates/issuectl/src/main.rs: clap args + cmd_* dispatch); SEQUENTIAL, bugs first
  ▶ intake-bug-issuectl-d6947128f6c9  bug; `label <slug> --remove <l> --json` prints EMPTY stdout + silently skips the mutation (no error envelope, exit 0). Flag-style --remove on positional-only OP; --json must emit a JSON error envelope + non-zero exit. collision: crates/issuectl/src/main.rs
    list-status-done  bug; `list --status done` returns "No issues found" though done issues exist (filter ignores closing statuses done/fixed/wontfix). collision: crates/issuectl/src/main.rs
    intake-feature-issuectl-d93eaa168c66  feature; `update --add-blocked-by @<slug>` / --remove-blocked-by (repeatable) — edit blocked_by via CLI like --add-related; unblocks DAG dependency edges without hand-editing frontmatter. collision: crates/issuectl/src/main.rs
    intake-feature-issuectl-035722451473  feature; `new --lane <lane>` (+ ideally --lane-seq) so an issue is born into the DAG in one call; mirror update --lane. collision: crates/issuectl/src/main.rs
    add-comment-alias  feature; add `comment` alias for `note` and/or accept --message/--body(-file -) alongside the positional body; vocabulary split (close has --comment, note is positional). collision: crates/issuectl/src/main.rs
    sync-commits-empty-main  bug; `sync-commits` default range on `main` is often HEAD..HEAD (merge-base == HEAD) → scans no commits, reports empty plan silently. Warn when the default range is empty on main. collision: crates/issuectl/src/main.rs
LANE blocked-upstream — PARKED, do not spawn until upstream-ossctl fix lands
    ossctl-cut-no-publish  high · bug; ossctl release cut doesn't actually publish → manual cargo publish. BLOCKED upstream-ossctl; when fixed, remove AGENTS caveat + re-point releases to ossctl.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: **triage + jononrakennus-sessio (ei koodia landattu).** Edellinen 9-issuen DAG oli tyhjennetty,
DAG jäi tyhjäksi (0 lanea). Tässä sessiossa: rebase+push (3 uutta intake-issuea originista), deep triage 8:sta
open∧¬laned issuesta, ja **uusi DAG rakennettu** @jarin "kaikki tulee tehdyksi" -pyynnöstä. 7 laned 3 laneen
(`release`, `cli-fixes` sarjassa, `blocked-upstream` parkissa), 1 suljettu (`issue-graph-view` obsolete). GLOBAL
HEAD `changelog-trailers-never` (0.11.0-blocker, design-first, option 1). `cli-fixes` jakaa `main.rs`-collisionin
`release`-lanen kanssa → scheduler sarjoittaa. `ossctl-cut-no-publish` parkissa (upstream-blocked). Kun changelog-
trailers landaa + cli-fixes tyhjenee + ossctl ratkeaa → ei enää tehtävää.

---

## Adjacent backlog (deferred — DAG:n ulkopuolella, ei ajossa)

Kaikki alla on labeloitu `deferred` issuectl:ssä (2026-08-04), joten ne eivät ole DAG-lanella
eivätkä laukaise drift-checkiä. Poista `deferred`-label kun otat takaisin peliin.

**Web/selain-UI: POISTETTU (`@remove-web-ui` done, 0.10.0).** Ei enää backlogissa. Kaikki entiset
kanban/web-enhancement-issuet (13 kpl) oli jo suljettu `obsolete` (2026-08-10).

**CLI-only visualisointi: `@issue-graph-view` SULJETTU `obsolete` (2026-08-15).** Ydin (worktree-planning-lens)
toimitettu jo `issuectl dag`:na; loput (mermaid/dot/svg-export) on kapea erillinen scope. Jos kaavioviennille
tulee konkreettinen tarve, filaa **tuore** kapea issue "lens 1: dep→mermaid" — älä reopenaa koko vanhaa visiota.
(`@epic-tree-view` shipattiin aiemmin.)

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

_Kaikki kolme triageed + foldattu DAGiin (2026-08-15, `needs-triage` poistettu) — nyt `cli-fixes`-lanen nodeja,
ks. Execution DAG yllä. Ei enää untriaged-jonossa._

- [x] 🐛 issuectl new: accept --lane → `cli-fixes` ([`intake-feature-issuectl-035722451473`](issues/intake-feature-issuectl-035722451473/item.md))
- [x] 🐛 issuectl update: add --add-blocked-by → `cli-fixes` ([`intake-feature-issuectl-d93eaa168c66`](issues/intake-feature-issuectl-d93eaa168c66/item.md))
- [x] 🐛 label: --remove --json silent no-op → `cli-fixes` (HEAD of lane) ([`intake-bug-issuectl-d6947128f6c9`](issues/intake-bug-issuectl-d6947128f6c9/item.md))
- [ ] 🐛 Piialiisan bugiraportti: issuectl label: accept --add/--remove flag aliases (canonical skills us… — jari via Telegram ([`intake-feature-issuectl-986ecd5a58a9`](issues/intake-feature-issuectl-986ecd5a58a9/item.md))
