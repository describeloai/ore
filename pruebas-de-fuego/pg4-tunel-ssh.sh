#!/usr/bin/env bash
# PG4 · UN POSTGRES DETRÁS DE UN BASTIÓN (ADR 0065, D-PG3), en Docker y sin cuentas.
#
#   La base (`pgdb`, TLS con una CA propia) vive en una red INTERNA, sin salida y sin entrada desde
#   fuera; el bastión (OpenSSH, `AllowTcpForwarding yes`) está en las dos. El driver sólo ve la
#   pública: a la base sólo se llega por el túnel, con la clave de la celda y la huella fijada.
#
#   1  sin túnel, la base no se alcanza
#   2  túnel sin huella: no autentica, y dice la huella vista para confirmarla
#   3  túnel con la huella y la CA: check, catalogo, Arrow, v1 y servir ×3; TLS verifica `pgdb`
#   4  una huella que no es: se niega sin autenticar
#   5  una clave que el bastión no tiene: se niega y dice dónde va la pública
#   6  el bastión no llega al host pedido: lo dice (y nombra AllowTcpForwarding)
#   7  un bastión sin reenvío (AllowTcpForwarding no): lo mismo
#   8  cancelar a mitad de un flujo, por el túnel: la consulta muere en el origen
#   9  4 lectores a la vez, cada uno con su túnel: 20/20
#
#   bash pruebas-de-fuego/pg4-tunel-ssh.sh
#
# Necesita Docker. Claves, CA y contraseñas de laboratorio, de usar y tirar.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm
PUB=ore-pg4-pub
PRIV=ore-pg4-priv
PW=lab

if [ "${1:-}" = "--dentro" ]; then
  cd /w
  command -v psql >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq postgresql-client python3 >/dev/null 2>&1; }
  cargo build --quiet -p ore-read-postgres 2>&1 | tail -3
  D="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-postgres"
  unset ORE_PG_LABORATORIO
  export ORE_SFTP_CLAVE=/claves/celda
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  cod() { python3 -c 'import sys,urllib.parse as u;print(u.quote(sys.argv[1],safe=""))' "$1"; }
  CA=$(python3 -c 'import urllib.parse as u;print(u.quote(open("/certs/ca.crt").read(),safe=""))')
  H=$(cod "$HUELLA")
  url() { echo "postgres://postgres:$PW@${1:-pgdb}:5432/postgres?sslrootcert=$CA${2:+&$2}"; }
  check() { printf '{"url":"%s"}' "$1" | timeout 60 "$D" check 2>&1; }
  ok() { case "$1" in *'"ok":true'*) return 0;; esac; return 1; }
  for _ in $(seq 60); do (exec 3<>/dev/tcp/bastion/22) 2>/dev/null && break; sleep 1; done

  r=$(check "$(url)"); ! ok "$r" && dice "1 sin túnel, la base no se alcanza ($(echo "$r" | cut -c1-90))" || falla "1: $r"

  r=$(check "$(url pgdb "ssh=ore@bastion")")
  ! ok "$r" && echo "$r" | grep -q "$HUELLA" && dice "2 sin huella: no autentica y enseña la vista" || falla "2: $r"

  U=$(url pgdb "ssh=ore@bastion&ssh_huella=$H")
  ok "$(check "$U")" && dice "3 check por el túnel, TLS verificando pgdb" || falla "3 check: $(check "$U")"
  printf '%s' "$U" | "$D" catalogo lab 2>/dev/null | grep -q '"public.t"' && dice "3 catalogo" || falla "3 catalogo: $(printf '%s' "$U" | "$D" catalogo lab 2>&1 | tail -2)"
  printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow","limit":1000}' "$U" | "$D" leer >/tmp/a.bin 2>/tmp/a.err \
    && [ "$(stat -c %s /tmp/a.bin)" -gt 1000 ] && dice "3 Arrow ($(stat -c %s /tmp/a.bin) B)" || falla "3 Arrow: $(cat /tmp/a.err)"
  v=$(printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"claveColumnas":["id"],"claves":[["7"]],"filtros":[]}' "$U" | "$D" leer lab 2>&1)
  [ "$v" = '{"id":"7"}' ] && dice "3 v1" || falla "3 v1: $v"
  n=$(for i in 1 2 3; do printf '{"id":"r%s","url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow","limit":100}\n' $i "$U"; sleep 0.5; done | "$D" servir 2>/dev/null | grep -a -c '"fin":"ok"')
  [ "$n" = 3 ] && dice "3 servir ×3" || falla "3 servir: $n/3"

  r=$(check "$(url pgdb "ssh=ore@bastion&ssh_huella=$(cod 'SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA')")")
  ! ok "$r" && echo "$r" | grep -qi "impersonat\|otro\|pinned" && dice "4 huella que no es: se niega" || falla "4: $r"

  r=$(ORE_SFTP_CLAVE=/claves/otra check "$U")
  ! ok "$r" && echo "$r" | grep -q "authorized_keys" && dice "5 clave que no está: se niega y dice authorized_keys" || falla "5: $r"

  r=$(check "$(url nada.invalido "ssh=ore@bastion&ssh_huella=$H")")
  ! ok "$r" && echo "$r" | grep -q "AllowTcpForwarding" && dice "6 el bastión no llega: lo dice" || falla "6: $r"

  r=$(check "$(url pgdb "ssh=ore@bastion2&ssh_huella=$(cod "$HUELLA2")")")
  ! ok "$r" && echo "$r" | grep -q "AllowTcpForwarding" && dice "7 bastión sin reenvío: lo dice" || falla "7: $r"

  printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id","v":"v"},"formato":"arrow"}' "$U" \
    | "$D" leer 2>/tmp/c.err | (sleep 30; cat >/dev/null) &
  sleep 5
  vivo=$(printf '{"url":"%s","objeto":"pg_stat_activity","proyeccion":{"s":"state"},"formato":"arrow","filtros":[{"columna":"application_name","operador":"eq","valor":"ore-read-postgres"},{"columna":"state","operador":"eq","valor":"idle in transaction"}]}' "$U" | "$D" leer 2>/dev/null | python3 -c 'import sys;print(sys.stdin.buffer.read().count(b"idle in transaction"))')
  pkill -TERM -f "debug/ore-read-postgres leer"
  sleep 4
  despues=$(printf '{"url":"%s","objeto":"pg_stat_activity","proyeccion":{"s":"state"},"formato":"arrow","filtros":[{"columna":"application_name","operador":"eq","valor":"ore-read-postgres"},{"columna":"state","operador":"eq","valor":"idle in transaction"}]}' "$U" | "$D" leer 2>/dev/null | python3 -c 'import sys;print(sys.stdin.buffer.read().count(b"idle in transaction"))')
  [ "$vivo" -ge 1 ] && [ "$despues" = 0 ] && dice "8 cortar por el túnel: sesiones abiertas $vivo → $despues" \
    || falla "8: activas $vivo → $despues · $(cat /tmp/c.err)"
  kill %1 2>/dev/null; wait 2>/dev/null

  for k in 1 2 3 4; do
    ( for n in 1 2 3 4 5; do printf '{"id":"c%s-%s","url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow","limit":%s}\n' $k $n "$U" $((500 * n)); sleep 0.3; done \
        | timeout 120 "$D" servir >/tmp/c$k.bin 2>/tmp/c$k.err ) &
  done
  wait
  ok=$(cat /tmp/c?.bin | grep -a -c '"fin":"ok"')
  [ "$ok" = 20 ] && dice "9 4 lectores a la vez por túneles: 20/20" || falla "9: $ok/20 $(cat /tmp/c?.err | head -3)"

  echo
  [ $fallos = 0 ] && echo "PG4: todo verde" || echo "PG4: $fallos fallos"
  exit $fallos
fi

command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() {
  docker rm -f pgdb bastion bastion2 >/dev/null 2>&1
  docker volume rm -f pg4-certs pg4-claves >/dev/null 2>&1
  docker network rm $PUB $PRIV >/dev/null 2>&1
}
limpiar
docker network create $PUB >/dev/null
docker network create --internal $PRIV >/dev/null
docker volume create pg4-certs >/dev/null
docker volume create pg4-claves >/dev/null
docker run --rm -v pg4-certs:/c -v pg4-claves:/k alpine:3.20 sh -c '
  apk add -q openssl openssh-keygen >/dev/null; cd /c
  openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.crt -days 2 -subj "/CN=ORE PG4 CA" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -keyout srv.key -out srv.csr -subj "/CN=pgdb" 2>/dev/null
  printf "subjectAltName=DNS:pgdb\n" > ext
  openssl x509 -req -in srv.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out srv.crt -days 2 -extfile ext 2>/dev/null
  chmod 644 *.crt srv.key
  ssh-keygen -q -t ed25519 -N "" -f /k/celda -C celda; ssh-keygen -q -t ed25519 -N "" -f /k/otra -C otra
  chmod 644 /k/*' || exit 1
docker run -d --name pgdb --network $PRIV -e POSTGRES_PASSWORD=$PW -v pg4-certs:/c:ro --entrypoint sh postgres:16-alpine -c \
  'cp /c/srv.crt /c/srv.key /var/lib/postgresql/ && chown postgres /var/lib/postgresql/srv.* && chmod 600 /var/lib/postgresql/srv.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/var/lib/postgresql/srv.crt -c ssl_key_file=/var/lib/postgresql/srv.key' >/dev/null || exit 1
bastion() { # NOMBRE REENVIO
  docker create --name $1 --network $PUB -v pg4-claves:/k:ro alpine:3.20 sh -c "
    apk add -q openssh-server >/dev/null && ssh-keygen -A >/dev/null
    adduser -D ore && passwd -u ore >/dev/null 2>&1; mkdir -p /home/ore/.ssh && cp /k/celda.pub /home/ore/.ssh/authorized_keys
    chown -R ore /home/ore/.ssh && chmod 700 /home/ore/.ssh && chmod 600 /home/ore/.ssh/authorized_keys
    # sshd se queda con la PRIMERA aparición, y Alpine ya trae AllowTcpForwarding no.
    sed -i '/^ *AllowTcpForwarding/d; /^ *PasswordAuthentication/d' /etc/ssh/sshd_config
    printf 'PasswordAuthentication no\nAllowTcpForwarding $2\nPermitOpen any\n' >> /etc/ssh/sshd_config
    exec /usr/sbin/sshd -D -e" >/dev/null || exit 1
  docker network connect $PRIV $1
  docker start $1 >/dev/null
}
bastion bastion yes
bastion bastion2 no
huella() { for _ in $(seq 60); do h=$(docker exec $1 sh -c 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub 2>/dev/null' | awk '{print $2}'); [ -n "$h" ] && { echo "$h"; return; }; sleep 1; done; }
HUELLA=$(huella bastion); HUELLA2=$(huella bastion2)
for _ in $(seq 60); do docker exec pgdb pg_isready -q && break; sleep 1; done
docker exec -e PGPASSWORD=$PW pgdb psql -U postgres -XAtq -c "create table t as select g id, repeat(md5(g::text),8) v from generate_series(1,300000) g; alter table t add primary key (id)" >/dev/null
docker run --rm --network $PUB -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v pg4-certs:/certs:ro -v pg4-claves:/claves:ro \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main -e PW=$PW \
  -e HUELLA="$HUELLA" -e HUELLA2="$HUELLA2" -w /w "$RUST_IMG" bash pruebas-de-fuego/pg4-tunel-ssh.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
rc=${PIPESTATUS[0]}
limpiar
exit $rc
