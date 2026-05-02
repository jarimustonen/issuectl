---
created: 2026-05-01
updated: 2026-05-01
type: feature
reporter: jari
assignee: jari
status: open
priority: high
related: ["#13", "#99", "#108"]
labels: [infra, email, monitoring, production]
---

# 107. Tuotantokelpoinen sähköpostimonitorointi

_Source: #13 follow-up — `/llm-review` consensus_

## Description

`#13` toi end-to-end-monitoriskriptin (Ansible-rooli `healthcheck-monitor`,
cron `*/5`), joka todentaa probella koko ketjun
`mail.maalla.dev → grooveserve → mailgun → mail.maalla.dev`. Skripti
toimii happy-pathilla, mutta `/llm-review` paljasti kaksi rakenteellista
puutetta jotka estävät pitämästä monitoria oikeasti tuotantokelpoisena:

1. **Self-monitoring dead-man's-switch.** Monitori asentuu
   `grooveserve-server`-koneelle samaan playbookiin (`server.yml`).
   Jos hostti kaatuu (kernel panic, verkkokatko, levy täynnä, cron
   alas), monitori ei aja itseään, eikä hälytys tule. Juuri ne
   katkokset jotka ovat operationaalisesti tärkeimpiä, jäävät
   havaitsematta.
2. **Circular alert path + jaettu credential.** Probe, IMAP-pollaus
   ja hälytyksen lähetys käyttävät kaikki samaa `mail.maalla.dev`-
   palvelinta sekä samaa `jari`-tilin salasanaa. Jos maalla.dev
   kaatuu, sekä probe että hälytys epäonnistuvat — silent failure.
   Lisäksi `jari@maalla.dev` on henkilökohtainen postilaatikko,
   ei probelle dedikoitu — `cleanup_reply`-toiminto on vaarallinen
   (ks. korjattu UID EXPUNGE -bugi #13:ssa) niin kauan kuin se ajaa
   oikeaa käyttäjän postilaatikkoa vasten.

Tuotantokelpoinen ratkaisu yhdistää nämä kaksi: **erillinen monitorointi-
prosessi erillisellä hostilla, dedikoitu probe-postilaatikko, ja
hälytyspolku joka ei riipu monitoroitavasta järjestelmästä eikä
maalla.dev:sta**.

## Scope

- [ ] **Päätös scheduler/host-arkkitehtuurista:**
  - **A**: oma pieni VPS (Hetzner CX11 tms) erillisessä lokaatiossa
  - **B**: SaaS-heartbeat (healthchecks.io / Better Stack / Cronitor) joka
    pingaa monitoria + alertoi
  - **C**: GitHub Actions cron joka ajaa proben ulkopuolelta
  - **D**: hybridi — paikallinen monitori jatkaa probea + healthchecks.io
    dead-man "no successful run in N min" -hälytyksenä
- [ ] **Dedikoitu probe-postilaatikko mail.maalla.dev:lle**
  (esim. `grooveserve-probe@mail.maalla.dev`) — ei jari-henkilökohtaista
  - SOPS:iin uudet salaisuudet:
    - `mail_secrets.healthcheck_probe_smtp_password`
    - `mail_secrets.healthcheck_probe_imap_password`
    - `mail_secrets.healthcheck_alert_token` (webhook tms)
  - host_vars päivitys, `healthcheck_probe_imap_user: "grooveserve-probe"`
- [ ] **Erillinen hälytyskanava** joka EI mene maalla.dev:n kautta:
  - webhook (ntfy.sh, Pushover, Telegram)
  - SaaS-heartbeat-alertointi (kohdan A/B/D mukaisesti)
  - SMS / Slack / Mattermost — riippuu valinnasta
- [ ] **Credentials pois argv-vuotopolulta** — `curl --user`-flagi
  jättää salasanat `/proc/<pid>/cmdline`:n näkyville. Käytä
  `--netrc-file`-konffia (deployataan 0600 root-omistuksella) tai
  vaihda Pythonin `imaplib`/`smtplib`-pohjaiseen toteutukseen
  (käsitelty osana mahdollista #110-Python-rewrite-issuea jos
  filataan).
- [ ] **Documentaatio päivitetty:** `operations/AGENTS.md`:n
  Healthcheck-monitori-osio päivitetään kuvaamaan oikeaa
  riippumattomuus-tilaa (nykyinen sanamuoto on liian optimistinen).

## Implementation hints

- Halvin "kevyt" valinta on todennäköisesti **D**: jätä paikallinen
  monitori (probe + IMAP-pollaus toimii hyvin), mutta lisää
  `curl https://hc-ping.com/<uuid>` jokaisen onnistuneen ajon loppuun.
  healthchecks.io alertoi jos pingiä ei tule N minuutissa — Pushover/
  Telegram-kanavalla joka EI riipu meidän infrasta. Aikataulu: ~2h
  signupit + parametri SOPS:iin + scriptiin pari riviä.
- Valinta **A** vaatii eniten työtä (uusi host, Ansible-inventory,
  oma rooli) mutta antaa oikean riippumattomuuden + omistuksen.
- Probe-postilaatikon provisiointi maalla.dev-puolella vaatii
  yhteistyötä siellä (uusi tili, salasana, SPF/DKIM jos eri domainia).
- Älä bundlaa Python-rewriteä tähän — se on oma issue-arvoinen
  vaihtoehto jos halutaan myös tehdä se (`/llm-review` ehdotti, mutta
  ei välttämätön jos bash-skriptin haavoittuvuudet (EXPUNGE blast
  radius, shell-quote injection, NDR false-positive) on jo korjattu
  #13:n korjauspatchilla).

## Acceptance criteria

1. Hostin kaatuminen (esim. shutdown -h now) → hälytys tulee enintään
   N minuutin kuluessa (N riippuu valitusta heartbeat-cadensista).
2. mail.maalla.dev:n alasajo → hälytys tulee toista kanavaa pitkin
   (ei probekanavan kautta).
3. Probe-credentials kompromisi ei anna pääsyä jariin tai
   mail.maalla.devin muihin käyttäjiin.
4. Operatiivinen runbook: mistä alert tulee, mitä tehdä, miten
   valeahälytys vaiennetaan.

## Related

- `#13` — alkuperäinen monitoriskripti
- `#99` — lokaali smoke-testi -muunnelma
- `#108` — periodinen siivousprosessi (riippuu tästä — dedikoitu
  postilaatikko mahdollistaa bulkkisiivouksen)
- `/llm-review`-raportti `history/review-healthcheck-monitor.md`
  (worktree b1-healthcheck-monitor)
