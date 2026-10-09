#!/usr/bin/env bash
# P6·5 en el laboratorio (ADR 0058): el pool precalentado. Con el reconciliador de verdad y el arranque de una
# VM emulado (ARRANQUE=35 s, el de P3, en el compute_ctl de mentira: no contesta hasta entonces):
#   A · en frío (POOL=0): despertar = arrancar la VM entera;
#   B · desde el pool (POOL=3): despertar = /configure a un cómputo ya arrancado y vacío;
#       p50 y p95 de cada uno; un cómputo usado no vuelve al pool;
#   C · la IP reutilizada (P6·0, 7): tras dormir, un Postgres impostor en la IP vieja con el mismo rol y otra
#       contraseña; el proxy (con esa dirección en caché) falla contra él, la olvida, despierta y entra.
# Para no esperar 60 s por muestra, «dormir ya» atrasa ultima_actividad: la vigilancia lo duerme en ≤ 5 s.
#
#   lab.sh arriba && p65.sh        (unos 12 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
ms() { date +%s%3N; }
plano_con() { ARRANQUE=$1 POOL=$2 dc up -d plano >/dev/null 2>&1; sleep 3
  dc exec -T cliente bash -c 'for i in $(seq 1 30); do exec 3<>/dev/tcp/172.29.51.7/8100 && break; sleep 1; done' 2>/dev/null; }
sin_pool() { local p; p=$(docker ps -aq --filter label=ore.dev/pool=si); [ -n "$p" ] && docker rm -f $p >/dev/null
  sql_plano "delete from plano.pool_computo" >/dev/null; }
libres() { sql_plano "select count(*) from plano.pool_computo where estado = 'libre'"; }
percentil() { "$PY" -c 'import sys,statistics as s
v=sorted(int(x) for x in sys.argv[2:]); q=float(sys.argv[1])
i=min(len(v)-1, max(0, round(q*(len(v)-1)))); print(v[i])' "$@"; }

P="p65-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
sql() { dc exec -T -e PGPASSWORD="$CLAVE" -e PGCONNECT_TIMEOUT=90 cliente psql \
  "host=$VM.$DOMINIO hostaddr=172.29.51.20 port=4432 user=$ROL dbname=$P sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" "$@"; }
observado() { campo "$(pide demo GET "$E")" estado observado; }
dormir_ya() { sql_plano "update plano.endpoint set ultima_actividad = now() - interval '1 hour' where vm = '$VM'" >/dev/null
  local t0; t0=$(date +%s); until [ "$(observado)" = dormido ] || [ $(( $(date +%s) - t0 )) -ge 60 ]; do sleep 1; done
  [ "$(observado)" = dormido ]; }
despertar() { local t0 n; t0=$(ms); n=$(sql -Atqc "select 1" 2>&1 | tail -1); [ "$n" = 1 ] && echo $(( $(ms) - t0 )) || echo "fallo:${n:0:80}"; }

echo "── A · en frío: ARRANQUE=35 s, sin pool"
plano_con 35 0; sin_pool
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p65\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
t0=$(date +%s); VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
bien "crear en frío: $(( $(date +%s) - t0 )) s hasta listo"
hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"60"}')" >/dev/null
FRIO=()
for i in 1 2 3 4; do
  dormir_ya || { mal "no se durmió"; break; }
  t=$(despertar); case "$t" in fallo*) mal "despertar en frío: $t";; *) FRIO+=("$t"); echo "  · despertar en frío $i: $t ms";; esac
done
[ ${#FRIO[@]} -gt 0 ] && bien "en frío: p50 $(percentil 0.5 "${FRIO[@]}") ms · p95 $(percentil 0.95 "${FRIO[@]}") ms (${#FRIO[@]} muestras)"

echo "── B · desde el pool: ARRANQUE=35 s, POOL=3"
plano_con 35 3
t0=$(date +%s); until [ "$(libres)" -ge 3 ] || [ $(( $(date +%s) - t0 )) -ge 120 ]; do sleep 2; done
bien "el pool: $(libres) cómputos libres (vacíos, compute_ctl en empty) a los $(( $(date +%s) - t0 )) s"
POOL_MS=(); USADOS=()
for i in 1 2 3 4 5 6 7 8; do
  dormir_ya || { mal "no se durmió"; break; }
  t0=$(date +%s); until [ "$(libres)" -ge 1 ] || [ $(( $(date +%s) - t0 )) -ge 90 ]; do sleep 1; done
  t=$(despertar); c=$(campo "$(pide demo GET "$E")" computo)
  case "$t" in fallo*) mal "despertar desde el pool: $t";; *) POOL_MS+=("$t"); USADOS+=("$c"); echo "  · despertar desde el pool $i: $t ms ($c)";; esac
done
[ ${#POOL_MS[@]} -gt 0 ] && bien "desde el pool: p50 $(percentil 0.5 "${POOL_MS[@]}") ms · p95 $(percentil 0.95 "${POOL_MS[@]}") ms (${#POOL_MS[@]} muestras)"
distintos=$(printf '%s\n' "${USADOS[@]}" | sort -u | grep -c pool-)
[ "$distintos" = "${#USADOS[@]}" ] && bien "cada despertar, un cómputo del pool distinto: ninguno usado volvió" || mal "se repitió un cómputo del pool: ${USADOS[*]}"
vivos=$(for c in "${USADOS[@]}"; do docker ps -q --filter name="^/$c\$"; done | wc -l)
[ "$vivos" = 1 ] && bien "de los ${#USADOS[@]} usados sólo queda el que sirve ahora: los demás se destruyeron al dormir" || mal "quedan $vivos de los usados"

echo "── C · la IP reutilizada por otro"
X=$(campo "$(pide demo GET "$E")" direccion)
[ "$(sql -Atqc 'select 1' | tail -1)" = 1 ] && bien "despierto en $X, y el proxy tiene esa dirección en caché"
dormir_ya && bien "dormido: $X queda libre"
docker run -d --name p65-impostor --network p5lab_lab --ip "$X" -e POSTGRES_USER="$ROL" -e POSTGRES_PASSWORD=otra-cosa \
  -e POSTGRES_DB="$P" -e POSTGRES_HOST_AUTH_METHOD=scram-sha-256 postgres:17 >/dev/null
until docker exec p65-impostor pg_isready -q -h 127.0.0.1 2>/dev/null; do sleep 1; done; sleep 2
bien "un Postgres impostor en $X: el mismo rol ($ROL), otra contraseña"
t=$(despertar)
case "$t" in fallo*) mal "con un impostor en la IP vieja no entra: $t";;
  *) [ "$(campo "$(pide demo GET "$E")" direccion)" != "$X" ] \
       && bien "entra en ${t} ms, en su cómputo nuevo ($(campo "$(pide demo GET "$E")" direccion)), no en el impostor" || mal "entró en $X";; esac
docker logs p65-impostor 2>&1 | grep -q "password authentication failed\|authentication failed" \
  && bien "el impostor vio el intento y lo rechazó: el proxy olvidó la dirección y volvió a despertar" \
  || echo "  · el impostor no vio el intento (el proxy no usó la dirección en caché)"
docker rm -f p65-impostor >/dev/null

plano_con 0 0; sin_pool
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P6·5 (laboratorio) ✓ todo" || { echo "P6·5 (laboratorio) ✗ $fallos fallos"; exit 1; }
