# Assessment: create body-file review

Source: [`history/review-create-body-file.md`](review-create-body-file.md)

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Naive body extraction can duplicate required sections after a Markdown horizontal rule[^f1] | CONFIRMED | OCCASIONAL | IMPROVES | NONE | HIGH | FIX |
| F2 | Structured-body renderer lacked an adjacent unit test[^f2] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F3 | Shipped issue template contradicted body-file behavior[^f3] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F4 | User-visible fix lacked an Unreleased changelog entry[^f4] | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F5 | Intake body-file structured-versus-free-text semantics are unresolved[^f5] | CONFIRMED | OCCASIONAL | NEUTRAL | MODERATE | HIGH | DISCUSS |
| F6 | Boolean body mode permits contradictory core states[^f6] | CONFIRMED | RARE | NEUTRAL | MODERATE | HIGH | DROP (Rule 1b: RARE, no readability gain) |
| F7 | Plain-prose body-file behavior is an unintended regression[^f7] | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |

**FIX: 4   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 1   DROP: 2**

## F5 — Intake body-file structured-versus-free-text semantics

- **Milloin tämä näkyy käyttäjälle** — kun intake-raportti annetaan `--body-file`-tiedostona ja sisältö on jo jäsennelty H2-otsikoilla.
- **Miten se näkyy** — intake käsittelee tiedoston vapaana tekstinä ja lisää oman `## Description` -otsikkonsa; tiedostossa jo oleva sama otsikko voi siksi kahdentua.
- **Miksi sillä on väliä** — intake on agenteille suositeltu vastaanottopolku, joten epäselvä sopimus tuottaa eri rakenteen kuin saman niminen `create --body-file` -valitsin.
- **Mistä päätös on kyse** — säilytetäänkö intaken nykyinen vastaanottorakenne, jossa raportti sijoitetaan generoituun kuvausosioon, vai määritelläänkö tiedostosyöte jäsennellyksi Markdowniksi kuten create-komennossa.
- Päätös vaatii erillisen yhteensopivuus- ja dokumentointiarvion; tämän tehtävän nimenomainen create-rajaus säilyttää nykyisen intake-käytöksen.

[^f1]: [`history/review-create-body-file.md:10`](review-create-body-file.md#L10)
[^f2]: [`history/review-create-body-file.md:18`](review-create-body-file.md#L18)
[^f3]: [`history/review-create-body-file.md:26`](review-create-body-file.md#L26)
[^f4]: [`history/review-create-body-file.md:34`](review-create-body-file.md#L34)
[^f5]: [`history/review-create-body-file.md:44`](review-create-body-file.md#L44)
[^f6]: [`history/review-create-body-file.md:49`](review-create-body-file.md#L49)
[^f7]: [`history/review-create-body-file.md:54`](review-create-body-file.md#L54)
