---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: feature
reporter: jari
assignee: jari
status: done
priority: normal
labels: [dev-env, gsdev, webmail]
related: ["#11"]
---

# 39. Roundcube webmail to local dev stack

_Source: local demo of email-agent round-trip_

## Description

Lokaalissa kehitysympäristössä ei ole tällä hetkellä yhden selain-UI:n
edestakaista sähköpostipiiriä agentin kanssa. Mailpit näyttää vain
ulospäin lähtevät viestit ja GreenMail tarjoaa pelkän IMAP/SMTP:n ilman
käyttöliittymää. Sähköpostin kirjoittaminen agentille edellyttää joko
työpöydän mail-clientin asennusta tai `gsdev mail send-via-imap` -CLI:n
käyttöä.

Tuotannossa webmail on Roundcube (CLAUDE.md). Lisätään Roundcube myös
local-dev-stackiin per-worktree-kontiksi GreenMailia vasten, jotta:

1. Demoaminen tapahtuu yhden selain-UI:n kautta (kirjoita → vastaus
   samaan inboxiin).
2. Local-dev-ympäristö vastaa tuotannon webmail-pinoa.
3. Worktree-isolaatio säilyy: jokaisella worktreellä oma Roundcube-port
   ja oma GreenMail-tilien joukko.

## Scope

- [ ] `roundcube/roundcubemail` -kontti `operations/dev/compose.dev.yml`:n
      ulkopuolelle (per-instanssi kuten Mailpit/GreenMail), osoittaa
      per-instanssin GreenMailiin (IMAPS + SMTP, plain auth,
      TLS-insecure dev-self-signed sertifikaatille).
- [ ] Per-instance webportti (uusi base, kävelee ylöspäin per worktree).
- [ ] gsdev-laajennus:
      - `gsdev/containers.py`: Roundcube up/down rinnakkain Mailpitin
        kanssa.
      - `gsdev/instance.py`: Roundcube käynnistys osana
        `instance ensure` -flow:ta (opt-in vai default? — päätetään
        toteutuksessa).
      - `gsdev/cli.py` + uusi `gsdev/roundcube.py`:
        `gsdev roundcube up/down/open` (open = avaa selaimessa).
      - `gsdev status`: roundcube_running -tila.
      - `gsdev/ports.py`: Roundcube-portin allokointi.
- [ ] GreenMail-käyttäjäkanta: vähintään yksi end-user-tili (esim.
      `jari@grooveserve.local`) jolta voi lähettää viestin
      `assistant@grooveserve.local`-osoitteeseen. Provisioidaan
      `gsdev imap up`:n yhteydessä.
- [ ] `AGENTS-LOCAL-DEV.md` ja `tools/dev/AGENTS.md` päivitetään
      käyttöohjeineen.
- [ ] `operations/dev/README.md` -quickstart päivitetään.

## Quick Test

Lokaalin worktreen sisällä:

```bash
gsdev imap up
gsdev roundcube up
gsdev roundcube open      # avaa selaimen
# Kirjaudu jari@grooveserve.local / devpassword
# Kirjoita uusi viesti: assistant@grooveserve.local, body="Tässä lounaskuitti"
# Lähetä → odota n. 10 sek
# Refreshaa Inbox → agentin vastaus näkyy
```

Toiseksi: kahdessa rinnakkaisessa worktreessä eri portit, eri
GreenMail-instanssit, ei interferenssiä toistensa kanssa.

## Out of scope

- Tuotannon Roundcube-deploymentin muutokset.
- Issue #11 (loppukäyttäjälle näkyvä tositteiden web-UI) — eri pino,
  eri auth.
- Mahdollinen `gsdev roundcube relay-from-mailpit` -ominaisuus jolla
  Mailpitin captured viestit voisi releaseen GreenMailiin (jos myöhemmin
  hyödyllinen).
