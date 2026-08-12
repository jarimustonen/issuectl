---
created: 2026-08-12
updated: 2026-08-12
type: task
status: open
priority: normal
---

# Re-establish write-under-flock test coverage

## Description

# Re-establish "mutations write under the repo flock" test coverage

## Tausta
`@remove-web-ui` poisti kuusi `*_publishes_before_releasing_flock`-testiä + `install_lock_probe`/
`assert_probe_saw_held`-apurit. Ne todensivat SSE-julkaisun järjestyksen suhteessa lukkoon, mutta
toimivat samalla ainoana coveragena invariaanteille "mutaatio kirjoittaa levylle pidellen
`.issuectl/write.lock`-flockia". Julkaisuseam poistui web-UI:n mukana; itse flock-invariantti on
edelleen koodissa (`WriteLock::acquire` jokaisen mutaation alussa), mutta nyt testaamatta.

## Milloin tämä näkyy käyttäjälle
Vasta jos tuleva refaktorointi vahingossa siirtää kirjoituksen lukon ulkopuolelle.

## Miten se näkyy
Rinnakkaiset `issuectl`-mutaatiot (useampi agentti / esim. orchestratectl-worktreet samaan
repoon) voisivat revitä toisensa päälle ilman että mikään testi huomaa regressiota.

## Miksi sillä on väliä
Repo-tason kirjoitussarjallisuus on `issuectl`:n ydinlupaus (ks. `mutate/mod.rs` moduulidoc).
Hiljainen coverage-aukko tässä on juuri se luokka bugia jonka fan-out-työnkulut paljastaisivat
tuotannossa, eivät CI:ssä.

## Miksi tämä vaatii oman suunnittelunsa
Deterministinen, ei-flakky testi tarvitsee uuden testiseamin (esim. eristetty lukko-scoped
mutaatio執行aja jota voi instrumentoida, tai kanava-synkronoitu toinen säie joka pitää lukkoa ja
todistaa että mutaatio blokkaa). EventHub-pohjaista probea ei saa palauttaa. Seamin muotoilu on
suunnittelupäätös, ei mekaaninen korjaus — siksi omana työnään.
