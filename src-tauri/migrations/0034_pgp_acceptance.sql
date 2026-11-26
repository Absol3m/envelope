-- OpenPGP à l'envoi : à qui on fait confiance pour chiffrer, et avec quelle clé on signe.
-- Acceptation d'une clé de correspondant **pour une adresse** (comme Thunderbird) : la même clé peut être
-- vérifiée pour une adresse et refusée pour une autre. On ne chiffre jamais pour une clé non acceptée.
CREATE TABLE pgp_acceptance (
    fingerprint TEXT NOT NULL REFERENCES pgp_keys(fingerprint) ON DELETE CASCADE,
    email       TEXT NOT NULL,                  -- minuscules
    decision    TEXT NOT NULL CHECK (decision IN ('unverified', 'verified', 'rejected')),
    decided_at  INTEGER NOT NULL,
    PRIMARY KEY (fingerprint, email)
);
CREATE INDEX pgp_acceptance_email ON pgp_acceptance(email);

-- Clé de signature d'une adresse d'expéditeur (adresse du compte ou alias).
CREATE TABLE pgp_signing_keys (
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email       TEXT NOT NULL,                  -- minuscules
    fingerprint TEXT NOT NULL REFERENCES pgp_keys(fingerprint) ON DELETE CASCADE,
    PRIMARY KEY (account_id, email)
);
