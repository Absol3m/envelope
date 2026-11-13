#!/bin/sh
# Serveur IMAP de test (Dovecot dans Docker, sans TLS, quota de 40 Ko par boîte) pour les essais « réels » :
#   scripts/imap-test-server.sh start   puis   ENVELOPE_TEST_IMAP=127.0.0.1:3143 cargo test -p envelope -- --ignored
#   scripts/imap-test-server.sh stop
# Une boîte neuve par identifiant (n'importe quel nom, mot de passe « pass »). Alpine (et non l'image dovecot, amd64
# seulement) pour tourner aussi sur les Mac Apple Silicon. Jamais pour un vrai compte : mots de passe en clair.
set -eu
dir=$(cd "$(dirname "$0")" && pwd)
case "${1:-}" in
  start)
    docker rm -f envelope-imap-test >/dev/null 2>&1 || true
    docker run -d --name envelope-imap-test -p 3143:143 -v "$dir/imap-test-dovecot.conf:/etc/dovecot/dovecot.conf:ro" alpine:3.18 \
      sh -c 'apk add --no-cache dovecot >/dev/null 2>&1 && mkdir -p /srv/mail && chown 1000:1000 /srv/mail && exec dovecot -F' >/dev/null
    echo "attente du serveur (installation de Dovecot)…"
    for _ in $(seq 1 60); do
      if (printf 'a logout\r\n'; sleep 1) | nc -w 2 127.0.0.1 3143 2>/dev/null | grep -q 'Dovecot ready'; then echo "prêt : 127.0.0.1:3143"; exit 0; fi
      sleep 1
    done
    echo "le serveur ne répond pas : docker logs envelope-imap-test" >&2; exit 1 ;;
  stop) docker rm -f envelope-imap-test >/dev/null ;;
  *) echo "usage : $0 start|stop" >&2; exit 2 ;;
esac
