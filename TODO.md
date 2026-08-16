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

**Tila (2026-08-16, 0.11.0-release + cli-fixes-rupeama):** `main` **vihreä** (1058 testiä, fmt+clippy puhtaat —
0 uutta clippy-varoitusta, 52 pre-existing). **`origin/main` == `main` (pushattu).** **Live-release nyt `v0.11.0`**
(crates.io + binäärit + Homebrew + tag kaikki ulkona). `Cargo.toml == 0.11.0`. **Aktiivinen työ tyhjä** — DAG:ssa
vain parkissa oleva `@ossctl-cut-no-publish` (upstream-blocked). Ei ajossa olevia workereita.

**Tämä sessio: 7 unittia landattu + 0.11.0 julkaistu.** Kulku:
1. **DAG-merge:** foldattu uusi `@sync-commits-empty-main` `cli-fixes`-laneen (collision main.rs, seq 60).
2. **Release-lane:** `@changelog-trailers-never` (GLOBAL HEAD) landattu design-first — **`issuectl close --stamp`**
   amendaa HEADin `Fixes-Issue: @<slug>`-trailerilla (option 1, issuectl-puoli; 4-model review). Trailer-vetoinen
   `issuectl changelog` toimii nyt eteenpäin nolla-vaivalla.
3. **cli-fixes-lane (kaikki sarjassa `main.rs`-collisionin takia, kukin reviewattu + vihreä + trailer-stampattu):**
   `@intake-bug-issuectl-d6947128f6c9` (label --remove --json error-envelope + flag-form), `@list-status-done`
   (--status done palauttaa closed/archived), `@intake-feature-issuectl-d93eaa168c66` (update --add-blocked-by),
   `@intake-feature-issuectl-035722451473` (new --lane/--lane-seq/--add-collision), `@add-comment-alias`
   (comment-alias + --message/--body-file), `@sync-commits-empty-main` (tyhjän default-rangen varoitus).
4. **RELEASE 0.11.0 (jari valitsi OPTION a — kertaluontoinen käsinbackfill):** CHANGELOG `[0.11.0]` kuratoitu
   käsin kattaen KAIKKI v0.10.0:n jälkeen toimitetut unitit (tämän session 8 + ~15 historiallista trailerittomasta
   pushatusta committista), + huomautus että trailer-automaatio alkoi kesken syklin (0.12.0→ automaattinen).
   Bump 0.10.0→0.11.0 (root + caret-dep), `release: 0.11.0`-commit, push.
5. **ossctl cut EPÄONNISTUI (odotettu `@ossctl-cut-no-publish`-bugi):** publish-phase failasi, MITÄÄN ei uploadattu
   (varmistettu crates.io-indeksistä). → **manuaalinen fallback (AGENTS RELEASE-OPPI):** `cargo publish -p
   issuectl-core` → `-p issuectl` → `git tag v0.11.0` → push tag. Molemmat cratet crates.io:ssa, release.yml
   (cargo-dist) ajoi vihreänä → binäärit + Homebrew-formula + GitHub Release.

**Sivutuotteet:**
- **Orchestratectl-issue filattu:** `run-merge-stamp` (orchestratectl-repo) — `run merge` pitäisi stampata
  Fixes-Issue-traileri landing-committiin, jotta "nolla-vaivaa"-lupaus on end-to-end (issuectl-puoli tehty
  `close --stamp`:llä; tämä on toinen puolisko). Tähän asti workereille briefataan `close --stamp`.
- **Intake-duplikaatti suljettu:** `@intake-feature-issuectl-986ecd5a58a9` (label --add/--remove flag-aliakset)
  suljettu `obsolete` — jo toimitettu 0.11.0:ssa (`@intake-bug-issuectl-d6947128f6c9`).

**Seuraava askel:** **ei aktiivista työtä.** Kaikki laned unitit landattu, 0.11.0 ulkona. `@ossctl-cut-no-publish`
pysyy parkissa kunnes upstream-ossctl korjaa. Uusi rupeama = odota intakea (`/issue-intake` / `/stint-start`
nostaa jonon) tai ota `deferred`-backlogista jokin takaisin peliin. Jos releaset halutaan taas ossctl:n kautta,
se vaatii `@ossctl-cut-no-publish`-fixin ensin.

**⚠️ RELEASE-OPPI (VAHVISTUI TÄSSÄ SESSIOSSA):** ossctl `release cut` EI julkaise oikeasti
(`@ossctl-cut-no-publish`, upstream-blocked) — publish-phase failaa ("core not visible on index within 300s"),
MITÄÄN ei uploadata. → release vaatii manuaalisen `cargo publish -p issuectl-core` → (odota indeksi) →
`-p issuectl` → `git tag vX.Y.Z <release-commit>` → `git push origin vX.Y.Z` -fallbackin. Tag laukaisee
`release.yml`:n (cargo-dist) → binäärit + Homebrew (EI double-publishaa crates.io:ta). Ennen fallbackia: bump +
CHANGELOG-finalisointi + `release:`-commit + push main. **Varmista aina crates.io-indeksistä ETTEI core jo
uploadattu** ennen manuaalista publishia (double-publish failaa). ossctl-run jää `in_progress`-tilaan sen omaan
journaliin (`release verify <run-id>` näyttää unreconciled — vaaraton, ei estä mitään).

**⚠️ Minor-bump-gotcha:** `crates/issuectl/Cargo.toml`:n sisäinen `issuectl-core = { …, version = "X" }`
on caret-vaatimus → bumppaa se vastaamaan uutta minoria samassa release-commitissa (0.10.0 → 0.11.0).
Vain minor/major-raja vaatii tämän, ei patch.

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

## Execution DAG (2026-08-16)

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
GLOBAL HEAD-OF-LINE: (none spawnable) — all active work drained (7 units landed + 0.11.0 released 2026-08-16). Only ossctl-cut-no-publish remains, PARKED (upstream-blocked, do not spawn). Next round: wait for intake or pull something back from the deferred backlog.
LANE blocked-upstream — PARKED, do not spawn until upstream-ossctl fix lands
    ossctl-cut-no-publish  high · bug; ossctl release cut doesn't actually publish → manual cargo publish (CONFIRMED AGAIN in the 0.11.0 cut: publish-phase fails "core not visible on index within 300s", nothing uploaded). BLOCKED upstream-ossctl; when fixed, remove AGENTS caveat + re-point releases to ossctl.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: **0.11.0-release + cli-fixes-rupeama.** GLOBAL HEAD `changelog-trailers-never` landattu design-first
(`close --stamp` → trailer-stamppaus, option 1), sitten `cli-fixes`-lane tyhjennetty sarjassa (`main.rs`-collision):
6 unittia, kukin reviewattu + vihreä + trailer-stampattu. Sitten **0.11.0 julkaistu** (jari: option a = kertaluontoinen
CHANGELOG-käsinbackfill kattaen kaikki v0.10.0:n jälkeen toimitetut unitit). ossctl cut failasi odotetusti
(`ossctl-cut-no-publish`) → manuaalinen `cargo publish` + tag-fallback → crates.io + binäärit + Homebrew ulkona.
Sivussa: `run-merge-stamp` filattu orchestratectl-repoon (end-to-end trailer-stamppauksen toinen puolisko),
intake-duplikaatti `intake-feature-issuectl-986ecd5a58a9` suljettu obsolete. **DAG nyt tyhjä paitsi parkissa oleva
`ossctl-cut-no-publish`.**

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
- [x] 🐛 label: --remove --json silent no-op → landattu 0.11.0:ssa ([`intake-bug-issuectl-d6947128f6c9`](issues/intake-bug-issuectl-d6947128f6c9/item.md))
- [x] 🐛 label: accept --add/--remove flag aliases → **suljettu `obsolete` (2026-08-16)**, duplikaatti — jo toimitettu 0.11.0:ssa `intake-bug-issuectl-d6947128f6c9`:llä ([`intake-feature-issuectl-986ecd5a58a9`](issues/intake-feature-issuectl-986ecd5a58a9/item.md))
