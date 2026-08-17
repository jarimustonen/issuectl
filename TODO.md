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

**Tila (2026-08-17, cli-fixes-rupeama + 0.14.0 & 0.14.1 julkaistu):** `main` **vihreä** (1078 testiä, fmt +
clippy + `cargo doc` puhtaat). **`origin/main` == `main` (pushattu, `7f0cc83`).** **Live-release `v0.14.1`** —
crates.io (core + bin), GitHub Release 15 assetia, **ja Homebrew tap vihdoin ajan tasalla (0.14.1)**.
`Cargo.toml == 0.14.1`. Ei ajossa olevia workereita.

**DAG (2 spawnable headia, valmis rinnakkaisajoon):**
- **`cli-fixes`** (syvyys 2, sarjassa — `main.rs`): `@intake-feature-issuectl-77792e73735b` (seq 10) →
  `@intake-queue-legacy-mismatch` (seq 20)
- **`release-infra`** (syvyys 1): `@ossctl-cut-no-publish` — **uudelleen avattu verifiointiportiksi**, ei työtä
  vaan tarkistuslista seuraavaan cuttiin

**Tämä sessio: koko `cli-fixes`-lane (7 issueta) läpi + KAKSI julkaisua + Homebrew-jakelu korjattu.**
Ensimmäinen aalto ajoi kolme lanea rinnakkain; loput sarjassa `main.rs`-collisionin takia. Jokainen unitti
reviewattu (`/llm-review` + `/assess-findings`) + täysi green gate.

**0.14.0 — 7 issueta viidessä aallossa:**
1. `@intake-bug-issuectl-06c42e2d1123` — `doctor --fix` laskee jäljellä olevat findingit oikein (raportoi 1,
   listasi 9). Tiivistysrivi on se, jonka agentti/CI lukee päättääkseen onko repo puhdas.
2. `@audit-no-user-specifics` — julkisen paketin sweep. **Paketti EI ollut puhdas**: maintainer-spesifisiä
   defaultteja ja esimerkkejä löytyi ja poistettiin.
3. `@intake-feature-issuectl-c633267ba553` — `docs/design/lane-design.md`, lane-rakenteen suunnitteluopas.
4. `@neutralize-main-author-example` + `@create-help-slug-text` — `main.rs`:n esimerkki neutraloitu; `create
   --help` kertoo vihdoin oikean, otsikosta johdetun oletusslugin.
5. `@intake-bug-issuectl-7a79c97d9fa8` + `@intake-bug-issuectl-715670f2607f` — slugin lyhentyminen ja
   törmäyssuffiksit näkyvät ylätason `warnings`issa; `note`/`close` jakavat `--comment`/`--message`-sanaston.
6. `@intake-feature-issuectl-ff7665d266e6` — `update --type epic` migroi `reporter` → `owner` itse; uudet
   `--no-reporter` / `--no-assignee`; monitulkintaisissa virhe nimeää ajettavan komennon.
7. `@surface-lane-design` — `dag --help` selittää lane-designin, ja `dag` näyttää per-lane-syvyyden +
   spawnable-headien määrän.

**0.14.1 — Homebrew-jakelu.** Ks. RELEASE-OPPI alla.

**Sivutuotteet ja opit:**
- **Auditin esiskannaus oli VÄÄRÄSSÄ.** Edellinen triage merkitsi paketin "näyttää puhtaalta". Worker käskettiin
  tekemään sweep itse eikä luottamaan vihjeeseen — ja se löysi oikeita osumia. **Älä hyväksy esiskannausta
  lopputuloksena.**
- **`main.rs` on tämän repon pullonkaula.** 7 issueta osui siihen → koko rupeama sarjassa. Kaksi kertaa
  niputettiin kaksi pientä unittia samaan workeriin (säästi 2 slottia), mutta se on kiertotie. **`main.rs`:n
  pilkkominen on nyt konkreettinen skedulointi-investointi** — ja `issuectl dag` osaa itse näyttää sen luvun.
- **Filattu ossctl:ään kaksi bugia** (ks. RELEASE-OPPI): `@release-bump-plan-uncuttable`,
  `@release-tag-preempts-cargo-dist`.
- **Filattu tänne `@intake-queue-legacy-mismatch`:** `intake queue` listaa legacy-label-pohjaisia kohteita
  (`status: open` + `needs-triage`), mutta jokainen `intake`-transitio hylkää ne (validoi statuksesta). Mukana
  tuleva `/issue-intake`-skill väittää nimenomaan päinvastaista → agentti kutsuu `accept`ia ja saa kovan virheen.
  Admitointi vaatii tällä hetkellä `label --remove needs-triage` eli CLI:n intake-pinnan ohituksen.

**Seuraava askel:** `cli-fixes` sarjassa (`@intake-feature-issuectl-77792e73735b` → `@intake-queue-legacy-mismatch`).
GLOBAL HEAD = `@intake-feature-issuectl-77792e73735b`.

**⚠️ INTAKE-PREDIKAATTI-SUDENKUOPPA:** `/stint-handoff`:n dokumentoitu detect-predikaatti on `open ∧ via:telegram ∧
needs-triage`, mutta intakea saapuu myös `via:agent-*`-provenanssilla (sisarrepojen wrap-upit). Pelkkä
`via:telegram` **ei löydä niitä**. Käytä `issuectl list --status open --label needs-triage` (provenanssiagnostinen)
TAI `issuectl intake queue` — mutta huomaa `@intake-queue-legacy-mismatch`: jono listaa legacy-kohteita joita
`intake accept` ei suostu käsittelemään.

**⚠️ Siivousta odottava:** `issuectl__worktrees/wt-01m04ygzhw-canon-review-issuectl` (@286dcb3, **locked**) on
edellisen session canon-review-ajosta. Jätetty koskematta — poista käsin jos on turha
(`git worktree remove --force` + `git branch -D`).

**⚠️ RELEASE-OPPI (0.14.1 ajettiin ossctl 0.6.1:n engine-reitillä ja se JULKAISI oikeasti — vanha manuaalinen
fallback ei ole enää tarpeen, mutta cut EI ole valmis kun se tulostaa `release complete`):**
- **Älä käytä `ossctl release plan --bump`.** Se sinetöi suunnitelman jonka cut hylkää aina `plan_stale`:na, ja
  ehdottaa tilalle id:tä joka tarkoittaa *"julkaise uudelleen jo julkaistu versio"*. Tee bumppi käsin, sitten
  `plan` ilman lippuja. (`@release-bump-plan-uncuttable`)
- **`tag`-vaihe luo GitHub Releasen** → cargo-distin `host` kaatuu (`a release with the same tag name already
  exists`) → **`publish-homebrew-formula` skipataan** vaikka cut näyttää vihreältä. Korjaus: poista assetiton
  Release (`gh release delete vX.Y.Z --yes`, git-tagi säilyy) → `gh run rerun <id> --failed`.
  (`@release-tag-preempts-cargo-dist`)
- **Verifioi aina cutin jälkeen:** `gh release view vX.Y.Z --json assets --jq '.assets|length'` (odota ~15, **ei
  0**) ja että tap-formula nousi uuteen versioon. Tap seisoi 0.11.0:ssa kolmen julkaisun ajan koska kukaan ei
  tarkistanut (`@homebrew-tap-stale`).
- **Homebrew tulee cargo-distiltä**, `dist-workspace.toml`:n `installers`/`tap`/`publish-jobs`-riveiltä.
  ossctl:n oma homebrew-leg on inertti (`OSS-RELEASE.md`:ssä ei ole `distribution`-blokkia →
  `homebrew_tap: null`). **Älä aja `ossctl dist generate`ia** korjataksesi sen ilman erillistä päätöstä: se
  pyyhkisi self-hosted macOS-runner-overriden (`hauis`, ~67 s macOS-buildi vs 45+ min hostattu jono).

**⚠️ Minor-bump-gotcha:** `crates/issuectl/Cargo.toml`:n sisäinen `issuectl-core = { …, version = "X" }`
on caret-vaatimus → bumppaa se vastaamaan uutta minoria samassa release-commitissa. Vain minor/major-raja
vaatii tämän, ei patch (0.14.0 → 0.14.1 ei vaatinut).

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

## Intake

Saapuneet bugiraportit ja feature-pyynnöt elävät `issuectl`:ssä, eivät tässä tiedostossa.
Uusi kohde sisään `issuectl intake file`lla; jonon nostaa `/issue-intake` (tai `/stint-start`):

```bash
issuectl intake queue
issuectl ls --status open --label needs-triage
```

Hyväksytty kohde lanetetaan `issuectl`-frontmatteriin ja näkyy sen jälkeen `issuectl dag`issa.
