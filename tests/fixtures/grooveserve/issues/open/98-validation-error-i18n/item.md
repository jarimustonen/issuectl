---
created: 2026-05-01
updated: 2026-05-01
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#6"]
labels: [i18n, refactor, post-poc]
---

# 90. Validation-virheiden lokalisointi — structured codes + i18n

_Source: spin-off #6:n /llm-review-kierroksesta (M9)._

## Tausta

`OpError::InvalidInput` palauttaa tällä hetkellä **English-literal-stringejä**
ops-pinnalta UI-pintaan asti. Esim. `crates/ops/src/onboarding.rs`:

```rust
return Err(OpError::InvalidInput("home address is required".into()));
return Err(OpError::InvalidInput(format!(
    "phone number must be at most {MAX_PHONE_CHARS} characters"
)));
```

Reitit (`crates/server/src/http/routes/onboarding.rs`,
`reset_password.rs`, jne.) kutsuvat `flash_error(Some(&msg))` joka
renderöi viestin sellaisenaan **riippumatta** käyttäjän locale-asetuksesta.
Tuloksena suomenkieliselle tai ruotsinkieliselle käyttäjälle näkyy:
- Sivun chrome (otsikko, napit) lokalisoituna (Locale::Fi)
- Validointivirhe Englanniksi (hard-coded literal)

Sama vika kaikissa ops-kerroksen validoinneissa: `password_reset`,
`tenant::create_tenant`, `validate::name`, `currency`, `receipts`,
`expenses` — mikä tahansa joka palauttaa `OpError::InvalidInput`-arvon.

## Scope

**Järjestelmänlaajuinen refaktori**. Kuuluu omaan suunnitteluunsa,
ei mahdu yksittäisen reitin korjaukseen.

1. **Suunnittele**: viesti vai koodi?
   - Vaihtoehto A: lisää `OpError::ValidationCode(code: ValidationError)`
     -variantti, jossa `ValidationError` on enum (esim.
     `MissingHomeAddress`, `PhoneTooShort { min: usize }`,
     `DateInFuture`, …).
   - Vaihtoehto B: pidä `OpError::InvalidInput(String)`-shape mutta
     vaihda string-arvoksi well-known koodi
     ("validation.address.required") jonka i18n-pinta mappaa.
   - Vaihtoehto C: lisää erillinen `metadata: serde_json::Value`-kenttä
     jossa kantaa kontekstin (esim. `{"min": 7}` puhelinvalidaatorille)
     samalla kun viesti pysyy human-readable fallbackina.
2. **Päivitä kutsupisteet**:
   - kaikki `OpError::InvalidInput("...")` ops-cratessa.
   - kaikki call-sitet jotka renderöivät virheen UI:hin
     (lähinnä `crates/server/src/http/routes/*` reitit).
3. **Lisää i18n-namespace**:
   - `crates/server/src/http/i18n.rs::validation::*` (ehdotus) jossa
     `validation_address_required(l)`, `validation_phone_min_digits(l, n)`,
     jne.
4. **Testit**:
   - Ops-puoli: testit varmistavat että koodi/enum tulee oikeasta
     virheestä riippumatta locale:sta.
   - HTTP-puoli: testaa että fi/sv-renderaisukset eivät sisällä
     englanninkielisiä validointitekstejä.

## Pre-vaatimukset

- POC-vaihe ohi (#56 päätaso). Tämä on UX-paranus, ei korjaa toimivuutta.
- Päätös A vs B vs C ennen toteutusta — eri call-sitejen tarve voi
  ohjata valintaa.

## Pois scopesta

- Page-chrome i18n-stringit (jo lokalisoituja)
- Email-templatet (jo lokalisoituja `email_*_plain`/`email_*_intro`-
  funktioissa)
- DB-tason CHECK-virheet (`pg.check_violation`-bubblaus on eri ongelma —
  validate ennen DB:tä, älä luota CHECK-rikkomuksiin UI-virheiden
  lähteenä)

## Miksi ei nyt

- POC-vaiheessa kohdeasiakkaita ei ole vielä; väärän ratkaisun riski
  on suurempi kuin ei-lokalisoitujen virheiden hyväksyminen toistaiseksi.
- A vs B vs C valintaa varten halutaan kerätä kokemusta useammasta
  kutsupisteestä — tällä hetkellä validointivirheitä on lähinnä
  onboarding/password_reset/registration-poluilla.
- Refaktori touchaa ~30+ tiedostoa. Suunnittelu kannattaa erikseen.
