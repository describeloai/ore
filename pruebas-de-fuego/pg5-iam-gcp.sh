#!/usr/bin/env bash
# PG5 · CLOUD SQL / ALLOYDB SIN CONTRASEÑA (ADR 0065, D-PG2), en Docker y sin cuentas.
#
#   Lo que hace Google, de mentira y por partes: el Postgres tiene usuarios «IAM» cuya contraseña
#   es el token que les tocaría (así se comprueba que el driver lo manda como contraseña);
#   `ORE_GCP_TOKEN` hace de Workload Identity (el token de la celda); y una API de IAM de mentira
#   contesta `generateAccessToken` sólo a quien trae el token de la celda.
#
#   1  iam=gcp con el usuario de la celda: check, catalogo, Arrow y v1, sin contraseña en la URL
#   2  iam=gcp con contraseña en la URL: se niega (la pone el token)
#   3  iam=gcp sin usuario: se niega y dice cuál
#   4  un token que la base no acepta: se dice qué mirar (la bandera, el usuario, el rol)
#   5  suplanta=<cuenta del cliente>: entra como ella, con el token que da la suplantación
#   6  suplanta a una cuenta que no lo permite: 403 y el rol que falta
#   7  iam=aws: todavía no (PG6), y se dice
#
#   bash pruebas-de-fuego/pg5-iam-gcp.sh
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm
RED=ore-pg5
PW=lab

if [ "${1:-}" = "--dentro" ]; then
  cd /w
  cargo build --quiet -p ore-read-postgres 2>&1 | tail -3
  D="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-postgres"
  export ORE_PG_LABORATORIO=1 ORE_GCP_TOKEN=tok-celda-123 ORE_GCP_IAM=http://iam:8080/v1
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  check() { printf '{"url":"%s"}' "$1" | timeout 30 "$D" check 2>&1; }
  ok() { case "$1" in *'"ok":true'*) return 0;; esac; return 1; }
  for _ in $(seq 60); do (exec 3<>/dev/tcp/pg/5432) 2>/dev/null && break; sleep 1; done; sleep 2
  U="postgres://ore-driver-demo%40p.iam@pg:5432/postgres?iam=gcp"

  ok "$(check "$U")" && dice "1 check como ore-driver-demo@p.iam, con el token de la celda" || falla "1 check: $(check "$U")"
  printf '%s' "$U" | "$D" catalogo lab 2>/dev/null | grep -q '"public.t"' && dice "1 catalogo" || falla "1 catalogo"
  printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"formato":"arrow"}' "$U" | "$D" leer >/tmp/a.bin 2>/tmp/a.err \
    && [ "$(stat -c %s /tmp/a.bin)" -gt 100 ] && dice "1 Arrow" || falla "1 Arrow: $(cat /tmp/a.err)"
  v=$(printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"claveColumnas":["id"],"claves":[["3"]],"filtros":[]}' "$U" | "$D" leer lab 2>&1)
  [ "$v" = '{"id":"3"}' ] && dice "1 v1" || falla "1 v1: $v"

  r=$(check "postgres://ore-driver-demo%40p.iam:x@pg/postgres?iam=gcp")
  ! ok "$r" && echo "$r" | grep -q "no lleva contraseña" && dice "2 con contraseña en la URL: se niega" || falla "2: $r"
  r=$(check "postgres://pg/postgres?iam=gcp")
  ! ok "$r" && echo "$r" | grep -q "usuario IAM" && dice "3 sin usuario: lo dice" || falla "3: $r"
  r=$(ORE_GCP_TOKEN=caducado check "$U")
  ! ok "$r" && echo "$r" | grep -q "cloudsql.iam_authentication" && dice "4 token no aceptado: dice qué mirar" || falla "4: $r"

  S="postgres://lector%40cliente.iam@pg/postgres?iam=gcp&suplanta=lector%40cliente.iam.gserviceaccount.com"
  ok "$(check "$S")" && dice "5 suplantando a lector@cliente: entra como ella" || falla "5: $(check "$S")"
  r=$(printf '{"url":"%s","objeto":"public.t","proyeccion":{"id":"id"},"claveColumnas":["id"],"claves":[["5"]],"filtros":[]}' "$S" | "$D" leer lab 2>&1)
  [ "$r" = '{"id":"5"}' ] && dice "5 y lee" || falla "5 lee: $r"
  r=$(check "postgres://nadie%40x.iam@pg/postgres?iam=gcp&suplanta=nadie%40x.iam.gserviceaccount.com")
  ! ok "$r" && echo "$r" | grep -q "serviceAccountTokenCreator" && dice "6 suplantación no concedida: 403 y el rol" || falla "6: $r"
  r=$(check "postgres://u@pg/postgres?iam=aws")
  ! ok "$r" && echo "$r" | grep -q "PG6" && dice "7 iam=aws: todavía no, y se dice" || falla "7: $r"

  echo
  [ $fallos = 0 ] && echo "PG5: todo verde" || echo "PG5: $fallos fallos"
  exit $fallos
fi

command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f pg iam >/dev/null 2>&1; docker network rm $RED >/dev/null 2>&1; }
limpiar
docker network create $RED >/dev/null
docker run -d --name pg --network $RED -e POSTGRES_PASSWORD=$PW postgres:16-alpine >/dev/null || exit 1
# La API de IAM de mentira: sólo suplanta a `lector@cliente…` y sólo a quien trae el token de la celda.
docker run -d --name iam --network $RED python:3.12-alpine python -c '
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        bueno = self.headers.get("authorization") == "Bearer tok-celda-123"
        if bueno and "lector@cliente.iam.gserviceaccount.com:generateAccessToken" in self.path:
            cuerpo, estado = json.dumps({"accessToken": "tok-suplantado", "expireTime": "2099-01-01T00:00:00Z"}), 200
        else:
            cuerpo, estado = json.dumps({"error": {"code": 403, "message": "Permission iam.serviceAccounts.getAccessToken denied"}}), 403
        self.send_response(estado); self.send_header("content-type", "application/json"); self.end_headers()
        self.wfile.write(cuerpo.encode())
http.server.HTTPServer(("", 8080), H).serve_forever()' >/dev/null || exit 1
for _ in $(seq 60); do docker exec pg pg_isready -q && break; sleep 1; done; sleep 2
docker exec -e PGPASSWORD=$PW pg psql -U postgres -XAtq -v ON_ERROR_STOP=1 -c "
  create table t as select g id from generate_series(1,100) g; alter table t add primary key (id);
  create role \"ore-driver-demo@p.iam\" login password 'tok-celda-123';
  create role \"lector@cliente.iam\" login password 'tok-suplantado';
  grant select on t to \"ore-driver-demo@p.iam\", \"lector@cliente.iam\";" >/dev/null || exit 1
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/pg5-iam-gcp.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
rc=${PIPESTATUS[0]}
limpiar
exit $rc
