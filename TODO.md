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

**Tila (2026-08-14):** `main` **vihreä** (1219 testiä, 0 fail, 0 clippy-erroria), työpuu puhdas,
`main == origin`. **Live-release yhä `v0.10.0`**, mutta mainissa on **julkaisematonta työtä**
(8 yksikköä tämän session ekasta puoliskosta). `Cargo.toml == 0.10.0` (ei bumpattu). Ei release-gejä auki.

**Tämä sessio, 2 vaihetta:**

**1) Rupeama — 8 yksikköä landasi mainiin (4 aaltoa, ei releasea):**
`@pidev-pi-skill-lifecycle` (pi-korpuksen lifecycle: provenance-manifesti, `pi-status` drift, `pi-prune`),
`@collapse-configsource-seam` (ConfigSource-seamin romautus), `@flock-write-test-coverage`
(write-under-flock-testikattavuus takaisin), `@pi-manifest-locking` (manifestin lukitus),
**2 HIGH pi-korpus-bugia:** `@pi-corpus-symlink-traversal` (polkupako-esto) +
`@pi-corpus-metadata-error-misclass` (pi_prune ei enää pudota manifestiriviä metadata-virheestä =
tietohäviö-korjaus), `@dag-lists-closed-issues` (dag ei enää listaa suljettuja unscheduledissa),
`@epic-tree-view` (`issuectl epic tree <slug>`). Kaikki `/llm-review`+`/assess-findings` läpi.

**2) PO-triage (iso karsinta):** review-cascade tuotti ~19 spin-offia; PO-kokpit (glasspad) käytiin
läpi @jarin kanssa. **Suljettiin 14 `wontfix`** (defensiivistä/havainnointi/putkitusta epärealistisia
uhkia vastaan — "ei fiksailla epätodennäköisiä ihmeellisyyksiä tämän maturiteetin ohjelmistossa").
**Pidettiin 6.** **Filattiin 1 uusi:** `@dag-inprogress-is-spawnable`.

**⚠️ ISO LINJAUS — in-progress ≠ "nyt työn alla" (`@dag-inprogress-is-spawnable`):** `issuectl dag`
EI saa sulkea in-progressia pois `spawnable`sta. in-progress = *aloitettu, ei valmis*; dag:ia kysytään
vain kun mikään ei ole käynnissä → in-progress-issuet ovat **resumoitavia ehdokkaita jotka on nostettava
(aggressiivisesti)**, ei suljettava. Päällekkäisyyden esto = KUTSUJAN varausvastuu, ei dag:in. Korjaus:
poista `!underway`-poissulku + `IN_PROGRESS`-vakio (dag.rs:80/466/470), päivitä docstring (dag.rs:44-50).
**Sisar-issue orchestratectl-repossa:** `stint-head-of-line-in-progress-eligible` — stint/orchestrate
head-of-line -konventio pitää linjata samaan (se sanoo nyt "eligible iff … not already in-progress").

**⚠️ SPIN-OFF-LAADUN TARKKAILU (UUSI, @jarin ohje):** seuraavilla korjauskierroksilla **tarkkaillaan
tarjottujen spin-offien laatua kriittisesti.** Havainto tästä sessiosta: tämän maturiteettitason
ohjelmistossa `/llm-review` taipuu tuottamaan tarpeettomia "keksitään keksimällä jotakin hiottavaa"
-suosituksia (14/19 spin-offia → drop). Älä ota review-cascaden spin-offeja annettuina — punnitse jokainen
todellista arvoa vasten ennen kuin nostat sen DAG:iin.

**Seuraava askel:** työstä 8-issuen DAG (alla; sisältää sisar-agentin 2026-08-14 filaaman `@update-set-body-flag`in). **Tämä kierros toimii myös tavallisten worktree-promptien
testinä** (@jari: "testataan tavallisia worktree prompteja"). **GLOBAL HEAD-OF-LINE: `@flock-write-lock`**
— flaky testi joka **PITÄÄ olla vihreä ennen seuraavaa releasea**; se + muu julkaisematon mainin työ menee
seuraavaan minoriin (0.11.0). Kun DAG tyhjenee → ei enää tehtävää.

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
GLOBAL HEAD-OF-LINE: flock-write-lock   ← flaky test, MUST be green before next release (0.11.0). LANE A ‖ LANE B disjoint. ossctl-cut high but upstream-blocked.
LANE A — main.rs (cmd_* + clap) + mutate/ + schema + crate::dag  (sarjata; jakavat hot-fileja)
    flock-write-lock  normal · bug; flaky test — write_lock_released_after_failed_mutation intermittently fails under the full parallel suite (introduced by flock-write-test-coverage). Make deterministic. MUST be green before next release. Touches mutate/ tests.
    dag-inprogress-is-spawnable  normal · bug; ISO LINJAUS — dag must NOT exclude in-progress from spawnable (in-progress = started-not-done, resumable; caller owns double-work prevention). Remove !underway + IN_PROGRESS const (dag.rs:80/466/470), update docstring (dag.rs:44-50), test in-progress head is spawnable. Touches crate::dag. Sibling: orchestratectl `stint-head-of-line-in-progress-eligible`.
    dag-reservations-run-id-object-shape  normal · improvement; dag reservations accept run_id object shape, not only array-of-holds. One-line + test. Touches crate::dag.
    configsource-load-return-value  normal · improvement; the ONE kept readability win — return schema/transitions load BY VALUE now the cache is gone (finishes collapse-configsource-seam). Touches mutate/ + config load.
    action-verb-json-echo-mutation  normal · improvement; update/label/close --json result doesn't echo the mutated field (.priority/.labels/.status = null). Echo resulting value. Touches cmd_* action-verb handlers main.rs + result objects.
    update-set-body-flag  normal · feature; `issuectl update` lacks a body-set flag that `new` has — add `--body-file`/`--description` (+ stdin `-`) to set/replace an existing body, matching new's exact flag names. Filed by a sibling agent 2026-08-14. Touches main.rs update clap + mutate/ body write.
LANE B — skill.rs + pi-corpus lifecycle
    pi-mirror-hint-accuracy  low · bug; install prints "skills mirrored" hint even when the pi block was skipped. Only kept pi-corpus item. Touches skill.rs/pi install.
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl)
    ossctl-cut-no-publish  high · bug; ossctl release cut doesn't actually publish → manual cargo publish. BLOCKED upstream-ossctl; when fixed, remove AGENTS caveat + re-point releases to ossctl.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: sessio kahdessa vaiheessa. **(1) Rupeama:** 8 yksikköä landasi mainiin (pi-korpuksen lifecycle
+ 2 HIGH pi-bugia, ConfigSource-romautus, flock-testit, dag-lists-korjaus, epic-tree) — kaikki `/llm-review`
läpi, ei releasea. **(2) PO-triage:** review-cascaden ~19 spin-offista **14 → wontfix, 6 → keep, 1 uusi**
(`dag-inprogress-is-spawnable`). 15 issueä terminaaliksi → pudotettu DAG:sta (8 shipattua + 14 wontfix −
päällekkäisyydet). DAG on nyt 8 aktiivia: LANE A ruuhkainen (6, sarjata; ml. sisar-agentin `update-set-body-flag`), LANE B 1, LANE C 1 (blocked).
Head = `flock-write-lock` (release-gate). Seuraava kierros testaa myös tavallisia worktree-prompteja.

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
