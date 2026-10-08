#!/usr/bin/env bash
# P4·1 · el contrato y el esqueleto (ADR 0058). Hecho cuando: una celda crea y lee un proyecto;
# otra celda no lo ve. Y P4·2·1: crear un proyecto crea su tenant DE VERDAD (se ve `Active` en el
# pageserver), y borrarlo lo quita.
#
# Con dos celdas DE VERDAD, cada una con su token: un pod de usar y tirar en su namespace con la
# cuenta de su `ore-serve` (`ore.dev/rol: control`), que pide al servidor de metadatos su token de
# Workload Identity con audiencia `ore-postgres` y llama a `ore-postgres`. El token no sale del pod.
#
#   p41.sh [celda-a] [celda-b]       (por defecto demo y victor: dos organizaciones distintas)
#
# Lo que el pod necesita para salir de su celda hacia `ore-pg` (la salida de `ore-serve` hacia el
# producto es P4·6) va en una NetworkPolicy de la prueba, sólo para sus pods, y se borra al acabar.
export ORE_PG_COMPUTO=pod
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}; B=${2:-victor}
P="p41-$(date +%s | tail -c 6)"
PRUEBA=p41
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A" "$B"

echo "── $A crea el proyecto $P y lo lee"
mapfile -t R < <(en "$A" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/proyectos" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'" \
  "pide POST /v1/postgres/proyectos '{\"id\":\"Mal_Id\"}'")
espera 202 "crear" "${R[0]}"
OP=$(campo "${R[0]}" operacion id); HECHA=$(campo "${R[0]}" operacion hecha); CELDA=$(campo "${R[0]}" proyecto celda)
TENANT=$(campo "${R[0]}" proyecto tenant)
[ "$HECHA" = False ] && echo "  ✓ la operación $OP nace en curso (tenant $TENANT)" || { echo "  ✗ operación: ${R[0]}"; fallos=$((fallos+1)); }
espera 200 "leerlo" "${R[1]}"
case "${R[2]}" in *"\"$P\""*) echo "  ✓ está en su lista";; *) echo "  ✗ no está en su lista: ${R[2]}"; fallos=$((fallos+1));; esac
espera 409 "crearlo otra vez no crea dos" "${R[3]}"
espera 400 "un id que no vale" "${R[4]}"
echo "    (celda $CELDA)"

echo "── el reconciliador crea el tenant y main en el almacenamiento"
T0=$(date +%s)
mapfile -t R < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET /v1/postgres/proyectos/$P")
[ "$(campo "${R[0]}" estado)" = hecha ] && echo "  ✓ operación hecha (≤ $(( $(date +%s)-T0 )) s, con el pod)" \
  || { echo "  ✗ la operación: ${R[0]}"; fallos=$((fallos+1)); }
[ "$(campo "${R[1]}" estado observado)" = listo ] && echo "  ✓ el proyecto, listo" || { echo "  ✗ el proyecto: ${R[1]}"; fallos=$((fallos+1)); }
case "$(pageserver GET "/v1/tenant/$TENANT")" in *Active*) echo "  ✓ el tenant $TENANT está Active en el pageserver";;
  *) echo "  ✗ el pageserver no tiene el tenant activo: $(pageserver GET "/v1/tenant/$TENANT" | head -c 300)"; fallos=$((fallos+1));; esac

echo "── $B (otra organización) no lo ve"
mapfile -t R < <(en "$B" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/proyectos" \
  "pide DELETE /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/operaciones/$OP")
espera 404 "leerlo" "${R[0]}"
case "${R[1]}" in 200*) case "${R[1]}" in *"\"$P\""*) echo "  ✗ está en su lista: ${R[1]}"; fallos=$((fallos+1));; *) echo "  ✓ no está en su lista · 200";; esac;;
  *) echo "  ✗ su lista: ${R[1]}"; fallos=$((fallos+1));; esac
espera 404 "borrarlo" "${R[2]}"
espera 404 "ver la operación de $A" "${R[3]}"

echo "── sin un token de celda para ore-postgres, nada"
mapfile -t R < <(en "$A" \
  "pide_con TN GET /v1/postgres/proyectos" \
  "pide_con TB GET /v1/postgres/proyectos" \
  "pide_con TI GET /v1/postgres/proyectos")
espera 401 "sin token" "${R[0]}"
espera 401 "con la firma rota" "${R[1]}"
espera 401 "con el token de la celda para ore-iam (otra audiencia)" "${R[2]}"

echo "── $A lo borra"
mapfile -t R < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
espera 202 "borrar" "${R[0]}"
OPB=$(campo "${R[0]}" operacion id)
mapfile -t R < <(en "$A" \
  "hasta_hecha /v1/postgres/operaciones/$OPB" \
  "pide GET /v1/postgres/proyectos/$P" \
  "pide GET /v1/postgres/operaciones/$OP")
[ "$(campo "${R[0]}" estado)" = hecha ] && echo "  ✓ borrado hecho" || { echo "  ✗ el borrado: ${R[0]}"; fallos=$((fallos+1)); }
espera 404 "ya no está" "${R[1]}"
espera 200 "la operación de crear sigue ahí" "${R[2]}"
case "$(pageserver GET "/v1/tenant/$TENANT")" in *Active*) echo "  ✗ el tenant sigue en el pageserver"; fallos=$((fallos+1));;
  *) echo "  ✓ el tenant ya no está en el pageserver";; esac

echo
[ $fallos = 0 ] && echo "P4·1 ✓ todo" || { echo "P4·1 ✗ $fallos fallos"; exit 1; }
