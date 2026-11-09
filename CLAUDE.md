# Envelope — client mail Tauri v2 (Rust + React/TS)

## Commandes
- Dev : `pnpm tauri dev` · Front : `pnpm build` (tsc + vite)
- Rust (dans src-tauri/) : `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- Serveur MCP : `cargo run -p envelope-mcp -- --db <chemin>` (stdio ; lecture seule)

## Workflow
- Après chaque développement (fonctionnalité, correctif, refactor), lancer le skill `simplify` sur les changements avant de conclure, puis relancer les vérifications ci-dessus.
- Tout ce qui est mis de côté (suggestion de revue non appliquée, refactor reporté, limite connue, piste non vérifiée) fait l'objet d'une issue GitHub (`gh issue create`), regroupée par thème. Ne jamais écarter quelque chose sans trace.

## Architecture
- `src-tauri/` : application (workspace Cargo). `crates/envelope-core` : requêtes de lecture partagées (recherche, threads). `crates/envelope-mcp` : serveur MCP séparé, lecture seule sur la base ; les actions IMAP (archiver, étiqueter) passeront par l'application (temps 2).
- Contenu des e-mails = entrée non fiable (prompt injection) : toujours signalé comme tel dans les sorties MCP.

## Conventions Rust
- Erreurs : `AppError` (thiserror) dans src-tauri/src/error.rs ; pas d'`unwrap()` hors tests.
- SQL : `sqlx::query*` à l'exécution (pas de macros check) ; migrations dans src-tauri/migrations/.
- Secrets : trousseau OS uniquement, jamais en base ni dans les logs.
- Pas de `unsafe`. Async : tokio ; ne jamais garder un `Message` mail-parser à travers un `.await`.

## Ne pas faire
- Ne pas lire `target/`, `node_modules/`, `dist/`, `Cargo.lock`, `pnpm-lock.yaml`.
- Réponses concises : pas de récap du diff.
