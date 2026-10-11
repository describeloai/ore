#!/usr/bin/env bash
# Lo común a las pruebas de PRODUCCIÓN de P5 y P6 (ADR 0058, «la ventana 1»). Se carga con `source`.
#
# El reparto es el de un cliente de verdad:
#   · el plano (crear proyectos, leerlos, cambiar sus ajustes) se habla COMO UNA CELDA: un pod de usar
#     y tirar con la cuenta de su ore-serve y su token de Workload Identity (../celdas.sh, el de P4);
#   · los datos entran DESDE ESTE PC, por internet: psql, pgbench y Node en contenedores de Docker.
#     Desde 0060 C2 (b33ea828) la entrada no tiene IP pública: el TCP entra por el túnel de Cloudflare
#     (`cloudflared access tcp --hostname pg.ontoforge.org`, en 127.0.0.1:$TUNEL_PUERTO de este PC;
#     los contenedores llegan por host.docker.internal). Por el túnel no llega el nombre del endpoint
#     (SNI): va en `options=endpoint=<id>`, como en Neon para los clientes sin SNI, y el TLS es de
#     extremo a extremo con el proxy (sslmode=require: el certificado es de *.europe-west1.pg.paladio.io,
#     no de host.docker.internal). El HTTP (driver serverless) entra por api.$DOMINIO, un custom
#     hostname de Cloudflare for SaaS que da de alta 0060;
#   · lo que hay que mirar por dentro (el estado de un endpoint, las VMs, los nodos) se LEE de la
#     base de ore-postgres (en storcon-db) y con kubectl. Nada de esto escribe en el plano.
#
# Cada número va a $NUMEROS (una línea JSON: prueba, clave, valor) para el ADR.
PRUEBA=${PRUEBA:-prod}
source "$(dirname "${BASH_SOURCE[0]}")/../entorno.sh"
source "$(dirname "${BASH_SOURCE[0]}")/../celdas.sh"
DOMINIO=europe-west1.pg.paladio.io
# La alpine: trae el paquete de CA del sistema (/etc/ssl/cert.pem), que es lo que lee sslrootcert=system;
# la de Debian no lo trae.
PG_IMAGEN=postgres:17-alpine
NODE_IMAGEN=node:22-slim
NUMEROS="${NUMEROS:-$ORE_PG_TRABAJO/numeros.jsonl}"
TUNEL=pg.ontoforge.org
TUNEL_PUERTO=${TUNEL_PUERTO:-15432}
fallos=${fallos:-0}
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
numero() {   # numero CLAVE VALOR → $NUMEROS y por la salida
  printf '{"t":"%s","prueba":"%s","clave":"%s","valor":%s}\n' "$(date -u +%FT%TZ)" "$PRUEBA" "$1" "$2" >> "$NUMEROS"
  echo "  · $1 = $2"
}

# ── el plano, como una celda ────────────────────────────────────────────────────────────────────
crea() {   # crea CELDA PROYECTO → «rol clave host host_pool» cuando el endpoint está listo
  local c=$1 p=$2 r
  r=$(en "$c" "pide POST /v1/postgres/proyectos '{\"id\":\"$p\",\"dueno\":\"user:prueba\"}'" | head -1)
  local rol clave; rol=$(campo "$r" rol nombre); clave=$(campo "$r" rol contrasena)
  [ -n "$rol" ] || { echo "✗ $c no crea $p: ${r:0:300}" >&2; return 1; }
  listo_ep "$p" || { echo "✗ $p no queda listo ($(observado "$p"))" >&2; return 1; }
  # El nombre público, como lo compone la API con --dominio (host, host_pool): el `vm` del endpoint.
  local vm; vm=$(plano_sql "select vm from plano.endpoint where proyecto = '$p' and id = 'principal'")
  echo "$rol $clave $vm.$DOMINIO $vm-pooler.$DOMINIO"
}
ajusta() {   # ajusta CELDA PROYECTO CUERPO-JSON → los ajustes del endpoint (dormir_tras, cu…)
  en "$1" "pide POST /v1/postgres/proyectos/$2/ramas/main/endpoints/principal/ajustes '$3'" | head -1
}
acceso() {   # acceso CELDA PROYECTO CUERPO-JSON → quién entra (P5·6)
  en "$1" "pide POST /v1/postgres/proyectos/$2/acceso '$3'" | head -1
}
borra() { en "$1" "pide DELETE /v1/postgres/proyectos/$2" >/dev/null; }

# ── lo de dentro, leído ─────────────────────────────────────────────────────────────────────────
plano_sql() { k exec storcon-db-0 -- env Q="$1" sh -c 'psql -U "$POSTGRES_USER" -d ore_postgres -Atc "$Q"'; }
observado() { plano_sql "select observado from plano.endpoint where proyecto = '$1' and id = 'principal'"; }
computo_de() { plano_sql "select coalesce(computo, vm) from plano.endpoint where proyecto = '$1' and id = 'principal'"; }
listo_ep() {   # listo_ep PROYECTO [SEGUNDOS] → espera a `listo`
  local t0; t0=$(date +%s)
  until [ "$(observado "$1")" = listo ] || [ $(( $(date +%s) - t0 )) -ge "${2:-600}" ]; do sleep 2; done
  [ "$(observado "$1")" = listo ]
}
dormido_ep() {   # dormido_ep PROYECTO [SEGUNDOS] → espera a `dormido`
  local t0; t0=$(date +%s)
  until [ "$(observado "$1")" = dormido ] || [ $(( $(date +%s) - t0 )) -ge "${2:-600}" ]; do sleep 2; done
  [ "$(observado "$1")" = dormido ]
}
nodos_pg() { kubectl get nodes -l cloud.google.com/gke-nodepool=pg --no-headers 2>/dev/null | grep -c .; }

# ── los datos, desde este PC ────────────────────────────────────────────────────────────────────
tunel() {   # deja `cloudflared access tcp` escuchando en 127.0.0.1:$TUNEL_PUERTO (uno para todas las pruebas)
  local pid; pid=$(cat "$ORE_PG_TRABAJO/tunel.pid" 2>/dev/null)
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && return
  cloudflared access tcp --hostname "$TUNEL" --url "127.0.0.1:$TUNEL_PUERTO" > "$ORE_PG_TRABAJO/tunel.log" 2>&1 &
  echo $! > "$ORE_PG_TRABAJO/tunel.pid"; sleep 2
  kill -0 $! 2>/dev/null || { echo "✗ cloudflared no arranca: $(tail -3 "$ORE_PG_TRABAJO/tunel.log")" >&2; return 1; }
}
tunel_cierra() { local pid; pid=$(cat "$ORE_PG_TRABAJO/tunel.pid" 2>/dev/null) && kill "$pid" 2>/dev/null; rm -f "$ORE_PG_TRABAJO/tunel.pid"; }
# conn HOST ROL BASE [SEGUNDOS]: el endpoint es la primera etiqueta del host (ep-…, o ep-…-pooler para
# el pool), y va en options; sslsni=0 para que el proxy no vea host.docker.internal como nombre.
conn() { echo "host=host.docker.internal port=$TUNEL_PUERTO user=$2 dbname=$3 sslmode=require sslsni=0 options=endpoint=${1%%.*} connect_timeout=${4:-30}"; }
pc_psql() {   # pc_psql CLAVE CONNINFO [args psql…] → lo que contesta psql (la última línea)
  local clave=$1 ci=$2; shift 2
  tunel >&2 || return 1
  docker run --rm -e PGPASSWORD="$clave" "$PG_IMAGEN" psql "$ci" -v ON_ERROR_STOP=1 -Atq "$@" 2>&1 | tail -1
}
pc_ms() {   # pc_ms CLAVE CONNINFO → ms de conectar y contestar `select 1` (o «fallo:…»)
  local t0 r; t0=$(ms); r=$(pc_psql "$1" "$2" -c "select 1")
  [ "$r" = 1 ] && echo $(( $(ms) - t0 )) || echo "fallo:${r:0:120}"
}
pc_pgbench() {   # pc_pgbench CLAVE CONNINFO [args pgbench…]
  local clave=$1 ci=$2; shift 2
  tunel >&2 || return 1
  docker run --rm -e PGPASSWORD="$clave" "$PG_IMAGEN" pgbench "$@" "$ci" 2>&1
}
percentil() { python -c 'import sys
v=sorted(int(x) for x in sys.argv[2:]); q=float(sys.argv[1])
print(v[min(len(v)-1, max(0, round(q*(len(v)-1))))] if v else "null")' "$@"; }
