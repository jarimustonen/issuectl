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

**Tila (2026-08-04):** `main` == **v0.6.6**, juuri julkaistu (tag `v0.6.6`
pushattu; cargo-dist + crates.io + Homebrew-pipeline käynnistetty GitHubissa).
Työpuu puhdas. Vihreä (fmt, clippy **63 = ei uusia varoituksia**, testisuite läpi).

**Tässä sessiossa landattu ja julkaistu v0.6.6:ssa:**
- **@add-low-priority-value** — `--priority` hyväksyy nyt `low`/`normal`/`high`.
- **@cli-ux-subcommand-friction** (#2/#3/#5) + **@unreachable-return-in-cmd-show** —
  positional title `new`:lle, actionable `set`-listakenttävihje, `note --as`/argjärjestys,
  build-hygienia (`cmd_show`).
- **@note-from-file-rejects-headings** — `note --from-file` hyväksyy `##`/`###`-otsikot
  (demote managed-sectionin alla).
- **@json-close-requires-expected-version** + **@json-update-expected-version-ergonomics** —
  `--expected-version` on nyt **opt-in** `--json`-kirjoituksissa (D4=B superseded);
  `version`-avain top-levelissä myös write-tuloksissa.

**OSS-init adoptoitu:** `OSS-RELEASE.md` on **approved** (maturity `mvp`). cargo-dist
säilyy release-engineinä — `/oss-release-cut` EI saa regeneroida `release.yml`:ää.
Kaksi ossctl-havaintoa filattu **ossctl-repoon** (ei tänne): pre-1.0-maturity-gate +
cargo-dist-release-mallinnus.

**Iso linjaus:** **kanban/web-board on holdissa** (kukaan ei käytä sitä nyt). Kaikki
kanban/web-issuet + build-only-if + `@focus-areas` labeloitu `deferred` → **Adjacent
backlog** (alla, DAG:n ulkopuolella). Fokus on **CLI-työkalussa**.

**Seuraava askel:** ks. **Execution DAG**. GLOBAL HEAD-OF-LINE = **@refs-issue-hint-false-fire**.
Aktiivisia CLI-issueita jäljellä 4 (2 bugia, 1 feature, 1 improvement). Huom:
`@new-body-flag`:n `--body`-alias shipattiin jo v0.6.5:ssä — jäljellä oleva scope on vain
`--body-file`; trimmaa issue ennen rakentamista.

---

## Execution DAG (2026-08-04)

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
GLOBAL HEAD-OF-LINE: mutation-not-found-classification   ← main.rs head; refs-issue-hint runs in parallel (hooks.rs)
LANE A — crates/issuectl/src/main.rs (clap + cmd_* handlers + fn main error rendering + parse_apply_patch)
  ▶ mutation-not-found-classification         (main.rs:1786 error render; reads mutate/mod.rs MutateError)
    new-body-flag                             (only --body-file/stdin remains; --body shipped v0.6.5; also mutate/new_issue.rs)
    apply-json-expected-version-consistency   DECISION: option 1 (consistent) vs option 2 (strict) — needs user call
LANE C — crates/issuectl-core/src/hooks.rs + git_trailers.rs (commit-hook Refs-Issue hint)
  ▶ refs-issue-hint-false-fire
LANE D — intake flow (design→docs/ now; later schema.rs + main.rs + skill templates)
  ▶ standard-intake-flow   HIGH · DESIGN-FIRST — design doc only (docs/, parallel-safe); NO impl until user approves
```
<!-- execution-dag:end -->

Kaari-lyhyesti: kaikki neljä ovat pieniä CLI-korrektius/ergonomiafiksejä. **Lane-korjaus
2026-08-04:** todelliset footprintit tarkistettu — `refs-issue-hint-false-fire` on
`hooks.rs`:ssä (EI main.rs), `mutation-not-found-classification` koskee `main.rs`:n
`fn main`-virherenderöintiä (EI vain mutate/). Niinpä kolme neljästä (mutation-not-found,
new-body-flag, apply-json) osuvat `main.rs`-kuumatiedostoon → **sekvensoitava** LANE A:ssa;
vain `refs-issue-hint` (LANE C, hooks.rs) on turvallisesti rinnakkainen. `apply-json` on
aito päätöskysymys (make-consistent vs keep-strict) → nostettu käyttäjälle, ei autonomisesti.

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
