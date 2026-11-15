#!/bin/sh
# Contrat de la liste (CLAUDE.md) : une évolution du moteur de liste/recherche ou de la démo qui la reproduit doit toucher
# le scénario partagé (src-tauri/tests/contract/list_threads.json). Usage : check-contract-touched.sh <commit de base>
# Échappatoire : « [contrat inchangé] » dans un message de commit de la plage (changement sans effet sur la liste).
set -eu
base="${1:-}"
case "$base" in ""|0000000000000000000000000000000000000000) echo "::warning::contrat de la liste : pas de commit de base, contrôle ignoré"; exit 0 ;; esac
git fetch --no-tags --depth=1 origin "$base" >/dev/null 2>&1 || { echo "::warning::contrat de la liste : base introuvable, contrôle ignoré"; exit 0; }
changed=$(git diff --name-only "$base" HEAD)
engine='src-tauri/crates/envelope-core/src/(thread|search)\.rs|src/lib/(mock|searchSpec)\.ts'
if echo "$changed" | grep -Eq "^($engine)$" && ! echo "$changed" | grep -q '^src-tauri/tests/contract/list_threads.json$'; then
  if git log --format=%B "$base..HEAD" | grep -q '\[contrat inchangé\]'; then
    echo "contrat déclaré inchangé"; exit 0
  fi
  echo "Le moteur de liste/recherche ou la démo a changé sans cas dans src-tauri/tests/contract/list_threads.json." >&2
  echo "Ajoutez un cas au scénario, ou « [contrat inchangé] » dans le message de commit si la liste n'est pas concernée." >&2
  exit 1
fi
echo "contrat de la liste : ok"
