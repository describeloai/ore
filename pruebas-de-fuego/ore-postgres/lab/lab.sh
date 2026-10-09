#!/usr/bin/env bash
# El laboratorio local de P5 (ADR 0058): `lab.sh arriba | abajo`, y lo que usan las pruebas.
#
#   arriba   genera en .secretos/ el certificado autofirmado de *.europe-west1.pg.paladio.io,
#            el token del proxy y las contraseñas de las bases (nada sale de ahí ni se imprime),
#            y levanta compose.yaml. El binario de ore-postgres es el de /tt/p4pg (volumen
#            ore-pruebas-t): compilarlo antes con `cargo build --release -p ore-postgres`.
#   abajo    lo quita todo, .secretos/ incluido.
#
# Sourced (`source lab.sh`), da a las pruebas:
#   pide CELDA METODO RUTA [CUERPO]   → la API de ore-postgres como esa celda (JSON en stdout)
#   campo JSON a b c                  → un campo
#   reconcilia VM COMPUTO             → hace de reconciliador: el cómputo arranca con los roles y
#                                       bases del endpoint y la fila queda `listo` con su dirección
#   hechas                            → las operaciones en curso, hechas (y 2 s para que el proxy olvide)
#   barre                             → y al borrar: las filas que la API dio por borradas se van
#   entra ROL CLAVE BASE SNI          → psql por el proxy, lo que contesta (la última línea)
set -uo pipefail
LAB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -W 2>/dev/null || pwd)"
DOMINIO=europe-west1.pg.paladio.io
export MSYS_NO_PATHCONV=1
dc() { docker compose -f "$LAB/compose.yaml" "$@"; }
PY=$(command -v python3 || command -v python)
azar() { "$PY" -c 'import secrets, sys; sys.stdout.write(secrets.token_urlsafe(32))'; }   # sin el fin de línea de Windows

arriba() {
  local S="$LAB/.secretos"
  mkdir -p "$S/tls" "$S/proxy"
  if [ ! -s "$S/tls/tls.crt" ]; then
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 7 \
      -subj "/CN=*.$DOMINIO" -addext "subjectAltName=DNS:*.$DOMINIO" \
      -keyout "$S/tls/tls.key" -out "$S/tls/tls.crt" 2>/dev/null
    chmod 644 "$S/tls/tls.key"
    azar > "$S/proxy/token"
    local b c; b=$(azar); c=$(azar)
    printf 'POSTGRES_PASSWORD=%s\n' "$b" > "$S/base.env"
    printf 'ORE_POSTGRES_URL=postgresql://postgres:%s@172.29.51.5/ore_postgres\n' "$b" > "$S/plano.env"
    printf 'POSTGRES_PASSWORD=%s\n' "$c" > "$S/computo.env"
  fi
  dc build -q computo-a || return 1
  dc up -d --wait --wait-timeout 120 base iam redis computo-a computo-b cliente >/dev/null || return 1
  # La base vacía de las pruebas de contrato del crate (ORE_POSTGRES_PRUEBA_URL).
  dc exec -T base psql -U postgres -qc 'create database ore_postgres_prueba' >/dev/null 2>&1
  dc up -d plano proxy >/dev/null || return 1
  for _ in $(seq 1 60); do
    dc exec -T cliente bash -c 'exec 3<>/dev/tcp/172.29.51.7/8100 && exec 4<>/dev/tcp/172.29.51.20/4432' 2>/dev/null && break
    sleep 1
  done
  dc logs plano | tail -8
}
abajo() { dc down -v --remove-orphans >/dev/null 2>&1; rm -rf "$LAB/.secretos"; }

pide() {
  local c=$1 m=$2 r=$3 b=${4:-}
  dc exec -T cliente bash -c 'exec 3<>/dev/tcp/172.29.51.7/8100' 2>/dev/null || { echo '{"error":"sin plano"}'; return; }
  docker run --rm --network p5lab_lab curlimages/curl:8.10.1 -s -X "$m" \
    -H "Authorization: Bearer $c" -H 'Content-Type: application/json' ${b:+-d "$b"} "http://172.29.51.7:8100$r"
}
campo() { local j=$1; shift; "$PY" -c 'import json,sys
v=json.loads(sys.argv[1])
for k in sys.argv[2:]: v=(v or {}).get(k)
print("" if v is None else v)' "$j" "$@"; }
sql_plano() { dc exec -T base psql -U postgres -d ore_postgres -v ON_ERROR_STOP=1 -Atc "$1"; }

reconcilia() {   # reconcilia VM COMPUTO (computo-a | computo-b)
  local vm=$1 c=$2 ip
  ip=$(docker inspect -f '{{(index .NetworkSettings.Networks "p5lab_lab").IPAddress}}' "p5lab-$c-1")
  # Los roles de la rama del endpoint, con su verificador tal cual: el cómputo nunca ve la contraseña.
  sql_plano "select format('do \$\$ begin if exists (select from pg_roles where rolname = %L) then alter role %I login password %L;
                             else create role %I login password %L; end if; end \$\$;',
                             r.nombre, r.nombre, r.verificador, r.nombre, r.verificador)
               from plano.endpoint e join plano.rol r using (organizacion, proyecto, rama)
              where e.vm = '$vm' and r.deseado = 'vivo'" | dc exec -T -e PGOPTIONS='-c client_min_messages=warning' "$c" psql -q -U postgres -p 5432 -v ON_ERROR_STOP=1 >/dev/null
  sql_plano "select format('create database %I owner %I', b.nombre, b.dueno)
               from plano.endpoint e join plano.base b using (organizacion, proyecto, rama)
              where e.vm = '$vm' and b.deseado = 'vivo'" | while read -r s; do
    dc exec -T "$c" psql -q -U postgres -p 5432 -c "$s" </dev/null >/dev/null 2>&1; done   # sin </dev/null, exec se come el bucle
  # Las conexiones, como compute_ctl con la especificación: max_connections (reinicia, como al arrancar
  # la VM) y el pool de pgbouncer (pgbouncer.ini + recarga). Lo que dice la API del endpoint.
  # Con POOL_DE_NEON=1 se deja el pgbouncer.ini de la imagen tal cual (64 por pareja): el «antes».
  local org p r id celda ep maximas pool
  IFS='|' read -r org p r id < <(sql_plano "select organizacion, proyecto, rama, id from plano.endpoint where vm = '$vm'")
  celda=${org#org_}
  ep=$(pide "$celda" GET "/v1/postgres/proyectos/$p/ramas/$r/endpoints/$id")
  maximas=$(campo "$ep" conexiones maximas); pool=$(campo "$ep" conexiones pool_por_base)
  if [ -n "$maximas" ]; then
    if [ "${POOL_DE_NEON:-}" = 1 ]; then
      dc exec -T "$c" sed -i '/^max_db_connections=/d; s/^default_pool_size=.*/default_pool_size=64/' /etc/pgbouncer.ini
    else
      dc exec -T "$c" sh -c "sed -i '/^max_db_connections=/d; s/^default_pool_size=.*/default_pool_size=$pool/' /etc/pgbouncer.ini \
        && echo max_db_connections=$pool >> /etc/pgbouncer.ini"
    fi
    if [ "$(dc exec -T "$c" psql -U postgres -p 5432 -Atc 'show max_connections')" != "$maximas" ]; then
      dc exec -T "$c" psql -q -U postgres -p 5432 -c "alter system set max_connections = $maximas" >/dev/null
      dc restart "$c" >/dev/null 2>&1
      until dc exec -T "$c" pg_isready -q -h 127.0.0.1 -p 6432 2>/dev/null; do sleep 0.5; done
    else
      dc exec -T "$c" sh -c 'kill -HUP $(cat /tmp/pgbouncer.pid)'
    fi
  fi
  sql_plano "update plano.endpoint set observado = 'listo', direccion = '$ip', ip_pod = '$ip' where vm = '$vm';
             update plano.rama r set observado = 'lista' from plano.endpoint e
              where e.vm = '$vm' and (r.organizacion, r.proyecto, r.id) = (e.organizacion, e.proyecto, e.rama);
             update plano.proyecto p set observado = 'listo' from plano.endpoint e
              where e.vm = '$vm' and (p.organizacion, p.id) = (e.organizacion, e.proyecto);
             update plano.operacion o set estado = 'hecha', terminada = now() from plano.endpoint e
              where e.vm = '$vm' and (o.organizacion, o.proyecto) = (e.organizacion, e.proyecto) and o.estado = 'en-curso'" >/dev/null
}
hechas() {   # hechas: las operaciones en curso, hechas (el reconciliador no tiene nada que hacer en
             # ellas: configurar-acceso) y el tiempo de que olvidar las vea y el proxy relea
  sql_plano "update plano.operacion set estado = 'hecha', terminada = now() where estado = 'en-curso'" >/dev/null
  sleep 2
}
barre() {   # barre: lo que la API dio por borrado se va, como al final del reconciliador
  sql_plano "update plano.operacion set estado = 'hecha', terminada = now() where estado = 'en-curso';
             delete from plano.proyecto where deseado = 'borrado'" >/dev/null
  # Y las VMs se destruyen: los cómputos vuelven a estar vacíos.
  local c
  for c in computo-a computo-b; do
    dc exec -T "$c" psql -q -U postgres -p 5432 -Atc "select format('drop database %I with (force);', datname)
      from pg_database where datname not in ('postgres', 'template0', 'template1')
      union all select format('drop role %I;', rolname) from pg_roles where rolname !~ '^(pg_|postgres$|cloud_admin$)'"       | dc exec -T "$c" psql -q -U postgres -p 5432 >/dev/null 2>&1
  done
}

entra() {   # entra ROL CLAVE BASE SNI
  dc exec -T -e PGPASSWORD="$2" -e PGCONNECT_TIMEOUT=10 cliente \
    psql "host=$4.$DOMINIO hostaddr=172.29.51.20 port=4432 user=$1 dbname=$3 sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" \
    -Atc 'select current_user' 2>&1 | tail -1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  case "${1:-}" in arriba) arriba;; abajo) abajo;; *) echo "lab.sh arriba | abajo"; exit 64;; esac
fi
