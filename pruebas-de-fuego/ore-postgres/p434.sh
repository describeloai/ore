#!/usr/bin/env bash
# P4·3·4 · el cerco (ADR 0058). Hecho cuando: dos peticiones de escritura A LA VEZ sobre la misma rama
# dan una 202 y otra 409 (una sola VM); y, forzando un segundo cómputo de escritura POR FUERA del API,
# los safekeepers no dejan que los dos confirmen: medido qué le pasa al primero y si se pierde algo.
#
#   capa 1   la base: un endpoint de escritura vivo por rama (índice único)       → aquí, a la vez
#   capa 2   el reconciliador: no nace una VM mientras quede un runner con su nombre (p433, P3·5)
#   capa 3   los safekeepers: un proponente con término viejo pierde la votación   → aquí, medido
#
#   p434.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p434-$(date +%s | tail -c 6)"
PRUEBA=p434
INTRUSO=p434-intruso
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
orep() { k exec deploy/ore-postgres -- /bin/ore-postgres "$@"; }
quitar_intruso() { kc delete neonvm "$INTRUSO" --ignore-not-found --wait=true >/dev/null 2>&1
  kc wait --for=delete pod -l vm.neon.tech/name="$INTRUSO" --timeout=120s >/dev/null 2>&1
  kc delete configmap "$INTRUSO-config" --ignore-not-found >/dev/null; }
trap 'quitar_intruso; for c in $CELDAS_ABIERTAS; do kubectl -n t-$c delete networkpolicy prueba-$PRUEBA --ignore-not-found >/dev/null; done' EXIT
R="/v1/postgres/proyectos/$P/ramas"

echo "── $A crea $P (con su principal) y una rama dev"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\"}'")
OP=$(campo "${L[0]}" operacion id); TENANT=$(campo "${L[0]}" proyecto tenant)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide POST $R '{\"id\":\"dev\"}'")
[ "$(campo "${L[0]}" estado)" = hecha ] || { echo "  ✗ el proyecto: ${L[0]:0:300}"; exit 1; }
OP=$(campo "${L[1]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/main" "pide GET $R/main/endpoints/principal")
MAIN=$(campo "${L[1]}" timeline); DIR=$(campo "${L[2]}" direccion)
echo "  ✓ tenant $TENANT · main $MAIN · principal en $DIR"

echo "── capa 1: dos endpoints de escritura en dev, A LA VEZ"
mapfile -t L < <(en "$A" "pide POST $R/dev/endpoints '{\"id\":\"uno\"}' > /tmp/a & pide POST $R/dev/endpoints '{\"id\":\"dos\"}' > /tmp/b & wait; cat /tmp/a /tmp/b")
CODIGOS=$(printf '%s\n' "${L[0]%% *}" "${L[1]%% *}" | sort | tr '\n' ' ')
[ "$CODIGOS" = "202 409 " ] && echo "  ✓ una 202 y otra 409" || { echo "  ✗ $CODIGOS: ${L[*]:0:400}"; fallos=$((fallos+1)); }
for l in "${L[@]}"; do case "$l" in 409*) echo "    el 409 dice: $(campo "$l" error)";; 202*) OPG=$(campo "$l" operacion id);; esac; done
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OPG" "pide GET $R/dev/endpoints")
N=$(python -c 'import json,sys; print(len(json.loads(sys.argv[1].split(" ",1)[1])["endpoints"]))' "${L[1]}")
VMS=$(kc get neonvm -l "ore.dev/proyecto=$P" -o name | wc -l)
[ "$N" = 1 ] && [ "$VMS" = 2 ] && echo "  ✓ un endpoint en dev, y en el proyecto 2 VMs (principal y el de dev)" \
  || { echo "  ✗ endpoints en dev: $N · VMs del proyecto: $VMS"; fallos=$((fallos+1)); }

echo "── capa 3: un segundo cómputo de escritura en main, POR FUERA del API"
qo "$DIR" 'create table p434 (n int, quien text, cuando timestamptz default clock_timestamp())' >/dev/null
# El escritor: una conexión por fila, y apunta cuáles le confirmaron (`returning`). ⚠️ Con `-q`: sin él
# psql imprime también la etiqueta `INSERT 0 1` y todo parecía un fallo (medido, primera pasada).
k exec -i cliente-overlay -- sh -c 'cat > /tmp/p434.sh' <<'ESC'
rm -f /tmp/p434-ok /tmp/p434-err /tmp/p434-alto; touch /tmp/p434-err
# Hasta que lo paren (`/tmp/p434-alto`): tiene que seguir escribiendo MIENTRAS escribe el intruso, que
# tarda ~3 min en responder (medido: con 600 filas fijas acabó antes de que el intruso naciera).
i=0; while [ ! -f /tmp/p434-alto ] && [ $i -lt 6000 ]; do i=$((i+1))
  r=$(PGPASSWORD=cloud_admin PGCONNECT_TIMEOUT=3 psql -h "$1" -p 55433 -U cloud_admin -d postgres -qAtc \
      "insert into p434 (n, quien) values ($i, 'principal') returning n" 2>&1)
  case "$r" in "$i") echo "$i $(date +%s.%N | cut -c1-14)" >> /tmp/p434-ok;; *) echo "$i $(date +%s) $r" | head -1 >> /tmp/p434-err;; esac
  sleep 0.2
done
echo fin >> /tmp/p434-ok
ESC
k exec cliente-overlay -- sh -c "nohup sh /tmp/p434.sh $DIR >/dev/null 2>&1 &"
sleep 15; echo "  · el escritor lleva $(k exec cliente-overlay -- sh -c 'wc -l < /tmp/p434-ok') filas confirmadas por el principal"
F="$ORE_PG_TRABAJO/$INTRUSO.json"
orep especificacion --computo "$INTRUSO" --tenant "$TENANT" --timeline "$MAIN" --grupo "$P" > "$F" || { echo "  ✗ sin especificación"; exit 1; }
kc create configmap "$INTRUSO-config" --from-file=config.json="$F" >/dev/null
TI=$(date +%s); plantilla "$AQUI/vm.yaml" ORE_PG_VM="$INTRUSO" | kubectl apply -f - >/dev/null
until OVI=$(ip_overlay "$INTRUSO"); [ -n "$OVI" ] && [ "$(qo "$OVI" 'select 1' 2>/dev/null)" = 1 ]; do
  sleep 1; [ $(( $(date +%s)-TI )) -gt 600 ] && { echo "  · el intruso no llega a servir en 600 s (dato)"; break; }
done
VIVO=$(( $(date +%s)-TI )); echo "  · el intruso responde a los $VIVO s (en $OVI); escribe 20 filas"
BIEN_I=0; for i in $(seq 1 20); do
  [ "$(qo "$OVI" "insert into p434 (n, quien) values ($i, 'intruso') returning n" 2>/dev/null | head -1)" = "$i" ] && BIEN_I=$((BIEN_I+1)); sleep 0.5; done
echo "  · el intruso confirmó $BIEN_I de 20"
sleep 30; k exec cliente-overlay -- touch /tmp/p434-alto
echo "  · se para al escritor del principal (30 s después del intruso)"
until k exec cliente-overlay -- grep -q fin /tmp/p434-ok 2>/dev/null; do sleep 5; done

echo "── lo medido"
OK=$(k exec cliente-overlay -- sh -c 'grep -c -v fin /tmp/p434-ok'); ERR=$(k exec cliente-overlay -- sh -c 'wc -l < /tmp/p434-err')
ULT=$(k exec cliente-overlay -- sh -c 'grep -v fin /tmp/p434-ok | tail -1 | cut -d" " -f2')
echo "  · el principal confirmó $OK filas y falló $ERR; la última confirmada, $(( ${ULT%.*} - TI )) s después de levantar al intruso"
echo "  · su primer error: $(k exec cliente-overlay -- sh -c 'head -1 /tmp/p434-err' | cut -c1-200)"
echo "  · su última confirmada respecto al intruso: $(( ${ULT%.*} - TI - VIVO )) s desde que el intruso respondió"
for d in "$DIR" "$OVI"; do echo "  · leído en $d: $(qo "$d" "select quien, count(*) from p434 group by quien order by quien" 2>&1 | tr '\n' ' ' | cut -c1-200)"; done
# ¿Se perdió algo confirmado? Lo confirmado al principal tiene que estar en quien ha quedado escribiendo.
k exec cliente-overlay -- sh -c 'grep -v fin /tmp/p434-ok | cut -d" " -f1' > "$ORE_PG_TRABAJO/p434-ok"
for d in "$OVI" "$DIR"; do
  qo "$d" "select n from p434 where quien = 'principal' order by n" > "$ORE_PG_TRABAJO/p434-en-$d" 2>/dev/null
done
# Contra el que ha quedado sirviendo: medido, a veces gana el intruso (el principal deja de aceptar
# conexiones) y a veces el principal (el intruso nunca llega a servir y el principal se reinicia).
VIVE_P=$(qo "$DIR" 'select 1' 2>/dev/null); VIVE_I=$(qo "$OVI" 'select 1' 2>/dev/null)
QUEDA=$DIR; [ "$VIVE_I" = 1 ] && [ "$VIVE_P" != 1 ] && QUEDA=$OVI
[ "$VIVE_P" = 1 ] && [ "$VIVE_I" = 1 ] && echo "  · ⚠ los DOS aceptan conexiones al final" \
  || echo "  ✓ al final sólo sirve uno: $QUEDA"
# Lo que el intruso confirmó también ha de estar en el que queda (una sola historia, por términos).
qo "$QUEDA" "select count(*) from p434 where quien = 'intruso'" 2>/dev/null | grep -qx "$BIEN_I" \
  && echo "  ✓ y en él están también las $BIEN_I del intruso: una sola historia" \
  || { echo "  ✗ en $QUEDA no están las $BIEN_I del intruso"; fallos=$((fallos+1)); }
PERDIDAS=$(comm -23 <(sort "$ORE_PG_TRABAJO/p434-ok") <(sort "$ORE_PG_TRABAJO/p434-en-$QUEDA") | wc -l)
[ "$PERDIDAS" = 0 ] && echo "  ✓ ninguna fila que el principal confirmó falta en $QUEDA" \
  || { echo "  ✗ faltan $PERDIDAS filas confirmadas por el principal en $QUEDA"; fallos=$((fallos+1)); }

echo "── se quita el intruso y se borra el proyecto"
quitar_intruso
mapfile -t L < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ borrado" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·3·4 ✓ (capa 1 en verde; capa 3: lo medido, arriba)" || { echo "P4·3·4 ✗ $fallos fallos"; exit 1; }
