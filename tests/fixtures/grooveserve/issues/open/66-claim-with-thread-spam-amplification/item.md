---
created: 2026-04-30
updated: 2026-04-30
type: bug
reporter: jari
assignee: jari
status: open
priority: normal
labels: [post-pilot, spam, abuse, threads]
---

# 66. `claim_with_thread` ennen spam-tarkistusta — thread-taulun spam-amplifikaatio

_Source: `crates/server/src/ingest/runner.rs:530-572`_

## Description

`process_message_inner` suorittaa toiminnot tässä järjestyksessä:

1. **Rivi 531** (`assistant@`-tili): `db::claim_with_thread()` — luo `threads`-rivin (tai elvyttää olemassa olevan) ja `email_processing`-rivin
2. **Rivi 546** (muut tilit): `db::try_claim_message()` — luo `email_processing`-rivin
3. **Rivi 571**: `spam::evaluate()` — vasta nyt määritetään spam-tuomio

**Spam-viesti `assistant@grooveserve.com`:lle ehtii luoda `threads`-rivin** ennen kuin se tuomitaan roskaksi. Jos sender on tuntematon, polku `skip_unknown_sender` (`runner.rs:961-988`) myös kirjoittaa terminal `email_processing`-rivin. Tuomion jälkeen viesti siirtyy Junk-kansioon, **mutta DB-rivit jäävät elämään ikuisesti**.

## Hyökkäysvektori

Julkinen `assistant@grooveserve.com`-osoite on tunnettu (DNS MX-record + Cloudflare config + servers/grooveserve-email.md). Roskapostittaja voi:

- Kasvattaa `threads`-taulua spamilla (1 rivi per uniikki spam-thread)
- Kasvattaa `email_processing`-taulua spamilla (1 rivi per spam-viesti)
- Phase 1:n volyymitasolla (alle 100 msg/päivä) ei ongelma
- **Julkisessa tuotannossa** kun assistant@-osoitteet ovat asiakkaiden tiedossa: kasvu nopeasti merkittävää

Stalwart + auth-triage suodattavat valtaosan ennen agenttista looppia, joten käytännön kasvu on hitaampaa kuin pahin teoreettinen tapaus.

## Reproduction

```sql
-- Ennen spam-viestiä:
SELECT count(*) FROM threads;
SELECT count(*) FROM email_processing WHERE recipient = 'assistant';

-- Lähetä spam assistant@:lle (esim. obvious-spam from external domain ilman DKIMia)

-- Stalwart suodattaa vakavimman, mutta jos viesti läpäisee MX:n:
-- → email_processing saa rivin (status='spam')
-- → threads saa rivin (jos viesti tuli assistant-tilille)

-- Jälkeen:
SELECT count(*) FROM threads;       -- +1
SELECT count(*) FROM email_processing WHERE recipient = 'assistant';  -- +1
```

## Suunnitelma (kun aika)

Aito korjaus vaatii valinnan:

(a) **Siirrä spam-check ennen claimia.** Mutta `is_known_sender` lookup on osa `spam::evaluate`:a → vaatii DB:tä joka tapauksessa, joten ei säästä DB-rounditrippiä. Vaatii claim-semantiikan uudelleenajattelun (TOCTOU-takeover).

(b) **Lisää siivoustehtävä** joka poistaa orphan thread-rivit kun assosioitu `email_processing` saa `spam_verdict='spam'`. Jälkikäteinen siivous, ei estä kasvua.

(c) **Älä luo thread-riviä ennen kuin ensimmäinen ei-spam-viesti saapuu.** Vaatii claim-loogikan uudelleenkirjoituksen — claim:ille tulee "deferred thread"-tila.

(d) **Hard-delete spam thread-rivit** kun `spam_verdict='spam'`. Yksinkertaisempi kuin (b), mutta menettää audit-jäljen siitä että spam yritettiin.

## Aikataulu

- **#56:n ulkopuolella.** Ei estä toimivaa pilottia.
- **Pilot-vaiheen jälkeen** kun:
  - Volyymitiedot ovat saatavilla
  - Asiakkaiden assistant@-osoitteet ovat julkisesti tunnettuja
  - DB-koko alkaa näkyä operatiivisena haittana (varmuuskopioiden koko, kyselyjen latenssi)
- Kytkeytyy **#57 D-aalto agent_runs/agent_steps**-trace-skeemaan: D-track tarvitsee päättää lokitetaanko spam-viestit trace-tauluihin. Mahdollinen yhteinen toteutus D-aallon kanssa.

## Notes

Lähde: `/llm-review` claude round 2 finding #22, dispute-D moderator's note. Katso `history/review-A4-phases-1-5.md` Minor Findings -osio.
