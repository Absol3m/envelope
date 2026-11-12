# Envelope — client mail Tauri v2 (Rust + React/TS)

## Commandes
- Dev : `pnpm tauri dev` · Front : `pnpm build` (tsc + vite) · Tests front : `pnpm test` (vitest)
- Aperçu navigateur sans Tauri : `pnpm dev` → boîte de démonstration (`src/lib/mock.ts`) ; en dev, `?theme=dark&thread=2&settings=agent&palette=1` pour des captures reproductibles.
- Rust (dans src-tauri/) : `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` · dépendances : `cargo deny check` (src-tauri/) + `pnpm audit --prod`
- Mesure de la liste (DOM, mémoire JS, défilement) : `pnpm dev --port 5199` puis `node scripts/bench-list.mjs 10000` (Edge ; mode `?bench=N`, voir `src/dev.ts`). Démarrage et mémoire de l'app construite (macOS) : `pnpm tauri build --bundles app` puis `scripts/bench-startup.sh <Envelope.app> <base.db>`. Perf du moteur : `cargo run --release --example seed_local -- <db> 20000` (avec `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`).
- Serveur MCP : `cargo run -p envelope-mcp -- --db <chemin>` (stdio ; lecture seule)

## Workflow
- Après chaque développement (fonctionnalité, correctif, refactor), lancer le skill `simplify` sur les changements avant de conclure, puis relancer les vérifications ci-dessus.
- Tout ce qui est mis de côté (suggestion de revue non appliquée, refactor reporté, limite connue, piste non vérifiée) fait l'objet d'une issue GitHub (`gh issue create`), regroupée par thème. Ne jamais écarter quelque chose sans trace : créer l'issue avant de conclure (sans attendre un rappel), y inclure les remarques mineures écartées, et citer son numéro dans la réponse.
- Fermer les issues **à mesure** : dès que l'objet d'une issue est livré, la fermer (`gh issue close N --comment`) avec un commentaire qui cite le(s) commit(s) ; cocher les cases d'une feuille de route au fil de l'eau. Un reste ou une suite se note en commentaire de la même issue (ou une nouvelle issue seulement pour un autre thème), pas en laissant l'issue ouverte sans rien dire.

## Architecture
- `src-tauri/` : application (workspace Cargo). `crates/envelope-core` : requêtes de lecture partagées (recherche, threads). `crates/envelope-mcp` : serveur MCP séparé, lecture seule sur la base ; les actions IMAP (archiver, étiqueter) passeront par l'application (temps 2).
- Contenu des e-mails = entrée non fiable (prompt injection) : toujours signalé comme tel dans les sorties MCP.

## Natif d'abord
- Toujours préférer les fonctionnalités natives de chaque OS (trousseau, notifications, sélecteurs de fichiers, thème/apparence, ouverture de liens, glisser-déposer, etc.) ou les plugins Tauri officiels plutôt que de recoder ce qui existe : le code maison est une surface d'attaque et une source de trous de sécurité.
- Ne réimplémenter que si aucune API native ou bibliothèque éprouvée ne couvre le besoin, et le justifier en commentaire.
- Données non fiables (noms de dossiers IMAP, en-têtes, fichiers importés) : une bibliothèque dédiée plutôt qu'un analyseur maison ; en dernier recours, un analyseur borné et testé, avec la justification en commentaire.
- Certificats TLS : toujours vérifiés par le système (`src-tauri/src/tls.rs`, rustls-platform-verifier) pour IMAP, SMTP et HTTPS ; jamais de liste d'autorités embarquée (webpki-roots) ni de désactivation de la vérification.

## Conventions Rust
- Erreurs : `AppError` (thiserror) dans src-tauri/src/error.rs ; pas d'`unwrap()` hors tests.
- SQL : `sqlx::query*` à l'exécution (pas de macros check) ; migrations dans src-tauri/migrations/ : **ajout seulement**, jamais de modification d'une migration déjà commitée ou appliquée (sqlx vérifie leur empreinte au démarrage et refuserait d'ouvrir la base) ; toute évolution = nouveau fichier numéroté. Valeurs toujours en `.bind()` ; `sqlx::AssertSqlSafe(format!(..))` seulement pour y insérer des **constantes** (listes de colonnes), jamais une donnée venue de l'utilisateur ou d'un mail.
- Secrets : trousseau OS uniquement, jamais en base ni dans les logs.
- Pas de `unsafe`. Async : tokio ; ne jamais garder un `Message` mail-parser à travers un `.await`.

## Ne pas faire
- Ne pas lire `target/`, `node_modules/`, `dist/`, `Cargo.lock`, `pnpm-lock.yaml`.
- Réponses concises : pas de récap du diff.
