-- Signature propre à un alias d'envoi ; vide : celle du compte s'applique.
ALTER TABLE identities ADD COLUMN signature TEXT NOT NULL DEFAULT '';
