---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
labels: [email, scaling, imap, performance]
related: ["#46"]
---

# 55. Rinnakkainen viestien käsittely — useita työntekijöitä per IMAP-tili

_Source: #46 review (2026-04-29)_

## Description

Yhden IMAP-tilin (`assistant@`) IDLE-loop käsittelee viestit
peräkkäin. Liite-extraction (vision-OCR) blokkaa loopin koko
käsittelyn ajaksi: 22 PDF-liitettä × 3-10 s per kutsu = 1-4 minuuttia
per viesti. Sinä aikana:

- Uudet viestit kyllä kertyvät INBOXiin (Stalwart vastaanottaa), mutta
  mikään ei prosessoi niitä. Käyttäjä odottaa.
- `process_retries` ei pyöri (sama loop ajaa ne myös). Rescue-jonon
  läpimeno hidastuu.

Prototyyppivaiheessa tämä ei haittaa — viestimäärät pieniä, latenssi
ei kriittinen. Mutta kuorman kasvaessa tämä muodostuu pullonkaulaksi.

## Tausta

`#46` laajensi extractionin koskemaan myös Suspicious/MonitorOnly-
viestejä. Aiemmin extraction blokkasi pelkästään Clean-polkua; nyt
myös spam-flagattujen liitteiden käsittely lukitsee loopin.

`run_imap_loop` (`services/email/src/main.rs:135`) ajaa per tili:

```
loop {
    process_retries(...)        // sequential
    fetch_unseen_uids(...)      // sequential
    for uid in uids {
        process_message(...)    // sequential, ~1-4 min worst case
    }
    idle(...)
}
```

## Korjaussuunnitelma (luonnos)

Useita vaihtoehtoja, päätös myöhemmin tarpeen ja kuorman valossa:

- [ ] **A. N rinnakkaista työntekijää per tili.** Spawnataan
      `tokio::task::JoinSet`iin N tehtävää jotka jakavat saman
      `mpsc::Receiver<uid>`:n. IMAP-fetch+IDLE pysyy yhden tehtävän
      ajamassa, raskaat per-viesti-kutsut menevät pooliin.
      Idempotency: `try_claim_message` jo turvaa että viesti kulkee
      vain kerran läpi. Helpoin, ei tietokantamuutoksia.

- [ ] **B. Tausta-extraction-jono.** `process_message_inner` työntää
      extraction-tehtävät erilliseen jonoon (DB-pohjainen?
      `attachments_pending_extraction`-taulu) ja palauttaa heti.
      Toinen worker kuluttaa jonon ja kirjoittaa
      `attachments`+`extractions`-rivit. Agenttiloop kutsutaan vasta
      kun extraction-jono on tyhjä viestin osalta. Vaatii enemmän
      kanavaa+statea, mutta palauttaa IMAP-loopin millisekunneissa.

- [ ] **C. Vision-kutsujen rinnakkaisuus per viesti.** `extract_attachments`
      käyttää `futures::stream::buffer_unordered(N)`:ää sen sijaan että
      ajaisi peräkkäin. 22 PDF:ää × 5 rinnakkain = ~5x nopeampi.
      Halpa muutos mutta ei poista loopin blokkausta —
      monta-attachmenttia-viestiä kärsii silti. Voi yhdistää A:n kanssa.

## Out of scope

- Yleinen async-arkkitehtuuriremontti (tämä on vain IMAP-loopin osalta).
- Multiple-IMAP-account scaling (jokaisella tilillä on jo oma loop).
- Anthropic API rate-limit -hallinta (#14 opex).

## Trade-off

- **+** Kuorman sieto: yhden ison liiteviestin saapuminen ei pysäytä
  muiden käsittelyä. Käyttäjäkokemus paranee.
- **+** Rescue-jonon (#45) latenssi paranee samalla loopilla.
- **−** Rinnakkaisuus tuo TOCTOU-/idempotency-bugeja jos `try_claim`
  ei pidä — pitää testata huolellisesti #1:n (`email_processing`-
  taulun atomic-claim) yhdessä.
- **−** Lisää koodia jota MVP ei vielä tarvitse.

## Notes

Liittyvät:
- **#46** — extraction now happens for non-Clean verdicts; widens the
  blocking surface.
- **#14** — opex-hallinta (rate limiting, token budget) — voi vaikuttaa
  miten worker-poolin koko kannattaa mitoittaa.
- **#45** — `gsadmin email rescue` pyörittää viestit retry-jonon
  kautta; sama IMAP-loop ajaa myös sen.
