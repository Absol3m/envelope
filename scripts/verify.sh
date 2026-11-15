#!/bin/sh
# Rejoue en local tout ce que la CI vérifie (.github/workflows/ci.yml), avant un push. À lancer depuis n'importe où.
#   scripts/verify.sh            vérifications complètes (macOS / ce système)
#   scripts/verify.sh --linux    ajoute les tests du front sous Linux (Docker) : l'ordre des fichiers, l'horloge et les
#                                chemins y diffèrent parfois de macOS (une CI a déjà échoué ainsi)
# Le hook de push (.githooks/pre-push, activé par `git config core.hooksPath .githooks`) l'appelle. Ne couvre pas Windows :
# les tests ne doivent donc rien supposer d'un séparateur de chemin (`/`), d'une horloge réelle ni d'un ordre de fichiers.
set -eu
cd "$(dirname "$0")/.."
step() { printf '\n\033[1m▶ %s\033[0m\n' "$*"; }

step "Versions concordantes"
scripts/check-version.sh ""

step "Contrat de la liste touché (commits pas encore poussés)"
base=$(git rev-parse --verify -q '@{u}' || git rev-parse --verify -q origin/main || true)
scripts/check-contract-touched.sh "${base:-}"

step "Front : installation, build, tests, traductions"
pnpm install --frozen-lockfile >/dev/null
pnpm build
pnpm test
node scripts/i18n-status.mjs --check

step "Rust : fmt, clippy"
cd src-tauri
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings

step "Rust : tests, depuis des types TypeScript vides (comme la CI : repère les fichiers orphelins ou périmés)"
cd ..
rm -rf src/lib/bindings
(cd src-tauri && cargo test --locked --workspace)
if [ -n "$(git status --porcelain -- src/lib/bindings)" ]; then
  git status --porcelain -- src/lib/bindings
  echo "Types TypeScript générés différents du dépôt : les commiter avec le changement Rust." >&2
  exit 1
fi

step "Dépendances : cargo deny, pnpm audit"
(cd src-tauri && cargo deny check advisories licenses bans sources)
pnpm audit --prod --audit-level moderate

if [ "${1:-}" = "--linux" ]; then
  step "Front sous Linux (Docker)"
  COPYFILE_DISABLE=1 tar --no-xattrs --exclude=node_modules --exclude=target --exclude=dist --exclude=.git \
    --exclude=src-tauri/gen -cf - . 2>/dev/null |
    docker run -i --rm node:lts sh -c 'mkdir /w && tar -xf - -C /w 2>/dev/null && cd /w && npm i -g pnpm@12 >/dev/null 2>&1 && pnpm install --frozen-lockfile >/dev/null 2>&1 && pnpm test' ||
    { echo "Tests du front en échec sous Linux (ou Docker indisponible)." >&2; exit 1; }
fi

printf '\n\033[1;32m✓ Tout ce que la CI vérifie passe en local.\033[0m\n'
