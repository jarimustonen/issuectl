---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
labels: [extraction, cost-control]
related: ["#15"]
---

# 110. Multi-page PDF support — sivumäärä-rajoitus + denial-of-wallet-suoja

_Source: matkalaskupalvelu_

## Description

Vision-OCR-pinta kapseloi liitteiden kokoa kolmella rajalla:

- `MAX_ATTACHMENT_SIZE = 10 MB` per liite
- `MAX_ATTACHMENTS_PER_MESSAGE = 15`
- `MAX_TOTAL_ATTACHMENT_BYTES = 25 MB` per viesti

Mutta PDF:n tapauksessa kustannus on **sivuperusteinen**, ei tavuperusteinen. 10 MB PDF voi sisältää satoja sivuja — Anthropic-vision veloittaa per sivu. 25 MB:n viesti voi olla hyväksyttävä byteiltä mutta laukaista satakertaisen API-kustannuksen.

## Why deferred (PoC-skoopin ulkopuolella)

PoC-käyttäjillä:
- Tyypillinen kuitti-PDF on 2–4 sivua
- Useammat sivut tarkoittavat yleensä lasku + matkalippu erikseen, edelleen ≤10 sivua
- Jos joku lähettää 100-sivuisen PDF:n, näemme sen heti tracingissa ja voimme reagoida käsin

Eli kustannussuoja ei ole MVP-vaiheessa kriittinen. Mutta kun otetaan käyttöön laajemmin, tämä pitää sulkea.

## Scope

- [ ] Lisää PDF page-count -tarkistus `unsupported_reason`-funktioon (tai erilliseen pre-vision-tarkistukseen)
- [ ] Käytä `lopdf` tai vastaava kirjasto sivumäärän lukuun
- [ ] Päätä raja (ehdotus: `MAX_PDF_PAGES = 10` MVP-jälkeiselle hetkelle, voidaan nostaa myöhemmin)
- [ ] Lisää `AttachmentSkipReason::TooManyPages` -varianti permanent-skip-poluille
- [ ] Päivitä `agent_hint_fi` joka kertoo käyttäjälle "lähetä lyhyempi PDF tai jaa useampaan viestiin"
- [ ] Päivitä `extraction_rescue.rs`-testit kattamaan uusi rejected-haara

## Background

Filed C1-worktreessä `/llm-review`-kierroksen löydösten pohjalta:
- GPT-5.5 §9 (PDF cost-abuse controls are incomplete)
