---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#13", "#107"]
labels: [infra, email, monitoring, housekeeping]
---

# 108. Tuotantokelpoinen siivousprosessi monitorointiviesteille

_Source: #13 follow-up — `/llm-review` consensus_

## Description

Healthcheck-monitorin probe-vastaukset (Re: healthcheck-probe …) voivat
jäädä postilaatikkoon kahdella tavalla:

1. Vastaus saapuu **timeoutin jälkeen** (greylisting, Mailgun-jonotus,
   spam-skannaus). Skripti on jo aikakatkaistu ja generoinut uuden
   `PROBE_ID`:n; vanhaa vastausta ei enää matchata, eikä siten siivota.
2. `cleanup_reply`-vaihe **epäonnistuu** transienttisesti
   (network blip, IMAP NO/BAD). Viesti merkataan `\Deleted` mutta
   ei expungetä — tai jää kokonaan siivomatta.

`/llm-review` ja oma manuaalitestaus vahvistivat että näitä jää
inboxiin (esim. UID 1299, 1300 jäivät review-testin aikana). Cronin
ajaessa `*/5` (tai `*/10`) ja inboxin kasvaessa pitkään, mailbox
saattaa hidastua, ylittää kiintiön, tai aiheuttaa SEARCHin
hidastumista.

Tarvitaan **erillinen periodinen siivousprosessi** joka poistaa
vanhentuneet probe-vastaukset turvallisesti.

## Scope

- [ ] **Implementaatio**: erillinen Ansible-task joka asentaa
  `email-healthcheck-sweep.sh` + cron `0 4 * * *` (kerran päivässä
  yöllä). Skripti:
  - Yhdistää IMAP:iin probe-tilillä (#107:n dedikoitu tili).
  - SEARCHaa `FROM "healthcheck@grooveserve.com"
    SUBJECT "healthcheck-probe" BEFORE <yesterday>` — vain vanhentuneet
    monitorointi-vastaukset.
  - Validoi UID-listan numeerisuuden.
  - Käyttää `UID EXPUNGE <uid-list>` (UIDPLUS — Stalwart tukee, ks.
    `IMAP4rev2 ENABLE … UIDPLUS …` capability).
  - Logittaa siivotut UIDt + määrät.
  - Hälyttää jos inboxissa on >N (esim. 100) probe-viestiä → joko
    monitor on rikki tai siivous ei toimi.
- [ ] **Riippuvuus #107:stä**: tämä on turvallinen ajaa **vain
  dedikoitua probe-postilaatikkoa vasten**. Henkilökohtaista
  `jari@maalla.dev`-tiliä vasten ajaminen riskeeraa väärän mailin
  poiston vaikka SEARCH on rajattu. Älä toteuta ennen kuin #107
  on tuonut dedikoidun tilin.
- [ ] **Operatiivinen vahvistus**: ensimmäinen siivous-ajo manuaalisesti
  `--dry-run`-flagilla ja loki tarkistettu ennen cronin aktivointia.

## Implementation hints

- Pidä erillisenä skriptinä (`email-healthcheck-sweep.sh`), älä
  bundlaa probe-skriptiin — eri vastuu, eri ajoaika, eri failure
  domain.
- IMAP-protokollatason logiikka pysyy curl+awk-yhdistelmänä jos
  probe-skripti pysyy bashissa. Jos probe-skripti rewrittetään
  Pythoniin (mahdollinen erillinen scope), siivousskripti olisi
  silloin samalla kielellä — luonteva paikka jakaa IMAP-helpper-
  funktiota.
- Hälytys-osa (`>N viestiä = anomaly`) voi käyttää samaa
  hälytyskanavaa kuin #107:n päämonitori.

## Acceptance criteria

1. Cron ajaa skriptiä päivittäin, logiin tulee
   `SWEEP — N messages purged` -rivi.
2. Inboxiin ei kasaudu yli (esim.) 50 probe-vastausta missään
   tilanteessa.
3. Skripti EI poista mitään muuta kuin
   `FROM healthcheck@grooveserve.com SUBJECT healthcheck-probe`
   -kriteeriin täsmäävää.
4. Manuaalinen `--dry-run`-tila listaa siivottavat UIDt ilman
   poistoa.

## Related

- `#13` — alkuperäinen monitoriskripti
- `#107` — tuotantokelpoinen monitorointi (blocker — dedikoitu
  postilaatikko vaaditaan ennen tätä)
- `/llm-review`-raportti `history/review-healthcheck-monitor.md`
