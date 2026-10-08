#!/usr/bin/env bash
# P4·6 · La API de ORE (ADR 0058). Hecho cuando: una persona crea proyecto, rama, endpoint y rol CON SU
# TOKEN DE ORE —por la entrada pública de su celda, `https://<celda>.ore.paladio.io/v1/postgres/…`— y se
# conecta desde la malla con la contraseña que le dio el API; el dueño del proyecto es ella (lo pone ORE).
#
# Lo que no se repite aquí lo fija el banco del CI (`pruebas-de-fuego/postgres-por-la-celda.sh`): sin la
# potestad 403, desde un puesto 403, sin `ore-iam` 503. En vivo, además: con `ore-postgres` parado, la
# gestión da 503 y la base sigue sirviendo.
#
#   ORE_TOKEN=<el token de acceso de la persona> p46.sh [celda]      (demo por defecto)
#
# ⛔ El token lo pone quien corre la prueba; no se imprime ni se guarda. La contraseña del rol, tampoco.
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
[ -n "${ORE_TOKEN:-}" ] || { echo "✗ falta ORE_TOKEN (el token de acceso de una persona de $A)"; exit 2; }
BASE="https://$A.ore.paladio.io/v1/postgres"
P="p46-$(date +%s | tail -c 6)"
R="/proyectos/$P/ramas"
fallos=0
# api METODO CAMINO [CUERPO] → "CODIGO cuerpo", como `pide` de celdas.sh
api46() { local f; f=$(mktemp)
  local c; c=$(curl -s -o "$f" -w '%{http_code}' -X "$1" -H "authorization: Bearer $ORE_TOKEN" \
    -H 'content-type: application/json' ${3:+-d "$3"} "$BASE$2")
  echo "$c $(tr -d '\n' < "$f")"; rm -f "$f"; }
campo() { python -c 'import json,sys; d=json.loads(sys.argv[1].split(" ",1)[1]); [d:=d[k] for k in sys.argv[2:]]; print(d)' "$1" "${@:2}" 2>/dev/null; }
hecha() { local l; for _ in $(seq 1 300); do l=$(api46 GET "/operaciones/$1"); case "$(campo "$l" estado)" in hecha|fallida) break;; esac; sleep 2; done; campo "$l" estado; }
como() { k exec cliente-overlay -- env PGPASSWORD="$2" PGCONNECT_TIMEOUT=5 \
  psql -h "$4" -p 55433 -U "$1" -d "$3" -Atc "$5" 2>&1 | tail -1; }
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }

echo "── la persona crea $P por https://$A.ore.paladio.io, con su token"
L=$(api46 POST /proyectos "{\"id\":\"$P\",\"dueno\":\"user:otra\"}")
[ "${L%% *}" = 202 ] || { echo "  ✗ crear: ${L:0:300}"; exit 1; }
DUENO=$(campo "$L" proyecto dueno); ROL=$(campo "$L" rol nombre); CLAVE=$(campo "$L" rol contrasena)
[ -n "$DUENO" ] && [ "$DUENO" != user:otra ] && bien "202; su dueño es $DUENO (lo pone ORE, no el cuerpo)" || mal "el dueño: «$DUENO»"
[ "$(hecha "$(campo "$L" operacion id)")" = hecha ] && bien "proyecto listo" || { mal "el proyecto no acaba"; exit 1; }

echo "── una rama, un endpoint en ella y un rol, todo con el mismo token"
L=$(api46 POST "$R" '{"id":"dev"}'); [ "${L%% *}" = 202 ] && [ "$(hecha "$(campo "$L" operacion id)")" = hecha ] \
  && bien "rama dev" || mal "rama: ${L:0:300}"
L=$(api46 POST "$R/dev/endpoints" '{"id":"uno"}'); [ "${L%% *}" = 202 ] && [ "$(hecha "$(campo "$L" operacion id)")" = hecha ] \
  && bien "endpoint uno en dev" || mal "endpoint: ${L:0:300}"
L=$(api46 POST "$R/dev/roles" '{"nombre":"app"}'); CLAVE_APP=$(campo "$L" rol contrasena)
[ "${L%% *}" = 202 ] && [ "$(hecha "$(campo "$L" operacion id)")" = hecha ] && bien "rol app en dev" || mal "rol: ${L:0:300}"
DIR=$(campo "$(api46 GET "$R/dev/endpoints/uno")" direccion)

echo "── se conecta desde la malla con la contraseña que le dio el API"
[ "$(como app "$CLAVE_APP" postgres "$DIR" 'select current_user')" = app ] && bien "app entra en dev ($DIR)" \
  || mal "app no entra: $(como app "$CLAVE_APP" postgres "$DIR" 'select 1')"
[ "$(como "$ROL" "$CLAVE" "$P" "$DIR" 'select current_user')" = "$ROL" ] && bien "y su rol $ROL, heredado por dev, en su base $P" \
  || mal "$ROL no entra en dev: $(como "$ROL" "$CLAVE" "$P" "$DIR" 'select 1')"

echo "── con ore-postgres parado: la gestión 503, la base sigue"
k scale deploy/ore-postgres --replicas=0 >/dev/null; k wait --for=delete pod -l ore.dev/rol=plano-postgres --timeout=120s >/dev/null 2>&1
L=$(api46 GET /proyectos); [ "${L%% *}" = 503 ] && bien "gestión: 503" || mal "sin ore-postgres: ${L:0:200}"
[ "$(como app "$CLAVE_APP" postgres "$DIR" 'select 1')" = 1 ] && bien "la base sigue sirviendo" || mal "la base no sirve sin el plano"
k scale deploy/ore-postgres --replicas=1 >/dev/null; k rollout status deploy/ore-postgres --timeout=180s >/dev/null
for _ in $(seq 1 30); do [ "$(api46 GET /proyectos | cut -c1-3)" = 200 ] && break; sleep 2; done

echo "── se borra"
L=$(api46 DELETE "/proyectos/$P"); [ "${L%% *}" = 202 ] && [ "$(hecha "$(campo "$L" operacion id)")" = hecha ] \
  && bien "borrado" || mal "borrar: ${L:0:300}"

echo
[ $fallos = 0 ] && echo "P4·6 ✓ todo" || { echo "P4·6 ✗ $fallos fallos"; exit 1; }
