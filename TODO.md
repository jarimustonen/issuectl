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

**Tila (2026-08-12, iso rupeama — KAKSI releasea):** **0.10.0 LIVE ja varmistettu.** Tag `v0.10.0`
originissa, molemmat cratet crates.io:ssa (`cargo publish`-fallback, ossctl yhä rikki), **`release.yml`
completed/success sekä 0.9.0:lle että 0.10.0:lle** (binäärit + shell-installer + Homebrew). `main == origin`,
työpuu puhdas, `v0.10.0` == `Cargo.toml`. **8 issueä suljettu/shipattu, kaksi releasea:**
- **0.9.0** = 4 featurea + 1 fix: `@warn-reserved-notes-section`, `@codex-prompt-variants`,
  `@pidev-dual-home-skills` (pi.dev WS4 dual-home), `@doctor-fix-merge-notes-comments`, ja fix
  `@rate-limit-test-flaky` (deterministinen limiter-testi injektoitavalla kellolla). `@note-missing-as-generic-error`
  suljettiin `fixed` (ei toistunut HEADilla → vain regressiotestit). `@events-jsonl-log` → `wontfix`.
- **0.10.0** = **BREAKING: `@remove-web-ui`** — koko selain/web-UI poistettu (`issuectl serve`, server/HTTP-layer,
  `/api`, kanban-frontend, SSE/watcher, user-boards, RepoConfigCache, `issuectl docs`). issuectl on nyt
  **puhdas AI-first CLI**. 12 web-only-deppiä pruunattu; 4-mallin `/llm-review` ei löytänyt korrektiusbugia.
  Review filasi 3 follow-upia (ks. Seuraava askel).

Rupeaman kaari: käyttäjä ajoi ensin 3 kaistaa rinnan (warn-notes + codex-variants landasivat), sitten
kiireellinen pidev-dual-home (WS4) landasi. CI meni hetkeksi punaiseksi flaky rate-limit-testistä →
korjattiin (injektoitava kello) → vihreä. Cutattiin **0.9.0** (3+1 featurea + fix). Sitten käyttäjä antoi
go:n web-UI:n poistolle → `@remove-web-ui` landasi → cutattiin **0.10.0** (breaking). Molemmat manuaalisella
`cargo publish`illa (ossctl-cut yhä rikki). **Ei release-gejä auki** — molemmat cutattu ja varmistettu.

**⚠️ RELEASE-OPPI (yhä voimassa): ossctl `release cut` EI julkaise oikeasti → 0.8.1 tehtiin `cargo publish`illa.**
Real cut ei uploadannut crates.io:hon (raportoi 300s "index visibility" -timeoutin cratelle jota se ei
koskaan uploadannut; crate 404:ssä 9 min, ei receiptejä). Juurisyy todennäk. ossctl:n publish-adapterin
bug (dry-run/no-op oikeassa cutissa), jäi kiinni koska `@wire-oss-release-as-release-path` verifioi vain
`release plan`-dry-runilla. Bug filattu upstream-ossctl-reppuun **`release-cut-publish-noop`** (high) +
downstream-tracker tässä **`@ossctl-cut-no-publish`** (high, DAG LANE C). AGENTS.md kuvaa `ossctl release
cut`:n polkuna (workaround-stepit poistettu — korjataan, ei kierretä). **⚠️ Kunnes ossctl korjattu:
seuraava release vaatii taas manuaalisen `cargo publish -p issuectl-core` → `-p issuectl` → tag → push
-fallbackin** (samat stepit kuin 0.8.1:ssä). **Toinen opetus (AGENTS.md:ssä):** `ossctl release cut`
julkaisee PUUN version — se EI bumppaa; siksi bump + CHANGELOG-finalisointi + `release:`-commit tehdään
ENNEN cutia. (Sekundäärifriktio: stale-lock esti `release abandon`in → ossctl
`@release-abandon-break-stale-lock`, filattu.)

**Shipatun sisällön täydet muutokset:** CHANGELOG `[0.9.0]` (2026-08-12) + `[0.10.0]` (2026-08-12,
breaking web-removal). Ei toisteta tässä.

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

**Iso linjaus (TOTEUTETTU 2026-08-12):** **web/selain-UI POISTETTU** — `@remove-web-ui` landasi ja
julkaistiin 0.10.0:ssa (breaking). issuectl on nyt puhdas AI-first CLI (ei `serve`ä, ei `/api`ta, ei
kanbania). Gate purettiin käyttäjän go:lla. Poiston `/llm-review` filasi 3 siivous-follow-upia:
`@collapse-configsource-seam`, `@flock-write-test-coverage`, `@new-and-update-blocked-by` (kaikki DAG:ssa).

**Seuraava askel:** **Ei releasea auki** (0.9.0 + 0.10.0 molemmat cutattu ja varmistettu). Aktiivinen jono
on 6 ei-deferred-issueä — kaikki `normal` paitsi upstream-blocked ossctl. **GLOBAL HEAD-OF-LINE:
`@pidev-pi-skill-lifecycle`** (LANE B, parallel-safe, oli käyttäjän "5 asap"-setissä). LANE A on ruuhkainen
(5 issueä, jakavat main.rs/mutate/-hotfileja → sarjataan):
- **`@pidev-pi-skill-lifecycle`** (normal, LANE B) — pi-korpuksen (~/.pi/agent/skills/) lifecycle:
  version-drift-näkyvyys, prune (orphanit esim. /triage-bugs), doctor-verify, uninstall-gap. collision: doctor/.
- **remove-web-ui-siivous-follow-upit (LANE A, 0.10.0-jälkipyykki):** `@collapse-configsource-seam`
  (improvement — nyt kun server poissa, single-impl ConfigSource-seam voi romahtaa), `@flock-write-test-coverage`
  (task — palauta write-under-flock-testikattavuus jonka server-testit ennen antoivat), `@new-and-update-blocked-by`
  (feature — `new --blocked-by` + `update --add-blocked-by`).
- **`@dag-lists-closed-issues`** (bug, LANE A) — `issuectl dag` listaa suljetut/terminaali-issuet
  "unscheduled"-osiossa; filtteröi ei-terminaaleihin. (Tämä hämäsi toista agenttia luulemaan shipattuja
  bugeja avoimiksi.)
- **`@epic-tree-view`** (feature, LANE A — un-deferattu tässä rupeamassa) — `issuectl epic tree <slug>`.
  Kevyt CLI-lisäys.
- **`@ossctl-cut-no-publish`** (high, bug) — yhä upstream-blocked; ei koodattavaa täällä ennen ossctlin
  korjausta. **Avoin C-päätös:** (1) jätä odottamaan, vai (2) verifiointi-worktree joka tarkistaa onko
  ossctl korjattu ja re-pointaa. Kunnes korjattu, seuraavakin release tarvitsee manuaalisen `cargo publish`in.

---

## Execution DAG (2026-08-12)

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
GLOBAL HEAD-OF-LINE: pi-corpus-metadata-error-misclass   ← last HIGH bug in shipped pi feature (pi_prune drops manifest row = data loss). LANE B (pi-corpus) ‖ LANE A (mutate/main.rs) run disjoint. ossctl-cut high but upstream-blocked.
LANE A — main.rs (cmd_* + clap) + mutate/ + parser/body_sections  (RUUHKAINEN — sarjata)
    flock-write-lock  normal · bug; flaky test — write_lock_released_after_failed_mutation intermittently fails under the full parallel suite (introduced Wave 2 by flock-write-test-coverage). Make deterministic. MUST be green before next release. Touches mutate/ tests.
    configsource-load-return-value  normal · improvement; collapse-seam-review-spinoff — return schema/transitions load BY VALUE now that the cache is gone. Touches mutate/ + config load.
    load-once-thread-schema  normal · improvement; collapse-seam-review-spinoff — thread loaded schema/rules through mutate helpers to stop redundant re-parses. Touches mutate/ + schema.
    epic-tree-view  normal · feature; `issuectl epic tree <slug>` — epic+lapset puuna. Uusi main.rs-subcommand + moduuli. Kevyt.
    dag-inprogress-schema-aware  normal · improvement; dag-review-spinoff — schema-aware in-progress/underway classification. Touches crate::dag.
    dag-reservations-run-id-object-shape  normal · improvement; dag-review-spinoff — dag reservations accept run_id object shape, not only array-of-holds. Touches crate::dag.
    action-verb-json-echo-mutation  normal · improvement; update/label/close --json-tulos ei echoa mutatoitua kenttää (.priority/.labels/.status = null). Echo resulting value. Touches cmd_* action-verb handlerit main.rs:ssä + result-objektit.
    new-and-update-blocked-by  normal · feature (RE-SCOPED 2026-08-12 → only `new --blocked-by` at creation; `update --add-blocked-by` half already done via `depend add/remove`, 6e95b07). Touches main.rs clap + mutate/.
LANE B — skill install + templates/ + skill.rs + pi-corpus lifecycle (skill distribution)  (RUUHKAINEN — pi-review-cascade, sarjata; all touch pi-corpus/skill.rs)
    pi-corpus-metadata-error-misclass  high · bug; pi-manifest-lock-review-spinoff — metadata errors misclassified as Missing → pi_prune drops the manifest row (data loss).
    pi-corpus-fd-relative-hardening  normal · improvement; pi-symlink-review-spinoff — harden mutating paths with descriptor-relative no-follow ops (close TOCTOU + hard-link overwrite). Deepens the symlink fix.
    pi-prune-digest-gate  normal · improvement; pidev-lifecycle-review-spinoff — gate pi-prune on a content digest before removing. Touches skill.rs/pi prune.
    pi-manifest-fsync-durability  normal · improvement; pi-lock-review-spinoff — save_pi_manifest lacks fsync durability.
    pi-mirror-atomic-writes  normal · improvement; pi-lock-review-spinoff — mirror SKILL.md writes are non-atomic (torn file on crash).
    pi-status-check-exit  low · improvement; pidev-lifecycle-review-spinoff — pi-status exit-code semantics for drift/orphans.
    pi-mirror-hint-accuracy  low · bug; pi-lock-review-spinoff — install prints "skills mirrored" hint even when pi block skipped.
    pi-status-shared-lock  low · improvement; pi-lock-review-spinoff — pi_status reads lock-free → can report a torn snapshot.
    pi-corpus-cross-tool-lock  low · improvement; pi-lock-review-spinoff — issuectl and orchestratectl hold separate locks; no cross-tool serialization on shared skill dirs.
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl)
    ossctl-cut-no-publish  high · bug; ossctl release cut ei julkaise oikeasti → manuaalinen cargo publish. BLOCKED upstream-ossctl:llä; kun korjattu, poista AGENTS-caveat + re-point releaset ossctliin.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: iso rupeama, **2 releasea**. Wave 1 (warn-notes + codex-variants) + kiireellinen pidev-dual-home
landasivat → flaky CI korjattiin → **0.9.0 cut**. Sitten `remove-web-ui` (breaking) landasi → **0.10.0 cut**.
8 issueä terminaaliksi → pudotettu DAG:sta (`warn-reserved-notes-section`, `codex-prompt-variants`,
`pidev-dual-home-skills`, `doctor-fix-merge-notes-comments`, `note-missing-as-generic-error`,
`rate-limit-test-flaky`, `events-jsonl-log`, `remove-web-ui`). Tilalle 5 uutta aktiivia: 3 remove-web-ui-
siivous-follow-upia (`collapse-configsource-seam`, `flock-write-test-coverage`, `new-and-update-blocked-by`)
+ `dag-lists-closed-issues` (bug) + `epic-tree-view` (un-deferattu). LANE A on ruuhkainen (5) → sarjata;
`pidev-pi-skill-lifecycle` (LANE B) ajaa rinnalla. `ossctl-cut-no-publish` (LANE C) yhä upstream-blocked.

---

## Adjacent backlog (deferred — DAG:n ulkopuolella, ei ajossa)

Kaikki alla on labeloitu `deferred` issuectl:ssä (2026-08-04), joten ne eivät ole DAG-lanella
eivätkä laukaise drift-checkiä. Poista `deferred`-label kun otat takaisin peliin.

**Web/selain-UI: POISTETTU (`@remove-web-ui` done, 0.10.0).** Ei enää backlogissa. Kaikki entiset
kanban/web-enhancement-issuet (13 kpl) oli jo suljettu `obsolete` (2026-08-10).

**CLI-only visualisointi (deferred):**
`@issue-graph-view` (ent. massively-periodic-surprise) — `issuectl graph` -moottori + lensit
(deps / worktree-planning / epic-rollup); lens 2 osin jo `issuectl dag`:ssä. **EI kevyt** (koko graph-
moottori + mermaid/dot/svg); arvioitu tässä rupeamassa → pidetään deferred, jos tarve konkretisoituu tee
vain lens 1 (dep→mermaid). (`@epic-tree-view` un-deferattiin — nyt DAG:ssa, ei enää tässä.)

_(`@events-jsonl-log` suljettu `wontfix` 2026-08-12 — ei tullut tarpeen.)_

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
