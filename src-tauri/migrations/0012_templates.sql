-- Modèles de message : texte réutilisable (objet + corps HTML), propre à l'utilisateur, commun à tous les comptes.
CREATE TABLE templates (
    id        INTEGER PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    subject   TEXT NOT NULL DEFAULT '',
    body_html TEXT NOT NULL DEFAULT ''
);
