-- Message brut (RFC 5322) conservé dans le magasin `raw/` : source de vérité, ré-analysable.
-- NULL : pas encore récupéré ; '' : introuvable sur le serveur (ne plus réessayer).
ALTER TABLE messages ADD COLUMN raw_hash TEXT;
-- Rapatriement de l'historique : borne haute (UID, exclue) du prochain lot à télécharger, vers l'arrière.
-- NULL : pas commencé (on part du plus petit UID local) ; <= 1 : historique complet.
ALTER TABLE folders ADD COLUMN backfill_next INTEGER;
