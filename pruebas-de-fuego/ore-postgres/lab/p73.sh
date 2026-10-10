#!/usr/bin/env bash
# P7·3 en el laboratorio (ADR 0058): los golpes, con Libro trabajando.
#   1 · matar el cómputo           (docker kill: como una VM que muere; restartPolicy Never)
#   2 · cambiar las CU             (la API, con carga)
#   3 · reiniciar el proxy
#   4 · reiniciar ore-postgres     (el plano de control)
#   5 · tirar Redis 30 s           (por donde el plano le dice al proxy qué olvidar)
# Tras cada uno, VENTANA s de carga. Pasa si los invariantes cuadran, ninguna operación de los
# clientes se pierde (un error que el reintento del cliente tapa se cuenta, no suspende) y cada
# pieza vuelve a trabajar. Lo que ve cada pieza, y la espera más larga, quedan en la tabla.
# Sin dormir (dormir_tras 0): un despertar no se confunde con un golpe.
#
#   lab.sh arriba && p73.sh        (unos 10 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
LIBRO="$(cd "$LAB/../libro" && pwd -W 2>/dev/null || pwd)"
lc() { docker compose -f "$LIBRO/compose.yaml" --env-file "$LIBRO/.env" "$@"; }
datos() { docker run --rm -v libro-datos:/datos alpine:3.20 sh -c "$1"; }
informe() { lc run --rm --no-deps -T usuarios python -u /libro/informe/informe.py "$@"; }
concilia() { lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | grep '^{' | tail -1; }
iso() { date -u +%Y-%m-%dT%H:%M:%S.000Z; }
VENTANA=60

P="libro-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
observado() { campo "$(pide demo GET "$E")" estado observado; }

echo "── el proyecto $P (demo), sin dormir"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:libro\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"0"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
ep=$(pide demo GET "$E")
cat > "$LIBRO/.env" <<EOF
EP_HOST=$(campo "$ep" host)
EP_POOL=$(campo "$ep" host_pool)
BASE=$P
ROL=$ROL
CLAVE=$CLAVE
PUERTO_PG=4432
OPS=6
INTERVALO_CONCILIADOR=30
INTERVALO_INFORMES=45
EOF
lc build -q 2>&1 | tail -3
docker volume rm libro-datos >/dev/null 2>&1
lc up -d >/dev/null 2>&1
sleep 45
c=$(concilia); case "$c" in *'"ok":true'*) bien "Libro trabajando, los invariantes cuadran";; *) mal "antes de golpear: ${c:0:200}"; exit 1;; esac

TABLA=()
golpe() {   # golpe NOMBRE ORDEN…
  local nombre=$1; shift
  echo "── $nombre"
  local t0; t0=$(iso)
  "$@"
  sleep "$VENTANA"
  local t1; t1=$(iso)
  local j; j=$(informe --desde "$t0" --hasta "$t1" --json | grep '^{' | tail -1)
  local c; c=$(concilia)
  local fila; fila=$("$PY" -c 'import json,sys
r=json.loads(sys.argv[1]); p=r["por_pieza"]
api=p.get("libro-api",{}); web=p.get("libro-webhooks",{})
print(len([e for e in r["atribuibles_perdidos"] if "libro-api" in e or "libro-webhooks" in e]),
      len([e for e in r["atribuibles_perdidos"] if "libro-informes" in e or "conciliador" in e]),
      r["absorbidos_total"], api.get("max_ms",0), web.get("max_ms",0), api.get("total",0), web.get("total",0))' "$j")
  read -r perd crones absor api_ms web_ms n_api n_web <<<"$fila"
  local ultimos; ultimos=$(datos "tail -20 /datos/operaciones.jsonl" | grep -c '"resultado":"hecha"')
  case "$c" in *'"ok":true'*) ;; *) mal "$nombre: los invariantes no cuadran: ${c:0:250}";; esac
  [ "$perd" = 0 ] || mal "$nombre: $perd operaciones de los clientes perdidas"
  [ "$ultimos" -ge 15 ] || mal "$nombre: no vuelve a trabajar ($ultimos de las 20 últimas, bien)"
  [ "$perd" = 0 ] && [ "$ultimos" -ge 15 ] && case "$c" in *'"ok":true'*) bien "invariantes, nada perdido, y de vuelta: API ${api_ms} ms de espera máxima, webhooks ${web_ms} ms, $absor errores absorbidos, $crones crones fallidos";; esac
  TABLA+=("$nombre|$n_api|$n_web|$api_ms|$web_ms|$absor|$crones")
  [ "$absor" != 0 ] && informe --desde "$t0" --hasta "$t1" | grep -E '^    · ' | head -4
}

matar() {
  local antes; antes=$(date +%s%3N)
  docker kill "$VM" >/dev/null
  local t=$(date +%s); until [ "$(observado)" != listo ] || [ $(( $(date +%s) - t )) -ge 30 ]; do sleep 0.5; done
  local visto=$(( $(date +%s%3N) - antes ))
  t=$(date +%s); until [ "$(observado)" = listo ] || [ $(( $(date +%s) - t )) -ge 90 ]; do sleep 0.5; done
  local reparado=$(( $(date +%s%3N) - antes ))
  [ "$(observado)" = listo ] && bien "el plano vio la muerte a los $visto ms y lo dejó reparado a los $reparado ms" || mal "no se reparó: $(observado)"
}
cus() { [ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"cu_min":"0.5","cu_max":"2"}')")" = hecha ] && bien "CU 0,5–2 con carga (valen en el próximo arranque)" || mal "ajustes de CU"; }

golpe "1 · matar el cómputo" matar
golpe "2 · cambiar las CU" cus
golpe "3 · reiniciar el proxy" eval "dc restart proxy >/dev/null 2>&1"
golpe "4 · reiniciar ore-postgres" eval "dc restart plano >/dev/null 2>&1"
golpe "5 · tirar Redis 30 s" bash -c "docker compose -f '$LAB/compose.yaml' stop redis >/dev/null 2>&1; sleep 30; docker compose -f '$LAB/compose.yaml' start redis >/dev/null 2>&1"

echo "── la tabla"
printf '  %-28s %6s %6s %10s %10s %9s %7s\n' golpe api webh "api máx" "webh máx" absorbidos crones
for f in "${TABLA[@]}"; do IFS='|' read -r a b c d e g h <<<"$f"; printf '  %-28s %6s %6s %8s ms %8s ms %9s %7s\n' "$a" "$b" "$c" "$d" "$e" "$g" "$h"; done
lc stop usuarios banco >/dev/null 2>&1
echo "── el recorrido entero"
informe | sed 's/^/  /'

[ -n "${GUARDA:-}" ] || { lc down -v >/dev/null 2>&1; hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null; rm -f "$LIBRO/.env"; }
echo
[ $fallos = 0 ] && echo "P7·3 (laboratorio) ✓ todo" || { echo "P7·3 (laboratorio) ✗ $fallos fallos"; exit 1; }
