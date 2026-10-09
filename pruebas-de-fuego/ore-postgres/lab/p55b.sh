#!/usr/bin/env bash
# P5·5b en el laboratorio (ADR 0058): las conexiones, a la medida del cómputo.
#   · max_connections sale de las CU máximas (0,25 CU → 112) y el 90 % se reparte como pool de pgbouncer
#     entre las bases de la rama (max_db_connections), así que la suma nunca pasa de Postgres;
#   · ANTES (el pgbouncer.ini de la imagen tal cual: 64 por pareja rol+base): 150 clientes en cada una de
#     dos bases a la vez llenan Postgres hasta el borde, y según el momento un cliente choca con el
#     «server login has been failing» de pgbouncer o una conexión DIRECTA no cabe (un dato, no se exige);
#   · DESPUÉS (lo que manda la especificación): lo mismo sin fallos, en Postgres a lo sumo el 90 %, y la
#     conexión directa entra.
#
#   lab.sh arriba && p55b.sh
source "$(dirname "$0")/lab.sh"
fallos=0
P="p55b-$(date +%s | tail -c 6)"
R="/v1/postgres/proyectos/$P/ramas"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p55b\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
[ -n "$(listo demo "$P")" ] || { echo "✗ main no queda lista"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$R" '{"id":"chica"}')")" = hecha ] || { echo "✗ la rama chica"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$R/chica/endpoints" '{"id":"chica","cu_min":"0.25","cu_max":"0.25"}')")" = hecha ]   || { echo "✗ el endpoint chica"; exit 1; }
# Una base más: configurar-rama → /configure → compute_ctl rehace el pool (pgbouncer_settings) sin reiniciar.
[ "$(hecha demo "$(pide demo POST "$R/chica/bases" "{\"nombre\":\"otra\",\"dueno\":\"$ROL\"}")")" = hecha ]   || { echo "✗ la base otra"; exit 1; }
EP=$(pide demo GET "$R/chica/endpoints/chica"); VM=$(campo "$EP" vm)
[ -n "$VM" ] || { echo "✗ sin endpoint: ${EP:0:300}"; exit 1; }
echo "── $P: rama chica, endpoint de 0,25 CU ($VM) con dos bases ($P y otra)"
m=$(campo "$EP" conexiones maximas); pool=$(campo "$EP" conexiones pool_por_base)
[ "$m" = 112 ] && [ "$pool" = 50 ] && echo "  ✓ la API: max_connections $m, pool por base $pool (90 % entre las 2 bases)" \
  || { echo "  ✗ la API dice max $m, pool $pool"; fallos=$((fallos+1)); }

por() { local h=$1 b=$2; shift 2
  dc exec -T -e PGPASSWORD="$CLAVE" -e PGHOST="$h.$DOMINIO" -e PGHOSTADDR=172.29.51.20 -e PGPORT=4432 \
    -e PGUSER="$ROL" -e PGDATABASE="$b" -e PGSSLMODE=verify-full -e PGSSLROOTCERT=/llaves/tls/tls.crt \
    -e PGCONNECT_TIMEOUT=15 cliente "$@"; }
dos_a_la_vez() {   # → «hechas fallidas pico»: 150 clientes por -pooler en cada base, 15 s
  local T; T=$(mktemp -d)
  ( m=0; while [ ! -f "$T/fin" ]; do
      n=$(en_vm "$VM" "select count(*) from pg_stat_activity where usename = '$ROL'")
      [ "${n:-0}" -gt $m ] && m=$n; sleep 0.5; done; echo $m > "$T/pico" ) &
  # Cada transacción retiene su conexión de servidor 50 ms: el pool la pide entera.
  local carga='echo "select pg_sleep(0.05);" > /tmp/s.sql && exec pgbench -n -f /tmp/s.sql -c 150 -j 4 -T 15'
  por "$VM-pooler" "$P" sh -c "$carga" > "$T/a" 2>&1 &
  por "$VM-pooler" otra sh -c "$carga" > "$T/b" 2>&1 &
  # A mitad de la carga, una conexión DIRECTA (sin pool): ¿le queda sitio en Postgres?
  sleep 8; por "$VM" "$P" psql -Atc 'select 1' > "$T/directa" 2>&1
  wait %2 %3; touch "$T/fin"; wait
  local h=0 f=0 x
  for x in a b; do
    h=$((h + $(sed -n 's/^number of transactions actually processed: \([0-9]*\).*/\1/p' "$T/$x" | head -1 | grep . || echo 0)))
    f=$((f + $(sed -n 's/^number of failed transactions: \([0-9]*\).*/\1/p' "$T/$x" | head -1 | grep . || echo 0)))
    grep -qiE "error|fatal|aborted" "$T/$x" && f=$((f + 1))
  done
  echo "$h $f $(cat "$T/pico") $(tail -1 "$T/directa" | tr ' ' '_' | cut -c1-120)"; { echo "[a]"; grep -iE "error|fatal|processed|failed" "$T/a" | sort | uniq -c | head -5; echo "[b]"; grep -iE "error|fatal|processed|failed" "$T/b" | sort | uniq -c | head -5; } >&2; rm -rf "$T"
}

# El pool de la VM, a mano (lo que escribió compute_ctl en su pgbouncer.ini, y SIGHUP para que lo relea).
pool_a() { docker exec "$VM" sh -c "sed -i 's/^default_pool_size=.*/default_pool_size=$1/; s/^max_db_connections=.*/max_db_connections=$2/' /tmp/pgbouncer.ini && kill -HUP \$(cat /tmp/pgbouncer.pid)"; }
docker exec "$VM" grep -q "^max_db_connections=$pool" /tmp/pgbouncer.ini && echo "  ✓ compute_ctl puso el pool de la especificación ($pool por base)"   || { echo "  ✗ el pgbouncer.ini de la VM no lleva max_db_connections=$pool"; fallos=$((fallos+1)); }
pool_a 64 0   # «antes»: la imagen tal cual, 64 por pareja y sin techo por base
echo "── ANTES: el pgbouncer.ini de la imagen (64 por pareja), Postgres de 112"
read -r h f pico directa < <(dos_a_la_vez 2>/dev/null)
# El «antes» es un dato, no una comprobación: llena Postgres siempre, pero que un cliente choque con el
# «server login has been failing» de pgbouncer (15 s tras un rechazo) o que la directa no quepa depende
# del momento (2 de 5 pasadas el 2026-10-09).
echo "  · 2×150 por el pool: $h transacciones, $f fallidas, pico $pico conexiones en Postgres (de 109 para usuarios)"
[ "$directa" = 1 ] && echo "  · la directa a mitad de carga: entró esta vez" || echo "  · la directa a mitad de carga: no cupo (${directa//_/ })"

pool_a "$pool" "$pool"
echo "── DESPUÉS: lo que manda la especificación (pool $pool por base)"
read -r h f pico directa < <(dos_a_la_vez 2>/dev/null)
[ "$f" = 0 ] && [ "$h" -gt 0 ] && echo "  ✓ 2×150 clientes por el pool, $h transacciones, 0 fallidas"   || { echo "  ✗ $f fallos"; fallos=$((fallos+1)); }
[ "$pico" -le 100 ] && echo "  ✓ en Postgres, como mucho $pico conexiones del rol (≤ 90 % de 112)"   || { echo "  ✗ pico $pico en Postgres"; fallos=$((fallos+1)); }
[ "$directa" = 1 ] && echo "  ✓ y la conexión directa entra: queda el 10 %"   || { echo "  ✗ la directa no entra: ${directa//_/ }"; fallos=$((fallos+1)); }

hecha demo "$(pide demo DELETE "$R/chica/endpoints/chica")" >/dev/null
hecha demo "$(pide demo DELETE "$R/chica")" >/dev/null
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P5·5b (laboratorio) ✓ todo" || { echo "P5·5b (laboratorio) ✗ $fallos fallos"; exit 1; }
