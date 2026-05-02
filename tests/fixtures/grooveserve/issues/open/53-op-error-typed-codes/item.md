---
created: 2026-04-29
updated: 2026-04-29
type: improvement
reporter: jari
assignee: jari
status: open
priority: normal
related: ["#42"]
labels: [i18n, ops, refactor]
---

# 53. OpError::InvalidInput vuotaa englantia UI:hin — typitetyt virhekoodit

_Source: #42-monikielisyys review (OpenAI flagasi: "InvalidInput leaks
English to UI", Claude vahvisti)_

## Description

`OpError::InvalidInput(String)` käytetään ops-kerroksessa palauttamaan
validointivirheitä. Esimerkkejä `services/api/src/ops/`:

- `tenant.rs::create_tenant`: `OpError::InvalidInput("all fields are required".into())`
- `email.rs::normalize`: `OpError::InvalidInput("email is required".into())`
- `validate.rs::name`: `OpError::InvalidInput("name contains disallowed characters".into())`
- `invitation.rs::invite_user`: `OpError::InvalidInput("cannot invite as admin in MVP".into())`

Route-kerros bubblaa `msg`-stringin suoraan `flash_error`:iin tai vastaavaan
UI-elementtiin. Tulos: i18n-tavoite "lokalisoi kaikki käyttäjälle näkyvät
stringit" ei toteudu näiden virheiden kohdalla, ja ops-kerros on kytketty
esitykseen englanninkielisten viestiensä kautta.

## Scope

Korvaa `OpError::InvalidInput(String)` typitetyillä koodeilla:

```rust
pub enum InvalidInputCode {
    AllFieldsRequired,
    EmailRequired,
    InvalidEmailAddress,
    NameRequired,
    NameContainsDisallowedCharacters,
    CannotInviteAsAdmin,
    CouldNotDeriveUniqueSlug,
    PasswordTooShort,
    InvalidLocale,
    UserMustBeDisabledFirst,
    // ...
}

pub enum OpError {
    InvalidInput(InvalidInputCode),
    // ...
}
```

Route-kerros kääntää koodin lokalisoiduksi:

```rust
pub fn invalid_input_message(locale: Locale, code: InvalidInputCode) -> &'static str {
    use InvalidInputCode::*;
    match code {
        AllFieldsRequired => i18n::all_fields_required(locale),
        EmailRequired     => i18n::email_required(locale),
        // ...
    }
}
```

## Acceptance criteria

- [ ] Kaikki `OpError::InvalidInput(_)` palautukset käyttävät enum-koodia.
- [ ] Lisätty `i18n`-funktiot jokaiselle koodille kolmella kielellä.
- [ ] Route-kerros kutsuu `invalid_input_message(locale, code)` ja
      renderöi sen flash-bannerina.
- [ ] Yksikkötestit vahvistavat että jokaiselle koodille löytyy käännös.
- [ ] Päivitetty kaikki kutsupaikat (testitkin), ei rikkonaisia
      `match`-haaroja.

## Out of scope

- `OpError`:n muut variantit (`AlreadyExists(String)`, `Forbidden`, jne)
  ovat oma asia. `AlreadyExists`:n payload jää, koska se on yleensä
  asiakaskäyttäjälle näkyvä asia ja kytketty tietokenttiin (esim.
  sähköpostiosoite).
