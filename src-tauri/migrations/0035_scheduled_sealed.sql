-- Envois programmés / boîte d'envoi : message OpenPGP déjà signé et chiffré (MIME brut) et son enveloppe SMTP.
-- Signé à la programmation : à l'heure prévue, ni clé ni phrase secrète ne sont nécessaires. `message` (JSON) reste
-- la source pour rouvrir le message dans le composer ; ces deux colonnes sont NULL pour un message ordinaire.
ALTER TABLE scheduled ADD COLUMN sealed   BLOB;
ALTER TABLE scheduled ADD COLUMN envelope TEXT;   -- JSON : {"from": adresse|null, "to": [adresses]} (Bcc compris)
