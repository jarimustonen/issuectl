---
created: 2026-04-27
updated: 2026-04-27
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
epic: 5
related: ["#33", "#31"]
labels: [ai, performance, cost]
---

# 35. Prompt-cache-strategia kerrostettulle system promptille

_Source: #33 LLM review_

## Description

#33 ottaa käyttöön kerrostetun system promptin (Block 1: SOUL+AGENTS+SALIENCE,
Block 2: USER.md, Block 3: session context). Anthropicin prompt cache toimii
prefiksipohjaisesti — `cache_control: ephemeral` -breakpointti cachettaa
kaiken request-prefiksin alusta breakpointtiin asti.

Designissa Block 1 on cachettu, Block 2 ja 3 eivät. Tämä on pragmaattinen
v1-valinta. Mutta:

- Block 2 (`USER.md`) muuttuu vain kun käyttäjäprofiili muuttuu, ei joka
  kutsulla → siihen voisi laittaa toisen breakpointin.
- Tools-array on osa cache-hashia → niiden sijoittelu vaikuttaa.
- Mahdollinen siirtymä Deepseek-malliin (jos se tehdään) muuttaa
  cache-rajat ja hinnoittelumallia.

Tähän palataan kun #33:n toteutus on tuotannossa ja telemetria näyttää
todellista cache-käyttäytymistä.

## Scope

- [ ] Telemetria: `cache_creation_input_tokens`, `cache_read_input_tokens`,
  cache miss/hit-prosentit per malli
- [ ] Tokenize todelliset SOUL/AGENTS/SALIENCE-tekstit; varmista
  Block 1:n koko (#33-design olettaa ~1 000 tokens, todellinen ehkä
  2 500-3 000)
- [ ] Toinen `cache_control`-breakpointti Block 2:lle, jos telemetria
  osoittaa hyödyn
- [ ] Tools-arrayn sijoittelu suhteessa system-blokkeihin
- [ ] Deterministinen `USER.md`-renderointi (kenttäjärjestys, NULL-
  representaatio) jotta cache pysyy stabiilina
- [ ] Kustannusvertailu: Anthropic vs Deepseek samaan workloadiin

## Riippuvuus

Tämä issue käsitellään vasta kun #33 on tuotannossa eikä siitä ole
ratkaisematta toiminnallisia bugeja. Premature optimization is real.
