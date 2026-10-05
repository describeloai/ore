#!/usr/bin/env bash
# EL COORDINADOR (ADR 0053 F4·2 y F6·1) — `POST /federation/read` de punta a
# punta: `ore-serve` decide, `ore-federation` lee, Postgres de verdad detrás. Y
# (F6·1) lo que un puesto pide para su SQL: el reparto de la sentencia, las
# lecturas ya decididas y el final de una lectura.
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
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
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
mkdir -p "$A/packages/pg/views"
cat > "$A/packages/pg/views/v_es.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: v_es, namespace: pg }
spec:
  owner: team:fed
  dialect: duckdb
  sql: |
    SELECT id AS ident, pais FROM pg.public.clientes
  columns:
    ident: { type: Integer }
    pais: { type: String }
YAML
( cd "$A" && git add -A && git -c user.email=t@t -c user.name=t commit -qm semilla && git remote add origin "$FORJA" && git push -q origin HEAD:main ) \
  || { echo "no se sembró la forja"; exit 1; }

# ── la cola de los puestos (F6·1): un repositorio pelado con la plantilla ─────
COLA="$TMP/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TMP/cola-semilla"
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || { echo "no se rindió la plantilla del puesto"; exit 1; }
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" \
   "$TMP/rendido/plantilla-capa-jvm.txt" "$TMP/rendido/plantilla-capa-node.txt" "$TMP/cola-semilla/"
( cd "$TMP/cola-semilla" && git init -q -b main && git add -A && git -c user.name=t -c user.email=t@t commit -qm plantilla \
  && git remote add origin "$COLA" && git push -q origin HEAD:main ) || { echo "no se sembró la cola"; exit 1; }

# ── la pasarela y el servidor ────────────────────────────────────────────────
DIR_CONECTORES="$(cd "$(dirname "$FED")" && pwd)"
PF=$(libre)
"$FED" --escucha "127.0.0.1:$PF" --conectores "$DIR_CONECTORES" --tipos postgres >"$TMP/fed.log" 2>&1 &
PIDS="$PIDS $!"
PS=$(libre); BASE="http://127.0.0.1:$PS"
ORE_PASARELA="127.0.0.1:$PF" FORJA_TOKEN=no-hace-falta "$SERVE" --forja "file://$FORJA" --ore "$ORE" --cola "file://$COLA" \
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

# ── F6·1 · el SQL de un puesto ───────────────────────────────────────────────
AG='x-ore-sujeto: agente:local'
C=$(pide POST /puestos '{}')
[ "$C" = 201 ] || falla "F6 · abrir el puesto: $C $(cat "$TMP/out.json")"
P=puesto-ana-python
curl -s -o /dev/null --max-time 2 -H "$AG" "$BASE/puestos/$P/pendiente" &
for _ in $(seq 1 20); do pide GET /puestos/$P >/dev/null; grep -q '"estado":"vivo"' "$TMP/out.json" && break; sleep 0.2; done
grep -q '"estado":"vivo"' "$TMP/out.json" || falla "F6 · el agente no reclamó el puesto: $(cat "$TMP/out.json")"
del_puesto() { # ruta cuerpo → código; cuerpo en $TMP/out.json
  curl -s -o "$TMP/out.json" -w '%{http_code}' -X POST -H "$AG" -H "x-ore-puesto: $P" -H 'content-type: application/json' "$BASE$1" -d "$2"
}
mira() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if eval(sys.argv[2]) else 1)' "$TMP/out.json" "$1"; }

C=$(del_puesto /puestos/$P/sql '{"texto":"SELECT id FROM pg.public.clientes WHERE pais = '"'"'ES'"'"' AND upper(nota) = '"'"'UNO'"'"'"}')
[ "$C" = 200 ] && mira "(l:=d['fuentes']['pg.public.clientes']['federada'])['columnas']==['id','pais','nota'] and l['empujados']==[{'columna':'pais','operador':'eq','valor':'ES'}] and len(l['enElMotor'])==1" \
  && dice "F6 · sql() con una Table: la lectura ya repartida (pais = ES al origen, upper(nota) al motor)" || falla "F6 · sql con Table: $C $(cat "$TMP/out.json")"
C=$(del_puesto /puestos/$P/sql '{"texto":"SELECT ident FROM pg.v_es WHERE pais IN ('"'"'ES'"'"', '"'"'PT'"'"') LIMIT 5"}')
[ "$C" = 200 ] && mira "'FROM pg.public.clientes' in (f:=d['fuentes'])['pg.v_es']['vistaFederada'] and f['pg.public.clientes']['federada']['limit']==5 and f['pg.public.clientes']['federada']['empujados'][0]['operador']=='in'" \
  && dice "F6 · sql() con una vista sobre la Table: su SQL, y la tabla leída con el IN y el limit de quien la lee" || falla "F6 · sql con vista: $C $(cat "$TMP/out.json")"
C=$(del_puesto /puestos/$P/sql '{"texto":"SELECT * FROM pg.public.prohibida"}')
[ "$C" = 422 ] && mira "d['codigo']=='OOS2044' and d['nombre']=='pg.public.prohibida'" \
  && dice "F6 · forbidden sin filtro → 422 OOS2044 antes de tocar el origen" || falla "F6 · forbidden: $C $(cat "$TMP/out.json")"
C=$(del_puesto /puestos/$P/explain '{"texto":"SELECT id FROM pg.public.clientes WHERE pais = '"'"'ES'"'"' LIMIT 2"}')
[ "$C" = 200 ] && mira "d['plan']['lecturas'][0]['limit']==2 and 'al origen' in d['texto']" \
  && dice "F6 · POST /puestos/{id}/explain: el plan y su texto" || falla "F6 · explain: $C $(cat "$TMP/out.json")"
# El final de una lectura: lo que va en los trailers, para un SDK.
curl -s -D "$TMP/cab.txt" -o "$TMP/arrow.bin" -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' \
  "$BASE/federation/read" -d '{"tabla":"pg.public.clientes","limit":1}'
ID=$(grep -i '^ore-lectura:' "$TMP/cab.txt" | cut -d: -f2 | tr -d ' \r')
sleep 0.3
C=$(pide GET "/federation/read/$ID")
[ -n "$ID" ] && [ "$C" = 200 ] && mira "d['estado']=='completo' and d['filas']==1 and d['tabla']=='pg.public.clientes'" \
  && dice "F6 · GET /federation/read/{id}: completo, 1 fila" || falla "F6 · final: id=$ID $C $(cat "$TMP/out.json")"
C=$(curl -s -o "$TMP/out.json" -w '%{http_code}' -H 'x-ore-sujeto: persona:bea' "$BASE/federation/read/$ID")
[ "$C" = 404 ] && dice "F6 · el final de otra persona → 404" || falla "F6 · bea vio el final de ana: $C"

# ── 5 · apagarla la apaga ────────────────────────────────────────────────────
C=$(pide PUT /fuentes/pg '{"federation":false}')
R=$(leer '{"tabla":"pg.public.clientes"}')
case "$C|$R" in 200\|403*federacion*) dice "5 · apagada otra vez → 403";; *) falla "5 · apagar: $C $R";; esac

# ── 6 · lo que queda dicho, y lo que no sale ─────────────────────────────────
sleep 0.5
N=$(grep -c "^federation:read" "$TMP/serve.log")
[ "$N" -ge 9 ] && dice "6 · cada lectura anotada ($N, también las negadas)" || falla "6 · $N anotaciones: $(grep federation "$TMP/serve.log" | head -3)"
grep "^federation:read" "$TMP/serve.log" | grep -q "ES" && falla "6 · ⛔ un VALOR de filtro en la anotación" || dice "6 · la anotación no lleva valores de filtro"
if grep -q -- "$CLAVE" "$TMP/serve.log" "$TMP/fed.log" "$TMP/out.json"; then falla "6 · ⛔ LA CLAVE DEL ORIGEN SALE"; else dice "6 · la clave del origen no sale (ni registros ni respuestas)"; fi

# ── 7 · F8 · una vía: catalogar y comprobar por la pasarela ─────────────────
#   `ore` con `ORE_PASARELA` no lanza el conector: lo pide a la pasarela, que lo
#   hace en la cola del origen (lo que `ore-serve` hace ya sin Job, F8·2).
git clone -q "$FORJA" "$TMP/f8"
SALE=$(ORE_PASARELA="127.0.0.1:$PF" "$ORE" source check pg --path "$TMP/f8" 2>&1)
case "$SALE" in *"pg · sí"*) dice "F8 · ore source check por la pasarela: sí";; *) falla "F8 · check: $SALE";; esac
ORE_PASARELA="127.0.0.1:$PF" "$ORE" source catalog pg --out "$TMP/f8-cat.json" --path "$TMP/f8" >"$TMP/f8.log" 2>&1
grep -q 'clientes' "$TMP/f8-cat.json" 2>/dev/null && dice "F8 · ore source catalog por la pasarela: el catálogo"   || falla "F8 · catalog: $(tail -5 "$TMP/f8.log")"
curl -s "http://127.0.0.1:$PF/v1/origins" -o "$TMP/out.json"
mira "(o:=[x for x in d['origenes'] if x['origen']=='pg'][0])['verbos'].get('check',0)>=1 and o['verbos'].get('catalog',0)>=1"   && dice "F8 · /v1/origins cuenta el check y el catálogo de pg" || falla "F8 · origins: $(cat "$TMP/out.json")"
SALE=$(ORE_PASARELA="127.0.0.1:1" "$ORE" source check pg --path "$TMP/f8" 2>&1)
case "$SALE" in *"no contesta"*) dice "F8 · sin pasarela no se lanza el conector a escondidas: no contesta";; *) falla "F8 · pasarela caída: $SALE";; esac
if grep -q -- "$CLAVE" "$TMP/f8.log" "$TMP/f8-cat.json" "$TMP/fed.log"; then falla "F8 · ⛔ LA CLAVE SALE"; else dice "F8 · la clave no sale"; fi

echo
[ "$MAL" = 0 ] && echo "✓ el coordinador (0053 F4·2, F6·1 y F8)" || { echo "✗ el coordinador"; tail -20 "$TMP/serve.log"; exit 1; }
