---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#33"]
labels: [agent, tools, sql]
---

# 37. Update-työkalut eivät voi tyhjentää valinnaisia kenttiä

_Source: `services/email/src/tools/{receipts,expenses}/update_*.rs`, `services/email/src/tools/user/update_user_preferences.rs`_

## Description

Agentin `update_receipt`, `update_expense` ja `update_user_preferences` -työkalut eivät tarjoa polkua valinnaisen kentän _tyhjentämiseen_. Käyttäjä ei voi pyytää esimerkiksi "poista päivämäärä kuitilta 42" tai "poista oletuskulkuneuvo profiilistani" — agentilla ei ole työkalua jolla muuttaa olemassa olevaa arvoa SQL `NULL`:ksi.

Syy: SQL-kuvio on per-kenttä `field = COALESCE($N, field)`. `NULL`-bind tarkoittaa "ei muutosta". Vahti `has_update` hylkää kutsut joissa kaikki kentät ovat `null` (#33 review:n M1, jossa rajoitus tunnistettiin) — eli pelkkä `{"home_address": null}` palauttaa "At least one field to update is required". Ja vaikka pääsisi vahdin ohi, COALESCE ei tee sitä mitä mallin kannalta pitäisi.

## Reproduction

```text
Käyttäjä: "Voitko poistaa työosoitteen profiilistani?"
Agentti: yrittää update_user_preferences { "work_address": null, "home_address": "<säilytetään>" }
→ COALESCE($3, work_address) = vanha arvo (NULL bindissä = ei muutosta)
→ tool_result palauttaa updated: true vaikka mitään ei tapahtunut
```

Sama `update_receipt`:lle ja `update_expense`:lle valinnaisten kenttien (`receipt_date`, `payment_method`, `vat_rate`, `vat_amount`, jne.) osalta.

## Background — miksi tämä spinoff #33:stä

#33-review (history/review-tool-skills-step3b-step4-impl.md) tunnisti tämän rajoituksen "Cannot unset fields"-löydöksenä (M1). Päätöstaulukossa se merkattiin **DISCUSS**-statuksella — _confirmed, OCCASIONAL, WORSENS readability if fixed inline (dynaaminen SQL on isompi muutos), MAJOR architecture_. Ei MVP-blocker, mutta oikea rajoitus joka pitää korjata kun tuote alkaa tarvita kenttien tyhjentämistä.

Spin-off:n syy: korjaaminen vaatii dynaamista SQL-rakentamista (per-kenttä `SET`-fragmentti, joka rakennetaan vain kun kenttä on _eksplisiittisesti_ läsnä JSON-syötteessä), ja sitä ei haluta niputtaa #33:n review-fixien kanssa.

## Approach (luonnos)

Kolme suuntaa, kasvavassa kompleksisuudessa:

1. **Erillinen `clear_*`-työkalu** — esim. `clear_receipt_field { receipt_id, field }`. Yksinkertainen, mutta lisää työkalujen määrää ja vaatii skill-dokumentaation.
2. **Sentinel-arvo nykyisissä työkaluissa** — esim. `{"receipt_date": "__NULL__"}` tarkoittaa "tyhjennä". Hauras, vaatii skill-tason kommunikaation; mallin pitää muistaa erityisarvo.
3. **Dynaaminen SQL** — rakennetaan `UPDATE` vain niistä kentistä jotka esiintyvät JSON-syötteessä eksplisiittisesti, ja jossa `null` tarkoittaa "aseta NULL:ksi", puuttuminen tarkoittaa "ei muutosta". Tämä on bookkeeping-/REST-API:n tavanomainen tapa. Kallein, mutta semantiikaltaan oikea.

Suositus: **vaihtoehto 3**. JSON-tason `{"key": null}` on luonnollinen tapa ilmaista "tyhjennä", ja kaikki kolme update-työkalua hyötyy samasta SQL-rakentaja-helperistä.

## Implementation notes

- Helper voisi olla `tools/util.rs::DynamicUpdate` joka kerää `(column, value)` -pareja ja kääntää ne yhdeksi `UPDATE ... SET col1 = $1, col2 = $2 WHERE ...` -lauseeksi.
- JSON-syötteen tarkistus: `input.contains_key(field)` erottaa "ei lähetetty" / "lähetetty null" tilat. `serde_json::Value::Object`:lle tämä on suora `obj.contains_key(field)`.
- `update_user_preferences`:n `preferences`-kenttä on JSONB-merge — sitä ei muuteta, koska sillä on jo eri semantiikka (merge vs replace). Vain ylätason kentät (home_address jne.) muuttuvat.
- Säilytetään olemassa oleva COALESCE-mainen "no-op" -käytös: jos kenttää _ei_ lähetetä, se ei muutu. Vain JSON-eksplisiittinen `null` tyhjentää.

## Out of scope

- Sensitiiviset kentät (banking) — ne eivät ole näkyvissä update-työkalujen kautta lainkaan.
- `receipts.message_id`, `expenses.message_id`, `*.tenant_id`, `*.user_id` — ovat invariantteja eikä niitä saa muuttaa.

## See

- `history/review-tool-skills-step3b-step4-impl.md` §M1 — alkuperäinen löydös
- `issues/open/33-tool-skills/item.md` — emo-tiketti
- `services/email/src/tools/util.rs::optional_non_empty_str` — tämänhetkinen "tyhjä-string-käsittely" -helper joka jatkaa toimintaansa rinnakkain
