#!/usr/bin/env bash
# UN CASSANDRA CON TLS DE VERDAD (ADR 0070, §TLS real), con autenticación, en Docker.
#
#   bash pruebas-de-fuego/cassandra/tls-lab.sh [puerto]        # por defecto, el 9044
#
# Genera una CA propia y un certificado de servidor con `IP:127.0.0.1` y `DNS:localhost`, firmado por ella;
# arranca `cassandra:5` con `client_encryption_options` encendido y SIN permitir conexiones sin TLS; y
# escribe lo que `cassandra.sh` necesita:
#
#   export CASSANDRA_TLS_ADMIN_URL='cassandra://cassandra:cassandra@127.0.0.1:9044/?contacto=solo&tls=true'
#   export CASSANDRA_TLS_CA=/ruta/a/ca.pem        # la CA en la que SÍ se confía
#
# El cliente confía en la CA por `SSL_CERT_FILE` (el almacén del sistema, más lo que diga esa variable); no hay
# otra forma de darle una CA a ORE, a propósito. `cassandra.sh` la pone solo en el entorno de lo que prueba.
#
# Para pararlo:  docker rm -f ore-cassandra-tls
set -eu
PUERTO="${1:-9044}"
NOMBRE=ore-cassandra-tls
OSSL="$(command -v openssl || true)"
[ -n "$OSSL" ] || OSSL="/c/Program Files/OpenSSL-Win64/bin/openssl.exe"
[ -x "$OSSL" ] || { echo "falta openssl"; exit 2; }
export MSYS_NO_PATHCONV=1

DIR="${TLS_LAB_DIR:-$(mktemp -d)}"
mkdir -p "$DIR"
cd "$DIR"
# Certificados como los de verdad: una CA que declara serlo y para qué sirve (Python 3.13 y OpenSSL con la
# comprobación estricta lo exigen), y una hoja con su uso, su SAN y quién la firmó.
CA_EXT=(-addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" -addext "subjectKeyIdentifier=hash")
"$OSSL" req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 30 -subj "/CN=ORE · CA de prueba" "${CA_EXT[@]}" 2>/dev/null
"$OSSL" req -x509 -newkey rsa:2048 -nodes -keyout otra.key -out otra.pem -days 30 -subj "/CN=ORE · otra CA (no de confianza)" "${CA_EXT[@]}" 2>/dev/null
"$OSSL" req -newkey rsa:2048 -nodes -keyout servidor.key -out servidor.csr -subj "/CN=localhost" 2>/dev/null
printf 'subjectAltName=IP:127.0.0.1,IP:::1,DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nauthorityKeyIdentifier=keyid\nsubjectKeyIdentifier=hash\n' > servidor.ext
"$OSSL" x509 -req -in servidor.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out servidor.pem -days 30 -extfile servidor.ext 2>/dev/null
"$OSSL" pkcs12 -export -in servidor.pem -inkey servidor.key -certfile ca.pem -name cassandra -out keystore.p12 -passout pass:ks-pass 2>/dev/null

# Docker en Windows quiere la ruta de Windows para montar.
MONTAJE="$DIR"
command -v cygpath >/dev/null 2>&1 && MONTAJE="$(cygpath -m "$DIR")"

docker rm -f "$NOMBRE" >/dev/null 2>&1 || true
docker run -d --name "$NOMBRE" -p "$PUERTO:9042" -e MAX_HEAP_SIZE=1G -e HEAP_NEWSIZE=256M \
  -v "$MONTAJE:/certs:ro" --entrypoint bash cassandra:5 -c "
    sed -i 's/^authenticator:.*/authenticator: PasswordAuthenticator/; s/^authorizer:.*/authorizer: CassandraAuthorizer/' /etc/cassandra/cassandra.yaml
    sed -i '/^client_encryption_options:/,/^[a-z_]*:/ {
      s/^  enabled: false/  enabled: true\n  optional: false\n  require_client_auth: false\n  protocol: TLS\n  accepted_protocols: [TLSv1.2, TLSv1.3]\n  store_type: PKCS12\n  keystore_password: ks-pass/
      s#^  keystore: conf/.keystore#  keystore: /certs/keystore.p12#
    }' /etc/cassandra/cassandra.yaml
    exec docker-entrypoint.sh cassandra -f" >/dev/null

echo "arrancando el Cassandra con TLS (tarda ~40 s)…" >&2
for _ in $(seq 1 60); do
  docker exec -e SSL_VALIDATE=false "$NOMBRE" cqlsh --ssl -u cassandra -p cassandra -e "select now() from system.local" >/dev/null 2>&1 && break
  sleep 5
done
docker exec -e SSL_VALIDATE=false "$NOMBRE" cqlsh --ssl -u cassandra -p cassandra -e "select now() from system.local" >/dev/null 2>&1 \
  || { echo "el Cassandra con TLS no arrancó: docker logs $NOMBRE" >&2; exit 1; }

echo "export CASSANDRA_TLS_ADMIN_URL='cassandra://cassandra:cassandra@127.0.0.1:$PUERTO/?contacto=solo&tls=true'"
echo "export CASSANDRA_TLS_CA='$(cygpath -m "$DIR/ca.pem" 2>/dev/null || echo "$DIR/ca.pem")'"
echo "export CASSANDRA_TLS_OTRA_CA='$(cygpath -m "$DIR/otra.pem" 2>/dev/null || echo "$DIR/otra.pem")'"
