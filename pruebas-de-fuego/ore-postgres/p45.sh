#!/usr/bin/env bash
# P4·5 · los avisos del almacenamiento (ADR 0058). Hecho cuando: el `storage_controller` avisa a
# `ore-postgres` (no al stub `avisos`) y el cómputo se reconfigura solo, sin perder lo confirmado.
#
#   A · el aviso de verdad: con un escritor en marcha (una fila cada 0,2 s), se reinicia el pageserver;
#       se mira si el controller avisa, si ore-postgres reconfigura la VM, el hueco de escritura y si
#       falta alguna fila que el cómputo confirmó.
#   B · el aviso, provocado: el mismo PUT que haría el controller, desde su pod y con SU token infra.
#
# Con UN pageserver el tenant vuelve al mismo nodo: lo que se prueba es la cadena (aviso → ore-postgres
# → compute_ctl /configure), no el cambio de nodo. Dos pageservers son de P8.
#
#   p45.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p45-$(date +%s | tail -c 6)"
PRUEBA=p45
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
avisos_de() { k logs deploy/ore-postgres 2>/dev/null | grep -c "aviso · tenant $1: ep-"; }

echo "── $A crea $P"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET /v1/postgres/proyectos/$P/ramas/main/endpoints/principal")
DIR=$(campo "${L[1]}" direccion); VM=$(campo "${L[1]}" vm)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ $VM en $DIR · tenant $TENANT" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }
CP=$(kubectl -n ore-pg get deploy storage-controller -o jsonpath='{.spec.template.spec.containers[0].args}')
case "$CP" in *ore-postgres*avisos*) echo "  ✓ el controller avisa a ore-postgres";; *) echo "  ✗ el controller avisa a: $CP"; fallos=$((fallos+1));; esac
ANTES=$(avisos_de "$TENANT"); echo "  · avisos de este tenant hasta ahora: $ANTES (el de su alta)"

echo "── A · con un escritor en marcha, se reinicia el pageserver"
qo "$DIR" 'create table p45 (n int)' >/dev/null
k exec -i cliente-overlay -- sh -c 'cat > /tmp/p45.sh' <<'ESC'
rm -f /tmp/p45-ok /tmp/p45-err
for i in $(seq 1 600); do
  r=$(PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=3 psql -h "$1" -p 5432 -U cloud_admin -d postgres -qAtc \
      "insert into p45 values ($i) returning n" 2>&1)
  case "$r" in "$i") echo "$i $(date +%s.%N | cut -c1-14)" >> /tmp/p45-ok;; *) echo "$i $(date +%s) $r" | head -1 >> /tmp/p45-err;; esac
  sleep 0.2
done
echo fin >> /tmp/p45-ok
ESC
k exec cliente-overlay -- sh -c "nohup sh /tmp/p45.sh $DIR >/dev/null 2>&1 &"
sleep 15
TK=$(date +%s); k delete pod pageserver-0 --wait=false >/dev/null
k wait --for=condition=Ready pod/pageserver-0 --timeout=300s >/dev/null && echo "  · pageserver-0 de vuelta a los $(( $(date +%s)-TK )) s"
sleep 30
DESPUES=$(avisos_de "$TENANT")
[ "$DESPUES" -gt "$ANTES" ] && echo "  ✓ el controller avisó y ore-postgres reconfiguró $VM ($ANTES → $DESPUES)" \
  || echo "  · el reinicio NO provocó aviso para este tenant ($ANTES → $DESPUES): mismo nodo, misma ubicación (dato)"
until k exec cliente-overlay -- grep -q fin /tmp/p45-ok 2>/dev/null; do sleep 5; done
OK=$(k exec cliente-overlay -- sh -c 'grep -c -v fin /tmp/p45-ok'); ERR=$(k exec cliente-overlay -- sh -c 'cat /tmp/p45-err 2>/dev/null | wc -l')
HUECO=$(k exec cliente-overlay -- sh -c 'grep -v fin /tmp/p45-ok | cut -d" " -f2' | python -c '
import sys; t=[float(x) for x in sys.stdin.read().split()]; print(round(max((b-a for a,b in zip(t,t[1:])), default=0),1))')
echo "  · el cómputo confirmó $OK y falló $ERR; mayor hueco entre confirmaciones: $HUECO s"
k exec cliente-overlay -- sh -c 'grep -v fin /tmp/p45-ok | cut -d" " -f1' | sort > "$ORE_PG_TRABAJO/p45-ok"
qo "$DIR" 'select n from p45' | sort > "$ORE_PG_TRABAJO/p45-hay"
FALTAN=$(comm -23 "$ORE_PG_TRABAJO/p45-ok" "$ORE_PG_TRABAJO/p45-hay" | wc -l)
[ "$FALTAN" = 0 ] && echo "  ✓ no falta ninguna fila confirmada" || { echo "  ✗ faltan $FALTAN confirmadas"; fallos=$((fallos+1)); }

echo "── B · el aviso, provocado: el PUT del controller con su token infra"
ANTES=$(avisos_de "$TENANT")
R=$(kubectl -n ore-pg exec deploy/storage-controller -- sh -c "curl -s -w ' %{http_code}' -X PUT \
  -H \"Authorization: Bearer \$CONTROL_PLANE_JWT_TOKEN\" -H 'Content-Type: application/json' \
  --data '{\"tenant_id\":\"$TENANT\",\"stripe_size\":null,\"shards\":[{\"node_id\":1,\"shard_number\":0}]}' \
  http://ore-postgres.ore-pg.svc.cluster.local.:8100/avisos/notify-attach")
case "$R" in *'"reconfigurados":1'*200) echo "  ✓ 200, 1 cómputo reconfigurado";; *) echo "  ✗ $R"; fallos=$((fallos+1));; esac
[ "$(avisos_de "$TENANT")" -gt "$ANTES" ] && echo "  ✓ y ore-postgres lo dice en su registro" || { echo "  ✗ no está en su registro"; fallos=$((fallos+1)); }
[ "$(qo "$DIR" 'select count(*) > 0 from p45')" = t ] && echo "  ✓ el cómputo sigue sirviendo tras el /configure" || { echo "  ✗ no sirve"; fallos=$((fallos+1)); }
R=$(kubectl -n ore-pg exec deploy/storage-controller -- sh -c "curl -s -o /dev/null -w '%{http_code}' -X PUT --data '{}' \
  http://ore-postgres.ore-pg.svc.cluster.local.:8100/avisos/notify-attach")
[ "$R" = 401 ] && echo "  ✓ sin el token: 401" || { echo "  ✗ sin token: $R"; fallos=$((fallos+1)); }

echo "── se borra"
mapfile -t L < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ borrado" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·5 ✓ todo" || { echo "P4·5 ✗ $fallos fallos"; exit 1; }
