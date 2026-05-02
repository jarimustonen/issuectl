---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: wontfix
priority: normal
labels: [email, retries, attachments]
related: ["#43", "#45", "#46"]
---

# 47. Retry path: re-fetch attachments from IMAP when DB cache empty

_Source: Roundcube round-trip jossa rescue ei nähnyt liitteitä_

## Description

`process_retries` luottaa siihen, että `db::load_extraction_summaries`
palauttaa kaikki liitteet jotka tarvitaan agentin kutsuun. Tämä toimii
jos viesti on aikaisemmin prosessoitu Clean-polulla. Ei toimi:

- Jos viesti hylättiin spam-polulla ennen extractiota (ks. #46), TAI
- Jos `extractions`-taulu on jostain syystä tyhjentynyt
  (esim. retention-policy poistanut)

Vaihtoehtoinen lähestyminen: retry-polku **hakee viestin uudestaan
IMAPista** `message_id`:n perusteella ja ajaa extractionin tuoreena.
Tämä on kalliimpi (vaatii IMAP-yhteyden retry-pollerille), mutta on
robusti silloinkin kun DB on epätäydellinen.

## Korjaussuunnitelma

- [ ] `db::load_extraction_summaries` palautusarvo: jos `Vec` on
      tyhjä **ja** `email_processing`-rivissä `attachments_count > 0`
      (tai vastaava signaali), tee re-fetch.
  - Tämä vaatii että `claim`-vaiheessa tallennetaan attachment count
    vähintään, jotta tiedämme tarvitseeko re-fetch:iä.
- [ ] IMAP-yhteyden hallinta retry-polusta: nykyisin vain
      account-loop pitää sessionia. Refactor: jaa session retry-
      pollerin kanssa, tai avaa erillinen session retry-aikoihin.
- [ ] Re-fetch käyttää `IMAP UID SEARCH HEADER Message-ID <id>`
      -lookupia. Jos viesti ei löydy (käyttäjä on poistanut), retry
      epäonnistuu kontrolloidusti — ei kaada looppia.

## Out of scope

- Liitteiden tallennus blob-storageen / lokaaliin tiedostoon (eri
  arkkitehtuurinen päätös).
- IMAP-folderissa olevien viestien siivous.

## Suhde #46:een

- **#46** ("extract aina"): yksinkertainen, ennaltaehkäisevä — ajaa
  vision-OCR:n riippumatta verdiktistä, jolloin DB:n cache on aina
  täydellinen ja retry tarvitsee vain DB:tä.
- **#47** (tämä): puolustava — hyväksyy että cache voi olla
  epätäydellinen ja palauttaa sen IMAPista.

Voidaan tehdä kumpikin, tai vain #46. Jos #46 toteutetaan ja
extraction-tulokset säilytetään pysyvästi, #47:n tarve poistuu.
Suositus: tee **#46 ensin**. #47 vasta jos osoittautuu että cache
voi vanhentua tai katoaa retentionin myötä.

## Päätös 2026-04-29: wontfix

Suljettu wontfix-tilaan toteuttamatta. Perustelu:

- Tarpeeton kun #46 ajetaan extraction kaikille verdikteille — DB:n
  cache on aina täydellinen ja retry tarvitsee vain DB:tä.
- Tällä hetkellä `extractions`-rivit kirjoitetaan kerran eivätkä
  vanhene: ei retention-policya, ei TTL:ää, ei siivousta. Cache ei
  voi tyhjentyä itsestään.
- Toteutus olisi merkittävästi raskaampi kuin #46:n: skeemamuutos
  (`attachments_count`), IMAP-session jakaminen retry-pollerille,
  `UID SEARCH HEADER Message-ID` -haku useasta folderista, kontrolloitu
  failure jos käyttäjä on poistanut viestin IMAPista — sekä testit.
- MVP-vaihe: "toiminnallisuuden oikeellisuus on ainoa ensisijainen
  tavoite". #46 ratkaisee raportoidun bugin yksinkertaisemmin.

Avataan uudestaan jos retention-policy tai cache-decay osoittautuu
todelliseksi tarpeeksi.
