---
created: 2026-04-27
updated: 2026-04-27
type: chore
reporter: jari
assignee: jari
status: open
priority: high
epic: 5
related: ["#33", "#26"]
labels: [privacy, gdpr, compliance, security]
---

# 36. Privacy review — pysyvän käyttäjämuistin GDPR-katselmus

_Source: #33 LLM review_

## Description

#33:n tuoma pysyvä `user_profiles.notes_md` (vapaamuotoinen markdown
agentin kirjoittamana käyttäjästä) on GDPR:n alaista henkilödataa.
SALIENCE.md kieltää salaisuuksien tallennuksen, mutta tämä ei ole
riittävä kontrolli — kaikki notes-bodyn sisältö on henkilödataa, ei
vain tunnistetiedot.

Tämä issue tehdään kun #33 toimii. Privacy review tehdään ennen MVP:tä
asiakkaiden kanssa.

## Scope

- [ ] **Audit-taulu** kaikille muistipäivityksille:
  ```sql
  CREATE TABLE user_profile_note_revisions (
      id BIGSERIAL PRIMARY KEY,
      tenant_id BIGINT NOT NULL REFERENCES tenants(id),
      user_id BIGINT NOT NULL REFERENCES users(id),
      message_id TEXT,
      old_notes_md TEXT,
      new_notes_md TEXT,
      actor TEXT NOT NULL DEFAULT 'agent',
      created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  );
  ```
- [ ] **GET /api/me/memory** — käyttäjä näkee mitä hänestä on tallennettu
- [ ] **DELETE /api/me/memory/notes** — vapaamuotoisen muistin poisto
- [ ] **DELETE /api/me/account** — koko datan poisto cascadea myöten
  (varmista `ON DELETE CASCADE` `user_profiles`-relaatiossa)
- [ ] **Retention-politiikka**: kuinka kauna passiiviset käyttäjien notes
  säilyvät? (Esim. 24 kk inaktiivisuuden jälkeen siivous?)
- [ ] **Sensitive-data denylist**: agentti hylkää tallennuksen jos teksti
  sisältää SSN/IBAN/CC-pattern-matcheja
- [ ] **Consent-UX**: ensimmäisellä viestillä käyttäjä saa tietää että
  pysyvä muisti on käytössä; opt-out
- [ ] **Päivitysten näkyvyys**: agentti mainitsee tallennukset (jo §10
  designissa), mutta tämä on _informointia_ ei _suostumusta_
- [ ] **Telemetria**: hylkäysten määrä, audit-trailin koko, retention-
  jobin tulokset

## Why separate from #33

GDPR-arkkitehtuuri on oma kokonaisuutensa. #33:ssä pidämme markdownin
yksinkertaisena ja teknisesti turvallisena (manipulaatiosuoja #34:ssä).
Tämä issue lisää privacy-kerroksen päälle ennen MVP:tä asiakkaille.
