# Envelope — client mail Tauri v2 (Rust + React/TS)

## Commandes
- Dev : `pnpm tauri dev` · Front : `pnpm build` (tsc + vite)
- Rust (dans src-tauri/) : `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Workflow
- Après chaque développement (fonctionnalité, correctif, refactor), lancer le skill `simplify` sur les changements avant de conclure, puis relancer les vérifications ci-dessus.

## Conventions Rust
- Erreurs : `AppError` (thiserror) dans src-tauri/src/error.rs ; pas d'`unwrap()` hors tests.
- SQL : `sqlx::query*` à l'exécution (pas de macros check) ; migrations dans src-tauri/migrations/.
- Secrets : trousseau OS uniquement, jamais en base ni dans les logs.
- Pas de `unsafe`. Async : tokio ; ne jamais garder un `Message` mail-parser à travers un `.await`.

## Ne pas faire
- Ne pas lire `target/`, `node_modules/`, `dist/`, `Cargo.lock`, `pnpm-lock.yaml`.
- Réponses concises : pas de récap du diff.
