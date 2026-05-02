---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: open
priority: normal
epic: 56
related: ["#13"]
labels: [dev-env, email, monitoring]
---

# 99. Healthcheck — lokaali smoke-testi -muunnelma

_Source: #13 follow-up_

## Description

`#13` toi tuotannon end-to-end-monitorin (Ansible-rooli
`healthcheck-monitor`, cron `*/5`, hälytys mail.maalla.dev:n kautta).
Sama skripti pitäisi pystyä ajamaan myös lokaalia gsdev-stackia vasten
smoke-testinä, ilman tuotannon SMTP/IMAP-kredentiaaleja tai oikeaa
mailgun-relayta.

`#56`-epicin Phase 0 -lista pyytää `#13`:n lopuksi siirron lokaaliin
smoke-testiin — alkuperäinen `#13` jätti tämän erilliseksi
follow-upiksi koska gsdev:n nykyinen `compose.dev.yml` sisältää vain
PostgreSQL:n. Lokaalia SMTP/IMAP-muonitusta (esim. Mailpit, Maddy tai
lokaali Stalwart) ei ole vielä olemassa.

## Scope

- [ ] Päätös lokaalin SMTP/IMAP-emulaation toteutuksesta:
  - **Mailpit** (in-memory web-UI + IMAP) — kevein
  - **Maddy / Stalwart-lokaali** — lähempänä tuotantoa, raskaampi
  - Muu? Suora emaili-pino dev-CLI:ssä ilman ulkopuolista palvelua?
- [ ] Muutos `operations/dev/compose.dev.yml` ja `tools/dev/gsdev/`-
  puolella: SMTP+IMAP-kontti pystyyn, tarvittavat tilit luotu, env-vars
  paikallaan
- [ ] `gsdev healthcheck` (tai vastaava) -komento joka kutsuu
  `email-healthcheck.sh`-skriptiä lokaali-asetuksilla — joko ajamalla
  Ansible-roolin generoima skripti tai oman thin-wrapperin kautta. Sama
  exit-koodit (0 = OK, 1 = FAIL).
- [ ] Skripti pitää sopeutua niin että `--ssl-reqd` ei pakota lokaalissa
  (port 1025 / plain SMTP), tai vaihtoehtoisesti lokaali pino tukee
  STARTTLS:ää itse-allekirjoitetulla sertillä.
- [ ] Smoke-testi on osa `gsdev`:n perusajoa (esim. `gsdev doctor`
  tai uusi `gsdev healthcheck`) — ei vaadi cronia, ajetaan kerran
  pyynnöstä.

## Implementation hints

- `operations/infra/ansible/roles/healthcheck-monitor/templates/email-healthcheck.sh.j2`
  on jo parametrisoitu (SMTP/IMAP host, user, salasana host_vars-
  muuttujista). Lokaalin variantin pitäisi pystyä uudelleenkäyttämään
  sama skriptipohja.
- Auto-reply-puoli toimii kun `grooveserve-server` ajetaan lokaalisti
  ja kuuntelee `healthcheck@<lokaali-domain>`-tiliä — `imap_listener_accounts`-
  konfiguraatio on jo olemassa.
- Vältä erottamasta scriptin ja gsdev-komennon "totuutta" — sama
  toiminta-spec ja sama exit-koodi-konventio molemmissa.

## Notes

`#13`:n tuotantoasennus paljasti kaksi bugia jotka jo on korjattu:
1. Stalwart IMAP `SEARCH SUBJECT` on token-pohjainen — koko
   "Re: <subject>"-fraasi ei matchaa, mutta yksittäinen tunniste
   (PROBE_ID) matchaa.
2. IMAP-vastausten UID-rivit päättyvät CRLF:ään — `awk` jätti
   `\r`:n UIDiin, mikä rikkoi myöhemmän curl `--request`-merkkijonon
   (curl error 3, "URL using bad/illegal format").

Nämä korjaukset ovat osa skripti-templatea ja siirtyvät automaattisesti
lokaali-versioon.
