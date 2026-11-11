-- Vérification d'identité de l'expéditeur : mécanisme qui a échoué d'après l'en-tête Authentication-Results
-- du serveur de réception (« dmarc », « spf » ou « dkim »), NULL si rien à signaler. Voir `mail::authres`.
ALTER TABLE messages ADD COLUMN auth_fail TEXT;
