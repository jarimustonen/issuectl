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
related: ["#44"]
commits:
  - hash: 4b1026b
    summary: "feat(gsadmin): email rescue — bulk re-queue suspicious mail from registered users"
---

# 45. gsadmin: bulk-rescue "suspicious / no auth" -viestit tunnetuilta käyttäjiltä

_Source: known-sender-bypass landautui sen jälkeen kun viestejä oli jo käsitelty_

## Description

Spam-triage merkitsee viestin `suspicious`-tilaan kun
`Authentication-Results`-headeria ei ole. Tämä on oikein
tuntemattomille lähettäjille, mutta **rekisteröityneeltä käyttäjältä**
tulleelle viestille se on virhe — known-sender-bypass on tarkoitettu
juuri näiden käsittelyyn (#43:n vaihe 2 / `db::is_known_sender`).

Voi syntyä tilanteita joissa viesti ehti `suspicious`-tilaan ennen kuin
korjaus oli paikallaan (esim. transient-bug, tai uusi feature
landannut myöhässä). Operaattorin pitää voida poimia juuri nämä
"rekisteröityneellä käyttäjällä, mutta no-auth → suspicious"-rivit ja
laittaa ne uudelleenkäsittelyyn yhdellä komennolla — ei käsin
`set-status` riviltä toiselle.

## Scope

- [ ] Uusi komento `gsadmin email rescue`:
  - Etsii rivit joilla
    `status = 'suspicious'` JA
    `spam_reason = 'no authentication results'` JA
    `from_addr` löytyy `users.email`-taulusta (case-insensitive).
  - Asettaa heille `status = 'retryable'`, `retry_count = 0`,
    `next_retry_at = NOW()` jolloin retry-poller poimii.
  - `--dry-run` (default): listaa rivit jotka muuttuisivat,
    EI tee muutoksia.
  - `--apply`: tekee muutoksen. Vaaditaan eksplisiittisesti.
  - `--account <name>`: rajoita yhteen vastaanottajatiliin.
  - `--from <pattern>`: rajoita yhteen lähettäjään (ILIKE-haku).
  - `--json`: rakenteellinen output.
- [ ] Tarkka spam_reason -ehto
      ('no authentication results') varmistaa ettei rescue koske
      muista syistä suspicious-tilassa olevia rivejä (esim.
      DMARC fail, jolloin viesti on luultavasti aito spam).
- [ ] Audit-loki: tulee automaattisesti `cli.py`-wrapperin kautta.
      Lisäksi rescue-komennon output sisältää muuttuneet rivit
      JSONissa, jotta jälkikäteen on selvää mikä tapahtui.

## Quick Test

```bash
# Aluksi näet mitä tapahtuisi
gsadmin email rescue --dry-run

# Halutessasi rajoita
gsadmin email rescue --dry-run --account assistant --from "@itsellesi.fi"

# Toteuta
gsadmin email rescue --apply

# Verifioi: status retryable, retry_count 0, next_retry_at äskettäin
gsadmin email show <id>
```

## Out of scope

- Useamman spam_reason-tyypin rescue (vaatisi case-by-case
  arviointia).
- Cross-DB user-lookup (#26): tämä komento käyttää
  `grooveserve_email`:n omaa `users`-taulua, kuten
  `db::is_known_sender` Rust-puolella.
- IMAP-side reset: jos viesti on jo `Seen`-tilassa Stalwartissa,
  retry-poller ei tarvitse IMAP:ia (käyttää tallennettua
  `body_plain`:ia).
