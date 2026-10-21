#!/bin/bash
# Un Phoenix Query Server con TLS de verdad (0071 M6): una CA propia, un certificado de servidor con
# DNS:localhost e IP:127.0.0.1, y el PQS sirviendo HTTPS en el 8766. El 8765 del laboratorio normal no se toca.
#
#   docker build -t ore-phoenix-lab pruebas-de-fuego/phoenix         # una vez
#   bash pruebas-de-fuego/phoenix/tls-lab.sh [carpeta]               # por defecto /tmp/phx-tls
#
# Deja en <carpeta>/ca.pem la CA que hay que declarar al cliente (el conector del sistema no la conoce) y el
# contenedor `phx-tls`. Para parar: docker rm -f phx-tls.
#
# BASIC y DIGEST no se activan aqui: el PQS 6.0.0 no tiene ninguna propiedad que los encienda (su
# BasicAuthenticationServerCustomizer es de pruebas, con un usuario fijo, y no esta conectado). En produccion
# la autenticacion la pone un proxy delante, o Kerberos/SPNEGO, que queda fuera de la v1 (0071 7).
set -euo pipefail
export MSYS_NO_PATHCONV=1
IMAGEN=${IMAGEN:-ore-phoenix-lab}
DIR=${1:-/tmp/phx-tls}
CLAVE=ore-tls-lab
mkdir -p "$DIR"
# En Git Bash para Windows docker quiere una ruta de Windows.
if command -v cygpath >/dev/null 2>&1; then MONTA=$(cygpath -w "$DIR"); else MONTA=$DIR; fi
rm -f "$DIR"/*.p12 "$DIR"/*.pem "$DIR"/*.crt "$DIR"/*.csr "$DIR"/hbase-site.xml

# 1. La CA, el certificado del servidor firmado por ella y su almacen de claves (keytool, dentro de la imagen).
docker run --rm -v "$MONTA:/certs" --entrypoint bash "$IMAGEN" -c '
set -e
cd /certs
K="keytool -storepass '"$CLAVE"' -storetype PKCS12"
$K -genkeypair -alias ca -keyalg EC -groupname secp256r1 -dname "CN=CA del laboratorio Phoenix" \
   -ext bc:c -ext ku:c=keyCertSign,cRLSign -validity 30 -keystore ca.p12 2>/dev/null
$K -genkeypair -alias srv -keyalg EC -groupname secp256r1 -dname "CN=servidor" -validity 30 -keystore srv.p12 2>/dev/null
$K -certreq -alias srv -keystore srv.p12 -file srv.csr 2>/dev/null
$K -gencert -alias ca -keystore ca.p12 -infile srv.csr -outfile srv.crt -rfc -validity 30 \
   -ext san=dns:localhost,ip:127.0.0.1 -ext eku=serverAuth -ext ku=digitalSignature 2>/dev/null
$K -exportcert -alias ca -keystore ca.p12 -rfc -file ca.pem 2>/dev/null
$K -importcert -alias ca -file ca.pem -keystore srv.p12 -noprompt 2>/dev/null
$K -importcert -alias srv -file srv.crt -keystore srv.p12 2>/dev/null
chmod a+r /certs/*
'

# 2. El hbase-site.xml del laboratorio con las propiedades de TLS del PQS.
sed 's#</configuration>##' "$(dirname "$0")/hbase-site.xml" > "$DIR/hbase-site.xml"
cat >> "$DIR/hbase-site.xml" <<EOF
  <property><name>phoenix.queryserver.tls.enabled</name><value>true</value></property>
  <property><name>phoenix.queryserver.tls.keystore</name><value>/certs/srv.p12</value></property>
  <property><name>phoenix.queryserver.tls.keystore.password</name><value>$CLAVE</value></property>
  <property><name>phoenix.queryserver.tls.truststore</name><value>/certs/srv.p12</value></property>
  <property><name>phoenix.queryserver.tls.truststore.password</name><value>$CLAVE</value></property>
</configuration>
EOF

# 3. El PQS con TLS en el 8766.
docker rm -f phx-tls >/dev/null 2>&1 || true
docker run -d --name phx-tls -p 8766:8765 \
  -v "$MONTA:/certs:ro" -v "$MONTA/hbase-site.xml:/opt/hbase/conf/hbase-site.xml:ro" "$IMAGEN" >/dev/null

# 4. Esperar a que conteste (la primera conexion crea SYSTEM.* y puede dar 500).
for i in $(seq 1 60); do
  c=$(curl -s --ssl-no-revoke -o /dev/null -w '%{http_code}' --cacert "$DIR/ca.pem" -X POST https://localhost:8766/ || true)
  if [ "$c" = "500" ] || [ "$c" = "200" ]; then echo "phx-tls responde por HTTPS (HTTP $c); CA en $DIR/ca.pem"; exit 0; fi
  if [ "$(docker inspect -f '{{.State.Running}}' phx-tls)" != "true" ]; then break; fi
  sleep 3
done
echo "phx-tls no contesta por HTTPS: docker logs phx-tls" >&2
exit 1
