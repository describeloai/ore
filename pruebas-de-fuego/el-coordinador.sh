#!/usr/bin/env bash
# EL COORDINADOR (ADR 0053 F4·2) — `POST /federation/read` de punta a punta:
# `ore-serve` decide, `ore-federation` lee, Postgres de verdad detrás.
#
#   PG_URL=postgres://postgres:x@localhost:5432 \
#   SERVE=target/debug/ore-serve ORE=target/debug/ore FED=target/debug/ore-federation \
#     bash pruebas-de-fuego/el-coordinador.sh
#
# La credencial de la fuente sale aquí del entorno (`FED_PG_URL`): sin custodio,
# como `ore`. Con custodio, del custodio como el agente (eso lo prueba F4·3).
set -u
SERVE="${SERVE:-target/debug/ore-serve}"
ORE="${ORE:-target/debug/ore}"
FED="${FED:-target/debug/ore-federation}"
# Absolutas: `ore-serve` corre `ore` desde el clon de la forja, no desde aquí.
abs() { echo "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; }
SERVE=$(abs "$SERVE"); ORE=$(abs "$ORE"); FED=$(abs "$FED")
PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432}"
PY=$(command -v python3 || command -v python)
TMP="$(mktemp -d)"
PIDS=""
trap 'for p in $PIDS; do kill $p 2>/dev/null; done; rm -rf "$TMP"' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }
libre() { "$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()'; }

# ── el origen ───────────────────────────────────────────────────────────────
psql "$PG_URL/postgres" -qc "drop database if exists fed" -qc "create database fed" >/dev/null || { echo "sin Postgres en $PG_URL"; exit 1; }
psql "$PG_URL/fed" -q -v ON_ERROR_STOP=1 <<'SQL' >/dev/null || { echo "no se sembró el origen"; exit 1; }
create table clientes (id integer primary key, pais text, nota text);
insert into clientes values (1, 'ES', 'uno'), (2, 'PT', 'dos'), (3, 'ES', 'tres');
create table prohibida (id integer primary key, pais text);
insert into prohibida values (1, 'ES');
SQL
# La fuente lee con un usuario suyo y una clave larga: la que se busca después
# en registros y respuestas (la del administrador sería demasiado corta para
# que buscarla dijera algo).
CLAVE="clave-del-origen-$$-no-debe-salir"
psql "$PG_URL/fed" -q -v ON_ERROR_STOP=1 \
  -c "drop role if exists fed_lector" \
  -c "create role fed_lector login password '$CLAVE'" \
  -c "grant select on all tables in schema public to fed_lector" >/dev/null || { echo "sin rol de lectura"; exit 1; }
SERVIDOR="${PG_URL#*://}"; SERVIDOR="${SERVIDOR#*@}"
export FED_PG_URL="postgres://fed_lector:$CLAVE@$SERVIDOR/fed"

# ── el árbol, en una forja de fichero ────────────────────────────────────────
FORJA="$TMP/forja.git"; A="$TMP/arbol"
git init -q --bare -b main "$FORJA"
git init -q -b main "$A"
mkdir -p "$A/packages/pg/public/tables"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: fed, version: 0.1.0 }
datasources:
  - name: pg
    type: postgres
    connectionEnv: FED_PG_URL
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: fed }
spec:
  owner: team:fed
  conduits:
    contextSurface.workspace: { oos.maturity: DRAFT }
YAML
cat > "$A/packages/pg/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: pg, version: 0.1.0, status: draft, domain: pg }
spec: { owner: "team:fed", exports: [pg.public.clientes, pg.public.prohibida] }
YAML
cat > "$A/packages/pg/public/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: public, namespace: pg }
spec: { owner: team:fed }
YAML
cat > "$A/packages/pg/public/tables/clientes.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: clientes, namespace: pg, schema: public }
spec:
  datasource: pg
  object: "public.clientes"
  columns:
    id: { type: Integer, physicalType: integer, required: true }
    pais: { type: String, physicalType: text }
    nota: { type: String, physicalType: text }
  reads:
    fullScan: cheap
    predicatePushdown: [eq, in]
  changes:
    key: [id]
YAML
cat > "$A/packages/pg/public/tables/prohibida.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: prohibida, namespace: pg, schema: public }
spec:
  datasource: pg
  object: "public.prohibida"
  columns:
    id: { type: Integer, physicalType: integer, required: true }
    pais: { type: String, physicalType: text }
  reads:
    fullScan: forbidden
    predicatePushdown: [eq]
  changes:
    key: [id]
YAML
( cd "$A" && git add -A && git -c user.email=t@t -c user.name=t commit -qm semilla && git remote add origin "$FORJA" && git push -q origin HEAD:main ) \
  || { echo "no se sembró la forja"; exit 1; }

# ── la pasarela y el servidor ────────────────────────────────────────────────
DIR_CONECTORES="$(cd "$(dirname "$FED")" && pwd)"
PF=$(libre)
"$FED" --escucha "127.0.0.1:$PF" --conectores "$DIR_CONECTORES" --tipos postgres >"$TMP/fed.log" 2>&1 &
PIDS="$PIDS $!"
PS=$(libre); BASE="http://127.0.0.1:$PS"
ORE_PASARELA="127.0.0.1:$PF" FORJA_TOKEN=no-hace-falta "$SERVE" --forja "file://$FORJA" --ore "$ORE" \
  --bind "127.0.0.1:$PS" --identidad cabecera --no-es-produccion --organizacion fed >"$TMP/serve.log" 2>&1 &
PIDS="$PIDS $!"
for _ in $(seq 1 80); do curl -s -o /dev/null "$BASE/salud" && curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done

# Una petición: estado, cuerpo y trailers (curl no da los trailers: Python).
leer() { # cuerpo → "codigo|ore-estado|ore-filas|ore-motivo|cuerpo-o-bytes"
  "$PY" - "$PS" "$1" <<'PYX'
import socket, sys
puerto, cuerpo = int(sys.argv[1]), sys.argv[2].encode()
s = socket.create_connection(("127.0.0.1", puerto))
s.sendall(b"POST /federation/read HTTP/1.1\r\nhost: x\r\nx-ore-sujeto: persona:ana\r\ncontent-type: application/json\r\ncontent-length: %d\r\nconnection: close\r\n\r\n" % len(cuerpo) + cuerpo)
f = s.makefile("rb")
codigo = f.readline().split()[1].decode()
cab = {}
while True:
    l = f.readline().decode().strip()
    if not l: break
    k, v = l.split(":", 1); cab[k.lower()] = v.strip()
fin, n, texto = {}, 0, b""
if cab.get("transfer-encoding") == "chunked":
    while True:
        t = int(f.readline().strip(), 16)
        if t == 0: break
        n += len(f.read(t)); f.readline()
    while True:
        l = f.readline().decode().strip()
        if not l: break
        k, v = l.split(":", 1); fin[k.lower()] = v.strip()
else:
    texto = f.read(int(cab.get("content-length", "0")))
print("|".join([codigo, fin.get("ore-estado", ""), fin.get("ore-filas", ""), fin.get("ore-motivo", ""), texto.decode() if texto else str(n)]))
PYX
}
pide() { # metodo ruta [cuerpo] → código; cuerpo en $TMP/out.json
  local m=$1 r=$2 c=${3:-}
  curl -s -o "$TMP/out.json" -w '%{http_code}' -X "$m" -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' "$BASE$r" ${c:+-d "$c"}
}

# ── 1 · apagada: la fuente no se lee en vivo ─────────────────────────────────
R=$(leer '{"tabla":"pg.public.clientes"}')
case "$R" in 403\|*federacion*|403\|*\"codigo\":\"federacion\"*) dice "1 · fuente apagada → 403 federacion";; *) falla "1 · apagada: $R";; esac

# ── 2 · encenderla: PUT /fuentes/pg, en main, con su conducto ───────────────
C=$(pide PUT /fuentes/pg '{"federation":true}')
[ "$C" = 200 ] && grep -q '"federation":true' "$TMP/out.json" && dice "2 · PUT /fuentes/pg {federation:true} → 200 ($(cat "$TMP/out.json"))" || falla "2 · encender: $C $(cat "$TMP/out.json")"
C=$(pide GET /fuentes)
grep -q '"federation":true' "$TMP/out.json" && dice "2 · GET /fuentes la dice encendida" || falla "2 · GET /fuentes: $(cat "$TMP/out.json")"
git --git-dir="$FORJA" show main:conduits.yaml | grep -q "federation.read" && dice "2 · encender autorizó federation.read en main" || falla "2 · sin federation.read en main"

# ── 3 · leer ─────────────────────────────────────────────────────────────────
R=$(leer '{"tabla":"pg.public.clientes","columnas":["id","pais"]}')
case "$R" in 200\|completo\|3\|*) dice "3 · leer la tabla → 200, flujo Arrow, completo, 3 filas";; *) falla "3 · leer: $R";; esac
R=$(leer '{"tabla":"pg.public.clientes","filtros":[{"columna":"pais","operador":"eq","valor":"ES"}]}')
case "$R" in 200\|completo\|2\|*) dice "3 · con un filtro empujado → 2 filas";; *) falla "3 · filtro: $R";; esac
R=$(leer '{"tabla":"pg.public.clientes","limit":1}')
case "$R" in 200\|completo\|1\|*) dice "3 · con limit 1 → 1 fila";; *) falla "3 · limit: $R";; esac

# ── 4 · lo que se niega antes de tocar el origen ─────────────────────────────
R=$(leer '{"tabla":"pg.public.prohibida"}')
case "$R" in 422*OOS2044*) dice "4 · forbidden sin filtro → 422 OOS2044";; *) falla "4 · forbidden: $R";; esac
R=$(leer '{"tabla":"pg.public.clientes","filtros":[{"columna":"id","operador":"gt","valor":"1"}]}')
case "$R" in 422*empuje*) dice "4 · un filtro que no se empuja → 422 empuje";; *) falla "4 · empuje: $R";; esac
R=$(leer '{"tabla":"pg.public.nada"}')
case "$R" in 404*) dice "4 · una tabla que no existe → 404";; *) falla "4 · nada: $R";; esac

# ── 5 · apagarla la apaga ────────────────────────────────────────────────────
C=$(pide PUT /fuentes/pg '{"federation":false}')
R=$(leer '{"tabla":"pg.public.clientes"}')
case "$C|$R" in 200\|403*federacion*) dice "5 · apagada otra vez → 403";; *) falla "5 · apagar: $C $R";; esac

# ── 6 · lo que queda dicho, y lo que no sale ─────────────────────────────────
sleep 0.5
N=$(grep -c "^federation:read" "$TMP/serve.log")
[ "$N" -ge 8 ] && dice "6 · cada lectura anotada ($N, también las negadas)" || falla "6 · $N anotaciones: $(grep federation "$TMP/serve.log" | head -3)"
grep "^federation:read" "$TMP/serve.log" | grep -q "ES" && falla "6 · ⛔ un VALOR de filtro en la anotación" || dice "6 · la anotación no lleva valores de filtro"
if grep -q -- "$CLAVE" "$TMP/serve.log" "$TMP/fed.log" "$TMP/out.json"; then falla "6 · ⛔ LA CLAVE DEL ORIGEN SALE"; else dice "6 · la clave del origen no sale (ni registros ni respuestas)"; fi

echo
[ "$MAL" = 0 ] && echo "✓ el coordinador (0053 F4·2)" || { echo "✗ el coordinador"; tail -20 "$TMP/serve.log"; exit 1; }
