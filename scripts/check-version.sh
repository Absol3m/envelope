#!/usr/bin/env bash
# Échoue si la version déclarée par les manifestes diffère entre eux, ou de l'étiquette de
# publication donnée en premier argument (avec ou sans « v » initial).
#   scripts/check-version.sh            # les manifestes concordent
#   scripts/check-version.sh v0.1.0     # ... et avec l'étiquette (workflow de publication)
# Une étiquette de pré-version (v0.1.0-alpha.1) est comparée à la version sans suffixe.
set -euo pipefail
cd "$(dirname "$0")/.."

toml_version() { sed -n 's/^version *= *"\(.*\)"/\1/p' "$1" | head -1; }
json_version() { sed -n 's/^ *"version": *"\(.*\)".*/\1/p' "$1" | head -1; }

root="$(json_version package.json)"
expected="${1:-$root}"
expected="${expected#v}"
expected="${expected%%-*}"

status=0
check() { # <fichier> <version qui s'y trouve>
  if [[ "$2" != "$expected" ]]; then
    echo "version incohérente : $1 vaut '$2', attendu '$expected'" >&2
    status=1
  fi
}
check package.json "$root"
check src-tauri/tauri.conf.json "$(json_version src-tauri/tauri.conf.json)"
for manifest in src-tauri/Cargo.toml src-tauri/crates/*/Cargo.toml; do
  check "$manifest" "$(toml_version "$manifest")"
done

[[ $status -eq 0 ]] && echo "tous les manifestes sont à la version $expected"
exit $status
