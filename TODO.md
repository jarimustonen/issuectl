# TODO — issuectl

Työjono ja handoff `/stint`-työrupeamille. Tämä tiedosto on `/stint`:n
käynnistyspiste: lue ensin alla oleva handoff-block, sitten aja `issuectl dag`.
Issue-viittaukset ovat `@slug`-muodossa — koko backlog elää `issuectl`:ssä,
tämä tiedosto on vain kuratoitu handoff-näkymä.

Operating-faktat (deploy, green gate, hot files) ovat
[AGENTS.md](AGENTS.md#operating-policy-for-stint):ssä. Pysyvät opit eivät kuulu
tähän tiedostoon: päätökset → `docs/decisions/`, mekaniikka → `docs/design/`,
säännöt → `AGENTS.md`.

---

## 🔄 Continue here · ALOITA TÄSTÄ

**Tila (2026-09-03, rupeama 5 + release valmis):** `main` on puhdas ja julkaistu.
Live-release on **v0.17.1 kaikissa kanavissa**: `issuectl` ja `issuectl-core`
crates.io:ssa 0.17.1, GitHub Releasessa 12 odotettua assetia, cargo-dist-ajon
lopputulos `success`, ja Homebrew-tapissa 0.17.1. Paikallinen Homebrew-binääri sekä
`/issue`, `/issue-new` ja `/issue-intake` -ohjeet ovat 0.17.1:ssa. Integroitu täysi
green gate meni läpi, samoin release-bumpin jälkeisen dogfood-refreshin täysi gate.
Repoon ei ole eläviä tai huomiota odottavia orchestratectl-ajoja.

**Rupeama 5:**
- @intake-bug-issuectl-af715e8b5283 korjasi doctorin false positivet: issuectl:n omat
  intake/review-metadatakentät tunnetaan oletusskeemassa, mutta aidosti tuntemattomista
  avaimista varoitetaan edelleen.
- @intake-bug-issuectl-3148539799d8 korjasi `create --body-file` -polun: valmis
  rakenteinen Markdown sijoitetaan H1:n alle ilman tyhjää, duplikoitua
  `## Description` -otsikkoa. Schema saa edelleen lisätä puuttuvat pakolliset H2-stubit.
  Työ kävi läpi poikkeuksellisen pitkän provider-retry/review-polun; lopullinen toteutus,
  review-evidence ja gate ovat valmiit ja issue suljettu fixed-tilaan.
- v0.17.1 sisältää lisäksi edellisen valmistellun kolmikon: Linuxin clap-stack-korjauksen,
  toimituksen merkitseviin sulkutiloihin rajatun DoD-portin ja canonical `.data.path`
  -ohjeen `/issue`-skillissä.

**Release-havainto:** ensimmäinen cut pysähtyi ennen publishia, koska paikallinen
`dist` puuttui. Sama journaloitu run jatkettiin checksum-verifioidulla, disposablella
cargo-dist 0.28.2:lla (täsmää `dist-workspace.toml`:iin); molemmat cratet julkaistiin,
tag laukaistiin kerran ja kaikki neljä kohdetta verifioituivat. Pysyvää unmanaged
cargo-dist-asennusta ei jätetty. Homebase-konvergenssi päivitti issuectl:n, mutta sen
pitkä ajo päättyi transienttiin `macos-defaults`-timeoutiin; tuore `homebase fleet
status` näyttää kaikki tuetut unitit vihreinä. Release-bumpin jälkeen kuusi seurattua
dogfood-kopiota päivitettiin erillisessä housekeeping-commitissa v0.17.1:een.

**Seuraavan stintin valmisteltu intentio:** tee molemmat hyväksytyt follow-upit;
ajantasainen järjestys ja rinnakkaisuus luetaan aina `issuectl dag --json --reservations
'[]'` -pinnalta. @json-export-import-headings korjaa issuectl:n oman JSON
export→import-round-tripin rakenteellisen otsikkoduplikaation. @release-bump-refresh-dogfood
automatisoi kuuden repossa seurattavan agenttiohjekopion versionpäivityksen engine-owned
release-bumpiin eristetyssä ympäristössä, jotta release-commit on heti testivihreä.

**Operatiivinen siivous:** kahden epäonnistuneen mutta lopullisella toteutuksella
syrjäytetyn create-body-reviewn säilytetyt työtilat ovat vielä levyllä
(runit `01m1g9865gsc5kjrq3f91rzc2m` ja `01m1gc62273xn52kzkgtpd1p73`). Ne eivät omista
aktiivista työtä eikä niitä saa pelastaa tai mergeätä; poista ne vain tarkoituksellisessa,
ihmisen valvomassa cleanupissa, kun lopullisen v0.17.1-toteutuksen säilyminen on vielä
varmistettu.

<details>
<summary>Edellinen handoff (2026-08-20)</summary>

**Tila (2026-08-20, rupeama 3 valmis + triage-pass):** rupeaman jälkeen ajettiin vielä
lanettomien issueiden triage: @apply-inline-json suljettiin duplikaattina ja sen sisältö
taitettiin @apply-patch-from-stdin -issueen, joka samalla nimettiin intake-slugistaan.
Tuotekoodiin ei koskettu, vain issue-tiedostoihin ja tähän handoffiin. `main` **vihreä**
(edellisen rupeaman gate; 24 testibinääriä,
fmt/clippy/doc puhtaat), synkassa originin kanssa, ei ajossa olevia workereita eikä
worktreitä tässä repossa. **Live-release `v0.16.0` kaikissa kanavissa** (crates.io ×2,
GitHub Release 12 assetia, tap 0.16.0); `ossctl release verify` reconciloi 4/4 matches,
0 missing. Paikallinen binääri 0.16.0.

**Rupeama 3 (0.16.0):** seitsemän yksikköä landattu, kaikki omina workereinaan.
1. @intake-bug-issuectl-d5b6669a98fe: `update --json` echottaa scheduling-kentät lukitusta
   kirjoituksen jälkeisestä tilasta (bugi toistui livenä heti rupeaman alussa).
2. @update-title-flag: `update --title` olemassa, ja `body set` säilyttää H1:n otsikottomalla
   bodylla, varoittaa kun otsikko vaihtuisi.
3. @update-canonical-forms: ADR 0004:n valmisteluslice, `update --patch-file` ja `--query`
   uudelleenkäyttävät `apply`:n ja `bulk`:n koneistoa. Parity-testit (520 riviä), foldattaviin
   komentoihin ei koskettu. Worker raportoi nolla parity-aukkoa.
4. @intake-feature-issuectl-42403ae544e3: `/issue`-skill dokumentoi lane-liput.
5. @intake-feature-issuectl-4f9dbc60a05e: `deferred`-label eläkkeelle, doctor-check + `--fix`.
6. @purge-telegram-surfaces: kanavanimi pois tuotepinnalta, `via:<channel>`-migraatio
   yleistetty (kattaa nyt myös historialliset `via:agent-*`-arvot).
7. @homebrew-double-writer-contract: homebrew-targetin adapteri `cargo-dist`.

**RELEASE-HAVAINTO (0.16.0 cut, ossctl 0.9.0):** kohdan 7 kontrahtikorjaus **toimi**:
`dist`-vaihe meni puhtaasti läpi eikä engine ajanut omaa tap-legiään lainkaan (0.15.0:n
false-red-syy poistui). MUTTA verify kaatui `gh-releases`-targettiin, ja tämä oli **aito
vika, ei kilpailutilanne**: cargo-distin workflow **peruuntui**, koska
`aarch64-unknown-linux-musl` jonotti hostatulla runnerilla 6 h (06:56 → 12:56) ja osui
GitHubin kovaan job-kattoon. Downstream-jobit (`host`, `publish-homebrew-formula`) skipattiin,
joten Releaseä ei syntynyt ja tap jäi 0.15.0:aan vaikka crates.io-julkaisu oli jo
peruuttamaton. Kaatuneiden jobien rerun meni läpi ~5 minuutissa. Diagnoosi: transientti
hostatun ARM64-kapasiteetin puute, ei konfiguraatiovika, joten `dist-workspace.toml` jätettiin
ennalleen (self-hosted-override koskee vain `aarch64-apple-darwin`, ja se job onnistui).

- **SEURATTAVA (Jari 2026-08-20):** jos tämä toistuu, se lakkaa olemasta huono tuuri ja
  muuttuu oikeaksi päätökseksi: oma build-kapasiteetti, targetin pudotus, vai manuaalinen
  rerun käytäntönä. Yksi datapiste ei riitä perusteluksi, joten nyt vain seurataan.
- **Backstop-tarkistukset pysyvät pakollisina.** Tässä cutissa ne olivat ainoa asia joka
  paljasti että alkuperäinen "tämä on vain kilpailutilanne" -tulkintani oli väärä. Työkalun
  oma verdikti on nyt ollut väärässä molempiin suuntiin (0.15.0 false-red, 0.16.0 false-calm).
- **Shipshape-parannusehdotus (havaittu ossctl 0.9.0:ssä 2026-08-20, ei vielä
  filattu):** verify päättelee toimituksen *kohteesta* (onko assetteja) eikä
  *delegoidusta ajosta*, joten "ei vielä",
  "valmis" ja "kaatui lopullisesti" näyttävät identtisiltä (nolla assettia). Jos verify lukisi
  delegoidun workflow'n tilan, se kertoisi syyn heti: `in_progress` → odota, `success` →
  tarkista kohde, `cancelled`/`failure` → punainen syineen. Lisäksi `pending` ansaitsee oman
  exit-koodinsa erillään `missing`ista. Rajattu odotus on toissijainen, se ei yksin korjaa
  sitä että "ei vielä" ja "kaatui" näyttävät samalta.

**Rinnakkainen sessio (2026-08-20 aamu, ei tästä rupeamasta):** toinen sessio landasi
mainiin intake-lifecyclen schema-käyttöönoton ja legacy-labelien migraation
(`91cfd8c`, `f8d80ce`). Green gate ajettu näiden päällä, main on vihreä.

**Seuraava askel:**
- @deprecate-triage-inbox: intake-konsolidaation viimeinen osa (`issuectl triage` +
  `issues/inbox/`-laskeutumisalue pois rinnakkaisena vastaanottomekanismina). Molemmat sen
  esityöt on toimitettu.
- **ADR 0004:n versiolappu korjattava:** ADR puhuu deprekointi-releasesta nimellä "0.16.0",
  mutta 0.16.0 vei valmisteluslicen (puhtaasti additiivinen, joten semver pakotti minoriin).
  Osoita lappu 0.17.0:aan kun deprekointi-issue filataan.
- @apply-patch-from-stdin: **triagattu 2026-08-20** (oli `intake-feature-issuectl-0b1bf129b13b`,
  nimetty kunnolla). `apply` ottaa patchin vain tiedostopolkuna; pyyntö on `-`/`--stdin`, ja
  duplikaatti @apply-inline-json suljettiin taittaen sen sisään kaksi lisäkohtaa: inline-`{`
  -argumentin tuki (valinnainen) ja se että virheviesti `cannot read patch file …` on varsinainen
  papercut — tuntemattoman muodon pitää nimetä hyväksytyt muodot. Avoin nyanssi: sisarrepon
  filaus-wrapper toi tämän legacy-konventiolla (`status: open` + `needs-triage`), eli juurisyy
  filaavassa päässä on yhä korjaamatta. Tyyppi `feature`, sisältö lukee bugina — retype-kandidaatti.
- **Jarin päätös (2026-08-20): seuraava rupeama tekee molemmat yllä olevat issuet**,
  @deprecate-triage-inbox ja @apply-patch-from-stdin, joko rinnakkain tai peräkkäin.
  Rinnakkaisuuden ainoa varaus on hot-file-sääntö: molemmat voivat koskea skill-templateja,
  joten pidä templatemuutokset vain deprekoinnin puolella tai aja ne peräkkäin. Kun molemmat
  ovat landanneet, tuotepuolelta ei ole muuta avointa työtä ja 0.17.0 on cuttivalmis
  (ADR 0004:n versiolappu, ks. yllä). Aloita rupeama lukemalla `issuectl dag`.
- `issuectl doctor` huomauttaa kahdesta asiasta: tuntematon frontmatter-avain
  `pidev-pi-skill-lifecycle: title`, ja `.issuectl/AGENTS.md` puuttuu.

</details>

<details>
<summary>Aiemmat rupeamat (2026-08-17)</summary>

**Tila (2026-08-17, ilta — kaksi rupeamaa samana päivänä valmiina):** `main` **vihreä**
(1082 testiä + integraatiot, fmt/clippy/doc puhtaat, orkestroija verifioi mergien jälkeen),
pushattu. **Live-release `v0.15.0` kaikissa kanavissa** (crates.io ×2, GitHub Release 12
assetia — pudotus 15:stä on Windows-drop, tap 0.15.0; kaikki backstop-verifioitu). Paikallinen
binääri päivitetty (`cargo install --path`). Ei ajossa olevia workereita eikä worktreitä.

**Rupeama 1 (arkkitehtuurikatselmus):** @split-main-rs (kolme hot-tiedostoa pilkottu puhtaana
siirtona: `main.rs` 9278→5 riviä → `cmd/`-perheet, `doctor.rs` → `doctor/`, `mutate/mod.rs`
7319→906 → verbitiedostot; hot-file-sääntö nyt per-perhemoduuli) + @cli-verb-surface
(ADR `docs/decisions/0004`: `update` ainoa valikoiva mutaatioverbi, foldit + deprekointi-ikkuna
valmistelu → piilotetut aliakset + varoitukset → poisto; uusi ylätason verbi vaatii
ADR-amendmentin). Lisäksi: `issuectl archive` käyttöön (60 issueta arkistoon), AGENTS.md
sääntökirjaksi (perustelut `docs/decisions/0002–0003` + `docs/design/`), ossctl 0.7.0 +
`distribution:`-blokki OSS-RELEASE.md:hen.

**Rupeama 2 (0.15.0):** kolme workeria rinnakkain, kaikki landattu —
1. @intake-bug-issuectl-bad8e7d6118a: `skill install --force` säilyttää repo-kirjoitetun
   `issues/AGENTS.md`:n; uusi `--force-scaffold` regeneroi eksplisiittisesti.
2. @intake-bug-issuectl-bf2580033c3a: henkilökohtaiset infra-viittaukset pois lähteestä.
3. @intake-queue-legacy-mismatch: jono status-strict + legacy-varoitus + `intake migrate
   --apply` -polku. → @deprecate-triage-inbox vapautui blockeristaan.
4. @intake-feature-issuectl-77792e73735b: `apply --help` näyttää `body_ops`-shapet
   (parser-pinnattu esimerkki). 5. @intake-bug-issuectl-fab0edad2e42: dag-järjestyspolitiikka
   näkyviin. Intake-triage: 5 kohdetta (1 suljettu obsoletena, loput lanetettu).

**RELEASE-HAVAINTO (0.15.0 cut, ossctl 0.7.0):** engine julkaisi cratet ja delegoi GitHub
Releasen oikein (0.6.1-bugit todistetusti korjattu; @ossctl-cut-no-publish suljettu). MUTTA
cut päättyi exit-2:een: ossctl ajaa **oman** homebrew-leginsä vaikka cargo-distin
`publish-jobs=["homebrew"]` omistaa tapin — legi kaatui transienttiin GitHub 503:een, verify-
vaihe jäi ajamatta ja run kirjautui failed-tilaan vaikka kaikki kohteet toimitettiin
(false-red). Filattu ossctl:ään: `@cut-runs-own`. Kunnes se on korjattu: **failed-cutin
jälkeen aja backstop-tarkistukset ennen johtopäätöksiä** (crates.io API, `gh release view`,
tap-formula) — dist-vaiheen kaatuminen ei tarkoita että julkaisu epäonnistui.

**Seuraava askel:**
- `verb-surface`-lane: @intake-bug-issuectl-d5b6669a98fe (update:n `--json`-echo ei raportoi
  juuri asetettuja lane-kenttiä; vahvistettu koodista) → @update-canonical-forms (ADR:n
  valmisteluslice: `update --patch-file`, `--query`, parity-testit) → @deprecate-triage-inbox.
  Sarjassa (sama echo/write-pinta).
- `skills`-lane: @intake-feature-issuectl-42403ae544e3 (skill dokumentoi lane-liput).
- `intake`-lane: @intake-feature-issuectl-4f9dbc60a05e (deferred-labelin eläköinti +
  doctor-check).
- @purge-telegram-surfaces — nyt vapaana ajettavaksi (skills- ja intake-laneissa ei ajossa
  mitään; törmää templateihin, joten älä rinnakkain 42403:n kanssa).
- ADR:n deprekointi-release (piilotetut aliakset + varoitukset + canonical-only skillit)
  filataan kun @update-canonical-forms on landannut.

</details>

**Dogfood:** `cargo install issuectl` / `brew upgrade jarimustonen/issuectl/issuectl` /
`cargo install --path crates/issuectl`. Skillit `/issue`, `/issue-new`, `/issue-intake`
tulevat `issuectl skill install`ista; uudet kohteet sisään `issuectl intake file`lla,
jonon nostaa `/issue-intake` (tai `/stint-start`).

---

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`,
`blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog
in this file. Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and
`issuectl dag` are the source of truth.

## Handoff-protokolla

`/stint-handoff` päivittää yllä olevan **🔄 Continue here** -blockin ja tarkistaa
`issuectl dag` -näkymän rupeaman lopussa. Committoi vain muuttuneet polut täsmällisesti
(`TODO.md` ja issue-tiedostot, jos niitä muutettiin) ennen `/wrap-up`:ia, jotta tuore
agentti voi jatkaa komennolla `/skill:stint-start`. Pidä `main` puhtaana rinnakkaisten
worktreiden takia (ks. globaali CLAUDE.md). Aja samalla `issuectl archive --dry-run` ja
arkistoi jos kertymää on.

## Intake

Saapuneet bugiraportit ja feature-pyynnöt elävät `issuectl`:ssä, eivät tässä
tiedostossa. Uusi kohde sisään `issuectl intake file`lla; jonon nostaa
`/issue-intake` (tai `/stint-start`):

```bash
issuectl intake queue
issuectl ls --status open --label needs-triage
```

Hyväksytty kohde lanetetaan `issuectl`-frontmatteriin ja näkyy sen jälkeen
`issuectl dag`issa.

Huom: raportoivan pään filaus-flow (sisarrepon wrapper) appendaa tänne oman
checklist-osionsa joka kerta. Se ei tule issuectl:n binääristä eikä templateista;
poista osio triagen yhteydessä kunnes lähdepää on korjattu.

## Piialiisan bugiraportit

- [ ] 🐛 Piialiisan bugiraportti: Issue skill reads the wrong create result path field — jari via Telegram ([`intake-bug-issuectl-71ea534241c2`](issues/intake-bug-issuectl-71ea534241c2/item.md))
- [ ] 🐛 Piialiisan bugiraportti: update JSON omits persisted blocked_by after add-blocked-by — jari via Telegram ([`intake-bug-issuectl-704cd8eb0a0e`](issues/intake-bug-issuectl-704cd8eb0a0e/item.md))
- [ ] 🐛 Piialiisan bugiraportti: issuectl create --body-file duplicates Description heading — jari via Telegram ([`intake-bug-issuectl-3148539799d8`](issues/intake-bug-issuectl-3148539799d8/item.md))
- [ ] 🐛 Piialiisan bugiraportti: doctor warns about issuectl-owned intake fields — jari via Telegram ([`intake-bug-issuectl-af715e8b5283`](issues/intake-bug-issuectl-af715e8b5283/item.md))
