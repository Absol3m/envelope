#!/usr/bin/env bash
# Échoue si les deux sources de version (package.json, [workspace.package] de Cargo) diffèrent entre elles ou
# de l'étiquette de publication donnée en premier argument (avec ou sans « v » initial), ou si un autre
# manifeste ne les reprend pas (tauri.conf.json → ../package.json, crates → version.workspace).
#   scripts/check-version.sh            # les manifestes concordent
#   scripts/check-version.sh v0.1.0     # ... et avec l'étiquette (workflow de publication)
# Une pré-version porte son suffixe dans les manifestes (0.1.0-alpha.3 pour v0.1.0-alpha.3) : le système de mise à
# jour compare les versions de l'application installée et publiée, qui doivent donc différer d'une alpha à l'autre.
set -euo pipefail
cd "$(dirname "$0")/.."

toml_version() { sed -n 's/^version *= *"\(.*\)"/\1/p' "$1" | head -1; }
json_version() { sed -n 's/^ *"version": *"\(.*\)".*/\1/p' "$1" | head -1; }

root="$(json_version package.json)"
expected="${1:-$root}"
expected="${expected#v}"

status=0
check() { # <fichier> <version qui s'y trouve>
  if [[ "$2" != "$expected" ]]; then
    echo "version incohérente : $1 vaut '$2', attendu '$expected'" >&2
    status=1
  fi
}
check package.json "$root"
check "src-tauri/Cargo.toml ([workspace.package])" "$(toml_version src-tauri/Cargo.toml)"
# Les autres manifestes reprennent ces deux sources au lieu de répéter le numéro.
if [[ "$(json_version src-tauri/tauri.conf.json)" != "../package.json" ]]; then
  echo "src-tauri/tauri.conf.json : \"version\" doit valoir \"../package.json\"" >&2
  status=1
fi
for manifest in src-tauri/Cargo.toml src-tauri/crates/*/Cargo.toml; do
  if ! grep -q '^version.workspace = true$' "$manifest"; then
    echo "$manifest : [package] doit déclarer version.workspace = true" >&2
    status=1
  fi
done

[[ $status -eq 0 ]] && echo "tous les manifestes sont à la version $expected"
exit $status
