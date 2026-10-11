#!/usr/bin/env bash
# P7 en producción, versión corta (ADR 0058): Libro trabajando 2 h contra producción, con día y
# noche cada 10 min, los golpes de la receta comprimidos y el informe de los nueve objetivos.
#
#   p7.sh montar    el proyecto libro-p7 (dormir_tras 60), libro/.env, Libro arriba por el túnel
#                   y cada driver comprobado (Prisma por el pool, psycopg y JDBC directos)
#   p7.sh correr    la hora 0, el muestreador y el calendario, en segundo plano (p7.log)
#   p7.sh estado    lo que va: golpes hechos, operaciones, errores
#   p7.sh cerrar    la conciliación completa final, el informe (--medidas), y todo fuera
#
# El calendario, en minutos desde la hora 0 (ESCALA=60). La primera hora con el pool encendido
# (--pool-computos 2, ya en la malla) y la segunda sin él: `pool-fuera` en el minuto 60 lo quita de
# la malla 86 (un commit, con el go de la ventana) y lo apunta.
PRUEBA=p7
source "$(dirname "$0")/comun.sh"
A=demo
LIBRO="$AQUI/libro"
MED="$ORE_PG_TRABAJO/p7-medidas"
lc() { (cd "$LIBRO" && docker compose -f compose.yaml -f compose.gcp.yaml -f compose.tunel.yaml --env-file .env "$@"); }
PIEZAS="libro-api libro-webhooks libro-conciliador libro-informes usuarios banco"
PUERTO_HTTP=15443
CALENDARIO_P7="10 matar-vm
20 pageserver
30 safekeeper
40 migrar
50 proxy
60 pool-fuera
70 matar-vm
90 ore-postgres"

montar() {
  local f="$ORE_PG_TRABAJO/p7-$A"
  if [ ! -s "$f" ]; then
    local p="libro-p7-$(date +%s | tail -c 5)" r
    r=$(crea "$A" "$p") || exit 1
    case "$(ajusta "$A" "$p" '{"dormir_tras":"60"}')" in 2*) ;; *) echo "✗ ajustes" >&2; exit 1;; esac
    echo "$p $r" > "$f"
  fi
  read -r P ROL CLAVE HOST POOL < "$f"
  local clave_url; clave_url=$(python -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$CLAVE")
  mkdir -p "$MED"
  cat > "$LIBRO/.env" <<EOF
EP_HOST=$HOST
EP_POOL=$POOL
EP=${HOST%%.*}
BASE=$P
ROL=$ROL
CLAVE=$CLAVE
CLAVE_URL=$clave_url
PUERTO_PG=5432
PUERTO_TUNEL=$TUNEL_PUERTO
PUERTO_HTTP=$PUERTO_HTTP
RELOJ=1
DIA_S=360
NOCHE_S=240
OPS=4
INTERVALO_CONCILIADOR=120
COMPLETA_S=1800
INTERVALO_INFORMES=300
MEDIDAS=$MED
EOF
  echo "── $P (${HOST%%.*}), Libro por el túnel"
  tunel || exit 1
  http_abre
  lc up -d --build $PIEZAS 2>&1 | tail -3
  local t0; t0=$(date +%s)
  until [ "$(docker inspect -f '{{.State.Health.Status}}' libro-libro-api-1 2>/dev/null)" = healthy ] || [ $(( $(date +%s) - t0 )) -ge 600 ]; do sleep 5; done
  [ "$(docker inspect -f '{{.State.Health.Status}}' libro-libro-api-1 2>/dev/null)" = healthy ] \
    && bien "libro-api (Prisma): migrada y sana en $(( $(date +%s) - t0 )) s (el endpoint en la contraseña, por el pool)" \
    || { mal "libro-api no queda sana: $(lc logs --tail 5 libro-api 2>&1 | tail -5)"; return; }
  sleep 150   # una vuelta del conciliador y transferencias de usuarios
  local d; d=$(docker run --rm -v libro-datos:/datos alpine sh -c 'ls /datos; tail -n 2 /datos/conciliador.jsonl 2>/dev/null' 2>&1)
  echo "$d" | sed 's/^/    /' | tail -12
  for pieza in libro-conciliador libro-informes usuarios; do
    local e; e=$(lc logs --since 3m "$pieza" 2>&1 | grep -ciE 'error|exception|traceback')
    [ "$e" = 0 ] && bien "$pieza, sin errores" || mal "$pieza: $e líneas de error · $(lc logs --since 3m "$pieza" 2>&1 | grep -iE 'error|exception' | tail -2)"
  done
}

http_abre() {   # el 443 del proxy en este PC, para libro-webhooks (ver compose.tunel.yaml)
  local pid; pid=$(cat "$ORE_PG_TRABAJO/p7-http.pid" 2>/dev/null)
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && return
  ( while :; do kubectl -n ore-pg port-forward --address 0.0.0.0 svc/proxy-postgres "$PUERTO_HTTP:443" >> "$ORE_PG_TRABAJO/p7-http.log" 2>&1; sleep 1; done ) > /dev/null 2>&1 &
  echo $! > "$ORE_PG_TRABAJO/p7-http.pid"; sleep 3
}
http_cierra() { local pid; pid=$(cat "$ORE_PG_TRABAJO/p7-http.pid" 2>/dev/null) && { pkill -P "$pid" 2>/dev/null; kill "$pid" 2>/dev/null; }; rm -f "$ORE_PG_TRABAJO/p7-http.pid"; }

pool_fuera() {   # el pool fuera de la malla 86, en un worktree (el índice de C:\ORE es compartido)
  local t0 w; t0=$(date -u +%Y-%m-%dT%H:%M:%S.000Z); w="$ORE_PG_TRABAJO/wt-p7"
  ( cd "$AQUI/../.." && git fetch -q origin && git worktree add -q --detach "$w" origin/main \
    && sed -i '/# P6·5: el pool precalentado/d; /- --pool-computos/{N;d}' "$w/malla/86-postgres-el-plano-de-control.yaml" \
    && git -C "$w" commit -q -am "0058 P7 corto: el pool fuera a la mitad (minuto 60)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" && git -C "$w" push -q origin HEAD:main; git worktree remove --force "$w" )
  kubectl -n flux-system annotate gitrepository flux-system reconcile.fluxcd.io/requestedAt="$(date +%s)" --overwrite >/dev/null 2>&1
  local t=$(date +%s)
  until ! k get deploy ore-postgres -o jsonpath='{.spec.template.spec.containers[0].args}' | grep -q pool-computos || [ $(( $(date +%s) - t )) -ge 600 ]; do sleep 5; done
  k rollout status deploy/ore-postgres --timeout=300s >/dev/null
  printf '{"t":"%s","fin":"%s","golpe":"pool-fuera","ok":true,"detalle":"--pool-computos fuera de la malla 86"}\n' "$t0" "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" >> "$MED/eventos.jsonl"
  # Bajar el pool no retira las VMs que ya hay (ADR, ventana 1): fuera las libres, que no son de nadie.
  for v in $(plano_sql "select computo from plano.pool_computo"); do
    kc delete neonvm "$v" --wait=false >/dev/null 2>&1; kc delete configmap "$v-config" --ignore-not-found >/dev/null 2>&1
  done
  plano_sql "delete from plano.pool_computo" >/dev/null
  echo "$(date -u +%T) · pool-fuera: hecho"
}

correr() {
  read -r P _ < "$ORE_PG_TRABAJO/p7-$A"
  export MEDIDAS="$MED" PROYECTO="$P" ESCALA=60
  bash "$LIBRO/medir/golpes.sh" empieza
  nohup bash "$LIBRO/medir/muestrea.sh" 60 > "$MED/muestrea.log" 2>&1 &
  echo $! > "$MED/muestrea.pid"
  (
    cero=$(cat "$MED/inicio")
    while read -r min g; do
      falta=$(( cero + min * 60 - $(date +%s) )); [ "$falta" -gt 0 ] && sleep "$falta"
      echo "$(date -u +%T) · minuto $min · $g"
      case "$g" in pool-fuera) pool_fuera;; *) bash "$LIBRO/medir/golpes.sh" "$g";; esac
    done <<<"$CALENDARIO_P7"
    echo "$(date -u +%T) · calendario acabado; a las 2 h, p7.sh cerrar"
  ) > "$MED/p7.log" 2>&1 &
  echo $! > "$MED/calendario.pid"
  echo "hora 0: $(date -u +%T) · calendario y muestreador en marcha · $MED/p7.log"
}

estado() {
  echo "── desde la hora 0: $(( ($(date +%s) - $(cat "$MED/inicio")) / 60 )) min"
  tail -n 20 "$MED/p7.log"
  echo "── golpes"; [ -s "$MED/eventos.jsonl" ] && sed 's/"detalle".*//' "$MED/eventos.jsonl"
  echo "── el endpoint: $(observado "$(cut -d' ' -f1 "$ORE_PG_TRABAJO/p7-$A")") · VMs: $(kc get neonvm --no-headers 2>/dev/null | awk '{print $1}' | tr '\n' ' ')"
}

cerrar() {
  read -r P _ < "$ORE_PG_TRABAJO/p7-$A"
  for x in calendario muestrea; do [ -s "$MED/$x.pid" ] && kill "$(cat "$MED/$x.pid")" 2>/dev/null; done
  lc stop usuarios >/dev/null 2>&1
  echo "── la conciliación completa final"
  lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | grep '^{' | tail -1
  local desde; desde=$(date -u -d "@$(cat "$MED/inicio")" +%Y-%m-%dT%H:%M:%SZ)
  echo "── el informe, desde la hora 0 ($desde)"
  lc run --rm --no-deps -T usuarios python /libro/informe/informe.py --desde "$desde" --medidas /medidas 2>&1 | tee "$MED/informe.txt"
  lc run --rm --no-deps -T usuarios python /libro/informe/informe.py --desde "$desde" --medidas /medidas --json 2>/dev/null | grep '^{' | tail -1 > "$MED/informe.json"
  lc down -v >/dev/null 2>&1
  borra "$A" "$P"; rm -f "$ORE_PG_TRABAJO/p7-$A"; echo "  · $P borrado"
  tunel_cierra; http_cierra
}

case "${1:-}" in
  montar|correr|estado|cerrar) "$1";;
  *) sed -n '2,15p' "$0"; exit 64;;
esac
echo
[ $fallos = 0 ] && echo "P7 (producción) ✓ ${1}" || { echo "P7 (producción) ✗ $fallos fallos"; exit 1; }
