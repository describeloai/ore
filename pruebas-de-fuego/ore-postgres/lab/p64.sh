#!/usr/bin/env bash
# P6·4 en el laboratorio (ADR 0058): despertar al conectar. Con el reconciliador de verdad, dormir_tras 60 s,
# y tres ciclos de dormir y despertar:
#   1 · por TCP: un dormido despierta al conectar, los datos de antes de dormir siguen ahí, y las CU que se
#       cambiaron dormido valen al despertar (0,25 CU → max_connections 112);
#   2 · 20 conexiones a la vez contra uno dormido: entran todas y es UN despertar (una operación);
#   3 · por el pool (-pooler), por HTTP y por WebSocket (@neondatabase/serverless): despiertan igual.
# El cliente no ve más error que la espera.
#
#   lab.sh arriba && p64.sh        (unos 5 min: tres veces dormir de verdad)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
P="p64-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p64\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
echo "── $P ($VM), dormir_tras 60 s"
sql() { local h=${H:-$VM} p=${PUERTO_SQL:-4432}
  dc exec -T -e PGPASSWORD="$CLAVE" -e PGCONNECT_TIMEOUT=60 cliente psql \
  "host=$h.$DOMINIO hostaddr=172.29.51.20 port=$p user=$ROL dbname=$P sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" "$@"; }
observado() { campo "$(pide demo GET "$E")" estado observado; }
dormir() { local t0; t0=$(date +%s)
  until [ "$(observado)" = dormido ] || [ $(( $(date +%s) - t0 )) -ge 150 ]; do sleep 2; done
  [ "$(observado)" = dormido ] && [ -z "$(docker ps -aq --filter name="$VM")" ]; }
despertares() { sql_plano "select count(*) from plano.operacion where tipo = 'despertar-endpoint' and endpoint = 'principal' and proyecto = '$P'"; }
ms() { date +%s%3N; }

sql -Atqc "create table t as select g from generate_series(1, 10000) g" && bien "10 000 filas escritas"
[ "$(en_vm "$VM" 'show max_connections')" = 450 ] && bien "nació con 1 CU: max_connections 450"
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"60","cu_max":"0.25"}')")" = hecha ] && bien "ajustes: dormir_tras 60, cu_max 0,25 (para el siguiente arranque)"

echo "── 1 · dormido → TCP"
dormir && bien "dormido, sin cómputo" || mal "no se durmió"
t0=$(ms); n=$(sql -Atqc "select count(*) from t" 2>&1 | tail -1); t=$(( $(ms) - t0 ))
[ "$n" = 10000 ] && bien "psql a uno dormido: despierta y contesta en ${t} ms, con las 10 000 filas de antes de dormir" || mal "psql: ${n:0:140}"
[ "$(en_vm "$VM" 'show max_connections')" = 112 ] && bien "al despertar, las CU de los ajustes: max_connections 112" || mal "max_connections al despertar: $(en_vm "$VM" 'show max_connections')"
[ "$(despertares)" = 1 ] && bien "un despertar (operación despertar-endpoint, hecha)" || mal "despertares: $(despertares)"

echo "── 2 · dormido → 20 conexiones a la vez"
dormir && bien "dormido otra vez" || mal "no se durmió"
T=$(mktemp -d); t0=$(ms)
for i in $(seq 1 20); do sql -Atqc "select count(*) from t" > "$T/$i" 2>&1 & done; wait; t=$(( $(ms) - t0 ))
ok=$(cat "$T"/* | grep -cx 10000); rm -rf "$T"
[ "$ok" = 20 ] && bien "las 20 entran (todas en ${t} ms): el cliente no ve más que la espera" || mal "entran $ok de 20"
[ "$(despertares)" = 2 ] && bien "y fue UN despertar (2 en total con el anterior)" || mal "despertares: $(despertares) (esperaba 2)"

echo "── 3 · dormido → pool, HTTP y WebSocket"
dormir && bien "dormido otra vez" || mal "no se durmió"
t0=$(ms); n=$(H="$VM-pooler" sql -Atqc "select count(*) from t" 2>&1 | tail -1); t=$(( $(ms) - t0 ))
[ "$n" = 10000 ] && bien "por -pooler despierta y contesta en ${t} ms" || mal "pool: ${n:0:140}"
nodo() { docker run --rm --network p5lab_lab --add-host "$VM.$DOMINIO:172.29.51.20" --add-host "api.$DOMINIO:172.29.51.20" \
  -e NODE_EXTRA_CA_CERTS=/llaves/tls/tls.crt -v "$LAB/.secretos/tls:/llaves/tls:ro" -v "$LAB/p64.mjs:/w/p64.mjs:ro" \
  -v p5lab-npm:/w/node_modules -w /w node:22-slim sh -c '[ -d node_modules/@neondatabase/serverless ] || npm i -s --no-save --no-package-lock @neondatabase/serverless@1.1.0 ws@8.18.3 >/dev/null 2>&1; node p64.mjs "$@"' \
  -- "$ROL" "$CLAVE" "$P" "$VM" "$1"; }
docker volume create p5lab-npm >/dev/null
dormir && bien "dormido otra vez" || mal "no se durmió"
read -r via n t < <(nodo http); [ "$n" = 10000 ] && bien "por HTTP (neon()) despierta y contesta en ${t} ms" || mal "HTTP: $via $n $t"
dormir && bien "dormido otra vez" || mal "no se durmió"
read -r via n t < <(nodo ws); [ "$n" = 10000 ] && bien "por WebSocket (Pool) despierta y contesta en ${t} ms" || mal "WebSocket: $via $n $t"

hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P6·4 (laboratorio) ✓ todo" || { echo "P6·4 (laboratorio) ✗ $fallos fallos"; exit 1; }
