# TODO — issuectl

Työjono ja handoff `/stint`-työrupeamille. Tämä tiedosto on
`/stint`:n käynnistyspiste: lue ensin alla oleva handoff-block, sitten
aja `issuectl dag`. Issue-viittaukset ovat `@slug`-muodossa — koko backlog elää
`issuectl`:ssä (`issuectl list` / `issuectl dag`), tämä tiedosto on vain
kuratoitu handoff-näkymä.

Operating-faktat (deploy, green gate, hot files) ovat
[AGENTS.md](AGENTS.md#operating-facts-for-stint):ssä.

---

## 🔄 Continue here · ALOITA TÄSTÄ

**Tila (2026-08-16, cli-canon-rupeama + 0.12.0 & 0.13.0 julkaistu):** `main` **vihreä** (1070 testiä, fmt+clippy
puhtaat — 0 uutta clippy-varoitusta, 52 pre-existing). **`origin/main` == `main` (pushattu).** **Live-release nyt
`v0.13.0`** (crates.io + binäärit + Homebrew + tag kaikki ulkona, 14 assetia). `Cargo.toml == 0.13.0`.
**Aktiivinen työ tyhjä** — DAG:ssa vain parkissa oleva `@ossctl-cut-no-publish` (upstream-blocked). Ei ajossa
olevia workereita.

**Tämä sessio: koko `cli-canon`-lane (6 unittia) läpi + KAKSI julkaisua.** Kaikki sarjassa `main.rs`-collisionin
takia; jokainen reviewattu (`/llm-review` + `/assess-findings`) + täysi green gate + `close --stamp`.

**0.12.0 (additiiviset):**
1. `@cli-canon-config` — §8 `config path` / `config show`, per-arvo provenienssi (`source: "file"|"default"`).
2. `@cli-canon-skill-list` — §15 `skill list`, täydentää `list/install/print`-triadin.
3. `@cli-canon-help-json` — §14 `--help --json` juurelle + alikomennoille; tekstihelp ennallaan.

**0.13.0 (BREAKING):**
4. `@cli-canon-json-envelope` — §10 **koko `--json`-pinta enveloppiin**: `{"schema_version":1,"data":…,
   "warnings":[]}` stdoutiin, virheet `{"schema_version":1,"error":{…}}` stderriin. Lisäksi `version [--json]`
   (`supported_schemas[]` + `skills[{name,cli_version,schema_version}]`) §17-driftauditia varten.
5. `@cli-canon-create-verb` — §7 `create` on nyt primääri luontiverbi, `new` jää aliakseksi.
6. `@cli-canon-s22-clock` — §22:n **Clock-osuus**: injektoitava `Clock` (`crates/issuectl-core/src/clock.rs`,
   sama seam-idiomi kuin `ConfigSource`). Kaikki 14 suoraa `now()`-kutsua coresta pois; ainoa jäljellä oleva on
   Clockin oma real-toteutus. Deterministiset testit arkistobucketoinnille + kuunvaihdoksen rajatapaukselle.

**⚠️ BREAKING-MUUTOKSEN SEURAUS (0.13.0):** kaikki `issuectl --json` -kuluttajat rikkoutuivat. Mukana tulevat
skill-templatet + `CLAUDE.md`:n sopimuskuvaus päivitettiin samassa committissa, MUTTA sisarrepojen omat skriptit
eivät — ne pitää siirtää lukemaan `.data`-kentän alta. @jarin päätös 2026-08-16: *"ei haittaa, emme tue
taaksepäin yhteensopivuutta vielä"*.

**Sivutuotteet:**
- **`@cli-canon-s22`:n premissi oli VIRHEELLINEN ja korjattiin issueen.** Audit (`project-canon review
  --assume-defaults`) väitti *"no `crates/` directory — no core/cli split"*; todellisuudessa split on ollut
  olemassa pitkään ja core on jo clap-vapaa. §22:sta tehtiin siis vain Clock-osuus. Kaksi tietoista **hylkäystä**
  kirjattu issueen perusteluineen: (a) **I/O:n poisto coresta** — issuectl on tiedostojärjestelmäpohjainen
  tracker, markdown-tiedostot *ovat* domain; abstrahointi olisi coren uudelleenkirjoitus ilman testattavuushyötyä
  (testit jo hermeettisiä tempdireillä). (b) **crate-nimen vaihto `issuectl-cli`:ksi** — rikkoisi julkaistun
  crates.io-nimen kosmetiikan vuoksi. Jos nämä halutaan avata, ne ovat issuessa dokumentoituina.
- **CHANGELOG-korjaus:** `config`-worker vuoti diff-markkerit (`+`-merkit) CHANGELOGiin; korjattu käsin
  (`docs(changelog): fix leaked diff markers`).
- **Huomio, ei vielä filattu:** `create --help` -teksti sanoo satunnaisslugin olevan oletus kun `--slug` puuttuu,
  mutta `CLAUDE.md`:n mukaan oletus on **otsikosta johdettu** slug ja satunnainen on vain fallback. Help-teksti
  näyttää jääneen jälkeen todellisesta käytöksestä. Kannattaa filata + verifioida kumpi on oikeassa.

**Seuraava askel:** **ei aktiivista työtä.** `cli-canon`-lane tyhjennetty, 0.13.0 ulkona.
`@ossctl-cut-no-publish` pysyy parkissa kunnes upstream-ossctl korjaa. Uusi rupeama = odota intakea
(`/issue-intake` / `/stint-start` nostaa jonon), filaa yllä oleva `create --help` -epäjohdonmukaisuus, tai ota
`deferred`-backlogista jokin takaisin peliin.

**⚠️ Siivousta odottava:** `issuectl__worktrees/wt-01m04ygzhw-canon-review-issuectl` (@286dcb3, **locked**) on
edellisen session canon-review-ajosta eikä liity tähän rupeamaan. Jätetty koskematta — poista käsin jos on
turha (`git worktree remove --force` + `git branch -D`).

**⚠️ RELEASE-OPPI (0.12.0 ja 0.13.0 ajettiin suoraan manuaalireitillä — ossctl:ää ei edes yritetty, koska
`@ossctl-cut-no-publish` on yhä auki; molemmat menivät läpi puhtaasti):** ossctl `release cut` EI julkaise oikeasti
(`@ossctl-cut-no-publish`, upstream-blocked) — publish-phase failaa ("core not visible on index within 300s"),
MITÄÄN ei uploadata. → release vaatii manuaalisen `cargo publish -p issuectl-core` → (odota indeksi) →
`-p issuectl` → `git tag vX.Y.Z <release-commit>` → `git push origin vX.Y.Z` -fallbackin. Tag laukaisee
`release.yml`:n (cargo-dist) → binäärit + Homebrew (EI double-publishaa crates.io:ta). Ennen fallbackia: bump +
CHANGELOG-finalisointi + `release:`-commit + push main. **Varmista aina crates.io-indeksistä ETTEI core jo
uploadattu** ennen manuaalista publishia (double-publish failaa). ossctl-run jää `in_progress`-tilaan sen omaan
journaliin (`release verify <run-id>` näyttää unreconciled — vaaraton, ei estä mitään).

**⚠️ Minor-bump-gotcha:** `crates/issuectl/Cargo.toml`:n sisäinen `issuectl-core = { …, version = "X" }`
on caret-vaatimus → bumppaa se vastaamaan uutta minoria samassa release-commitissa (0.12.0 → 0.13.0).
Vain minor/major-raja vaatii tämän, ei patch.

**⚠️ Autonomy:** deployt/releaset TÄYSIN autonomisia (ei go/no-go, ei output-reviewia) — @jarin ohje 2026-08-10.

**Dogfood:** `cargo install issuectl` / `brew upgrade jarimustonen/issuectl/issuectl` / `cargo install
--path crates/issuectl`. Skillit `/issue`, `/issue-new`, `/issue-intake` tulevat `issuectl skill install`ista;
bugit/feature-pyynnöt sisään `issuectl intake file`lla, `/issue-intake` (tai `/stint-start`) nostaa jonon.

**OSS-init:** `OSS-RELEASE.md` approved (`mvp`). cargo-dist release-engine — `/oss-release-cut` EI
regeneroi `release.yml`:ää.

---

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag` are the source of truth.

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin ja tarkistaa
`issuectl dag` -näkymän rupeaman lopussa. Committoi vain muuttuneet polut täsmällisesti
(`TODO.md` ja issue-tiedostot, jos niitä muutettiin) ennen `/wrap-up`:ia, jotta tuore
agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md).

## Piialiisan bugiraportit

_Kaikki kolme triageed + lanetettu `issuectl`-frontmatteriin (2026-08-15, `needs-triage` poistettu). Ei enää untriaged-jonossa._

- [x] 🐛 issuectl new: accept --lane → `cli-fixes` ([`intake-feature-issuectl-035722451473`](issues/intake-feature-issuectl-035722451473/item.md))
- [x] 🐛 issuectl update: add --add-blocked-by → `cli-fixes` ([`intake-feature-issuectl-d93eaa168c66`](issues/intake-feature-issuectl-d93eaa168c66/item.md))
- [x] 🐛 label: --remove --json silent no-op → landattu 0.11.0:ssa ([`intake-bug-issuectl-d6947128f6c9`](issues/intake-bug-issuectl-d6947128f6c9/item.md))
- [x] 🐛 label: accept --add/--remove flag aliases → **suljettu `obsolete` (2026-08-16)**, duplikaatti — jo toimitettu 0.11.0:ssa `intake-bug-issuectl-d6947128f6c9`:llä ([`intake-feature-issuectl-986ecd5a58a9`](issues/intake-feature-issuectl-986ecd5a58a9/item.md))
- [ ] 🐛 Piialiisan bugiraportti: doctor --fix miscounts remaining findings (reports 1, lists 9) — jari via Telegram ([`intake-bug-issuectl-06c42e2d1123`](issues/intake-bug-issuectl-06c42e2d1123/item.md))
- [ ] 🐛 Piialiisan bugiraportti: update --type epic tells you to hand-edit the YAML instead of migrating… — jari via Telegram ([`intake-feature-issuectl-ff7665d266e6`](issues/intake-feature-issuectl-ff7665d266e6/item.md))
