# issuectl 0.6.1 — `doctor --fix` ei korjaa raportoimiaan asioita

**Reporter:** Jari Mustonen
**Date:** 2026-06-01
**Version:** `issuectl 0.6.1` (Homebrew, macOS arm64)
**Repo:** `3DBear/3dbear-monorepo` (private)

## Yhteenveto

`issuectl doctor` raportoi kaksi luokkaa korjattavia asioita ja ohjeistaa
"re-run with --fix". `issuectl doctor --fix` exit-koodi on **1**, ja
human-output päättyy riviin

> `Applied. 0 legacy dir(s) migrated, 0 flat-layout dir(s) migrated, 0 markdown file(s) rewritten, 0 \`## Notes\` rename(s), 0 AGENTS.md block(s) regenerated.`

…mutta varsinaisia korjauksia ei tehdä. Tiedostot ovat byte-identtisiä
ennen ja jälkeen ajon, ja seuraava `doctor`-ajo raportoi täsmälleen samat
löydökset. Käyttäjä jää loopiin ilman tapaa puhdistaa raporttia.

## Toistoaskeleet

```
$ issuectl --version
issuectl 0.6.1

$ issuectl doctor 2>&1 | head -12
Files with both `## Notes` and `## Comments` (manual merge required):
  simply-furry-order

Legacy values to coerce via schema aliases (re-run with --fix):
  amazingly-enchanted-vest: status closed → done
  marginally-venomous-clover: status closed → done
  massively-great-top: status closed → done
  tolerably-delicious-dock: status closed → done

.issuectl/AGENTS.md schema-derived block is out of date (re-run with --fix to regenerate).

$ md5 .issuectl/AGENTS.md
MD5 (.issuectl/AGENTS.md) = 887fc680cbb68d1873cde80f1145cf13

$ issuectl doctor --fix > /dev/null 2>&1; echo "exit=$?"
exit=1

$ md5 .issuectl/AGENTS.md
MD5 (.issuectl/AGENTS.md) = 887fc680cbb68d1873cde80f1145cf13   # identtinen

$ git status --short .issuectl/ issues/
                                                              # tyhjä — ei mitään muutettu

$ grep "^status:" issues/amazingly-enchanted-vest/item.md
status: closed                                                # yhä legacy-arvo, ei "done"
```

## Bugin todelliset oireet

### 1. Schema-alias-coersio (`closed → done`) ei tapahdu

Repon `.issuectl/schema.yaml` (tai vastaava) määrittelee aliaksen, jonka
mukaan `status: closed` pitäisi kanonisoida `status: done`:ksi. `doctor`
tunnistaa neljä rikkomusta ja ohjeistaa `--fix`:n. `--fix`-ajo ei
muokkaa kyseisten neljän issuen `item.md`:tä lainkaan.

### 2. `.issuectl/AGENTS.md` schema-derived block on out-of-date eikä regeneroidu

JSON-output (`issuectl doctor --fix --json`) paljastaa juuren:

```json
{
  "agents_md_drift": true,
  "agents_md_malformed": null,
  "agents_md_missing": false,
  "agents_md_regenerated": false,
  "issues_agents_md_rewritten": false,
  "legacy_issues_agents_md": false
}
```

`agents_md_drift: true` mutta `agents_md_regenerated: false` — doctor näkee
driftin samassa ajossa jossa pitäisi korjata se, mutta korjauspolkua ei
kutsuta.

### 3. Human-output väittää korjanneensa, JSON ei vastaavaa lippua aseta

Stdoutin viimeinen rivi `"Applied. ... 0 AGENTS.md block(s) regenerated."`
on harhaanjohtava: lukijalle jää kuva että ajo onnistui ja vain "ei ollut
mitään tehtävää", vaikka samassa ajossa stderriin (tai aiemmin stdoutiin)
oli juuri printattu lista korjauskohteista.

Lisäksi `--fix`-ajon exit-koodi on **1**, mutta mikään error-envelope ei
kerro miksi — `--json`-output palauttaa silti normaalin tulosobjektin, ei
`{"error":{...}}`-rakennetta jota CLI:n dokumentoitu `--json`-kontrakti
lupaa.

## Odotettu käyttäytyminen

`issuectl doctor --fix` joko

- **(a)** kanonisoi `status: closed → done` neljän issuen frontmatterissa
  ja regeneroi `.issuectl/AGENTS.md`:n schema-derived blockin, JA exit-koodi
  on `0` kun korjaukset tehtiin onnistuneesti, TAI
- **(b)** jos näille luokille ei (vielä) ole `--fix`-toteutusta,
  ilmoittaa siitä erikseen ("manual fix required: status alias coercion,
  AGENTS.md block regen — `--fix` does not yet implement these") sen sijaan
  että väittäisi raportin lopussa "Applied".

JSON-puolella `agents_md_drift && !agents_md_regenerated` -tilanteessa
pitäisi joko regeneroida tai palauttaa nimetty error-envelope.

## Vaikutus käyttäjälle

- `doctor` ei voi koskaan päätyä "clean"-tilaan — joka ajo raportoi
  samat 4 + 1 löydöstä.
- Käyttäjän pitää manuaalisesti `sed`/editorilla muuttaa neljä
  frontmatter-status-riviä ja regeneroida `.issuectl/AGENTS.md` ilman
  että työkalu kertoo mistä regen-templatesta tehdä se.
- AI-agentit (kuten Claude Code) tulkitsevat "Applied. 0 …" -rivin
  onnistumiseksi ja jättävät tehtävän kesken huomaamatta.

## Ympäristö

- macOS 15.5 (Darwin 25.5.0), Apple silicon
- `issuectl` asennettu Homebrew-tapilla `jarimustonen/issuectl/issuectl`
- Repo-layout: flat-slug (issuectl 0.5.1+ -konventio)
- Issue-määrä: ~150 (kaikki flat-layoutissa)
- `.issuectl/schema.yaml` repon mukana, ei custom-muutoksia coersio-aliaksiin

## Liitteet / lokit

`--fix`-ajon täysi JSON-tulosobjekti, stdout ja stderr saatavilla
pyynnöstä — leikattu tähän raporttiin vain relevantit kentät.
