-- Réglages de l'application qui doivent être connus du moteur avant l'interface (ex. verrouillage au démarrage).
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
