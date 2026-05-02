---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: task
reporter: jari
assignee: jari
status: done
priority: normal
labels: [handover, local-dev, demo]
related: ["#11", "#26", "#39", "#40", "#41", "#42", "#43", "#44", "#45", "#46", "#47", "#49"]
---

# 48. Handover: local-dev round-trip status (2026-04-29 session)

_Source: long Claude session, kontekstia täyttymässä, jatkettavissa puhtaalta pöydältä_

## Tarkoitus

Tämä issue on **session-state-snapshot** — ei pysyvä tehtävä. Kuvaa
mitä tehtiin 2026-04-29 aikana, mikä toimii nyt, ja mitkä yksityiskohdat
ovat tärkeitä seuraavalle istunnolle (joko jatkavalle ihmiselle tai
agentille). Sulje kun handover on tehty tai tieto vanhentunut.

## Mitä tehtiin tämän session aikana

### Issuet avattu

| # | Lyhyt | Tila |
|---|---|---|
| #39 | Roundcube webmail to local dev stack | **closed** (mergetty) |
| #40 | Käyttäjäsovellus erilliseen origin (`app.`) | open (suunnittelu) |
| #41 | Hyväksymishierarkia: admin/approver/user | open (suunnittelu) |
| #42 | Monikielisyys fi/sv/en, en oletus | open, **worktree pyörii** |
| #43 | Email-service ei auto-create tuntemattomista | **vaihe 1 mergetty**, vaiheet 2/3 auki |
| #44 | gsadmin email set-status | **closed** |
| #45 | gsadmin email rescue | **closed** |
| #46 | Extract attachments regardless of verdict | open |
| #47 | Retry path re-fetch attachments | open |

### Mainin commitit (push-tamattomia, n. 20 kpl)

Ks. `git log origin/main..HEAD --oneline`. Ydin:

- `b6668da` env.email.template FROM_NAME quoted
- `9e27eb7` known-sender no-AR spam bypass + round-trip docs
- `1ebdcdd` gsadmin email set-status
- `4b1026b` gsadmin email rescue
- `e01bc10` Mailbox::new (display name with parens)
- `7891b7e` ja seuraavat (#43 vaihe 1 — auto-create poistettu)

### Mitä paikallisesti pyörii (`_main`-instanssi)

| Palvelu | Portti | Tila |
|---|---|---|
| Postgres (gsdev shared) | `55432` | ✅ |
| api (`grooveserve-api`) | `53004` | ✅ nohup-tausta, db tyhjennetty |
| www (Vite/React Router SPA) | `55176` | ✅ |
| email-service | — (vain IMAP/SMTP-yhteydet) | ✅ nohup-tausta, **SMTP_PORT=53028 override** |
| Mailpit (outbound capture) | `58028` | ✅ |
| GreenMail IMAPS / SMTP | `53996` / `53028` | ✅ provisioidut tilit `assistant`, `healthcheck`, `jari` |
| Roundcube webmail UI | `59001` | ✅ |
| Worktree `i18n-locale-fi-sv-en` | api=53005 www=55177 | agent työssä |
| Worktree `ei-auto-create-...` | (poistettu mergen jälkeen?) | tarkista `workmux list` |

### Ympäristön kerrokset

- `tools/dev/gsdev` orkestroi per-instanssin Postgres-DB:t,
  Mailpit-, GreenMail- ja Roundcube-kontit, env-templatet
- `gsdev imap up` / `gsdev roundcube up` ovat opt-in
- Demo-käyttäjä: **jari@grooveserve.local / devpassword** (GreenMailissa
  + email-DB:ssä `users`-rivinä `gs-email-cli setup-tenant`-ajon kautta)

## Mitä toimii (todennettu tämän session aikana)

- ✅ Web-rekisteröinti `localhost:55176` → vahvistus Mailpitiin →
  `/set-password`-linkki → auto-login → /admin (Playwright e2e
  spec vihreä)
- ✅ Roundcube round-trip: viesti `jari@grooveserve.local` →
  `assistant@grooveserve.local` → agentin vastaus jari:n inboxiin
  GreenMailissa
- ✅ `gsadmin email set-status` ja `email rescue` — yksikkötestit ok,
  manuaalinen tuotantoflow valmis (tuotannossa SSH-tunneli, lokaalisti
  käytetty raakaa SQL:ä koska gsadmin yhdistää suoraan prodiin)
- ✅ Spam-bypass: known-sender ohittaa "no AR" -suspiciousin

## Mitä **ei** toimi / on auki

- ❌ **Liitteet rescue-polulla** (#46/#47): kun viesti merkitään
  ensin suspiciousiksi ja sitten rescued, agentti ei näe liitteitä
  koska extraction ei ajettu suspicious-haarassa. Demonstroitu Jarin
  Roundcube-testissä — agentti vastasi "en näe liitteitä" vaikka
  PDF:t olivat IMAPissa.
- ❌ Email-DB ja api-DB **eivät jaa käyttäjiä** (#43 vaihe 2 / #26).
  Jari on rekisteröitynyt apiin (`Itsellesi Oy / jari@itsellesi.fi`)
  mutta email-puolen tunnistautumiseen tarvitaan erillinen
  `gs-email-cli setup-tenant`-ajo (tehty `jari@grooveserve.local`-
  sähköpostille, joka ei ole sama identiteetti).
- ❌ Tuotannon `gsadmin email rescue / set-status` -komennot ovat
  toteutettu mutta **ei vielä testattu tuotantoa vasten** (vaatii SSH-
  yhteyden ja tuotannon Postgresin).

## Pikajatko: local-demo "agentti näkee liitteet"

Yksinkertaisin testi tämän hetken bugille (#46):

```bash
# 1. Flippaa nykyiset assistant-INBOX-viestit Unseen:iksi IMAPissa
python3 -c "
import imaplib, ssl
ctx = ssl._create_unverified_context()
m = imaplib.IMAP4_SSL('127.0.0.1', 53996, ssl_context=ctx)
m.login('assistant', 'devpassword')
m.select('INBOX')
typ, data = m.search(None, 'SEEN')
for n in data[0].split():
    m.store(n, '-FLAGS', '\\\\Seen')
m.logout()
"

# 2. Tyhjennä email_processing rivit
PGPASSWORD=devpassword psql -h 127.0.0.1 -p 55432 -U grooveserve grooveserve_email_main_main \
  -c "DELETE FROM thread_messages; DELETE FROM email_processing; DELETE FROM threads;"

# 3. Restart email-service (tappaa vanhan, käynnistää uudella binaarilla)
ps aux | grep grooveserve-email | grep -v grep | awk '{print $2}' | xargs -r kill
sleep 1
cd services/email
set -a && source .env.local && set +a && \
  SMTP_PORT=53028 nohup ./target/debug/grooveserve-email > /tmp/email-service.log 2>&1 &
disown

# 4. Seuraa
tail -f /tmp/email-service.log | grep -E "extraction|tool_use|Spam|reply"

# 5. Selvitys: gs-email-cli history näyttää tool_use-blokit
cd services/email
DATABASE_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main \
  ./target/debug/gs-email-cli history --email jari@grooveserve.local
```

Odotettu lopputulos: vision-OCR ajaa kaikki PDF:t,
`save_receipt`/`add_expense`-tool-kutsut näkyvät logissa ja
historiassa, agentin vastaus kuvaa mitä tallennettiin.

## Linkit jatkajille

- `services/email/AGENTS.md` — kuinka local-stack ajetaan, round-trip
  ohje
- `tools/admin/AGENTS.md` — gsadmin-komennot
- `AGENTS-LOCAL-DEV.md` — gsdev/workmux-arkkitehtuuri
- `tools/dev/AGENTS.md` — gsdev-komennot
- `issues/open/40-app-erillinen-origin/item.md` — pitkän aikavälin
  arkkitehtuurinen jako api/app/email
- `issues/open/26-multi-tenant-kayttajahallinta/design.md` (jos on) —
  miten api/email DB-mallit yhtyvät

## Verifiointi (2026-04-29 jatkokäsittely)

Demo ajettu uudestaan handover-worktreessä. `_main`-stackin nykyinen
pyörivä email-service-binääri (`/Users/jari/Sources/grooveserve-
monorepo/services/email/target/debug/grooveserve-email`, pid 77089,
käynnistetty 17:22) prosessoi `process_message`-polulla (ei retry):

| Viesti | Liitteet | Ekstraktiot | Receipts | Tila |
|---|---|---|---|---|
| `<ccfd056aabbc…>` | 1 | 1 | 1 | ✅ reply_sent, save_receipt + add_expense |
| `<fb8d90eca243…>` | 21 | 21 | 0 | ⚠️ reply_sent mutta MaxTokens (#49) |
| `<c9c702f74176…>` | 3 | 3 | 3 | ✅ reply_sent, 3 × save_receipt + 3 × add_expense |

**Yhteensä DB:ssä:** 24 attachments, 24 extractions, 4 receipts,
4 expenses. `gs-email-cli history --email jari@grooveserve.local`
näyttää 9 `[TOOL_USE]`-blokkia ja 9 vastaavaa `[TOOL_RESULT OK]`
-blokkia (`save_receipt × 4`, `add_expense × 4`,
`update_user_preferences × 1`).

### Johtopäätös

- ✅ **Process_message-polku toimii**: liitteet ekstraktoidaan,
  agentti näkee ekstraktiot, tool-kutsut menevät läpi, vastaus
  lähetetään.
- ✅ **Spam-bypass + Clean-verdikti** toimivat tunnetulle lähettäjälle.
- ⚠️ **Uusi vika (#49)**: 21-liitteen viestissä agentin ensimmäinen
  iteraatio palasi `stop_reason=MaxTokens` (output 4096 tokenia
  täynnä) ennen yhdenkään tool_use-blokin valmistumista — käyttäjä
  sai vain "Yhteenveto:"-pätkän. Erikseen aukaistu **#49**.
- 🔄 **#46 ja #47** ovat tarpeellisia retry-polulle ja
  spam-haaraan, eivät tähän demoon. Ne työstetään omissa
  worktreeissään (`extract-attachments-regardless-of-verdict`,
  `retry-path-re-fetch-attachments`).

### Worktree-tila

- `i18n-locale-fi-sv-en` — agent vielä `working`, **unmerged
  commits** olemassa → ei mergetty tässä sessiossa.
- `extract-attachments-regardless-of-verdict` (#46) — agent
  käynnistynyt
- `retry-path-re-fetch-attachments` (#47) — agent käynnistynyt
- `handover-…` (tämä) — issue-päivitykset committoidaan, sitten
  mergetään takaisin `main`:iin.

### Pushattavat committit

`git log origin/main..HEAD --oneline` näytti 14 committia (handover-
päivityksen jälkeen 16). Jätetään käyttäjän hyväksyntään ennen
`git push origin main`.
