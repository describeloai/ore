#!/usr/bin/env bash
# P5 en producción (ADR 0058, «la ventana 1»): la entrada desde internet, con el proxy ya desplegado
# (87 descomentada, el certificado `Ready`). Los datos, desde este PC; el plano, como una celda.
#
#   p5.sh todo            todo en orden (unos 15 min), o un paso:
#   p5.sh dns             los nombres resuelven a la IP de la entrada (también api., el del driver HTTP)
#   p5.sh conectar        psql verify-full entra con su contraseña y no con otra; el endpoint de otra
#                         organización, fuera; el pool (-pooler); 10 conexiones: p50/p95 de conectar
#   p5.sh pgbench         directo y por el pool, 10 clientes, 60 s: tps y latencia
#   p5.sh pool150         150 clientes por el pool: entran todos, y Postgres no pasa de su pool
#   p5.sh http            HTTP y WebSocket con @neondatabase/serverless (el p54.mjs del laboratorio)
#   p5.sh ip              quién entra (P5·6) con la IP real de este PC tras el balanceador
#   p5.sh migrar          P5·8: migración en vivo de la VM con pgbench corriendo
#   p5.sh limpiar         borra los proyectos de la prueba
#
# Los proyectos (uno de demo, uno de victor) se crean al primer paso que los necesita y se guardan en
# $ORE_PG_TRABAJO, para que los pasos se puedan correr sueltos.
PRUEBA=p5
source "$(dirname "$0")/comun.sh"
A=demo; B=victor

proyecto() {   # proyecto CELDA → carga ROL CLAVE HOST POOL (lo crea la primera vez)
  local c=$1 f="$ORE_PG_TRABAJO/p5-$1"
  if [ ! -s "$f" ]; then
    local p="p5-$c-$(date +%s | tail -c 5)"
    local r; r=$(crea "$c" "$p") || exit 1
    echo "$p $r" > "$f"
  fi
  read -r P ROL CLAVE HOST POOL < "$f"
}

dns() {
  echo "── los nombres"
  local ip; ip=$(kubectl -n flux-system get configmap ore-pg-entrada -o jsonpath='{.data.IP_ENTRADA}')
  echo "  · la IP de la entrada: $ip"
  proyecto $A
  for n in "$HOST" "$POOL" "api.$DOMINIO"; do
    local r; r=$(docker run --rm "$PG_IMAGEN" getent hosts "$n" | awk '{print $1}' | head -1)
    [ "$r" = "$ip" ] && bien "$n → $r" || mal "$n → ${r:-nada} (esperaba $ip)"
  done
}

conectar() {
  echo "── conectar desde internet (verify-full, CA del sistema)"
  proyecto $A
  local r; r=$(pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select current_user || ' ' || inet_server_port()")
  case "$r" in "$ROL "*) bien "entra como $ROL ($r)";; *) mal "no entra: $r";; esac
  r=$(pc_psql "mala-$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select 1")
  case "$r" in *authentication*|*password*) bien "con otra contraseña, no";; *) mal "con otra contraseña: $r";; esac
  local P_A=$P ROL_A=$ROL CLAVE_A=$CLAVE; proyecto $B
  r=$(pc_psql "$CLAVE_A" "$(conn "$HOST" "$ROL_A" "$P_A")" -c "select 1")
  [ "$r" = 1 ] && mal "con lo de demo entra en el endpoint de victor" || bien "con lo de demo en el endpoint de victor, no (${r:0:80})"
  proyecto $A
  r=$(pc_psql "$CLAVE" "$(conn "$POOL" "$ROL" "$P")" -c "select 1")
  [ "$r" = 1 ] && bien "por el pool ($POOL)" || mal "por el pool: $r"
  local v=() x
  for _ in $(seq 1 10); do x=$(pc_ms "$CLAVE" "$(conn "$HOST" "$ROL" "$P")"); case "$x" in fallo*) mal "$x";; *) v+=("$x");; esac; done
  # Cada medida incluye arrancar el contenedor de psql: se resta su coste, medido igual sin red.
  local base=() y; for _ in 1 2 3; do y=$(ms); docker run --rm "$PG_IMAGEN" true; base+=($(( $(ms) - y ))); done
  local b; b=$(percentil 0.5 "${base[@]}")
  numero p5_conectar_p50_ms $(( $(percentil 0.5 "${v[@]}") - b ))
  numero p5_conectar_p95_ms $(( $(percentil 0.95 "${v[@]}") - b ))
}

pgbench_() {
  echo "── pgbench desde internet (10 clientes, 60 s)"
  proyecto $A
  pc_pgbench "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -i -s 10 -q >/dev/null && bien "pgbench -i -s 10" || { mal "pgbench -i"; return; }
  local s; for modo in directo pool; do
    local h=$HOST; [ $modo = pool ] && h=$POOL
    s=$(pc_pgbench "$CLAVE" "$(conn "$h" "$ROL" "$P")" -c 10 -j 2 -T 60 -n)
    local tps lat err
    tps=$(grep -oE 'tps = [0-9.]+' <<<"$s" | head -1 | cut -d' ' -f3); lat=$(grep -oE 'latency average = [0-9.]+' <<<"$s" | cut -d' ' -f4)
    err=$(grep -oE 'number of failed transactions: [0-9]+' <<<"$s" | grep -oE '[0-9]+$')
    [ -n "$tps" ] && [ "${err:-0}" = 0 ] && bien "$modo: $tps tps, $lat ms de media, 0 fallos" || mal "$modo: ${s: -300}"
    numero "p5_pgbench_${modo}_tps" "${tps:-null}"; numero "p5_pgbench_${modo}_lat_ms" "${lat:-null}"
  done
}

pool150() {
  echo "── 150 clientes por el pool"
  proyecto $A
  pc_pgbench "$CLAVE" "$(conn "$POOL" "$ROL" "$P")" -c 150 -j 4 -T 40 -S -n > "$ORE_PG_TRABAJO/p5-pool150" &
  local pid=$! max=0 n
  sleep 10
  for _ in $(seq 1 10); do
    n=$(pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select count(*) from pg_stat_activity where backend_type = 'client backend'")
    [[ "$n" =~ ^[0-9]+$ ]] && [ "$n" -gt "$max" ] && max=$n; sleep 2
  done
  wait $pid
  local s; s=$(cat "$ORE_PG_TRABAJO/p5-pool150")
  local err; err=$(grep -oE 'number of failed transactions: [0-9]+' <<<"$s" | grep -oE '[0-9]+$')
  [ "${err:-x}" = 0 ] && bien "los 150 entran, 0 fallos ($(grep -oE 'tps = [0-9.]+' <<<"$s" | head -1))" || mal "150 por el pool: ${s: -300}"
  numero p5_pool150_conexiones_postgres_max "$max"
  echo "    (conexiones en Postgres, como mucho: $max; el pool por base es el 90 % de max_connections, P5·5b)"
}

http() {
  echo "── HTTP y WebSocket (@neondatabase/serverless)"
  proyecto $A; local P_A=$P ROL_A=$ROL CLAVE_A=$CLAVE VM_A=${HOST%%.*}
  proyecto $B; local VM_B=${HOST%%.*}
  docker volume create p5prod-npm >/dev/null
  docker run --rm -v "$AQUI/../lab/p54.mjs:/w/p54.mjs:ro" -v p5prod-npm:/w/node_modules -w /w "$NODE_IMAGEN" \
    sh -c '[ -d node_modules/@neondatabase/serverless ] || npm i -s --no-save --no-package-lock @neondatabase/serverless@1.1.0 ws@8.18.3 >/dev/null 2>&1; node p54.mjs "$@"' \
    -- "$ROL_A" "$CLAVE_A" "$P_A" "$VM_A" "$VM_B" && bien "HTTP y WebSocket, como en el laboratorio" || mal "HTTP/WebSocket"
}

ip() {
  echo "── quién entra, con la IP real de este PC"
  proyecto $A
  local mia; mia=$(curl -s https://api.ipify.org)
  echo "  · la IP pública de este PC: $mia"
  acceso $A "$P" "{\"ips_permitidas\":[\"$mia/32\"]}" >/dev/null; sleep 5
  local r; r=$(pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select 1")
  [ "$r" = 1 ] && bien "con $mia/32 permitida, entra: el proxy ve la IP real" || mal "con mi IP permitida: $r"
  acceso $A "$P" '{"ips_permitidas":["203.0.113.7/32"]}' >/dev/null; sleep 5
  r=$(pc_psql "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c "select 1")
  [ "$r" = 1 ] && mal "con otra IP permitida, entra" || bien "con otra IP permitida, no (${r:0:80})"
  acceso $A "$P" '{"ips_permitidas":[]}' >/dev/null
}

migrar() {
  echo "── P5·8: migración en vivo con pgbench desde internet"
  proyecto $A
  pc_pgbench "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -i -s 5 -q >/dev/null 2>&1
  pc_pgbench "$CLAVE" "$(conn "$HOST" "$ROL" "$P")" -c 4 -T 90 -P 1 > "$ORE_PG_TRABAJO/p5-migrar" 2>&1 &
  local pid=$!; sleep 20
  local vm mig fase; vm=$(computo_de "$P"); mig="p5-$(date +%s)"
  kc apply -f - >/dev/null <<EOF
apiVersion: vm.neon.tech/v1
kind: VirtualMachineMigration
metadata: { name: $mig, namespace: $ORE_PG_NS_COMPUTO }
spec: { vmName: $vm, preventMigrationToSameHost: true, allowPostCopy: false }
EOF
  local t=$(date +%s)
  until fase=$(kc get neonvmm "$mig" -o jsonpath='{.status.phase}' 2>/dev/null); [ "$fase" = Succeeded ] || [ "$fase" = Failed ] || [ $(( $(date +%s) - t )) -ge 300 ]; do sleep 1; done
  wait $pid
  local s; s=$(cat "$ORE_PG_TRABAJO/p5-migrar")
  local err; err=$(grep -oE 'number of failed transactions: [0-9]+' <<<"$s" | grep -oE '[0-9]+$')
  local peor; peor=$(grep -oE 'lat [0-9.]+ ms' <<<"$s" | awk '{print $2}' | sort -n | tail -1)
  [ "$fase" = Succeeded ] && bien "migración $mig: $fase" || mal "migración: ${fase:-sin fase}"
  [ "${err:-x}" = 0 ] && bien "pgbench sin un fallo durante la migración" || mal "pgbench durante la migración: ${s: -300}"
  numero p5_migrar_peor_segundo_lat_ms "${peor:-null}"
  kc delete neonvmm "$mig" --ignore-not-found >/dev/null
}

limpiar() {
  for c in $A $B; do
    [ -s "$ORE_PG_TRABAJO/p5-$c" ] && { read -r p _ < "$ORE_PG_TRABAJO/p5-$c"; borra "$c" "$p"; rm -f "$ORE_PG_TRABAJO/p5-$c"; echo "  · $p borrado"; }
  done
  docker volume rm p5prod-npm >/dev/null 2>&1 || true
}

case "${1:-}" in
  todo) dns; conectar; pgbench_; pool150; http; ip; migrar;;
  dns|conectar|pool150|http|ip|migrar|limpiar) "$1";;
  pgbench) pgbench_;;
  *) sed -n '2,20p' "$0"; exit 64;;
esac
echo
[ $fallos = 0 ] && echo "P5 (producción) ✓ ${1}" || { echo "P5 (producción) ✗ $fallos fallos"; exit 1; }
