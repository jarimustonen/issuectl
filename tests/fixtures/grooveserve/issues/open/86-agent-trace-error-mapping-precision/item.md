---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#82", "#58", "#62"]
labels: [agent, observability, error-handling, ops]
---

# 86. agent_trace error mapping precision (`map_sqlx_error`)

_Source: #82 `/llm-review` SPIN-OFF (GPT-5.5 #12)_

## Description

`crates/ops/src/agent_trace.rs::map_sqlx_error` mappaa kaikki
PostgreSQL FK-rikkomukset (SQLSTATE 23503) yhteen
`OpError::Forbidden`-virhevarianttiin ilman constraint-nimen tai
kontekstin liittämistä. Tämä on jaettu lookup kaikille
`agent_trace`-pinnan kirjoittajille (`start_run`, `record_step`,
`record_manual_run`).

`record_manual_run`:n tapauksessa lukuisat erilaiset FK-tilanteet
tuottavat saman virheen ilman vihjettä siitä mikä ankkuri kaatui:

- `(tenant_id, user_id) → tenant_users` — owner ei ole jäsen
- `actor_user_id → users` — actor ei ole olemassa
- `(linked_receipt_id, tenant_id, user_id) → receipts` — kuitti
  toisen tenantin tai stale
- `(linked_extraction_id, tenant_id, user_id) → extractions` — sama
  ongelma
- `(linked_attachment_id, tenant_id, user_id) → attachments` — sama
- `(linked_thread_message_id, tenant_id, user_id) → thread_messages`
  — sama (#82 lisäsi tämän kentän)

Devauksessa "miksi manuaalinen run hylkääntyi?" vaatii tällä hetkellä
psql-tasoisen tutkinnan tai DB-lokien lukemisen, koska
`OpError::Forbidden` ei kanna constraint-nimeä eikä
metadataa lokeista.

Sama koskee `start_run`:n `(tenant_id, user_id) → tenant_users`-
rikkomusta ja `record_step`:n composite-FK-rikkomuksia
(evidence-idit + run-tuple).

## Reproduction

```rust
// All of these surface identically as `OpError::Forbidden`:
let input = ManualRunInput {
    user_id: 999_999,                  // not in tenant_users
    linked_receipt_id: Some(stale_id), // wrong tenant
    // ...
};
agent_trace::record_manual_run(&pool, &ctx, input).await
// → OpError::Forbidden  (was it the user_id? receipt? both?)
```

## Suggested directions

Kolme suunnittelutason vaihtoehtoa, jotka kaikki vaikuttavat koko
`ops::agent_trace`-pintaan eivätkä vain manuaalisen runin polkuun:

1. **Lokita constraint-nimi warn-tasolla** ennen `Forbidden`-mappausta.
   Halvin vaihtoehto, virhepinta ei muutu, debugger saa vihjeen
   lokeista. Ei muutoksia kutsujapintaan.

   ```rust
   Some("23503") => {
       tracing::warn!(
           constraint = db_err.constraint(),
           "agent_trace FK violation"
       );
       return OpError::Forbidden;
   }
   ```

2. **Mapatkaa tunnetut constraint-nimet erillisiin
   `OpError`-variantteihin.** Esimerkiksi `OpError::ActorNotFound`,
   `OpError::EvidenceCrossOwner { kind: "receipt" }`. Vaatii
   `OpError`-enumin laajennuksen ja kutsujapinnan päivityksen
   `crates/server`-puolella jos siellä erotellaan virheitä. Eniten
   selkeyttä mutta laajin vaikutus.

3. **Strukturoidaan `OpError::Forbidden`-payload kantamaan
   constraint-nimi.** Esim.
   `OpError::ConstraintViolation { name: String }`. Yksi uusi variantti
   tai laajennus olemassaolevaan; debugger näkee constraint-nimen
   sekä lokeissa että `?`-propagaatiossa.

Päätös valitsisi yhden ja tehdään yhdenmukaisesti kaikissa kolmessa
trace-kirjoittajassa (`start_run`, `record_step`, `record_manual_run`).

## Out of scope

- `map_sqlx_error`-laajennus muille `ops::*`-moduuleille (receipts,
  expenses, attachments, ...). Tämä issue koskee vain
  `agent_trace`-pintaa. Jos vastaava laajennus on tarpeen muualla,
  filataan erillisinä.
- Phase 4:n UI-virhepinta — UI näyttää inhimillisen viestin, ei
  constraint-tason vihjeitä.

## Acceptance criteria

- [ ] Suunnittelupäätös tehty (vaihtoehto 1, 2, 3 tai jokin yhdistelmä)
  ja dokumentoitu issuen kommentteihin
- [ ] `agent_trace.rs::map_sqlx_error` toteuttaa päätöksen
- [ ] `start_run`, `record_step`, `record_manual_run` käyttävät samaa
  pintaa
- [ ] Testit tarkistavat että FK-rikkomukset surfaceavat odotetussa
  muodossa (constraint-nimellä lokissa tai typed-virheinä)
- [ ] AGENTS.md päivitetty jos `OpError`-pinta laajenee

## Riippuvuudet

- **Estyy:** ei mikään.
- **Estää:** ei mikään välitön. Tällä hetkellä #82:n
  `record_manual_run` toimii — debug-ergonomia paranee mutta
  toiminnallisuus ei estyne.

## Konteksti

#82:n `/llm-review`:n GPT-5.5-arvio nosti tämän SPIN-OFF-tason
löydöksi (`history/review-agent-trace-schema-cleanup.md` F17,
`history/assess-findings-agent-trace-schema-cleanup.md` F17). #82:n
schema-cleanupin yhteydessä lisättiin `actor_user_id`-FK ja
`linked_thread_message_id`-kenttä, jotka kasvattivat
`map_sqlx_error`:n vastattavaksi tulevien FK-rikkomusten lukumäärää.
Korjauksen scope ulottuu yli #82:n schema-puhdistuksen ja siirrettiin
tähän omaan issueeseen.

**EI epicissä #56.** Ei kuulu "toimivan testattavan perustan"
scopeen — debug-ergonomian parantamista, ei MVP-blokkeeri eikä
Phase 4 -ennakkoehto.
