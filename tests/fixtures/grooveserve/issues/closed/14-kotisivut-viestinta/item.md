---
created: 2026-04-26
updated: 2026-04-26
type: improvement
reporter: jari
assignee: jari
status: done
priority: normal
related: ["#3"]
labels: [website, messaging]
closed: 2026-04-26
---

# 14. Kotisivun viestinnän päivitys

_Source: sites/www/_

## Description

Päivitä grooveserve.com-kotisivun sisältö niin, että se korostaa palvelua ja yritystä, ei teknologiaa. Poista kaikki viittaukset AI:hin, tekoälyyn ja automaatioon. Korvaa teknologiakuvaukset palvelukeskeisellä viestinnällä ("we handle", "we do the work", "Grooveserve").

## Changes

- Hero: "AI-powered expense management" → "Expense management as a service", "Our AI handles" → "We handle"
- HowItWorks: "AI does the work" → "We do the work", "replaced all of that with AI" → "We take care of all of that for you"
- Features: "Receipt OCR" → "Receipt processing", "AI reads receipts" → "We read receipts"
- Pricing: "AI receipt processing" → "Receipt processing"
- Privacy policy: "AI processing" section → "How we process your data", removed all AI/artificial intelligence mentions
- Terms of service: "AI-powered" → removed, "AI-generated content" → "Generated reports", "AI-generated" → removed throughout
- Verify page: "our AI assistant" → "the Grooveserve team"
- E2E tests updated to match new headings
