#!/usr/bin/env bash
# LO QUE ARRASTRA UNA PROPUESTA DE ACTIVOS (0044 A.2, M1 y M2): una base
# standard creada EN UNA RAMA escribe siete cosas —sus datasets, su paquete y
# su schema, los punteros en el paquete de la fuente (0045) con el schema y el
# paquete de ella, y la politica de conductos de la copia—, y proponer SOLO
# los datasets tiene que llevar las siete y compilar sobre main.
#
# Medido el 2026-09-28 antes de M1: la propuesta llevaba los datasets y el
# paquete de la base, no compilaba (OOS2037, OOS2018, OOS4011) y `faltan`
# salia vacio.
#
#   1  POST /paquetes {type: standard, source: pg} en la rama   la rama escribe
#      9 activos: 2 Dataset, 2 Table (punteros), 2 Schema, 2 Package, 1 ConduitPolicy
#   2  POST /propuestas {activos: los 2 datasets}   201 · lleva los 9 · anadidos
#      los 7, cada uno con su porque · diagnosticos [] · faltan []
#   3  lo que ya esta en main no se arrastra: con los punteros y el schema de la
#      fuente fusionados, la siguiente propuesta de otro dataset no los lleva
#
# Uso:  ORE_BIN=target/debug bash pruebas-de-fuego/lo-que-arrastra.sh
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PF="${PUERTO_FORJA:-8931}"; P="${PUERTO:-8932}"; BASE="http://127.0.0.1:$P"
TMP="$(mktemp -d)"; SRV=""; FORJA=""
PY=$(command -v python3 || command -v python)
falla() { echo "✗ $*" >&2; [ -s "$TMP/srv.txt" ] && tail -15 "$TMP/srv.txt" >&2; exit 1; }
dice() { echo "  · $*"; }
limpiar() { for p in $SRV $FORJA; do kill "$p" 2>/dev/null; done; sleep 0.3; rm -rf "$TMP"; }
trap limpiar EXIT
buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\`"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\`"

# ── el arbol: una fuente `pg` catalogada (lo que deja el Job), nada mas ─────
A="$TMP/a"; mkdir -p "$A/packages/pg"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: pg, type: postgres, connectionEnv: PG_URL }
Y
cat > "$A/packages/pg/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: pg, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/pg/discover.catalog.json" <<'J'
{
  "source": "pg",
  "tables": [
    { "name": "ventas.clientes",
      "columns": [ { "name": "id", "type": "String", "required": true }, { "name": "ciudad", "type": "String" } ],
      "primaryKey": ["id"], "reads": { "fullScan": "cheap" },
      "changes": { "mode": "upsert", "key": ["id"], "witness": "log" } },
    { "name": "ventas.pedidos",
      "columns": [ { "name": "id", "type": "String", "required": true }, { "name": "cliente", "type": "String" } ],
      "primaryKey": ["id"], "reads": { "fullScan": "cheap" },
      "changes": { "mode": "upsert", "key": ["id"], "witness": "log" } }
  ]
}
J
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila"
BARE="$TMP/forja/t-demo/ontologia.git"
mkdir -p "$(dirname "$BARE")" && git init -q --bare -b main "$BARE"
( cd "$A" && git init -q -b main && git config core.autocrlf false && git add -A \
  && git -c user.name=banco -c user.email=banco@invalido commit -q -m "partida" \
  && git remote add origin "$BARE" && git push -q origin main ) || falla "no se pudo sembrar la forja"
BARE_URL="file://$(cd "$BARE" && pwd)"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) BARE_URL="file:///$(cd "$BARE" && pwd -W)";; esac
"$PY" "$RAIZ/pruebas-de-fuego/forja-de-mentira.py" "$BARE" "$PF" >"$TMP/forja.txt" 2>&1 &
FORJA=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PF/api/v1/version" && break; sleep 0.25; done
FORJA_TOKEN=de-mentira "$SERVE" --forja "$BARE_URL" --forja-api "127.0.0.1:$PF" --ore "$ORE" --bind "127.0.0.1:$P" \
  --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/srv.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 60); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
ANA='x-ore-sujeto: persona:ana'
BEA='x-ore-sujeto: persona:bea'
cuerpo() { cat "$TMP/r.json"; }
pide() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H "$3" -H 'content-type: application/json' ${5:+-H "x-ore-rama: $5"} ${4:+--data-binary "$4"} "$BASE$2"; }
tiene() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); assert eval(sys.argv[2]), d' "$TMP/r.json" "$1" 2>/dev/null; }

# ── 1 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /ramas "$ANA" '{"nombre":"std"}')" = "201" ] || falla "1 · la rama: $(cuerpo)"
[ "$(pide POST /paquetes "$ANA" '{"name":"std","type":"standard","source":"pg","only":["ventas.clientes","ventas.pedidos"]}' ana/std)" = "200" ] || falla "1 · la base en la rama: $(cuerpo)"
[ "$(pide GET /ramas/ana/std/cambios "$ANA")" = "200" ] || falla "1 · los cambios: $(cuerpo)"
TODOS="['ConduitPolicy:std','Dataset:std.ventas.clientes','Dataset:std.ventas.pedidos','Package:pg','Package:std','Schema:pg.ventas','Schema:std.ventas','Table:pg.ventas.clientes','Table:pg.ventas.pedidos']"
tiene "sorted(c['id'] for c in d['cambios'])==$TODOS" || falla "1 · la rama no escribio los nueve: $(cuerpo | head -c 600)"
tiene "[c for c in d['cambios'] if c['id']=='Dataset:std.ventas.pedidos'][0]['lee']==['Table:pg.ventas.pedidos']" || falla "1 · el dataset no dice que lee su puntero: $(cuerpo | head -c 600)"
dice "1 · una base standard en la rama escribe 9 activos (datasets, punteros en la fuente, schemas, paquetes, politica) · cada cambio dice lo que lee"

# ── 2 ───────────────────────────────────────────────────────────────────────
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/std","titulo":"los datasets","activos":["Dataset:std.ventas.clientes","Dataset:std.ventas.pedidos"]}')" = "201" ] || falla "2 · proponer los datasets: $(cuerpo)"
tiene "sorted(d['activos'])==$TODOS" || falla "2 · no lleva los nueve: $(cuerpo | head -c 700)"
tiene "len(d['anadidos'])==7 and d['anadidosPorque']['Table:pg.ventas.pedidos']=='lo lee \`std.ventas.pedidos\`' and d['anadidosPorque']['Schema:std.ventas'].startswith('el schema de') and d['anadidosPorque']['ConduitPolicy:std'].startswith('el conducto de la copia')" || falla "2 · los anadidos o su porque: $(cuerpo | head -c 900)"
tiene "d['diagnosticos']==[] and d.get('faltan',[])==[]" || falla "2 · la propuesta no compila sobre main: $(cuerpo | head -c 900)"
N=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["numero"])' "$TMP/r.json")
dice "2 · proponer SOLO los datasets: 201, lleva los 9, los 7 anadidos con su porque (lo lee, el schema de, el conducto de la copia…), compila sobre main"

# ── 3 ───────────────────────────────────────────────────────────────────────
# fusionada, lo que ya esta en main no vuelve a arrastrarse
[ "$(pide POST /propuestas/$N/fusionar "$ANA")" = "200" ] || falla "3 · fusionar (main libre): $(cuerpo)"
[ "$(pide POST /ramas "$ANA" '{"nombre":"otra"}')" = "201" ] || falla "3 · otra rama: $(cuerpo)"
cat > "$TMP/v.yaml" <<'Y'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: pedidos_es, namespace: std, schema: ventas }
spec:
  owner: "team:demo"
  from: { table: pg.ventas.pedidos }
  fields: { id: id }
Y
C=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X PUT -H "$ANA" -H 'content-type: text/plain' -H 'x-ore-rama: ana/otra' --data-binary "@$TMP/v.yaml" "$BASE/arbol/packages/std/ventas/datasets/pedidos_es.yaml")
[ "$C" = "201" ] || [ "$C" = "200" ] || falla "3 · el dataset nuevo: $C $(cuerpo)"
[ "$(pide POST /propuestas "$ANA" '{"rama":"ana/otra","activos":["Dataset:std.ventas.pedidos_es"]}')" = "201" ] || falla "3 · proponer el nuevo: $(cuerpo)"
tiene "d['activos']==['Dataset:std.ventas.pedidos_es'] and d['anadidos']==[] and d['diagnosticos']==[]" || falla "3 · arrastro lo que ya esta en main: $(cuerpo | head -c 700)"
dice "3 · con la base en main, un dataset nuevo que lee el mismo puntero va solo: lo que ya esta en main no se arrastra"

echo "✓ lo que arrastra una propuesta de activos (0044 A.2, M1): 1–3"
