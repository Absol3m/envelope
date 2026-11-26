#!/usr/bin/env bash
# Régénère les fixtures S/MIME des tests (src-tauri/src/smime.rs) avec OpenSSL. Les clés privées restent dans un dossier
# temporaire supprimé à la fin : seuls l'autorité de test (certificat public) et les messages signés sont versionnés.
# Validité de 100 ans (autorité et certificats) : les tests ne dépendent pas de l'horloge réelle. Les messages sont
# signés à la date de génération : si on régénère, mettre à jour `DATE` dans les tests de `smime.rs`.
set -euo pipefail
out="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
days=36500
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -subj "/CN=Envelope Test CA" -days $days \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign" 2>/dev/null
cp ca.pem "$out/ca.pem"

# leaf <nom> <rsa|ec> <extensions…> : certificat de alice@example.org signé par l'autorité de test
leaf() {
  local name="$1" kind="$2"; shift 2
  if [ "$kind" = ec ]; then openssl ecparam -name prime256v1 -genkey -noout -out "$name.key"
  else openssl genrsa -out "$name.key" 2048 2>/dev/null; fi
  openssl req -new -key "$name.key" -out "$name.csr" -subj "/CN=Alice Test"
  printf "basicConstraints=CA:FALSE\nsubjectAltName=email:alice@example.org\n%s\n" "$*" > "$name.cnf"
  openssl x509 -req -in "$name.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -out "$name.pem" -days $days -extfile "$name.cnf" 2>/dev/null
}
leaf rsa rsa "keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=emailProtection"
leaf ec ec "keyUsage=digitalSignature
extendedKeyUsage=emailProtection"
leaf noeku rsa "keyUsage=digitalSignature"
leaf tls rsa "extendedKeyUsage=serverAuth"

printf 'Content-Type: text/plain; charset=utf-8\r\n\r\nBonjour Bob\r\n' > body.txt
for n in rsa ec noeku tls; do
  openssl smime -sign -in body.txt -signer $n.pem -inkey $n.key -md sha256 -out "$out/${n}_detached.eml"
done
openssl smime -sign -in body.txt -signer rsa.pem -inkey rsa.key -md sha384 -out "$out/rsa_sha384_detached.eml"
openssl smime -sign -nodetach -in body.txt -signer rsa.pem -inkey rsa.key -md sha256 -out "$out/rsa_opaque.eml"
openssl smime -encrypt -in body.txt -aes256 -out "$out/rsa_encrypted.eml" rsa.pem
