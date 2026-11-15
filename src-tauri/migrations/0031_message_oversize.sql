-- Message trop volumineux pour la synchro : seul l'en-tête est stocké (sans corps ni pièces jointes) ; le contenu se
-- télécharge à la demande (`download_message`). `size` garde la taille annoncée par le serveur.
ALTER TABLE messages ADD COLUMN oversize INTEGER NOT NULL DEFAULT 0;
