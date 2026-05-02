---
created: 2026-04-28
updated: 2026-04-28
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#33", "#27"]
labels: [agent, tools, architecture]
---

# 34. Selvitä compound (transaktionaaliset) tool-yhdistelmät MVP:n jälkeen

_Source: `services/email/src/tools/`_

## Description

#33 design-katselmoinnissa nousi esiin ehdotus, että jotkin tool-yhdistelmät, joissa on liiketoiminnallinen invariantti (esim. `save_receipt` + `add_expense` joissa kuitti ilman kulua on ei-laskutettava), voitaisiin paketoida yhdeksi transaktionaaliseksi tooliksi (`save_receipt_expense`).

Hyödyt:

- Eliminoi luokan virheitä, joissa LLM unohtaa kutsua jälkimmäistä toolia.
- Tekee DB-mutaatiosta atomisen.
- Yksinkertaistaa agenttilooppia (vähemmän iteraatioita).
- Yhdistyy hyvin agenttisten transaktioiden kanssa (#27).

Ei tehdä #33:ssa, koska:

- MVP:ssä halutaan ensin saada perustyökalut + skill-arkkitehtuuri toimimaan eril­lisinä paloina.
- Compound tool -kuvio kannattaa suunnitella yhdessä #27:n agenttisen looppi­transaktion kanssa.
- Selkeämpää säilyttää matala-tason `save_receipt` ja `add_expense` rinnalla, jolloin compound tool on optimointi eikä ainoa polku.

## Tehtävä

Kun #33 ja #27 ovat valmiit ja perustoiminnallisuus toimii:

1. Listaa tool-yhdistelmät, joissa on liiketoiminnallinen invariantti
   (`save_receipt` + `add_expense`, mahdollisesti `update_receipt` + ko. expense-rivin amount-päivitys, jne.).
2. Suunnittele compound-tool-kuvio: nimi, signature, transaktiokäsittely, virheraportointi.
3. Päätä politiikka: kumpi on agentin oletuspolku — compound vai erilliset?
4. Toteuta valittu kuvio yhdelle yhdistelmälle pilottina (todennäköisesti `save_receipt_expense`).

## Riippuvuudet

- #33 (skill-pohjainen tool-arkkitehtuuri) — antaa `Tool`-traitin, johon compound tool helposti istuu.
- #27 (agenttinen looppi-transaktio) — määrittelee transaktiomallin tool-tasolla.
