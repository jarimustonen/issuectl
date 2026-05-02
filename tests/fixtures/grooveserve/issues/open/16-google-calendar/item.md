---
created: 2026-04-26
updated: 2026-04-26
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
labels: [integraatio, kalenteri, google]
---

# 16. Google Calendar -integraatio

_Source: services/email, AI-agentti_

## Description

Google Calendar -integraatio matkatietojen automaattiseen tunnistamiseen. Käyttäjän kalenterista luetaan tapahtumat ja tunnistetaan matkakandidaatit (sijainti, kesto, tyyppi). Kalenteri on **evidenssi ja konteksti** — ei matkalaskun totuuslähde. AI-agentti kysyy käyttäjältä tarkat ajat (Verohallinnon päivärahavaatimus: minuuttitarkkuus).

## Scope

- OAuth 2.0 -flow: PKCE, state-hallinta, `prompt=consent`, `access_type=offline`
- Token-tallennus PostgreSQL:iin (AES-256-GCM envelope encryption, SOPS/age-avain)
- Token-elinkaaritilat (connected/needs_reauth/revoked)
- `Events.list` minimaalisilla kentillä (`fields`-parametri, GDPR datan minimointi)
- SyncToken-pohjainen inkrementaalinen synkronointi (410 Gone, pagination, paramlock)
- PostgreSQL-pohjainen työjono synkronoinnille (FOR UPDATE SKIP LOCKED, jitter, backoff)
- Matkakandidaattien tunnistaminen (heuristiikka-ensin, LLM ambiguiteetissa)
- TripCandidate + Evidence -malli (confidence scoring, auditoitava todistusketju)
- Disconnect/revocation-flow
- `CalendarProvider`-trait (O365-valmius)

## Toteutustapa

Suora HTTP: `reqwest` + `serde` + `oauth2`-crate. Ei google-calendar3:a (unmaintained, RUSTSEC, CLI-oriented) eikä Oxide-kirjastoa (0.x, single-vendor).

## Dependencies

- Google Cloud Console -projekti (OAuth credentials)
- OAuth consent screen -verifiointi (**kriittinen polku**, 2-6 viikkoa, aloitettava heti)
- Privacy policy -päivitys (Google API Limited Use -kielioppi)
- Web-endpoint OAuth-callbackille (redirect_uri) — ei vielä olemassa
- Käyttäjäprofiili: kotikaupunki/toimisto-osoite (onboarding-flow)

## Päätökset (review-tulokset)

Analyysi kävi läpi 4 LLM:n kriittisen reviewn (Gemini, GPT-5.5, Claude Opus, DeepSeek). Konsensuspäätökset:

1. **Kirjasto:** Suora HTTP, ei valmiita kirjastoja
2. **Token-salaus:** Pakollinen (GDPR Art. 32), envelope encryption
3. **Kalenteri = evidenssi:** Ei tuota suoraan matkalaskun aikoja (Verohallinto vaatii minuuttitarkkuuden)
4. **Refresh token:** 7 päivän vanheneminen Testing-tilassa, `prompt=consent` pakollinen
5. **Polling:** PostgreSQL-työjono, ei tokio::interval
6. **SyncToken:** Sidottu kyselyparametreihin, 410 Gone -käsittely, per-kalenteri
7. **Data minimointi:** `fields`-parametri, ei description/attendees oletuksena
8. **PKCE:** Kyllä, vaikka confidential client (~4 riviä koodia)
9. **fromGmail:** Keskivahva signaali EU:ssa (Smart Features usein pois)
10. **Matkatunnistus:** Heuristiikka-ensin + LLM ambiguiteettiin + käyttäjän vahvistus

**Avoin:** Palvelun sijoitus (moduuli services/email:ssä vs. erillinen services/calendar)
