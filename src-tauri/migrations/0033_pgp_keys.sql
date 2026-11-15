-- Annuaire OpenPGP : certificats publics des correspondants et partie publique de nos propres clés.
-- La partie privée n'est jamais ici : elle vit dans le trousseau du système (`mail::secrets`), `has_secret` en garde la trace.
CREATE TABLE pgp_keys (
    fingerprint TEXT PRIMARY KEY,              -- majuscules, sans espaces
    armored     TEXT NOT NULL,                 -- certificat public, armuré
    user_ids    TEXT NOT NULL DEFAULT '[]',    -- JSON : identités (« Nom <adresse> »)
    has_secret  INTEGER NOT NULL DEFAULT 0,
    revoked     INTEGER NOT NULL DEFAULT 0,
    added_at    INTEGER NOT NULL
);
