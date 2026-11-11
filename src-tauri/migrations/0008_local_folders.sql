-- Dossiers locaux (« Sur cet ordinateur ») : un compte spécial, sans serveur, qui porte les mails
-- archivés en local depuis le client. Les messages y sont conservés sous leur forme brute (`raw/`).
INSERT OR IGNORE INTO accounts (id, email, display_name, provider, auth_kind, username, imap_host, smtp_host)
VALUES ('local', 'local@envelope.invalid', 'Sur cet ordinateur', 'local', 'password', 'local', '', '');
INSERT OR IGNORE INTO folders (account_id, path, name, role, delimiter) VALUES ('local', 'Archives', 'Archives', NULL, '/');
