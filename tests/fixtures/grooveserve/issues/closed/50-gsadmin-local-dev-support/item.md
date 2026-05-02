---
created: 2026-04-29
updated: 2026-04-29
closed: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
labels: [tooling, gsadmin, local-dev]
related: ["#48", "#49"]
epic: 56
commits:
  - hash: aa0a383
    summary: "feat(gsadmin): #50 local-dev support — generalise GSADMIN_*_DB_URL pattern"
  - hash: 30f4127
    summary: "fix(gsadmin): #6 cmd_status JSON parse no longer corrupted by stderr"
  - hash: 608bb08
    summary: "fix(gsadmin): #1 #3 — split DB-mode and log-mode; helpers refuse silent prod fallback"
  - hash: 5859630
    summary: "fix(gsadmin): #4 #5 #7 — operator footguns in local-dev mode"
  - hash: ab989b7
    summary: "chore(gsadmin): #9 #10 #11 #13 #16 — local-dev cleanups"
  - hash: fa0a5cd
    summary: "fix(gsadmin): #1 — readiness-poll for run_remote(combine_stderr=False)"
  - hash: 026b818
    summary: "test(gsadmin): #2 #M10 — exercise _follow_local_file for real; lock cmd_status help"
  - hash: 35fab54
    summary: "fix(gsadmin): #3 #M7 #M9 — password-reset uses is_local_db_mode; audit local_log direct"
---

# 50. gsadmin: tuki lokaalille dev-palvelimelle

_Source: 2026-04-29 demo, agentic loopin debugaus lokaalisti_

## Description

`gsadmin` on tällä hetkellä kovakoodattu prodille:

- `gsadmin/ssh.py:20` — `SERVER_IP = "204.168.196.71"`
- DB-komennot (`cmd_email`, `cmd_registrations`) avaavat SSH-tunnelin
  prod-Postgresiin
- `cmd_logs` ajaa `journalctl -u grooveserve-email` SSH:n yli
- `cmd_stalwart` tunneloi prod-Stalwart-API:in (lokaalisti on
  GreenMail, ei Stalwart)

Lokaalissa devauksessa joudutaan käyttämään käsin `psql`:ää, `jq`:ta
ja `gs-email-cli`:tä — sama tieto kuin gsadmin antaisi prodissa, mutta
copy-paste-rivinen workflow. Hidastaa kehitystä ja debugausta.

Yksi komento (`cmd_password_reset`) on jo viilattu fiksusti:
`GSADMIN_DIRECT_DB_URL`-env-yliajo ohittaa SSH-tunnelin ja yhdistää
lokaaliin DB:hen. Sama kuvio puuttuu muilta DB-komennoilta.

## Goal

Lokaali kehittäjä (ihminen tai agentti) voi ajaa relevantit
gsadmin-komennot lokaalia stackia vasten:

```bash
GSADMIN_DIRECT_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main \
  GSADMIN_LOG_FILE=/tmp/email-service.log \
  gsadmin email list
```

ilman SSH-yhteyttä prodiin.

## Scope (mitkä komennot tuetaan lokaalisti)

| Komento | Lokaalitukee? | Miten |
|---|---|---|
| `email list/show/search` | ✅ | `GSADMIN_DIRECT_DB_URL` ohittaa tunnelin |
| `email set-status` | ✅ | sama |
| `email rescue` | ✅ | sama |
| `registrations list/show/delete` | ✅ | `GSADMIN_DIRECT_DB_URL` (api-DB:lle eri DSN — voi olla `GSADMIN_API_DB_URL`) |
| `password-reset` | ✅ jo tukee | ei muutoksia |
| `logs` | ✅ | `GSADMIN_LOG_FILE`-yliajo lukee paikallista tiedostoa, ohittaa `journalctl`:n |
| `users list/show/create` | ❌ ei lokaalisti | Stalwart-only, lokaalisti on GreenMail (`gsdev imap`) |
| `stalwart settings/queue/banned` | ❌ ei lokaalisti | sama syy |
| `status` | osittain | DB-osio voi toimia, kontti-/levy-osiot eivät |
| `deploy` | ❌ ei sovellu | lokaali build + restart on käsin / `gsdev` |

## Rajaukset

- **Auth-DB ja email-DB ovat eri DSN:iä** lokaalisti (gsdev luo per-instanssin
  databaset esim. `grooveserve_email_main_main` ja
  `grooveserve_main_main`). Joko erilliset env-vars
  (`GSADMIN_EMAIL_DB_URL`, `GSADMIN_API_DB_URL`) tai yksi yhteinen
  + per-komento DSN-resoluutio.
- **Stalwart-komennot eivät yritä toimia lokaalisti** — selkeä
  virheviesti: "Local dev mode active (GSADMIN_DIRECT_DB_URL set);
  Stalwart commands require production SSH."
- **Audit-logi** kirjoitetaan edelleen `~/.local/state/gsadmin/log.jsonl`:iin,
  mutta merkittäköön `mode: local-dev` jotta prod-auditeista voi
  erottaa.

## Korjaussuunnitelma

- [x] Yleistä `GSADMIN_DIRECT_DB_URL`-mallin kaikkiin
      DB-komentoihin (`cmd_email.py`, `cmd_registrations.py`,
      `cmd_password_reset.py`)
- [x] Lisää `GSADMIN_EMAIL_DB_URL` ja `GSADMIN_API_DB_URL`
      (erilliset koska auth- ja email-DB ovat eri DSN:iä lokaalisti);
      `GSADMIN_DIRECT_DB_URL` jää backwards-compat-fallbackiksi
- [x] Yhteinen `gsadmin/db.py` — `email_db()`, `api_db()`,
      `is_local_dev_mode()`, `require_prod_mode()`
- [x] `GSADMIN_LOG_FILE`-yliajo `cmd_logs.py`:hen — paikallinen
      tiedosto sekä yksikertaisena lukuna että `--follow`-tilassa
- [x] Stalwart/users/deploy/status-komennot: typer-callback
      `require_prod_mode()` antaa selkeän virheen
- [x] Audit-logiin `mode: local-dev` / `mode: prod` -kenttä
- [x] Päivitä `tools/admin/AGENTS.md` lokaalimoodi-ohjeella
- [ ] (Jatkokehitys, oma issue myöhemmin) `gsadmin email trace`
      jo lisätty trace-toiminto kattaa pohjan; lisätoiveet erikseen

## Trade-off

- **+** AI-agentin debug-workflow paranee merkittävästi: yksi
  komento, ei copy-pasteja
- **+** `gsadmin email rescue` voidaan testata lokaalisti ennen
  prodiin viemistä
- **+** Sama komento prodissa ja lokaalisti → vähemmän eroavuuksia
  käyttäjän muistissa
- **−** Konfiguraation pinta-ala kasvaa — kahdesta env-varista
  tulee 4–5
- **−** Vähän koodimassan tuplaantumista jos `_db()`-helperit
  toteutetaan suoraviivaisesti. Yhteisellä helperillä vältettävissä.

## Quick Test (kun valmis)

```bash
# direnv .envrc tai shell-export
export GSADMIN_DIRECT_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_email_main_main
export GSADMIN_API_DB_URL=postgresql://grooveserve:devpassword@127.0.0.1:55432/grooveserve_main_main
export GSADMIN_LOG_FILE=/tmp/email-service.log

cd tools/admin
uv run gsadmin email list
uv run gsadmin email show <id>
uv run gsadmin logs --trace-id <uuid>
uv run gsadmin registrations list
```

Odotettu: ei SSH-yhteyttä, lokaali Postgres + lokitiedosto vastaa.
