---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
labels: [gsadmin, ops, email]
commits:
  - hash: 1ebdcdd
    summary: "feat(gsadmin): email set-status — operator escape hatch for stuck rows"
---

# 44. gsadmin: muuta yksittäisen sähköpostin tilaa

_Source: tuotannossa pitää voida palauttaa juuttunut viesti uudelleenkäsittelyyn_

## Description

`gsadmin email`:lla voi tällä hetkellä **listata, näyttää ja hakea**
sähköposti­käsittely­rivejä, mutta ei muokata niitä. Tuotannossa
voidaan päätyä tilanteisiin joissa viesti jäi väärään tilaan jonkin
bugin tai väliaikaisen filtterivirheen vuoksi (esim. spam-verdict
suspicious "no auth results" kun lähettäjä oli tunnistettu, mutta
known-sender-bypass ei vielä ollut paikallaan; tai sähköpostissa oli
liite jota OCR ei ymmärtänyt ja viesti merkittiin failed-tilaan).

Tarvitaan operaattorikomento jolla voi:

1. Asettaa yhden rivin uuteen tilaan (esim. `failed` → `retryable`).
2. Tarvittaessa nostaa `next_retry_at`:n nykyhetkeen, jotta
   email-servicen retry-poller poimii sen seuraavalla IDLE-syklillä.

## Scope

- [ ] Uusi komento `gsadmin email set-status <id> <new-status>`:
  - `<id>`: `email_processing.id` (numeerinen).
  - `<new-status>`: `processing`, `reply_sent`, `skipped`, `delivered`,
    `spam`, `suspicious`, `retryable`, `failed`, `processed`. Validoidaan
    enum-listaksi.
- [ ] Lippu `--retry-now`: jos `--retry-now` ja uusi tila on
      `retryable`, asetetaan myös `next_retry_at = NOW()` ja
      `retry_count = 0`. Muussa tapauksessa näitä ei kosketa.
- [ ] Konfirmaatio interaktiivisessa moodissa: ennen muutosta näytetään
      rivin nykyinen tila ja kysytään `[y/N]`. JSON-modessa ja
      `--yes`-flagilla ohitetaan.
- [ ] Audit-loki: kirjoita `audit.py`:n kautta merkintä jossa näkyy
      käyttäjä (SSH-tunnus), aika, viestin id, vanha → uusi tila,
      `--retry-now` käytössä vai ei.
- [ ] `--json` -tuki kuten muissa komennoissa: tulostaa rivin uudessa
      tilassa.

## Quick Test

```bash
# Etsi viesti
gsadmin email search --from "jari@itsellesi.fi" --json | jq '.[0]'

# Aseta uudelleenyritettäväksi
gsadmin email set-status 42 retryable --retry-now --yes

# Verifioi
gsadmin email show 42
# → status: retryable, next_retry_at: <äsken>

# Email-service poimii seuraavassa IDLEssä
gsadmin logs email --message-id "<...>" --follow
```

## Out of scope

- Bulk-päivitys (use case käsitellään erikseen jos tulee tarve).
- Viestin sisällön muokkaus.
- IMAP-puolen tilan muuttaminen (Seen-flag, kansio) — tämä vaikuttaa
  vain DB-puolen tilakoneeseen. Jos viesti pitää uudelleenfetchata
  IMAPista, se on eri komento (potentiaalisesti tuleva
  `gsadmin email refetch <id>`).

## Notes

Reuse-mahdollisuus: käytä `_connect(local_port)`-helperia
`cmd_email.py`:ssa nykyiseen tapaan. Audit-loki: ks. `audit.py`,
mallia esim. `cmd_registrations.py` `delete`:stä.
