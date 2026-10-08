#!/usr/bin/env bash
# P4·3·3 · endpoints (ADR 0058). Hecho cuando: un endpoint creado por el API responde a `select 1`, y
# borrarlo no deja ni VM ni ConfigMap.
#
# Todo por el API de `ore-postgres` como una celda: crear un proyecto ya trae su endpoint de escritura
# en main (`principal`); se le escribe por la overlay; uno de sólo lectura sobre main lee lo escrito;
# otro de escritura en main es un 409; y al borrar el proyecto no queda nada suyo en `ore-pg-computo`.
#
#   p433.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p433-$(date +%s | tail -c 6)"
PRUEBA=p433
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints"

echo "── $A crea $P: tenant, main y su endpoint de escritura"
T0=$(date +%s)
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
espera 202 "crear" "${L[0]}"; OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $E/principal")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ hecho a los $(( $(date +%s)-T0 )) s (VM incluida)" || { echo "  ✗ ${L[0]:0:400}"; exit 1; }
VM=$(campo "${L[1]}" vm); DIR=$(campo "${L[1]}" direccion)
echo "  · principal: $VM en $DIR ($(campo "${L[1]}" estado observado))"
[ "$(qo "$DIR" 'select 1')" = 1 ] && echo "  ✓ responde por la overlay" || { echo "  ✗ no responde en $DIR"; fallos=$((fallos+1)); }
qo "$DIR" 'create table p433 (n int); insert into p433 select generate_series(1, 500)' >/dev/null
[ "$(qo "$DIR" 'select count(*) from p433')" = 500 ] && echo "  ✓ escribe y lee: 500" || { echo "  ✗ no lee lo escrito"; fallos=$((fallos+1)); }

echo "── el cerco: otro de escritura en main, no; uno de lectura, sí"
mapfile -t L < <(en "$A" "pide POST $E '{\"id\":\"otro\"}'" "pide POST $E '{\"id\":\"lector\",\"tipo\":\"lectura\"}'")
espera 409 "otro de escritura en main" "${L[0]}"
espera 202 "uno de lectura" "${L[1]}"; OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $E/lector")
if [ "$(campo "${L[0]}" estado)" = hecha ]; then
  DL=$(campo "${L[1]}" direccion); echo "  ✓ lector listo en $DL"
  [ "$(qo "$DL" 'select count(*) from p433')" = 500 ] && echo "  ✓ el lector ve las 500" || { echo "  ✗ el lector ve: $(qo "$DL" 'select count(*) from p433' | head -c 200)"; fallos=$((fallos+1)); }
  case "$(qo "$DL" 'insert into p433 values (0)' 2>&1)" in *read-only*|*"read only"*) echo "  ✓ y no puede escribir";;
    *) echo "  ✗ el lector escribe"; fallos=$((fallos+1));; esac
else echo "  ✗ el lector: ${L[0]:0:400}"; fallos=$((fallos+1)); fi

echo "── borrar el proyecto no deja nada en ore-pg-computo"
mapfile -t L < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OP=$(campo "${L[0]}" operacion id); T0=$(date +%s)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ borrado a los $(( $(date +%s)-T0 )) s" || { echo "  ✗ ${L[0]:0:400}"; fallos=$((fallos+1)); }
R=$(kc get neonvm -l "ore.dev/proyecto=$P" -o name 2>/dev/null; kc get pods -l "ore.dev/proyecto=$P" -o name 2>/dev/null; kc get configmap "$VM-config" -o name 2>/dev/null)
[ -z "$R" ] && echo "  ✓ ni VMs, ni runners, ni ConfigMaps" || { echo "  ✗ quedan: $R"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·3·3 ✓ todo" || { echo "P4·3·3 ✗ $fallos fallos"; exit 1; }
