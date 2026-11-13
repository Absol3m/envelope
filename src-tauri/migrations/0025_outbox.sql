-- Boîte d'envoi : un message dont l'envoi SMTP a échoué faute de connexion reste dans `scheduled` (outbox = 1) et
-- repart seul dès que le serveur répond, avec un délai croissant (`attempts`, `next_try_at`). Jamais « en retard » :
-- contrairement à un envoi programmé, il n'a pas d'heure à respecter, seulement un réseau à retrouver.
ALTER TABLE scheduled ADD COLUMN outbox      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scheduled ADD COLUMN attempts    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scheduled ADD COLUMN next_try_at INTEGER;
