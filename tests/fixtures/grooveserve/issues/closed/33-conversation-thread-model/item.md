---
created: 2026-04-27
updated: 2026-04-28
closed: 2026-04-28
type: feature
reporter: jari
assignee: jari
status: done
priority: high
epic: 5
related: ["#26", "#27", "#31", "#32", "#34", "#35", "#36"]
labels: [ai, conversation, schema]
commits:
  - hash: da5ab4f
    summary: "feat(email): add schema + db plumbing for thread-based conversations (#33)"
  - hash: 662e7a0
    summary: "fix(email): tighten thread schema + backfill safety (#33 review)"
  - hash: 52cf2d2
    summary: "fix(email): harden thread resolution + claim logic (#33 review)"
  - hash: df67df5
    summary: "fix(email): deterministic fallback Message-ID + small cleanups (#33 review)"
  - hash: 19476d0
    summary: "test(email): integration tests for claim_with_thread (#33 review I5)"
  - hash: 9fb64c6
    summary: "feat(email): layered system prompt + thread-scoped reply path (#33)"
  - hash: 03f7896
    summary: "fix(email): apply review fixes for #33 stage 2"
---

# 33. Thread-pohjainen keskustelumalli + pysyvä käyttäjätila

_Source: agentin keskustelumalli — Phase 1 review_
_Status: see [status.md](./status.md) — vaiheet 1–4 valmiit, 6–8 jäljellä._

## Description

Agentin nykyinen keskusteluhistoria tallennetaan tasaisena listana per lähettäjä
(`conversations.sender`). Tämä on väärin:

- Vastaus aiempaan sähköpostiin = saman threadin jatkoa
- Uusi sähköposti (uusi aihe, ei vastaus) = uusi keskustelu
- Agentti ei pysty erottamaan näitä — kaikki on yhtä pitkää historiaa

Lisäksi agentti hukkaa kontekstin keskustelujen välillä. Pysyvät tiedot
(profiili, käynnissä olevan matkalaskun tila, kuitit) pitäisi:

- Poimia viesteistä ja tallentaa pysyvästi
- Injektoida agentin kontekstiin system promptin tai työkalujen kautta
- Olla riippumaton historian uudelleen-toistosta

## Scope

Issue kattaa neljä yhteenkuuluvaa kerrosta:

**1. Viestimalli (vain otsakepohjainen, ei subject-fallback)**
- [x] Thread-tunnistus `In-Reply-To` + `References` -otsakkeilla
- [x] `threads` + `thread_messages` (per-user-scoped, ks. status.md) + `conversations.thread_id`
- [x] `email_processing.thread_id` claim-aikaan (retry-idempotenssi)
- [x] Threadin elinkaari (active → idle → closed, 90 vrk max revive; `closed` ei revive)
- [x] Reuna-tapaukset: forward, sender-muutos, multi-recipient

**2. Agentin yleiset ohjeet (system prompt Block 1, openclaw-tyyliin kerrostettu)**
- [x] **SOUL.md** — agentin persoona ja ääni (englanniksi) _(tiedosto repossa)_
- [x] **AGENTS.md** — toimintasäännöt + self-model (englanniksi) _(tiedosto repossa)_
- [x] **SALIENCE.md** — mitä muistaa + manipulaationtorjunta (englanniksi) _(tiedosto repossa)_
- [x] Toteutus oikeina tiedostoina `services/email/prompts/*.md` ja
  `include_str!` _(`agent/prompts.rs::block1_persona_rules`, cache-control: ephemeral)_

**3. Käyttäjäkohtaiset taustatiedot (system prompt Block 2)**
- [x] **USER.md**-renderöinti kannasta jokaiseen kutsuun
  _(`agent/user_memory.rs::render_user_md`)_
- [x] Frontmatter (tyypitetty) + body (vapaamuotoinen markdown)
- [x] `null`-arvot näkyvät eksplisiittisinä aukkoina _(`yaml_or_null`)_
- [x] Re-render agenttisen loopin jokaisessa iteraatiossa

**4. Käyttäjätietojen päivitys**
- [x] `update_user_preferences` laajennus `language`-kentällä (BCP-47)
  _(handler validoi tagiformaatin)_
- [x] `update_user_notes` (uusi) — body, ohjeistettu SALIENCE.md:llä
  _(strippaa frontmatterin, 16 kB cap)_
- [x] `notes_md` ja `language` -kentät `user_profiles`-tauluun
- [x] `language`-backfill `preferences`-jsonbista _(atominen)_

**Concurrency-malli**
- [x] Sarjallinen käsittely per IMAP-tili (jo olemassa, dokumentoidaan
  invarianttina §14)
- [x] USER.md re-render per agenttisen loopin iteraatio

**Migraatio**
- [x] Inkrementaalinen migraatiopolku (vaiheet 1–4); `conversations.thread_id NOT NULL`
  -tiukennus tarkoituksellisesti pois scopesta — kirjoituspolut tunnetaan
- [x] Backfill gap-pohjaisesti splittautuviin legacy-threadeihin
  (window-funktioilla, batched, out-of-band; multi-tenant precondition)

## Toiseen issueen siirretty

- **#34** — `report_suspicious_message`-työkalu + manipulaationtunnistus
- **#35** — Prompt-cache-strategia (palataan kun toteutus toimii)
- **#36** — Privacy review (audit, GDPR-endpointit, retention)

## Deliverable

Suunnitteludokumentti `design.md`, joka kattaa kaikki neljä kerrosta yllä,
sekä skeemamuutokset (SQL), system prompt -rakenteen ja inkrementaalisen
migraatiopolun.
