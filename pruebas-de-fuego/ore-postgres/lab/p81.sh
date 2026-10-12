#!/usr/bin/env bash
# P8·1 en el laboratorio (ADR 0058): el SQL de la consola en el plano, contra un cómputo de verdad
# (Postgres 17 en un contenedor) y con el reconciliador de verdad.
#   · la contraseña del rol, guardada cifrada y visible (`GET …/roles/{n}/contrasena`);
#   · `POST …/ramas/main/sql`: crear, insertar y leer como el rol; el error con su posición; el
#     tope de filas; solo lectura; otra organización no entra;
#   · dormido, una consulta lo despierta (con su despertar_ms) y contesta;
#   · un resultado enorme tumba al hijo, no al plano.
#
#   SIN_PROXY=1 lab.sh arriba && p81.sh        (unos 3 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }

P="p81-$(date +%s | tail -c 6)"
R="/v1/postgres/proyectos/$P/ramas/main"
sql() {   # sql CELDA JSON-EXTRA → la respuesta de …/sql (base = el proyecto, rol = p81)
  pide "$1" POST "$R/sql" "{\"base\":\"$P\",\"rol\":\"p81\",$2}"
}
j() { "$PY" -c 'import json,sys; print(eval(sys.argv[2], {}, {"r": json.loads(sys.argv[1])}))' "$1" "$2" 2>/dev/null; }

echo "── la instancia y su contraseña"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p81\"}")
CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(campo "$(pide demo GET "$R/roles/p81/contrasena")" contrasena)" = "$CLAVE" ] \
  && bien "la contraseña guardada es la de crearla" || mal "la contraseña guardada no casa"
g=$(sql_plano "select contrasena_cifrada from plano.rol where proyecto = '$P' and nombre = 'p81'")
case "$g" in v1:*) [[ "$g" != *"$CLAVE"* ]] && bien "en la base, cifrada (v1:…)" || mal "en claro";; *) mal "no se guardó: $g";; esac

echo "── SQL como el rol"
r=$(sql demo '"sql":"create table cuenta (id int primary key, nombre text); insert into cuenta values (1, '"'"'ana'"'"'), (2, null); select id, nombre, current_user from cuenta order by id"')
[ "$(j "$r" 'r["ok"]')" = True ] && [ "$(j "$r" 'len(r["sentencias"])')" = 3 ] \
  && bien "tres sentencias en $(j "$r" 'r["ms"]') ms" || mal "las tres sentencias: ${r:0:300}"
[ "$(j "$r" 'r["sentencias"][2]["filas"]')" = "[['1', 'ana', 'p81'], ['2', None, 'p81']]" ] \
  && bien "las filas, con null de verdad, y current_user = p81" || mal "las filas: $(j "$r" 'r["sentencias"][2]')"
[ "$(docker exec -i "$VM" gosu postgres psql -h /tmp -U cloud_admin -d "$P" -Atqc 'select count(*) from cuenta')" = 2 ]   && bien "la tabla está en el cómputo, en la base $P" || mal "la tabla no está en el cómputo"
r=$(sql demo '"sql":"select 1 frm x"')
[ "$(j "$r" 'r["error"]["sqlstate"]')" = 42601 ] && [ "$(j "$r" 'r["error"]["posicion"]')" = 14 ] \
  && bien "el error de Postgres, con su posición (42601 en 14)" || mal "el error: ${r:0:300}"
r=$(sql demo '"sql":"select generate_series(1, 5000)","limite_filas":100')
[ "$(j "$r" 'len(r["sentencias"][0]["filas"])')" = 100 ] && [ "$(j "$r" 'r["sentencias"][0]["truncada"]')" = True ] \
  && bien "el tope: 100 de 5000, truncada" || mal "el tope: ${r:0:200}"
r=$(sql demo '"sql":"insert into cuenta values (3, null)","solo_lectura":true')
[ "$(j "$r" 'r["error"]["sqlstate"]')" = 25006 ] && bien "solo lectura: insertar da 25006" || mal "solo lectura: ${r:0:200}"
r=$(sql victor '"sql":"select 1"')
case "$r" in *"no hay ning"*) bien "otra organización: no ve el proyecto";; *) mal "otra organización: ${r:0:200}";; esac
r=$(sql demo '"sql":"select repeat('"'"'x'"'"', 1000000) from generate_series(1, 200)"')
case "$r" in *"no cabe"*) bien "~200 MB de resultado: el hijo cae, el plano contesta";; *) mal "el resultado enorme: ${r:0:200}";; esac
[ "$(j "$(sql demo '"sql":"select 1"')" 'r["ok"]')" = True ] && bien "y la siguiente consulta, bien" || mal "el plano no contesta tras el hijo"

echo "── dormido, la consulta lo despierta"
[ "$(hecha demo "$(pide demo POST "$R/endpoints/principal/ajustes" '{"dormir_tras":"60"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
T0=$(date +%s)
until [ "$(sql_plano "select observado from plano.endpoint where proyecto = '$P'")" = dormido ] || [ $(( $(date +%s) - T0 )) -ge 180 ]; do sleep 2; done
[ "$(sql_plano "select observado from plano.endpoint where proyecto = '$P'")" = dormido ] \
  && bien "dormido a los $(( $(date +%s) - T0 )) s" || mal "no se durmió"
r=$(sql demo '"sql":"select count(*) from cuenta"')
[ "$(j "$r" 'r["sentencias"][0]["filas"]')" = "[['2']]" ] \
  && bien "despertó y contestó: despertar_ms $(j "$r" 'r["despertar_ms"]'), sql $(j "$r" 'r["ms"]') ms; los datos siguen ahí" \
  || mal "dormido: ${r:0:300}"

echo "── apuntado, sin el texto"
n=$(sql_plano "select count(*) || ' · ' || count(error) from plano.sql_ejecutado where proyecto = '$P'")
echo "  · $n (ejecutadas · con error)"
pide demo DELETE "/v1/postgres/proyectos/$P" >/dev/null
echo
[ $fallos = 0 ] && echo "P8·1 (laboratorio) ✓" || { echo "P8·1 (laboratorio) ✗ $fallos fallos"; exit 1; }
