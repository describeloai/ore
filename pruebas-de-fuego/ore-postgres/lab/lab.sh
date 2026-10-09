#!/usr/bin/env bash
# El laboratorio local de P5 (ADR 0058): `lab.sh arriba | abajo`, y lo que usan las pruebas.
#
#   arriba   genera en .secretos/ el certificado autofirmado de *.europe-west1.pg.paladio.io,
#            el token del proxy y las contraseñas de las bases (nada sale de ahí ni se imprime),
#            la llave con que ore-postgres firma los tokens de los cómputos, y levanta compose.yaml.
#            El binario de ore-postgres es el de /tt/p4pg (volumen ore-pruebas-t), con el backend de
#            Docker: `cargo build --release -p ore-postgres --features laboratorio` (P6·1).
#   abajo    lo quita todo: los cómputos que creó ore-postgres, el volumen del almacén y .secretos/.
#
# Sourced (`source lab.sh`), da a las pruebas:
#   pide CELDA METODO RUTA [CUERPO]   → la API de ore-postgres como esa celda (JSON en stdout)
#   campo JSON a b c                  → un campo
#   hecha CELDA RESPUESTA             → espera a que la operación de una respuesta de la API acabe
#                                       (la cierra el reconciliador de verdad); dice su estado
#   listo CELDA PROYECTO [RAMA] [EP]   → espera a que el endpoint esté `listo`; dice su vm
#   en_vm VM SQL                      → psql como cloud_admin dentro del cómputo (el contenedor `ep-…`)
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
  mkdir -p "$S/tls" "$S/proxy" "$S/computo"
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
  # La llave con que ore-postgres firma los tokens de los compute_ctl (Ed25519, PKCS#8).
  [ -s "$S/computo/privada.pem" ] || { openssl genpkey -algorithm ed25519 -out "$S/computo/privada.pem" 2>/dev/null
    chmod 644 "$S/computo/privada.pem"; }
  docker build -q -t p5lab-computo:2 "$LAB/computo" >/dev/null || return 1
  dc up -d --wait --wait-timeout 120 base iam redis docker-api cliente >/dev/null || return 1
  # La base vacía de las pruebas de contrato del crate (ORE_POSTGRES_PRUEBA_URL).
  dc exec -T base psql -U postgres -qc 'create database ore_postgres_prueba' >/dev/null 2>&1
  dc up -d plano proxy >/dev/null || return 1
  for _ in $(seq 1 60); do
    dc exec -T cliente bash -c 'exec 3<>/dev/tcp/172.29.51.7/8100 && exec 4<>/dev/tcp/172.29.51.20/4432' 2>/dev/null && break
    sleep 1
  done
  dc logs plano | tail -8
}
abajo() {
  local vms; vms=$(docker ps -aq --filter label=ore.dev/laboratorio=p5lab)
  [ -n "$vms" ] && docker rm -f $vms >/dev/null
  dc down -v --remove-orphans >/dev/null 2>&1; rm -rf "$LAB/.secretos"
}

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

hecha() {   # hecha CELDA RESPUESTA → el estado final de su operación (hecha | fallida | sin-operacion)
  local op e; op=$(campo "$2" operacion id)
  [ -n "$op" ] || { echo sin-operacion; return; }
  for _ in $(seq 1 240); do
    e=$(pide "$1" GET "/v1/postgres/operaciones/$op"); case "$e" in *'sin plano'*) echo sin-plano; return;; esac
    e=$(campo "$e" estado)
    [ "$e" != en-curso ] && [ -n "$e" ] && { echo "$e"; return; }
    sleep 0.5
  done
  echo en-curso
}
listo() {   # listo CELDA PROYECTO [RAMA] [EP] → la vm, cuando el endpoint está listo (o nada)
  local r ep
  for _ in $(seq 1 240); do
    ep=$(pide "$1" GET "/v1/postgres/proyectos/$2/ramas/${3:-main}/endpoints/${4:-principal}")
    case "$ep" in *'sin plano'*|*'no hay ning'*) return;; esac
    [ "$(campo "$ep" estado observado)" = listo ] && { campo "$ep" vm; return; }
    sleep 0.5
  done
}
en_vm() { docker exec -i "$1" gosu postgres psql -h /tmp -U cloud_admin -d postgres -Atqc "$2"; }

entra() {   # entra ROL CLAVE BASE SNI
  dc exec -T -e PGPASSWORD="$2" -e PGCONNECT_TIMEOUT=10 cliente \
    psql "host=$4.$DOMINIO hostaddr=172.29.51.20 port=4432 user=$1 dbname=$3 sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" \
    -Atc 'select current_user' 2>&1 | tail -1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  case "${1:-}" in arriba) arriba;; abajo) abajo;; *) echo "lab.sh arriba | abajo"; exit 64;; esac
fi
