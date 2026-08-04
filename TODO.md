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

Lanes = hot-file families (AGENTS.md): `main.rs` (clap + cmd_* handlers),
`mutate/` (write paths), `schema.rs`, skill templates.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: refs-issue-hint-false-fire   ← start here on resume
LANE A — crates/issuectl/src/main.rs (clap + cmd_* handlers + error hints)
  ▶ refs-issue-hint-false-fire
    new-body-flag                            (only --body-file remains; --body shipped v0.6.5)
    apply-json-expected-version-consistency  collision: crates/issuectl-core/src/mutate/
LANE B — crates/issuectl-core/src/mutate/ (write paths)
  ▶ mutation-not-found-classification
```
<!-- execution-dag:end -->

Kaari-lyhyesti: kaikki neljä ovat pieniä CLI-korrektius/ergonomiafiksejä. LANE A ja LANE B
ovat disjointit (main.rs vs mutate/) → ajettavissa rinnan; LANE A:n sisällä sekvenssi.
`apply-json-expected-version-consistency` voi koskea myös `mutate/`:a (collision-tag) → jos
se ja LANE B:n solmu ovat yhtä aikaa live, sekvensoi.

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

---

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin JA mergeää
Execution DAG:n (drop landed, add active, keep order) rupeaman lopussa, ja committaa ne
omana committinaan (`git add TODO.md issues/ && git commit`) ennen `/wrap-up`:ia — näin
tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md).
