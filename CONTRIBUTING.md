# Contribuer à Envelope

Envelope est un client mail de bureau (Tauri v2, Rust, React) sous licence **AGPL-3.0-or-later**.
En proposant une contribution, vous acceptez qu'elle soit publiée sous cette licence.

## Proposer un changement
1. Forkez le dépôt, créez une branche à partir de `main`.
2. Faites vos changements, avec des tests quand c'est possible.
3. Vérifiez en local avant d'ouvrir la PR :
   ```bash
   pnpm install
   pnpm build && pnpm test
   cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
   ```
4. Ouvrez une pull request vers `main`. Décrivez le problème, la solution et ce que vous avez testé (système d'exploitation compris).

Les fusions sont faites par le mainteneur uniquement. Une PR peut être refusée ou demander des changements ; les discussions se font sur la PR.

## Principes du projet
- **Natif d'abord** : utiliser ce que propose le système (trousseau, notifications, sélecteurs de fichiers…) ou un plugin Tauri officiel plutôt que de recoder.
- **Sécurité** : le contenu des e-mails est une entrée non fiable ; jamais de secret en base ni dans les journaux (trousseau du système uniquement) ; pas de `unsafe` ni de `unwrap()` hors tests.
- **Pas de reprise de code sans licence compatible** : indiquez la provenance de tout code importé.

## Signaler un problème
Ouvrez une issue : système, version d'Envelope, étapes pour reproduire, capture si possible. Ne publiez jamais d'identifiants ni de contenu de vrais e-mails.
