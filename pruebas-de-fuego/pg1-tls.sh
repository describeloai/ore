#!/usr/bin/env bash
# PG1 · TLS DE VERDAD (ADR 0065, D-PG1), en Docker y sin cuentas. Lo que PG0 midió roto, comprobado:
#
#   1  CA propia sin darla: no conecta, y el error dice que falta `sslrootcert`
#   2  la CA en la URL (`sslrootcert=<PEM>`): check, catalogo, leer en Arrow y servir ×3
#   3  `require`, `verify-full` y `verify-ca` con la CA: conectan (antes, «invalid connection string»)
#   4  el certificado sin el nombre (pg-mal): no conecta y dice `verify-ca`; con `verify-ca`, sí
#   5  la CA de otro: no conecta (con `sslrootcert`, sólo esa CA vale)
#   6  un servidor sin TLS: no conecta y lo dice; `disable`/`allow`/`prefer` se niegan sin abrir nada
#   7  con ORE_PG_LABORATORIO=1, el servidor sin TLS se lee como antes
#   8  cancelar a mitad de un flujo, por TLS con la CA propia: la consulta muere en el origen
#
#   bash pruebas-de-fuego/pg1-tls.sh
#
# Necesita Docker. Las contraseñas y las CA son de laboratorio, de usar y tirar.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm
RED=ore-pg1
PW=lab

if [ "${1:-}" = "--dentro" ]; then
  cd /w
  command -v psql >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq postgresql-client >/dev/null 2>&1; }
  cargo build --quiet -p ore-read-postgres 2>&1 | tail -3
  D="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-postgres"
  unset ORE_PG_LABORATORIO
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  for h in pg16 pg-plano; do for _ in $(seq 60); do pg_isready -q -h $h && break; sleep 1; done; done
  cod() { python3 -c 'import sys,urllib.parse as u;print(u.quote(open(sys.argv[1]).read(),safe=""))' "$1"; }
  CA=$(cod /certs/ca.crt); OTRA=$(cod /certs/otra.crt)
  url() { echo "postgres://postgres:$PW@$1:5432/postgres${2:+?$2}"; }
  check() { printf '{"url":"%s"}' "$1" | timeout 30 "$D" check 2>&1; }
  ok() { case "$1" in *'"ok":true'*) return 0;; esac; return 1; }

  r=$(check "$(url pg16)")
  ! ok "$r" && echo "$r" | grep -q sslrootcert && dice "1 CA propia sin darla: no conecta y pide sslrootcert" || falla "1: $r"

  U=$(url pg16 "sslrootcert=$CA")
  ok "$(check "$U")" && dice "2 check con la CA en la URL" || falla "2 check: $(check "$U")"
  PGPASSWORD=$PW psql -h pg16 -U postgres -XAtq -c "drop table if exists t; create table t as select g id, repeat(md5(g::text),8) v from generate_series(1,300000) g; alter table t add primary key (id)" >/dev/null
  printf '%s' "$U" | "$D" catalogo lab 2>/dev/null | grep -q '"public.t"' && dice "2 catalogo" || falla "2 catalogo: $(printf '%s' "$U" | "$D" catalogo lab 2>&1 | tail -2)"
  printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow","limit":1000}' "$U" | "$D" leer >/tmp/a.bin 2>/tmp/a.err \
    && [ "$(stat -c %s /tmp/a.bin)" -gt 1000 ] && dice "2 leer en Arrow ($(stat -c %s /tmp/a.bin) B)" || falla "2 leer: $(cat /tmp/a.err)"
  n=$(for i in 1 2 3; do printf '{"id":"r%s","url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow","limit":100}\n' $i "$U"; done | "$D" servir 2>/dev/null | grep -a -c '"fin":"ok"')
  [ "$n" = 3 ] && dice "2 servir ×3" || falla "2 servir: $n/3"

  for m in require verify-full verify-ca; do
    r=$(check "$(url pg16 "sslmode=$m&sslrootcert=$CA")")
    ok "$r" && dice "3 sslmode=$m con la CA" || falla "3 $m: $r"
  done

  r=$(check "$(url pg-mal "sslrootcert=$CA")")
  ! ok "$r" && echo "$r" | grep -q verify-ca && dice "4 pg-mal: no conecta y propone verify-ca" || falla "4: $r"
  r=$(check "$(url pg-mal "sslmode=verify-ca&sslrootcert=$CA")")
  ok "$r" && dice "4 pg-mal con verify-ca: conecta" || falla "4 verify-ca: $r"

  r=$(check "$(url pg16 "sslrootcert=$OTRA")")
  ! ok "$r" && dice "5 la CA de otro: no conecta" || falla "5: $r"

  r=$(check "$(url pg-plano)")
  ! ok "$r" && echo "$r" | grep -qi "no habla TLS" && dice "6 sin TLS: no conecta y lo dice" || falla "6: $r"
  for m in disable allow prefer; do
    r=$(check "$(url pg-plano "sslmode=$m")")
    ! ok "$r" && echo "$r" | grep -q "no se admite" && dice "6 sslmode=$m se niega" || falla "6 $m: $r"
  done
  antes=$(PGPASSWORD=$PW psql -h pg-plano -U postgres -XAtq -c "select count(*) from pg_stat_activity where application_name='ore-read-postgres'")

  r=$(ORE_PG_LABORATORIO=1 check "$(url pg-plano)")
  ok "$r" && dice "7 con ORE_PG_LABORATORIO=1, sin TLS como antes" || falla "7: $r"

  # 8 · El consumidor va despacio: el portal se queda abierto; SIGTERM cancela en el origen.
  printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id","v":"v"},"formato":"arrow"}' "$U" \
    | "$D" leer 2>/tmp/c.err | (sleep 30; cat >/dev/null) &
  sleep 3
  q() { PGPASSWORD=$PW psql -h pg16 -U postgres -XAtq -c "select count(*) from pg_stat_activity where application_name='ore-read-postgres' and state<>'idle'"; }
  vivo=$(q)
  pkill -TERM -f "debug/ore-read-postgres leer"
  sleep 3
  despues=$(q)
  [ "$vivo" -ge 1 ] && [ "$despues" = 0 ] && dice "8 cancelar por TLS con la CA propia: $vivo → $despues en el origen ($(tr -d '\n' </tmp/c.err | cut -c1-120))" \
    || falla "8 cancelar: activas antes $vivo, después $despues · $(cat /tmp/c.err)"
  kill %1 2>/dev/null; wait 2>/dev/null

  echo
  [ $fallos = 0 ] && echo "PG1: todo verde" || echo "PG1: $fallos fallos"
  exit $fallos
fi

command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f pg16 pg-plano >/dev/null 2>&1; docker volume rm -f pg1-certs >/dev/null 2>&1; docker network rm $RED >/dev/null 2>&1; }
limpiar
docker network create $RED >/dev/null
docker volume create pg1-certs >/dev/null
docker run --rm -v pg1-certs:/c alpine:3.20 sh -c '
  apk add -q openssl >/dev/null; cd /c
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.crt -days 2 -subj "/CN=ORE PG1 CA" 2>/dev/null
  openssl req -x509 -newkey rsa:2048 -nodes -keyout otra.key -out otra.crt -days 2 -subj "/CN=Otra CA" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout srv.key -out srv.csr -subj "/CN=pg16" 2>/dev/null
  printf "subjectAltName=DNS:pg16\n" > ext
  openssl x509 -req -in srv.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out srv.crt -days 2 -extfile ext 2>/dev/null
  chmod 644 *.crt srv.key' || exit 1
docker run -d --name pg16 --network $RED --network-alias pg-mal -e POSTGRES_PASSWORD=$PW -v pg1-certs:/c:ro --entrypoint sh postgres:16-alpine -c \
  'cp /c/srv.crt /c/srv.key /var/lib/postgresql/ && chown postgres /var/lib/postgresql/srv.* && chmod 600 /var/lib/postgresql/srv.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/var/lib/postgresql/srv.crt -c ssl_key_file=/var/lib/postgresql/srv.key' >/dev/null || exit 1
docker run -d --name pg-plano --network $RED -e POSTGRES_PASSWORD=$PW postgres:16-alpine >/dev/null || exit 1
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v pg1-certs:/certs:ro \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main -e PW=$PW \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/pg1-tls.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
rc=${PIPESTATUS[0]}
limpiar
exit $rc
