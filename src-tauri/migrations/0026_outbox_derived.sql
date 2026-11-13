-- `outbox` était redondant : un message est en boîte d'envoi exactement quand `next_try_at` est renseigné (posé à la
-- mise en file et à chaque échec réseau, jamais pour un envoi programmé).
ALTER TABLE scheduled DROP COLUMN outbox;
