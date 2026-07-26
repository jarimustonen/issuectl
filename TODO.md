# TODO — issuectl

Työjono ja handoff `/stint`-työrupeamille. Tämä tiedosto on
`/stint`:n käynnistyspiste: lue ensin alla oleva handoff-block, sitten
työjono. Issue-viittaukset ovat `@slug`-muodossa — koko backlog elää
`issuectl`:ssä (`issuectl list`), tämä tiedosto on vain kuratoitu
näkymä "mitä kannattaa tehdä ja missä järjestyksessä".

Operating-faktat (deploy, green gate, hot files) ovat
[AGENTS.md](AGENTS.md#operating-facts-for-stint):ssä.

---

## 🔄 Continue here · ALOITA TÄSTÄ

**Tila (2026-07-26):** `main`:issa **3 julkaisematonta yksikköä** viimeisimmän
tagin (`v0.6.4`) päällä — CLI-alias-nippu (create/new-alias, `--body`, body-hint,
`assign`), rename self-slug -fix, ja doctor-fix-noop-siivous. Kaikki vihreitä
(fmt, clippy **0 uutta varoitusta**, koko testisuite läpi). **v0.6.5-julkaisu
odottaa käyttäjän go/no-go:ta** (release-autonomia = go/no-go required).

**Tässä rupeamassa landattu (2026-07-26):** @verb-alias-discoverability +
@assign-subcommand-alias (yhtenä CLI-alias-nippuna) ja @rename-stale-self-slug.
Suljettu jo-valmiina: @doctor-fix-noop (fix oli landannut jo aiemmin commiteissa
438d22f/1573d2a/5a49a14, issue vain jäänyt merkkaamatta).

**Seuraava askel:** ks. **Seuraavat — DAG** alla. `@fiercely-colossal-rabbits`
on **READY** (rename-fix vapautti `repo.rs`:n, ei enää törmää).
`@json-close-requires-expected-version` on **BLOCKED** design-päätöksen taakse.

**Avoin päätös ennen Tier 2:ta:** `@intensely-teeny-ink` (custom boards) syö
scopessaan useita Tier 3:n kanban-kandidaatteja (multiple boards, priority-sort,
card fields). Päätä sen rajaus ennen rakentamista, ettei tehdä päällekkäistä työtä.

---

## Seuraavat — DAG (riippuvuudet)

Ne kaksi seuraavaksi otettavaa issueta ja mistä ne riippuvat. Ylhäältä alas =
"vapauttaa / portittaa". `✅` = landattu, `⬜` = tekemättä, `⛔` = blokattu.

```
@rename-stale-self-slug  ✅ landattu (repo.rs)
        │
        │  vapauttaa — sama hot file repo.rs, ei enää rebase-törmäystä
        ▼
@fiercely-colossal-rabbits  ⬜ READY  ── chore/high. Cachaa canonical_hash
                                         /api/issues:lle. Ota tämä ensin.

[design-päätös: --json close/update -symmetria non-JSON-polun kanssa
 vs. D4=B:n (strict --expected-version) säilyttäminen]   ⛔ avoin (käyttäjän call)
        │
        │  portittaa — käytös on tietoinen valinta, ei laastari
        ▼
@json-close-requires-expected-version  ⬜ BLOCKED ── ei rakenneta ennen päätöstä
```

Kaari-lyhyesti: rename-fix jo poisti ainoan riippuvuuden canonical-cachelta →
**se on nyt vapaa otettavaksi**. json-close taas odottaa suunta­päätöstä
(symmetria vs. strict), ei koodia.

---

## Tier 1 — tee heti (bugit + halvat agenttivoitot)

Pieniä, korkean vipuvaikutuksen paloja. Sopivat itsenäisiksi worktreiksi.
Kierroksen 2026-07-26 jälkeen jäljellä on enää DAG:n kaksi solmua (ks. yllä).

- [x] **@doctor-fix-noop** — suljettu `fixed`; fix oli landannut jo aiemmin.
- [x] **@rename-stale-self-slug** — landattu 2026-07-26. `rename` päivittää nyt
      issuen oman `slug:`-kentän + regressiotesti.
- [x] **@verb-alias-discoverability** — landattu 2026-07-26 (CLI-alias-nippu).
- [x] **@assign-subcommand-alias** — landattu 2026-07-26 (CLI-alias-nippu).
- [ ] **@fiercely-colossal-rabbits** — `chore`, **high**, **READY** (ks. DAG).
      Cachaa `canonical_hash` (nyt lasketaan joka `/api/issues`-kutsulla per
      issue). Aito perf-ongelma isoilla repoilla; cache mtime+size-avaimella
      AppStateen. **Seuraavan rupeaman luontevin ensimmäinen pala.**
- [ ] **@json-close-requires-expected-version** — `bug`, **BLOCKED** (ks. DAG).
      `--json` muuttaa *vaadittujen argumenttien* pintaa → agentti-callerit
      kaatuvat. ⚠️ Design-käännös, ei laastari. Päätä ensin: symmetria
      non-JSON-polun kanssa vs. D4=B:n (strict `--expected-version`) säilytys.

## Tier 2 — seuraavaksi (yksi meaty feature, aito lähitarve)

- [ ] **@intensely-teeny-ink** — `feature`, **high**. Custom boards:
      käyttäjän määrittelemät sarakkeet (group_by epic/label/kenttä). Konkreettinen
      lähitarve: raahaa issuet 0.6/future-ämpäreihin ilman `item.md`-editointia.
      **Päätä scope ensin** — subsumoi osan Tier 3:sta.

## Tier 3 — defer / discuss (ei nyt)

**Kanban/web-board -kiillotus** (harkitse niputtamista custom-boards-työn jälkeen):
`@genuinely-cloistered-current` (multiple boards), `@truly-somber-payment`
(priority-sort), `@needlessly-flimsy-scarecrow` (card fields + värit),
`@partially-nasty-sack` (WIP-limit), `@fiercely-juicy-kettle` (copy-buttonit),
`@almost-homely-decision` (per-user view state), `@somewhat-flawless-letter`
(uncommitted-indikaattori), `@perfectly-white-linen` (undo),
`@needlessly-mysterious-volcano` (näppäimistö-a11y).

**Isommat visualisoinnit:** `@massively-periodic-surprise` (dependency graph),
`@needlessly-slippery-pan` (epic-puu).

**Strateginen:** `@focus-areas` — päätös tehty (ADR 0001: `areas: []`
skeemakenttä), valmis implementaatio-ADR:ää + rakentamista varten. Iso pala;
aikatauluta kun on tilaa.

**Build-only-if (älä rakenna ennen kuin tarve todistuu):**
`@supremely-accurate-body` (field-merge — vain jos 409-kitka todistuu),
`@somewhat-heady-zephyr` (events.jsonl — vain jos git-metriikat ei riitä),
`@practically-truculent-music` (watcher-race-observability — matala kiire).

## Siivous — tehty (2026-07-26)

- [x] **@excessively-beneficial-owner** suljettu `obsolete` (scope korvattu
      valmistuneella `@entirely-cowardly-aftermath`-designilla).
- [x] Suljetun epicin `@hugely-exciting-spiders` lapset käyty läpi: kaikki
      tosiasiassa 0.6.x:ssä shipatut kandidaatit oli jo suljettu `done`:ksi
      (~20 kpl). Auki jäi 10 kanban/board-kiillotuskandidaattia — ne **eivät**
      ole shipattu (varmistettu CHANGELOGista) ja elävät nyt Tier 3:ssa yllä.
      Mahdollinen v0.7.0-epic voidaan koota niistä myöhemmin.

---

## Handoff-protokolla

`/stint` päivittää yllä olevan **🔄 Continue here** -blockin rupeaman lopussa
ja committaa sen omana committinaan (`git add TODO.md && git commit`) ennen
muuta työtä — näin tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä
`main` puhtaana rinnakkaisten worktreiden takia (ks. globaali CLAUDE.md).
