-- Signature (texte brut) insérée dans les nouveaux messages et les réponses.
ALTER TABLE accounts ADD COLUMN signature TEXT NOT NULL DEFAULT '';
