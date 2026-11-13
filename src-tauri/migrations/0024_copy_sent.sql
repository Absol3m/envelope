-- Copie des messages envoyés dans « Envoyés » : activée par défaut, désactivable par compte.
ALTER TABLE accounts ADD COLUMN copy_sent INTEGER NOT NULL DEFAULT 1;
