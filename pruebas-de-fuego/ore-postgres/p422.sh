#!/usr/bin/env bash
# P4·2·2 · el reconciliador retoma lo que estaba en curso (ADR 0058). Hecho cuando: se reinicia
# `ore-postgres` a mitad de una operación y la retoma SIN crear dos tenants.
#
# Para que «a mitad» sea seguro y no cuestión de suerte, se le corta a `ore-postgres` la salida hacia
# el storage_controller (se quita esa regla de su NetworkPolicy `salida-del-plano`): la operación
# queda reintentando, con su error y su tenant ya nombrado. Entonces se mata el pod, se devuelve la
# salida (la del manifiesto) y se mira que el pod nuevo la termine y que en el pageserver haya
# exactamente UN tenant más (el suyo). Después se borra y se mira que vuelva a haber los de antes.
#
#   p422.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=pod
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p422-$(date +%s | tail -c 6)"
PRUEBA=p422
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
MANIFIESTO="$AQUI/../../malla/86-postgres-el-plano-de-control.yaml"
devolver() { kubectl apply -f "$MANIFIESTO" >/dev/null; }
trap 'devolver; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT
tenants() { pageserver GET /v1/tenant | python -c 'import json,sys; print(len(json.load(sys.stdin)))'; }

ANTES=$(tenants); echo "── en el pageserver hay $ANTES tenants"

echo "── se corta la salida de ore-postgres hacia el storage_controller"
k get networkpolicy salida-del-plano -o json | python -c '
import json,sys
p=json.load(sys.stdin)
p["spec"]["egress"]=[r for r in p["spec"]["egress"]
  if not any(t.get("podSelector",{}).get("matchLabels",{}).get("ore.dev/rol")=="storage-controller" for t in r.get("to",[]))]
for k in ("resourceVersion","uid","creationTimestamp","managedFields","generation"): p["metadata"].pop(k,None)
print(json.dumps(p))' | kubectl replace -f - >/dev/null
sleep 30   # Cilium tarda 15–30 s en aplicar un cambio de política (P3·4)

echo "── $A crea $P; la operación no puede terminar"
mapfile -t R < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
espera 202 "crear" "${R[0]}"
OP=$(campo "${R[0]}" operacion id); TENANT=$(campo "${R[0]}" proyecto tenant)
mapfile -t R < <(en "$A" "sleep 20; pide GET /v1/postgres/operaciones/$OP")
EST=$(campo "${R[0]}" estado); ERR=$(campo "${R[0]}" error)
if [ "$EST" = en-curso ] && [ -n "$ERR" ]; then echo "  ✓ en curso, reintentando: ${ERR:0:120}"
else echo "  ✗ no quedó a medias (estado $EST): ${R[0]:0:300}"; fallos=$((fallos+1)); fi

echo "── se mata ore-postgres, se devuelve la salida, y el pod nuevo la retoma"
T0=$(date +%s)
k delete pod -l ore.dev/rol=plano-postgres --wait=true >/dev/null
devolver
k rollout status deploy/ore-postgres --timeout=180s >/dev/null && echo "  · pod nuevo listo a los $(( $(date +%s)-T0 )) s"
mapfile -t R < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET /v1/postgres/proyectos/$P")
[ "$(campo "${R[0]}" estado)" = hecha ] && echo "  ✓ hecha a los $(( $(date +%s)-T0 )) s del reinicio" \
  || { echo "  ✗ la operación: ${R[0]:0:300}"; fallos=$((fallos+1)); }
[ "$(campo "${R[1]}" estado observado)" = listo ] && echo "  ✓ el proyecto, listo" || { echo "  ✗ el proyecto: ${R[1]:0:300}"; fallos=$((fallos+1)); }
case "$(pageserver GET "/v1/tenant/$TENANT")" in *Active*) echo "  ✓ su tenant $TENANT, Active";;
  *) echo "  ✗ su tenant no está activo"; fallos=$((fallos+1));; esac
DESPUES=$(tenants)
[ "$DESPUES" = $((ANTES+1)) ] && echo "  ✓ UN tenant más ($ANTES → $DESPUES)" || { echo "  ✗ $ANTES → $DESPUES tenants"; fallos=$((fallos+1)); }

echo "── se borra y quedan los de antes"
mapfile -t R < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OPB=$(campo "${R[0]}" operacion id)
mapfile -t R < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OPB")
[ "$(campo "${R[0]}" estado)" = hecha ] && echo "  ✓ borrado" || { echo "  ✗ el borrado: ${R[0]:0:300}"; fallos=$((fallos+1)); }
FIN=$(tenants)
[ "$FIN" = "$ANTES" ] && echo "  ✓ $FIN tenants, como al principio" || { echo "  ✗ $FIN tenants (eran $ANTES)"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·2·2 ✓ todo" || { echo "P4·2·2 ✗ $fallos fallos"; exit 1; }
