#!/usr/bin/env bash
# P4·2·3 · las ramas (ADR 0058). Hecho cuando: dos ramas, una de la punta y otra de un instante, creadas
# por el API de `ore-postgres` como una celda; lo escrito en una no se ve en la otra.
#
# Los cómputos aún no los crea el API (P4·3): se levantan con el arnés de P2/P3 (`vm.sh`, en pod) sobre
# los timelines que el API dio, y se miran con `q`.
#
#   p423.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=pod
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p423-$(date +%s | tail -c 6)"
PRUEBA=p423
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
R="/v1/postgres/proyectos/$P/ramas"
computo() {   # computo NOMBRE TIMELINE → la IP de su pod
  ORE_PG_VM=$1 bash "$AQUI/vm.sh" "$2" >/dev/null || { echo "✗ el cómputo $1 no arranca" >&2; return 1; }
  leer "pod-$1"
}
cuenta() { q "$1" 'select count(*) from p423'; }
limpiar() { for v in p423-main p423-punta p423-antes; do k delete pod "$v" --ignore-not-found --wait=false >/dev/null; \
  k delete configmap "$v-config" --ignore-not-found >/dev/null; done; }
trap 'limpiar; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT

echo "── $A crea $P"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
espera 202 "crear" "${L[0]}"
OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/main")
MAIN=$(campo "${L[1]}" timeline)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ tenant $TENANT · main $MAIN" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }
guardar tenant "$TENANT"

echo "── en main: 100 filas, el instante, 100 más"
IM=$(computo p423-main "$MAIN") || exit 1
q "$IM" 'create table p423 (n int, cuando timestamptz default clock_timestamp()); insert into p423 select generate_series(1,100)' >/dev/null
sleep 3; INSTANTE=$(q "$IM" "select to_char(now() at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')"); sleep 3
q "$IM" 'insert into p423 select generate_series(101,200)' >/dev/null
echo "  · main: $(cuenta "$IM") filas · instante $INSTANTE"
sleep 5   # que el pageserver haya recibido el WAL de las dos

echo "── dos ramas por el API: de la punta y del instante"
mapfile -t L < <(en "$A" "pide POST $R '{\"id\":\"punta\"}'")
espera 202 "rama punta" "${L[0]}"; OP=$(campo "${L[0]}" operacion id); TL_PUNTA=$(campo "${L[0]}" rama timeline)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" \
  "pide POST $R '{\"id\":\"antes\",\"instante\":\"$INSTANTE\"}'")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ punta, lista" || { echo "  ✗ punta: ${L[0]:0:300}"; fallos=$((fallos+1)); }
espera 202 "rama antes" "${L[1]}"; OP=$(campo "${L[1]}" operacion id); TL_ANTES=$(campo "${L[1]}" rama timeline)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/antes")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ antes, lista: el instante dio el LSN $(campo "${L[1]}" origen lsn)" \
  || { echo "  ✗ antes: ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo "── cada rama ve lo que tenía en su origen"
IP=$(computo p423-punta "$TL_PUNTA"); IA=$(computo p423-antes "$TL_ANTES")
[ "$(cuenta "$IP")" = 200 ] && echo "  ✓ punta: 200" || { echo "  ✗ punta: $(cuenta "$IP")"; fallos=$((fallos+1)); }
[ "$(cuenta "$IA")" = 100 ] && echo "  ✓ antes: 100 (lo de después del instante no está)" || { echo "  ✗ antes: $(cuenta "$IA")"; fallos=$((fallos+1)); }

echo "── lo escrito en una no se ve en las otras"
q "$IP" 'insert into p423 select generate_series(1,50)' >/dev/null
q "$IM" 'insert into p423 select generate_series(1,7)' >/dev/null
[ "$(cuenta "$IP")" = 250 ] && [ "$(cuenta "$IM")" = 207 ] && [ "$(cuenta "$IA")" = 100 ] \
  && echo "  ✓ punta 250 · main 207 · antes 100" \
  || { echo "  ✗ punta $(cuenta "$IP") · main $(cuenta "$IM") · antes $(cuenta "$IA")"; fallos=$((fallos+1)); }

echo "── borrar: main no, y luego todo"
limpiar
mapfile -t L < <(en "$A" "pide DELETE $R/main" "pide DELETE $R/antes")
espera 409 "borrar main" "${L[0]}"
espera 202 "borrar antes" "${L[1]}"; OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide DELETE /v1/postgres/proyectos/$P")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ antes, borrada" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }
case "$(pageserver GET "/v1/tenant/$TENANT/timeline/$TL_ANTES")" in *NotFound*|*"not found"*|"") echo "  ✓ su timeline ya no está en el pageserver";;
  *) echo "  ✗ el timeline sigue en el pageserver"; fallos=$((fallos+1));; esac
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ el proyecto, borrado" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·2·3 ✓ todo" || { echo "P4·2·3 ✗ $fallos fallos"; exit 1; }
