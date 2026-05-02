---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#6", "#26"]
labels: [security, agent, policy]
---

# 89. Agentin kirjoitusoikeudet käyttäjädataan — policy + vahvistuspolku

_Source: spin-off #6:n onboarding-toteutuksesta_

## Tausta

Pilotti-vaiheessa (2026-05-01) **kaikki** `users` + `user_profiles`
-kentät ovat agentin muokattavissa — onboarding-data,
preferences, notes_md, default_transport. Tämä on tarkoituksellinen
MVP-shortcut: agentti voi tarkentaa käyttäjän tietoja kun se oppii
niitä sähköpostien kautta (esim. "muista että syntymäpäiväni on
1985-06-15") ilman erillistä admin-pintaa.

Pilotin jälkeen tämä avoimuus on liian löyhä:
- IBAN ja vastaavat finanssitiedot eivät tule kuulua agentin pintaan
  (kun ne tulevat datamalliin) — virhe / prompt-injection voi
  korruptoida laskutuksen.
- Henkilötunnus / muut viranomais-IDt vaativat sopimuksen mukaan
  user-vahvistuksen ennen muutosta.
- Pankkitilin / työnantajan vaihto on identity-shifti, joka
  ansaitsee out-of-band-vahvistuksen.

## Scope

Tee **suunnitelma + kategorisaatio**, ei välttämättä koodia:

1. **Inventoi nykyiset kentät**:
   `users` (name, locale, password_hash, …),
   `user_profiles` (home_address, work_address, home_lat/lng,
   default_transport, default_vehicle, preferences, language,
   notes_md, date_of_birth, phone_number, employer_name).
2. **Luokittele jokainen kenttä kolmeen koriin**:
   - **A. Vapaa agent-write** — agentti kirjoittaa ilman vahvistusta
     (esim. notes_md, preferences, default_transport).
   - **B. Agent-write + user-confirm** — agent ehdottaa muutosta,
     käyttäjä vahvistaa sähköpostista ("klikkaa hyväksy"-linkkiä).
     Esim. home_address, employer_name.
   - **C. Pelkkä user-write** — agent ei voi kirjoittaa, vain
     käyttäjä /onboarding-tyyppisen lomakkeen kautta. Esim.
     date_of_birth, IBAN (kun tulee).
3. **Vahvistuspolku-design** kategorialle B:
   - Token-pohjainen confirm-link (`auth_tokens.purpose='confirm_change'`?)
   - TTL (esim. 24 h)
   - Diff-näyttö ("Agent ehdottaa: home_address muuttuu X → Y")
   - Reject-polku
4. **Toteutus-skissi**: missä gate elää? Ehdotus: `OpContext`-extentio
   (`required_confirmation: bool`) jonka agent-tool-pinta tarkistaa,
   tai erillinen `pending_profile_changes`-taulu johon agentin
   ehdotukset ladataan ja confirm-route promotoi ne `user_profiles`:iin.
5. **Audit-trail** kaikille kategoria-B / kategoria-C -kirjoituksille
   (jo nyt `user_profile_revisions` kantaa source_tool, joten
   pohja on olemassa).

## Pre-vaatimukset

- #6 onboarding (suljettu — luo identity-pohjan)
- Pilotti käynnissä — todelliset väärinkäyttö-skenaariot näkyvät
  (esim. agentti unohtaa vanhan osoitteen, kirjoittaa väärän
  työnantajan)

## Pois scopesta

- IBAN / pankkitilien hallinta (Procountor / Netvisor hoitaa, ei
  meidän DB:ssämme)
- Tenant-tason kentät (admin-CRUD jo olemassa, eri scope)
- LLM:n itse-rajoitus prompt-tasolla — tämä issue koskee
  schema-/op-tason gateä, ei prompt-engineeringiä

## Miksi ei nyt

- Pilotti-volyymillä todelliset abuse-cases eivät ole vielä
  näkyvissä; suunnitelma riskeeraa optimoida väärää tasoa
- Agent-tool-pinnan ja vahvistuspolku tarvitsevat asiakaspalautetta
  ennen lukitsemista
