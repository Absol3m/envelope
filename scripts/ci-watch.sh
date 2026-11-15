#!/bin/sh
# Attend la CI du dernier commit poussé et, en cas d'échec, affiche la raison. Usage : scripts/ci-watch.sh
# Une CI rouge se corrige tout de suite, avant toute autre tâche (voir CLAUDE.md).
set -eu
sha=$(git rev-parse HEAD)
echo "CI de ${sha%${sha#???????}} : attente du démarrage…"
id=""
for _ in $(seq 1 24); do
  id=$(gh run list --commit "$sha" --limit 1 --json databaseId -q '.[0].databaseId // empty')
  [ -n "$id" ] && break
  sleep 5
done
[ -n "$id" ] || { echo "aucune exécution trouvée (le commit est-il poussé ?)"; exit 2; }
while [ "$(gh run view "$id" --json status -q .status)" != completed ]; do sleep 15; done
if [ "$(gh run view "$id" --json conclusion -q .conclusion)" = success ]; then echo "✓ CI verte"; exit 0; fi
gh run view "$id" --json jobs -q '.jobs[] | select(.conclusion=="failure") | "✗ " + .name'
gh run view "$id" --log-failed 2>&1 | grep -E "error|Error|FAIL|panicked|failed|AssertionError" | head -15 | cut -c1-240
exit 1
