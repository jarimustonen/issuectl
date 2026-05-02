---
created: 2026-04-25
updated: 2026-05-01
closed: 2026-05-01
type: task
reporter: jari
assignee: jari
status: done
priority: normal
epic: 56
related: ["#1", "#99", "#107", "#108"]
labels: [infra, email, monitoring]
commits:
  - hash: 3e72828
    summary: "feat: end-to-end email healthcheck monitor (issue #13)"
  - hash: c45f48e
    summary: "chore(ansible): disable automatic healthcheck cron"
  - hash: af4a833
    summary: "fix(healthcheck-monitor): IMAP search + UID parsing, re-enable cron"
  - hash: 76aa6bb
    summary: "fix(healthcheck-monitor): apply /llm-review consensus fixes"
---

# 13. Healthcheck-monitori sähköpostipalvelulle

_Source: sähköpostipalvelu (issue #1)_

## Description

Toteuta monitori joka tarkistaa sähköpostipalvelun toimivuuden lähettämällä testin `healthcheck@grooveserve.com`:iin säännöllisin väliajoin ja varmistamalla vastauksen saapumisen.

## Scope

- [x] Cron/timer joka lähettää testiviestin healthcheck@:iin 5 minuutin välein
- [x] Vastauksen tarkistus (IMAP tai JMAP)
- [x] Hälytys jos vastaus ei saavu aikarajassa
- [x] Hälytyskanava (sähköposti / Slack / muu)

## Implementation

Shell-skripti (`email-healthcheck.sh`) joka testataa koko sähköpostiketjun end-to-end:

```
probe (mail.maalla.dev SMTP) → internet → MX (mail.grooveserve.com)
→ Stalwart → email-service auto-reply → Mailgun → mail.maalla.dev → IMAP
```

**Tiedostot:**
- `operations/infra/ansible/roles/healthcheck-monitor/` — oma Ansible-rooli
- `operations/infra/ansible/roles/healthcheck-monitor/templates/email-healthcheck.sh.j2` — monitoriskripti
- `operations/infra/ansible/roles/healthcheck-monitor/tasks/main.yml` — deployment (cron */5)
- `operations/infra/ansible/host_vars/grooveserve-email.yml` — konfiguraatio
- `operations/secrets/stalwart.enc.yaml` — probe-tilin salasana

**Toiminta:**
- Lähettää probe-viestin `jari.mustonen@iki.fi` → `healthcheck@grooveserve.com` ulkoisen reitin kautta
- Pollaa IMAP:ia 10s välein, max 120s timeout
- Onnistuminen: logittaa OK, siivoaa vastausviestin inboxista
- Epäonnistuminen: logittaa FAIL, lähettää hälytyssähköpostin mail.maalla.dev:n kautta (eri palvelin kuin monitoroitava)
- Hälytys rate-limitattu: max 1 alertti/tunti (state file)
- Logi: `/var/log/email-healthcheck.log`

**Deployment:**
```bash
cd operations/infra/ansible
ansible-playbook -i inventory.yml email.yml
```

## Notes

`healthcheck@grooveserve.com` ping-pong auto-reply on jo toiminnassa (issue #1). Tarvitaan vain monitorointiskripti joka lähettää ja tarkistaa vastauksen.

## Resolution (2026-05-01)

Ansible-rooli `healthcheck-monitor` on tuotannossa, cron `*/5` aktivoitu
takaisin (toggle `healthcheck_cron_enabled`, default true), ja
`operations/AGENTS.md` dokumentoitu.

**Manuaalisessa testissä paljastuneet bugit korjattu:**

1. `UID SEARCH SUBJECT "Re: <subject>"` ei matchannut Stalwart IMAP:in
   token-pohjaisessa indeksissä — fraasi-haku palautti aina nollan
   tuloksen vaikka vastaus oli inboxissa. Korjaus: hae vain
   uniikilla `PROBE_ID`-tunnisteella, joka on yksi token.
2. IMAP-vastauksen UID-rivit päättyvät CRLF:ään, mikä jätti `\r`:n
   `REPLY_UID`:in — myöhempi curl `--request "UID STORE 1300\r..."`
   epäonnistui virheellä `curl: (3) URL using bad/illegal format`.
   Korjaus: `tr -d '\r'` UID-eristyksessä.

Lokaali smoke-testi -muunnelma siirretty erilliseksi follow-upiksi
`#99` (vaatii Mailpit/Stalwart-lokaalin gsdev-stackiin).

**`/llm-review` round 2 (4 LLM:ää, 2 kierrosta):** löysi 28
huomiota, joista 11 korjattiin tässä työpussa (commit `76aa6bb`):
- `cleanup_reply` käyttää `UID EXPUNGE`:a (RFC 4315) — naiivi
  `EXPUNGE` olisi hävittänyt jari@maalla.dev:n kaikki
  `\Deleted`-flagatut viestit cron-ajossa.
- IMAP-haku `FROM healthcheck@<domain> AND SUBJECT <PROBE_ID>` +
  vastauksen verifiointi `In-Reply-To`:lla — NDR/bounce-viestit
  eivät enää valeennusta vihreäksi.
- Kaikki templatevit `| quote`-filtterillä — RCE-as-root rotaation
  yhteydessä jos uudessa salasanassa olisi `$(…)`.
- Credentials siirretty `0600 netrc`-tiedostoon — pois
  `/proc/<pid>/cmdline`:sta.
- Hälytysluokittelu (`PROBE_SEND` / `IMAP_POLL` / `NO_REPLY`),
  `CONSEC_FAIL >= 2` -kynnys, `alert_active`-tila + recovery
  ohittaa cooldownin.
- Cron `*/10`, `TIMEOUT=300`, ja log redirect `/var/log/email-healthcheck.cron.log`:iin.

Kolme rakenteellista löydöstä spawnattu erillisinä:
- `#107` — production-grade monitoring (oma host, dedikoitu
  postilaatikko, riippumaton hälytyskanava).
- `#108` — periodinen siivousprosessi orpoille probe-vastauksille.

Koko review-raportti: `history/review-healthcheck-monitor.md`
(worktree b1-healthcheck-monitor, gitignored).
