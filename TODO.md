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

**Tila (2026-07-26):** `main` puhdas, julkaistu versio **v0.6.4**.
Backlog käyty läpi ja tieroitettu (alla). Vanhentunut v0.6.0-kandidaatti-epic
`@hugely-exciting-spiders` suljettu `obsolete`-statuksella (sen lapsi-issuet
jäivät auki omina issueinaan).

**Seuraava askel:** aloita **Tier 1**:stä. Luontevin ensimmäinen rupeama on
bugi-nippu + halvat agenttiergonomia-voitot — ne ovat pieniä, selkeitä ja
puhtaita worktree-paloja. `@doctor-fix-noop` on jo `in-progress`, joten
tarkista sen tila ensin.

**Avoin päätös ennen Tier 2:ta:** `@intensely-teeny-ink` (custom boards) syö
scopessaan useita Tier 3:n kanban-kandidaatteja (multiple boards, priority-sort,
card fields). Päätä sen rajaus ennen rakentamista, ettei tehdä päällekkäistä työtä.

---

## Tier 1 — tee heti (bugit + halvat agenttivoitot)

Pieniä, korkean vipuvaikutuksen paloja. Sopivat itsenäisiksi worktreiksi.

- [ ] **@doctor-fix-noop** — `bug`, **high**, *in-progress*. `doctor --fix`
      exittaa 1 mutta ei tee raportoimiaan korjauksia (alias-koersiot,
      AGENTS.md-drift). Tarkista työn tila ennen uuden worktreen spawnaamista.
- [ ] **@rename-stale-self-slug** — `bug`. `rename` ei päivitä issuen omaa
      `slug:`-kenttää; `doctor --fix` leimaa slugin → ei enää latentti. Pieni.
- [ ] **@json-close-requires-expected-version** — `bug`. `--json` muuttaa
      *vaadittujen argumenttien* pintaa, ei pelkkää output-formaattia → agentti-
      callerit kaatuvat. ⚠️ Ei triviaali: nykyinen käytös on tietoinen valinta
      (virheteksti *"per design D4=B"*), joten korjaus on **design-käännös**, ei
      laastari. Päätä ensin: symmetria non-JSON-polun kanssa vs. D4=B:n peruminen.
- [ ] **@verb-alias-discoverability** — `feature`. Nippu error-hint/alias-fixejä:
      `create`→`new`-alias, `--body` `new`:lle, `body <slug>`-virheen vihje
      `body set`:iin. Halpaa; säästää jokaisen agentin arvaus-korjaus-kierrokset.
- [ ] **@assign-subcommand-alias** — `feature`. Lisää `assign`-alias
      `set --assignee`:lle. Triviaali; luonteva pari edellisen kanssa (voi tehdä
      samassa worktreessä "CLI-alias-nippuna").
- [ ] **@fiercely-colossal-rabbits** — `chore`, **high**. Cachaa `canonical_hash`
      (nyt lasketaan joka `/api/issues`-kutsulla per issue). Aito perf-ongelma
      isoilla repoilla; selkeä fix (cache mtime+size-avaimella AppStateen).

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

## Siivous (harkittavaksi)

- [ ] **@excessively-beneficial-owner** — `task`. "Claude Code launch button"
      -tutkimus, jonka scope on jo korvattu valmistuneella
      `@entirely-cowardly-aftermath`-designilla. **Suositus: sulje** `obsolete`.
- [ ] Suljetun epicin `@hugely-exciting-spiders` lapsi-issuet: käy läpi mitkä
      kandidaatit on jo shipattu 0.6.x:ssä ja sulje ne; loput → mahdollinen
      v0.7.0-epic.

---

## Handoff-protokolla

`/stint` päivittää yllä olevan **🔄 Continue here** -blockin rupeaman lopussa
ja committaa sen omana committinaan (`git add TODO.md && git commit`) ennen
muuta työtä — näin tuore agentti voi jatkaa `jatketaan @TODO.md`:sta. Pidä
`main` puhtaana rinnakkaisten worktreiden takia (ks. globaali CLAUDE.md).
