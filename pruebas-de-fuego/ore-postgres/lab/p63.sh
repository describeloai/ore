#!/usr/bin/env bash
# P6·3 en el laboratorio (ADR 0058): dormir. Con el reconciliador de verdad y dormir_tras = 60 s:
#   · con una consulta en curso NO duerme (su last_active es ahora);
#   · al acabar, duerme a su hora AUNQUE haya una sesión ociosa abierta (como Neon: las ociosas no
#     son actividad, P6·0), y esa sesión se corta;
#   · dormido: no queda cómputo (coste 0), los datos siguen en el almacén y queda el LSN de /terminate.
# Despertar al conectar es P6·4: aquí, conectar a uno dormido todavía falla.
#
#   lab.sh arriba && p63.sh        (unos 3 min y medio: son tiempos de verdad)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
P="p63-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p63\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"60"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
echo "── $P ($VM), dormir_tras 60 s"
sql() { dc exec -T -e PGPASSWORD="$CLAVE" -e PGCONNECT_TIMEOUT=10 cliente psql \
  "host=$VM.$DOMINIO hostaddr=172.29.51.20 port=4432 user=$ROL dbname=$P sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" "$@"; }
observado() { campo "$(pide demo GET "$E")" estado observado; }
T=$(mktemp -d)

sql -Atqc "create table t as select g from generate_series(1, 10000) g" && bien "10 000 filas escritas por el proxy"

echo "── una consulta de 80 s en curso"
sql -Atqc "select pg_sleep(80)" > "$T/larga" 2>&1 &
LARGA=$!
sleep 72
[ "$(observado)" = listo ] && bien "a los 72 s sigue despierto: la consulta en curso es actividad" || mal "se durmió con una consulta en curso"
wait $LARGA; FIN=$(date +%s)

echo "── quieto, con una sesión ociosa abierta"
( echo "select 1;"; sleep 110; echo "select 'sigo';" ) | sql -At > "$T/ociosa" 2>&1 &
OCIOSA=$!
until [ "$(observado)" = dormido ] || [ $(( $(date +%s) - FIN )) -ge 120 ]; do sleep 2; done
t=$(( $(date +%s) - FIN ))
[ "$(observado)" = dormido ] && bien "dormido a los ${t} s de la última consulta (dormir_tras 60 + vigilar cada 5 + parar), con la sesión ociosa abierta" \
  || mal "a los ${t} s sigue $(observado)"
wait $OCIOSA
grep -q sigo "$T/ociosa" && mal "la sesión ociosa sigue viva tras dormir" \
  || bien "la sesión ociosa se cortó: $(grep -iE 'closed|terminat|fatal|error' "$T/ociosa" | head -1 | cut -c1-90)"

echo "── dormido"
[ -z "$(docker ps -aq --filter name="$VM")" ] && bien "no queda cómputo: coste 0" || mal "queda el contenedor $VM"
TENANT=$(campo "$(pide demo GET "/v1/postgres/proyectos/$P")" tenant)
TIMELINE=$(campo "$(pide demo GET "/v1/postgres/proyectos/$P/ramas/main")" timeline)
docker run --rm -v p5lab-almacen:/a alpine:3.20 test -f "/a/$TENANT/$TIMELINE/PG_VERSION" \
  && bien "los datos siguen en el almacén (/almacen/$TENANT/…)" || mal "no están los datos en el almacén"
LSN=$(dc exec -T base psql -U postgres -d ore_postgres -Atc "select lsn_al_dormir from plano.endpoint where vm = '$VM'")
[ -n "$LSN" ] && bien "/terminate dijo el LSN final: $LSN" || mal "sin LSN de /terminate"
ep=$(pide demo GET "$E"); [ -n "$(campo "$ep" dormido_en)" ] && [ -z "$(campo "$ep" direccion)" ] \
  && bien "la API: dormido desde $(campo "$ep" dormido_en), sin dirección" || mal "la API: ${ep:0:200}"
r=$(sql -Atqc "select count(*) from t" 2>&1 | tail -1)
[ "$r" = 10000 ] && echo "  · conectar ya lo despierta (¿P6·4 hecho?)" || echo "  · conectar a uno dormido todavía falla (despertar es P6·4): ${r:0:80}"

rm -rf "$T"
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P6·3 (laboratorio) ✓ todo" || { echo "P6·3 (laboratorio) ✗ $fallos fallos"; exit 1; }
