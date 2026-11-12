#!/bin/bash
# Démarrage de l'application de production (macOS) : délai jusqu'à la fenêtre et jusqu'au contenu affiché, mémoire de
# chaque processus (moteur Rust + moteur web). Utilise un HOME jetable : la vraie base n'est pas touchée.
#   pnpm tauri build --bundles app
#   scripts/bench-startup.sh <Envelope.app> [base.db]     # base générée par `cargo run --release --example seed_local`
# Le tout premier lancement d'une application neuve et non signée peut prendre ~10 s (vérification de macOS) : relancer.
set -u
APP="${1:?usage : bench-startup.sh <Envelope.app> [base.db]}"; DB="${2:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"; TMP="$(mktemp -d)"
swiftc -O "$HERE/winwatch.swift" -o "$TMP/winwatch" || exit 1
H="$TMP/home"; D="$H/Library/Application Support/dev.envelope.app"; mkdir -p "$D"; [ -n "$DB" ] && cp "$DB" "$D/envelope.db"
pgrep -f com.apple.WebKit | sort > "$TMP/before"
T0=$(python3 -c 'import time;print(time.time())')
HOME="$H" "$APP/Contents/MacOS/$(ls "$APP/Contents/MacOS" | head -1)" > "$TMP/log" 2>&1 & PID=$!
read -r MS WIN < <("$TMP/winwatch" $PID)
CONTENT=-1   # une capture de fenêtre vide est petite ; chargée, elle dépasse ~60 Ko
for _ in $(seq 1 120); do
  screencapture -x -l "$WIN" "$TMP/c.png" 2>/dev/null
  if [ "$(stat -f %z "$TMP/c.png" 2>/dev/null || echo 0)" -gt 60000 ]; then CONTENT=$(python3 -c "import time;print(int((time.time()-$T0)*1000))"); break; fi
  sleep 0.05
done
sleep 6; pgrep -f com.apple.WebKit | sort > "$TMP/after"
echo "fenêtre : ${MS} ms · contenu affiché : ${CONTENT} ms après le lancement"
for p in $PID $(comm -13 "$TMP/before" "$TMP/after"); do footprint -p "$p" 2>/dev/null | grep -m1 Footprint | sed 's/ *\[[0-9]*\]: 64-bit//; s/^/  /'; done
kill $PID 2>/dev/null; sleep 1; rm -rf "$TMP"
