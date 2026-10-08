#!/usr/bin/env bash
# P4·3·1 · la especificación en Rust (ADR 0058). Hecho cuando: una VM creada con la especificación que
# escribe `ore-postgres` (ya no `especificacion.py`) escribe y lee, y su `compute_ctl` acepta los tokens
# firmados con la llave propia de `ore-postgres` y ningún otro.
#
# Hasta P4·3·3 el reconciliador no crea cómputos: aquí la especificación se le pide al `ore-postgres`
# desplegado (`ore-postgres especificacion`, por `kubectl exec`) y la VM se levanta con `vm.yaml`.
#
#   p431.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p431-$(date +%s | tail -c 6)"
VM=p431
PRUEBA=p431
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
orep() { k exec deploy/ore-postgres -- /bin/ore-postgres "$@"; }
limpiar() { kc delete neonvm "$VM" --ignore-not-found --wait=true >/dev/null 2>&1
  kc wait --for=delete pod -l vm.neon.tech/name="$VM" --timeout=120s >/dev/null 2>&1
  kc delete configmap "$VM-config" --ignore-not-found >/dev/null; }
trap 'limpiar; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT

echo "── $A crea $P"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET /v1/postgres/proyectos/$P/ramas/main")
MAIN=$(campo "${L[1]}" timeline)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ tenant $TENANT · main $MAIN" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }

echo "── la especificación, escrita por ore-postgres"
F="$ORE_PG_TRABAJO/$VM.json"
orep especificacion --computo "$VM" --tenant "$TENANT" --timeline "$MAIN" --grupo "$P" > "$F" \
  || { echo "  ✗ ore-postgres no la escribió"; exit 1; }
python - "$F" "$TENANT" "$MAIN" <<'PY' || fallos=$((fallos+1))
import json, sys
e = json.load(open(sys.argv[1])); s = {x["name"]: x["value"] for x in e["spec"]["cluster"]["settings"]}
ok = s["neon.tenant_id"] == sys.argv[2] and s["neon.timeline_id"] == sys.argv[3] and e["spec"].get("storage_auth_token")
print(f"  {'✓' if ok else '✗'} tenant, timeline y token de tenant · pageserver «{s['neon.pageserver_connstring']}» (del controller)")
print(f"  · safekeepers {s['neon.safekeepers']} · llave {e['compute_ctl_config']['jwks']['keys'][0]['kid']}")
sys.exit(0 if ok else 1)
PY

echo "── una VM con ella"
limpiar
kc create configmap "$VM-config" --from-file=config.json="$F" >/dev/null
T0=$(ms); plantilla "$AQUI/vm.yaml" ORE_PG_VM="$VM" | kubectl apply -f - >/dev/null
until OV=$(ip_overlay "$VM"); [ -n "$OV" ] && [ "$(qo "$OV" 'select 1' 2>/dev/null)" = 1 ]; do
  sleep 1; [ $(( $(ms)-T0 )) -gt 600000 ] && { echo "  ✗ la VM no responde en 10 min"; exit 1; }
done
echo "  ✓ responde a los $(( ($(ms)-T0)/1000 )) s"
qo "$OV" 'create table p431 (n int); insert into p431 select generate_series(1, 1000)' >/dev/null
[ "$(qo "$OV" 'select count(*) from p431')" = 1000 ] && echo "  ✓ escribe y lee: 1000 filas" || { echo "  ✗ no lee lo escrito"; fallos=$((fallos+1)); }

echo "── su compute_ctl acepta a ore-postgres, y a nadie más"
IP=$(ip_pod "$VM")
BIEN=$(orep token-computo "$VM"); OTRO=$(orep token-computo otro-computo)
case "$(api "$IP" 3080 GET /status "" "$BIEN")" in *running*) echo "  ✓ con su token: running";;
  *) echo "  ✗ con su token: $(api "$IP" 3080 GET /status "" "$BIEN" | head -c 200)"; fallos=$((fallos+1));; esac
case "$(api "$IP" 3080 GET /status "")" in *running*) echo "  ✗ SIN token contesta"; fallos=$((fallos+1));; *) echo "  ✓ sin token: no";; esac
case "$(api "$IP" 3080 GET /status "" "$OTRO")" in *running*) echo "  ✗ con el token de otro cómputo contesta"; fallos=$((fallos+1));;
  *) echo "  ✓ con el token de otro cómputo: no";; esac

echo "── se borra todo"
limpiar
mapfile -t L < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ VM, ConfigMap y proyecto" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·3·1 ✓ todo" || { echo "P4·3·1 ✗ $fallos fallos"; exit 1; }
