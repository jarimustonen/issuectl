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

**Tila (2026-08-06):** `main` == **v0.7.1** — TÄYSIN JULKAISTU: GitHub Release live,
Homebrew-tap päivitetty, **crates.io `0.7.1` indeksissä** (varmistettu `newest_version`),
release.yml + publish-crates.yml vihreät, CI vihreä. Työpuu puhdas.

**0.7.1:ssä shipannut (CHANGELOG täydellinen) — pieni intake-follow-up-rupeama:**
- **@distribute-intake-skills (feature)** — `skill install` + `init` asentavat nyt myös
  `/issue-new` (filer) + `/issue-intake` (queue-processor) skillit, version-pinnattuina
  `include_str!`-templateina + dogfood-guardattuna (sama kontrakti kuin `/issue`illä), joten
  koko intake-workflow matkaa binäärin mukana joka projektiin. **Vain Claude-Code-formaatti**
  toistaiseksi (ei Codex-prompt-varianttia — ks. follow-up alla).
- **@verify-intake-split-queue (task)** — §6 transitional split queue **verifioitu
  shipatuksi jo 0.7.0:ssa** (molemmat legacy-muodot, JSON + human, migrate-round-trip); ei
  tuotantokoodimuutosta, lisättiin yksi regressiotesti pinnaamaan human-moden `[legacy]`-flag
  + migraationudge. Turvaverkko homebase/deutschpad-migraatioille — vahvistettu toimivaksi.

**Tämän rupeaman follow-upit (DAG:issa alla):**
- **@show-json-omits-blocked-by** (bug, normal) — rinnakkaissession filaama: `--json show`
  ei tulosta `blocked_by`-kenttää. **DAG GLOBAL HEAD-OF-LINE** (ainoa normal-prio työ).
- **@awfully-courageous-attempt** (feature, low) — Codex-prompt-variantit `/issue-new` +
  `/issue-intake`ille (triviaalinen frontmatter-strip kuten `/issue`illä). Käyttäjän nosto
  handoffissa: Codex-client sietäisi Claude-formaatin mutta näyttäisi frontmatterin kohinana.
  **Build-only-if**: tee vasta jos näitä skillejä oikeasti kutsutaan Codexista jossain.

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

**Seuraava askel:** 0.7.1 ulkona ja dogfoodissa — odota käyttäjän palautetta. DAG:issa 1
normal-prio bug (**@show-json-omits-blocked-by**, GLOBAL HEAD-OF-LINE) + 3 low-prio riviä
(2 close-polkua LANE A:ssa, Codex-variantti LANE B:ssä). Ei kiireä; feedback-issuet menevät
edelle jos niitä tulee.

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
GLOBAL HEAD-OF-LINE: intensely-blushing-galley   ← LANE A close-path tidy-ups (show-json-omits-blocked-by landed 0.7.1+)
LANE A — main.rs (show/close handlers) + mutate/ close path + schema/issue_fields  (sequenced — kaikki koskevat cmd_show/cmd_close serialization/output)
  ▶ intensely-blushing-galley   low · improvement; promote closed_by → typed Issue field (top-level in show) + doctor heal + human close output. Follow-up llm-reviewsta close-as-flag-asymmetrylle.
    close-comment               low · feature; `close` hyväksyy `--comment/--note` → kirjaa sulkemisen perustelun samassa stepissä (nyt manuaalinen 2-vaihe). Filattu rinnakkaissessiosta.
LANE B — skill install + templates/ + skill.rs (skill distribution)
  ▶ awfully-courageous-attempt  low · feature; Codex-prompt-variantit /issue-new + /issue-intakelle (frontmatter-strip kuten /issuella). Build-only-if: vain jos Codex-käyttö todistuu.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: **`show-json-omits-blocked-by`** (bug, normal) korjattu ja landattu — `--json show`
tulostaa nyt top-level `blocked_by`in (+ johdettu `blocks`); 3/3 llm-review-konsensus, green gate ok;
pudotettu DAG:sta (fixed). Uusi GLOBAL HEAD-OF-LINE = **`intensely-blushing-galley`**; LANE A:n 2
low-prio close-polun riviä (galley + close-comment) sekvensoituna. LANE B:n `awfully-courageous-attempt`
ennallaan (build-only-if).

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
