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

**Tila (2026-08-16, 0.11.0-release + cli-fixes-rupeama):** `main` **vihreä** (1058 testiä, fmt+clippy puhtaat —
0 uutta clippy-varoitusta, 52 pre-existing). **`origin/main` == `main` (pushattu).** **Live-release nyt `v0.11.0`**
(crates.io + binäärit + Homebrew + tag kaikki ulkona). `Cargo.toml == 0.11.0`. **Aktiivinen työ tyhjä** — DAG:ssa
vain parkissa oleva `@ossctl-cut-no-publish` (upstream-blocked). Ei ajossa olevia workereita.

**Tämä sessio: 7 unittia landattu + 0.11.0 julkaistu.** Kulku:
1. **DAG-merge:** foldattu uusi `@sync-commits-empty-main` `cli-fixes`-laneen (collision main.rs, seq 60).
2. **Release-lane:** `@changelog-trailers-never` (GLOBAL HEAD) landattu design-first — **`issuectl close --stamp`**
   amendaa HEADin `Fixes-Issue: @<slug>`-trailerilla (option 1, issuectl-puoli; 4-model review). Trailer-vetoinen
   `issuectl changelog` toimii nyt eteenpäin nolla-vaivalla.
3. **cli-fixes-lane (kaikki sarjassa `main.rs`-collisionin takia, kukin reviewattu + vihreä + trailer-stampattu):**
   `@intake-bug-issuectl-d6947128f6c9` (label --remove --json error-envelope + flag-form), `@list-status-done`
   (--status done palauttaa closed/archived), `@intake-feature-issuectl-d93eaa168c66` (update --add-blocked-by),
   `@intake-feature-issuectl-035722451473` (new --lane/--lane-seq/--add-collision), `@add-comment-alias`
   (comment-alias + --message/--body-file), `@sync-commits-empty-main` (tyhjän default-rangen varoitus).
4. **RELEASE 0.11.0 (jari valitsi OPTION a — kertaluontoinen käsinbackfill):** CHANGELOG `[0.11.0]` kuratoitu
   käsin kattaen KAIKKI v0.10.0:n jälkeen toimitetut unitit (tämän session 8 + ~15 historiallista trailerittomasta
   pushatusta committista), + huomautus että trailer-automaatio alkoi kesken syklin (0.12.0→ automaattinen).
   Bump 0.10.0→0.11.0 (root + caret-dep), `release: 0.11.0`-commit, push.
5. **ossctl cut EPÄONNISTUI (odotettu `@ossctl-cut-no-publish`-bugi):** publish-phase failasi, MITÄÄN ei uploadattu
   (varmistettu crates.io-indeksistä). → **manuaalinen fallback (AGENTS RELEASE-OPPI):** `cargo publish -p
   issuectl-core` → `-p issuectl` → `git tag v0.11.0` → push tag. Molemmat cratet crates.io:ssa, release.yml
   (cargo-dist) ajoi vihreänä → binäärit + Homebrew-formula + GitHub Release.

**Sivutuotteet:**
- **Orchestratectl-issue filattu:** `run-merge-stamp` (orchestratectl-repo) — `run merge` pitäisi stampata
  Fixes-Issue-traileri landing-committiin, jotta "nolla-vaivaa"-lupaus on end-to-end (issuectl-puoli tehty
  `close --stamp`:llä; tämä on toinen puolisko). Tähän asti workereille briefataan `close --stamp`.
- **Intake-duplikaatti suljettu:** `@intake-feature-issuectl-986ecd5a58a9` (label --add/--remove flag-aliakset)
  suljettu `obsolete` — jo toimitettu 0.11.0:ssa (`@intake-bug-issuectl-d6947128f6c9`).

**Seuraava askel:** **ei aktiivista työtä.** Kaikki laned unitit landattu, 0.11.0 ulkona. `@ossctl-cut-no-publish`
pysyy parkissa kunnes upstream-ossctl korjaa. Uusi rupeama = odota intakea (`/issue-intake` / `/stint-start`
nostaa jonon) tai ota `deferred`-backlogista jokin takaisin peliin. Jos releaset halutaan taas ossctl:n kautta,
se vaatii `@ossctl-cut-no-publish`-fixin ensin.

**⚠️ RELEASE-OPPI (VAHVISTUI TÄSSÄ SESSIOSSA):** ossctl `release cut` EI julkaise oikeasti
(`@ossctl-cut-no-publish`, upstream-blocked) — publish-phase failaa ("core not visible on index within 300s"),
MITÄÄN ei uploadata. → release vaatii manuaalisen `cargo publish -p issuectl-core` → (odota indeksi) →
`-p issuectl` → `git tag vX.Y.Z <release-commit>` → `git push origin vX.Y.Z` -fallbackin. Tag laukaisee
`release.yml`:n (cargo-dist) → binäärit + Homebrew (EI double-publishaa crates.io:ta). Ennen fallbackia: bump +
CHANGELOG-finalisointi + `release:`-commit + push main. **Varmista aina crates.io-indeksistä ETTEI core jo
uploadattu** ennen manuaalista publishia (double-publish failaa). ossctl-run jää `in_progress`-tilaan sen omaan
journaliin (`release verify <run-id>` näyttää unreconciled — vaaraton, ei estä mitään).

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
