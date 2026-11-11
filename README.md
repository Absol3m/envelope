# Envelope

Client mail desktop open source (Tauri v2 + Rust + React) avec serveur MCP intégré pour agents IA.

## Développement
```bash
pnpm install
pnpm tauri dev
```

## Serveur MCP (lecture seule)
Donne à un agent IA (Claude, Cursor…) l'accès à la boîte locale : recherche, lecture et résumé de conversations.
```bash
cargo build --release -p envelope-mcp --manifest-path src-tauri/Cargo.toml
claude mcp add envelope -- /chemin/vers/envelope-mcp   # --db CHEMIN ou $ENVELOPE_DB si besoin
```
Construit sur le SDK officiel Rust du protocole MCP (`rmcp`) : le protocole est celui du SDK, seuls les outils sont à nous.
Outils : `list_accounts`, `list_folders`, `search_emails`, `get_thread`, `summarize_thread`. Aucun ne modifie la base ni n'envoie de message ; le contenu des e-mails est signalé comme non fiable.

## Contribuer
Voir [CONTRIBUTING.md](CONTRIBUTING.md).

## Licence
[GNU AGPL-3.0 ou ultérieure](LICENSE).
