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

**Tila (2026-08-04):** `main` == **v0.6.6** (EI uutta releasea tässä rupeamassa —
kolme fixiä landasi mainiin mutta ne ratsastavat vasta seuraavaan releaseen). Työpuu
puhdas.

**Tässä rupeamassa landattu mainiin (ei vielä releasessa):**
- **@refs-issue-hint-false-fire** — Refs-Issue-muistutus siirretty pre-commitistä
  **commit-msg**-hookiin, joka lukee lopullisen viestin `git interpret-trailers`illa →
  ei enää false-fire `-F`/stdin-committeihin.
- **@mutation-not-found-classification** — write-verbit (`update`/`close`/`set`/…)
  palauttavat nyt vakaan `error.code: "not-found"` `--json`:issa geneerisen
  `command-failed`in sijaan (kaikki 8 verbiä testattu).
- **@new-body-flag** — `issuectl new --body-file <path>` (+ `-` = stdin) asettaa
  alkubodyn; skill-templatet synkassa. (`--body`-inline shippasi jo v0.6.5:ssä.)

**Päätökset tässä rupeamassa:**
- **@apply-json-expected-version-consistency → DECIDED keep-strict** (option 2).
  `apply --json` pitää `expected_version`:n **pakollisena** (deliberate exception:
  multi-field patch = read-modify-write, altistunein lost-update:lle). Kirjattu
  **D4a**:ksi `docs/design/web-edit-sync.md`:hen; issue suljettu `wontfix` (ei koodimuutosta).
- **@standard-intake-flow (HIGH) → design APPROVED.** `docs/design/intake-flow.md`
  → *"Approved decisions (2026-08-04)"* (authoritative). Päätökset: **reuse `type`**
  (ei `kind`); skillit **`/issue-new`** (filing) + **`/issue-intake`** (processing,
  korvaa `/triage-bugs`, ajaa `/worktree-bug-analysis`); **concurrency OUT** — OD-12
  dropped, OD-2 lease-free, **ei concurrency-tietoa issueihin**; muut ODt = suositus A;
  `deferred` pysyy `active`-luokassa.

**🚧 ISO KÄYNNISSÄ OLEVA — intake-toteutuksen `/orchestrate`-kampanja.**
Run **`01kz6c65y5kns14ce2rwm7tbxx`** (`intake-flow-build`), omalla
integraatiobranchillaan (**main koskematon** kunnes katselmoit + mergeät). Rakentaa
4 riippuvuusjärjestettyä yksikköä approved-designia vasten: **schema → `intake`-komennot
→ `/issue-new`+`/issue-intake`-skillit + `/triage-bugs` eläkkeelle → migraatio**.
**SEURAAVA AGENTTI:** tarkista tila `orchestratectl run show 01kz6c65y5kns14ce2rwm7tbxx`
(+ `run list` child-yksiköille). Kun valmis: katselmoi integraatiobranch ja mergeä
mainiin. Kampanja voi filata **uusia child-issueita** (schema-foundation,
intake-commands, …) — **merge ne DAG:iin seuraavassa `/stint-start`issa** (drift-check
nostaa ne). Kampanja pausettaa (äänimerkki) vain kovaan forkkiin.

**OSS-init (ennallaan):** `OSS-RELEASE.md` **approved** (`mvp`). cargo-dist pysyy
release-engineinä — `/oss-release-cut` EI regeneroi `release.yml`:ää.

**Iso linjaus (ennallaan):** **kanban/web-board holdissa**. Kaikki kanban/web-issuet +
build-only-if + `@focus-areas` `deferred` → **Adjacent backlog**. Fokus CLI:ssä.

**Seuraava askel:** odota/katselmoi intake-kampanja (yllä). Kaikki muut lanet
tyhjentyivät tässä rupeamassa (3 fixiä landasi, 2 päätöstä suljettu). GLOBAL
HEAD-OF-LINE = **@standard-intake-flow** (in-campaign — kampanja omistaa, ei
itsenäisesti spawnattavissa).

---

## Execution DAG (2026-08-05)

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
GLOBAL HEAD-OF-LINE: standard-intake-flow   ← /orchestrate campaign (run 01kz6c65y5kns14ce2rwm7tbxx) is DEAD (never dispatched — see 2026-08-05 note); needs relaunch, then campaign owns it
LANE A — main.rs (cmd_* handlers)
  ▶ close-as-flag-asymmetry   low · improvement; `close` should accept `--as <author>` like `note` does. collision: main.rs (shared with LANE D campaign's integration branch — reconcile at campaign merge)
LANE D — intake flow (schema.rs + main.rs + mutate/ + skill templates)
  ▶ standard-intake-flow   HIGH · design APPROVED (docs/design/intake-flow.md); campaign run 01kz6c65y5kns14ce2rwm7tbxx never started (zombie supervisor) — must be relaunched
```
<!-- execution-dag:end -->

Kaari-lyhyesti: rupeaman kaikki 4 pientä CLI-yksikköä terminoituivat (3 fixiä landasi:
`refs-issue-hint-false-fire`, `mutation-not-found-classification`, `new-body-flag`;
`apply-json-expected-version-consistency` suljettu `wontfix` = päätetty keep-strict) →
pudotettu DAG:sta. Jäljellä vain **LANE D `standard-intake-flow`**, jonka toteutus on nyt
`/orchestrate`-kampanjassa (run `01kz6c65y5kns14ce2rwm7tbxx`) omalla integraatiobranchillaan.
Kampanja voi filata child-issueita — seuraava `/stint-start` mergeää ne uusiksi laneiksi
(schema → intake-komennot → skillit → migraatio) drift-checkin nostamana.

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
