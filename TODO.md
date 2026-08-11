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

**Tila (2026-08-11, rinnakkaiskaista-rupeama):** **0.8.1 yhä live; 3 uutta featurea landasi `main`iin,
EI vielä julkaistu.** Käyttäjä käynnisti ensin 3 DAG-kaistaa rinnan; `@warn-reserved-notes-section` (LANE A) +
`@codex-prompt-variants` (LANE B) landasivat. Handoffin jälkeen nousi **kiireellinen `@pidev-dual-home-skills`**
(pi.dev-migraation WS4) — spawnattiin issuectl-worktree, **landasi** (dual-home `~/.pi/agent/skills/`iin).
Kaikki kolme: green gate vihreä (fmt clean, 1081 testiä, ei uusia clippy-varoituksia), `/llm-review` ajettu.
`main == origin`, työpuu puhdas. **Release PIDETTIIN** (ei cutattu): featuret low/high-prio + CHANGELOG
`[Unreleased]` **tyhjä** + release-polku yhä rikki (ks. release-oppi). Batchataan → seuraava rupeama cuttaa
0.9.0:n (3 featurea). ⚠️ Huom: `@pidev-dual-home-skills` filasi follow-upin `@pidev-pi-skill-lifecycle`.

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

**Mitä tässä rupeamassa tehtiin (2 featurea, landattu `main`iin, EI releasea):**
- **`@warn-reserved-notes-section`** (low, feature, `done`) — `issuectl new` ja `body set` varoittavat nyt
  authoring-aikaan kun issue-body käyttää varattua `## Notes`-otsikkoa (jonka doctor migratoi `## Comments`iin).
  Ei-fataali (ei blokkaa kirjoitusta), varoitus sekä human- (`emit_warnings_to_stderr`) että `--json`
  `warnings`-kentässä. Detektointi single-sourced uudesta `body_sections::LEGACY_SECTION_ALIASES`-constista
  (`reserved_section_warnings`, reuse fence-aware `all_h2_sections`-skanneri; ei uutta regexiä). Help-teksti
  `new --body-file` + `body set` mainitsee varatut sektiot. 4 mallin `/llm-review` löysi yksimielisen
  kriittisen bugin (false negative kun bodyssä `---` horizontal rule ennen `## Notes` + CRLF) → korjattu
  (`do_new` skannaa raa'an bodyn, ei frontmatter-splitattua renderöityä docia).
- **`@codex-prompt-variants`** (low, feature, `done`) — `/issue-new` + `/issue-intake` asentuvat nyt
  molemmissa formaateissa kuten `/issue`: Claude-skill `.claude/skills/<slug>/SKILL.md` + Codex-prompt
  `.codex/prompts/<slug>.md` (frontmatter strippattu, body identtinen). Uudet templatet
  `issue-new-prompt.md` / `issue-intake-prompt.md` `include_str!`-embedattu; `IntakeSkill::{template,
  install_path,label}` agent-parametrisoitu. Dogfood sync-testi vahtii nyt **6 kopiota** (oli 4) ja vaatii
  niiden olemassaolon. AGENTS.md template-taulu + sync-sääntö + hot-files-lista päivitetty. `/llm-review`
  (gemini-3.1-pro, gpt-5.6-sol, opus-4-7) + `/assess-findings`, 6 FIX-löydöstä sovellettu.

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

**Seuraava askel:**
- **RELEASE PENDING (3 landattua featurea odottaa):** kun release cutataan, ensin **täytä CHANGELOG
  `[Unreleased]`** kaikilla kolmella (`@warn-reserved-notes-section` + `@codex-prompt-variants` +
  `@pidev-dual-home-skills`; workerit eivät lisänneet merkintöjä — `[Unreleased]` on tyhjä), sitten
  version-bump. Kaikki additiivisia → **minor-bump 0.8.1 → 0.9.0** (muista **caret-gotcha**: bumppaa myös
  `crates/issuectl/Cargo.toml`:n sisäinen dep `version =` 0.8.0 → 0.9.0, ks. gotcha yllä). **⚠️ ossctl
  `release cut` yhä rikki → julkaisu manuaalisella `cargo publish`-fallbackilla** kunnes
  `@ossctl-cut-no-publish` ratkeaa.
- **✅ `@pidev-dual-home-skills` LANDANNUT** (high, feature, `done`; commitit `a15328f`/`8ae1a85`/`408c999`,
  green + 4-mallin `/llm-review`): `issuectl skill install` / `init` dual-homeaa nyt jokaisen Claude
  `SKILL.md`:n myös `~/.pi/agent/skills/<name>/SKILL.md`:hen (home-global; repo-local Claude/Codex ennallaan;
  vain SKILL.md peilataan; `skill::pi_skills_root`). Review korjasi fatal-pi-write-bugin + non-absolute-HOME-
  footgunin. **Follow-up filattu: `@pidev-pi-skill-lifecycle`** (normal — pi-korpuksen prune/verify/version-
  drift/uninstall; LANE B). Issuectl-osuus WS4:stä valmis; sisar-binäärit (orchestratectl ym.) tekevät omansa.
- **Aktiivinen jono:** ei-deferred issuet: `@pidev-pi-skill-lifecycle` (normal, DAG head), `@doctor-fix-merge-notes-comments` +
  `@note-missing-as-generic-error` (low, warn-notes-follow-upit) ja **`@ossctl-cut-no-publish`** (high, bug — juurisyy
  upstream-ossctl:ssä; ei koodattavaa täällä ennen kuin ossctl korjaa, sitten poista AGENTS-caveat +
  re-point releaset ossctliin). **Avoin C-päätös (käyttäjä ei vielä valinnut):** (1) jätä odottamaan
  ossctlin korjausta, vai (2) käynnistä verifiointi-worktree joka tarkistaa onko ossctl jo korjannut ja
  tekee re-pointin. Tämä ratkaisee myös release-polun.
- **Deferred:** portitettu `@remove-web-ui` (odottaa seuraaja-luonnosta), CLI-only `@issue-graph-view` +
  `@epic-tree-view`, `@events-jsonl-log` (build-only-if). `@focus-areas` suljettu `wontfix` (ADR 0001
  tallessa). Uusi rupeama: ei selkeästi koodattavaa jonossa (ossctl-cut on upstream-blocked) → nosta
  deferredistä tai uudesta intakesta, tai cutataan pending-release.

---

## Execution DAG (2026-08-11)

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
GLOBAL HEAD-OF-LINE: pidev-pi-skill-lifecycle   ← ylin selkeästi koodattava (normal). ossctl-cut on high MUTTA upstream-blocked (ei koodattavaa täällä)
LANE A — main.rs (cmd_* + clap) + mutate/ + parser/body_sections
    doctor-fix-merge-notes-comments  low · feature; doctor --fix auto-mergeää ## Notes → ## Comments myös kun molemmat olemassa (nyt "manual merge required"). Touches doctor/ + body_sections.
    note-missing-as-generic-error  low · bug; `issuectl note` ilman --as printtaa geneerisen helpin clapin missing-arg-viestin sijaan. Touches main.rs clap + note-handler.
LANE B — skill install + templates/ + skill.rs (skill distribution)
    pidev-pi-skill-lifecycle  normal · feature; pi-korpuksen (~/.pi/agent/skills/) lifecycle: version-drift-näkyvyys, prune (orphanit esim. /triage-bugs), doctor-verify, uninstall-gap. Follow-up dual-homelle. collision: doctor/ (LANE A). orchestratectl-repossa identtinen sisar-issue.
LANE C — release pipeline (.github/workflows/*.yml + cargo-dist + ossctl)
    ossctl-cut-no-publish  high · bug; ossctl release cut ei julkaise oikeasti → manuaalinen cargo publish. BLOCKED upstream-ossctl:llä; kun korjattu, poista AGENTS-caveat + re-point releaset ossctliin.
```
<!-- execution-dag:end -->

Kaari-lyhyesti: käyttäjä ajoi kaikki 3 kaistaa rinnan. **`warn-reserved-notes-section`** (LANE A) +
**`codex-prompt-variants`** (LANE B) landasivat puhtaasti (green + `/llm-review`) → **terminal `done` →
pudotettu DAG:sta**. **Release pidettiin** (low-prio + tyhjä CHANGELOG `[Unreleased]` + rikki release-polku).
`ossctl-cut-no-publish` (LANE C) pysyy — upstream-blocked, ei koodattavaa täällä ennen ossctlin korjausta.
Jäljellä vain yksi aktiivinen node, sekin blokattu → DAG käytännössä tyhjä; seuraava rupeama nostaa
deferredistä / intakesta tai cutataan pending-release.

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
