#!/usr/bin/env bash
# 0038 P6b · Medida: lo que la CONSOLA escribe y corre en un schema que no es
# `default`, con su código de verdad (rubix-platform `lib/ejecucion/borrador.ts`
# y `kind.ts`, ejecutados por Node quitando tipos) contra un árbol descubierto y
# un `ore-serve` de verdad.
#
#   1  el borrador de una View desde el schema `rubix_demo_ventas`: su ruta, su
#      `metadata.schema`, su `from` de tres partes — y `ore validate` no empeora
#   2  el mismo desde `default`: en `packages/ventas/views/`, v1alpha12, y
#      tampoco empeora
#   3  el de un Dataset en el schema: tampoco empeora
#   4  Run: `nombreCualificado` lee el schema del documento, y la ruta de tres
#      partes llega a la vista (409: sin copia) donde la de dos partes no (404)
#
# No va en CI: necesita la consola al lado (`CONSOLA`, por defecto
# C:/rubix-platform).
#
# Uso: bash pruebas-de-fuego/medida-borrador-en-schema.sh
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
CONSOLA="${CONSOLA:-C:/rubix-platform}"
PUERTO="${PUERTO:-8917}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""
HARNES="$CONSOLA/.medida-borrador"
falla() { echo "✗ $*" >&2; limpiar; exit 1; }
dice() { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$TMP" "$HARNES"; }
trap limpiar EXIT
ORE="$RAIZ/target/release/ore"; SERVE="$RAIZ/target/release/ore-serve"
[ -x "$ORE" ] || ORE="$ORE.exe"; [ -x "$SERVE" ] || SERVE="$SERVE.exe"

# ── el árbol: una base descubierta, schema del origen `rubix_demo_ventas` ──
R="$TMP/arbol"; mkdir -p "$R"
( cd "$R" && "$ORE" init . --name demo >/dev/null 2>&1 \
  && cp "$RAIZ/crates/ore-cli/tests/catalogos/bigquery-rubix-demo-ventas.json" cat.json \
  && "$ORE" discover --from cat.json --out packages/ventas \
       --only rubix_demo_ventas.clientes --only rubix_demo_ventas.Pedidos --owner team:ventas --no-model --type foreign >/dev/null 2>&1 ) \
  || falla "0 · discover"
# La fuente, declarada en el manifiesto raíz (lo que `ore source add` deja en una celda).
printf 'datasources:\n  - { name: bq_ventas, type: bigquery, connectionEnv: BQ_VENTAS }\n' >> "$R/ontology.config.yaml"
# Y el conducto de la copia autorizado (lo que el aprovisionamiento de una celda
# declara): sin él, un Dataset es OOS4011 en `default` igual que en un schema.
printf 'apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: default }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      oos.maturity: DRAFT\n' > "$R/conduits.yaml"
errores() { ( cd "$R" && "$ORE" validate . 2>&1 | tail -1 ); }
# ⛔ Un árbol que YA compila: el validador se para en la primera fase que falla, y
#   uno con errores de antes taparía los del borrador en fases posteriores (medido:
#   con los OOS2010 de un discover que modela, todo salía «igual»). `--no-model` es
#   lo que escribe el alta de una base desde la consola.
ANTES=$(errores)
[ "$ANTES" = "ok · sin errores" ] || falla "0 · el árbol de partida no compila: $ANTES"
"$SERVE" --repo "$R" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --identidad cabecera --no-es-produccion > "$TMP/srv.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
curl -sf -H 'x-ore-sujeto: persona:ana' "$BASE/paquetes/ventas/esquema" > "$TMP/esquema.json" || falla "0 · /esquema"
dice "0 · árbol: $ANTES · esquema servido"

# ── el código de la consola, tal cual, con los especificadores para Node ──
mkdir -p "$HARNES"
sed "s#from './kind'#from './kind.ts'#" "$CONSOLA/lib/ejecucion/borrador.ts" > "$HARNES/borrador.ts"
cp "$CONSOLA/lib/ejecucion/kind.ts" "$HARNES/kind.ts"
cat > "$HARNES/genera.mts" <<'JS'
import { readFileSync, writeFileSync } from 'node:fs';
import { textoDelBorrador } from './borrador.ts';
import { nombreCualificado } from './kind.ts';
const [esquema, kind, schema, salida] = process.argv.slice(2);
const tablas = JSON.parse(readFileSync(esquema, 'utf8')).tables;
if (kind === 'nombre') {
  const [ruta, fichero] = [schema, salida];
  console.log(JSON.stringify(nombreCualificado(ruta, readFileSync(fichero, 'utf8'))));
} else {
  const b = textoDelBorrador(kind as 'view' | 'dataset', 'ventas', tablas, 'rubix_demo_ventas.clientes', 'demo', schema || undefined);
  writeFileSync(salida, JSON.stringify(b));
}
JS
genera() { # kind schema → $TMP/b.json
  ( cd "$HARNES" && node --experimental-strip-types --no-warnings genera.mts "$TMP/esquema.json" "$1" "$2" "$TMP/b.json" ) || falla "genera $1 $2"
  python -c "import json,sys; b=json.load(open(sys.argv[1],encoding='utf-8')); print(b['ruta'])" "$TMP/b.json"
}
escribe() { # escribe el borrador en el árbol; imprime su ruta
  python - "$TMP/b.json" "$R" <<'PY'
import json, sys, os
b = json.load(open(sys.argv[1], encoding='utf-8'))
p = os.path.join(sys.argv[2], b['ruta'])
os.makedirs(os.path.dirname(p), exist_ok=True)
open(p, 'w', encoding='utf-8', newline='\n').write(b['texto'])
print(b['ruta'])
PY
}

# ── 1 · View en el schema ──
RUTA=$(genera view rubix_demo_ventas)
[ "$RUTA" = "packages/ventas/rubix_demo_ventas/views/clientesQuestion.yaml" ] || falla "1 · la ruta: $RUTA"
escribe >/dev/null
grep -q 'apiVersion: oos.dev/v1alpha13' "$R/$RUTA" && grep -q 'schema: rubix_demo_ventas }' "$R/$RUTA" || falla "1 · la metadata: $(cat "$R/$RUTA")"
grep -q 'from: { view: ventas.rubix_demo_ventas.clientes }' "$R/$RUTA" || falla "1 · el from: $(grep from: "$R/$RUTA")"
[ "$(errores)" = "$ANTES" ] || falla "1 · el árbol empeoró: $(cd "$R" && "$ORE" validate . 2>&1 | grep -A3 clientesQuestion)"
dice "1 · View en el schema: $RUTA · v1alpha13 · from ventas.rubix_demo_ventas.clientes · el árbol igual"
VISTA="$RUTA"

# ── 2 · View desde default ──
RUTA=$(genera view default)
[ "$RUTA" = "packages/ventas/views/clientes.yaml" ] || falla "2 · la ruta: $RUTA"
escribe >/dev/null
grep -q 'apiVersion: oos.dev/v1alpha12' "$R/$RUTA" && grep -q 'metadata: { name: clientes, namespace: ventas }' "$R/$RUTA" || falla "2 · la metadata: $(cat "$R/$RUTA")"
grep -q 'from: { view: ventas.rubix_demo_ventas.clientes }' "$R/$RUTA" || falla "2 · el from"
[ "$(errores)" = "$ANTES" ] || falla "2 · el árbol empeoró: $(cd "$R" && "$ORE" validate . 2>&1 | head -8)"
rm "$R/$RUTA"
dice "2 · View desde default: $RUTA · v1alpha12 · lee el schema en tres partes · el árbol igual"

# ── 3 · Dataset en el schema ──
RUTA=$(genera dataset rubix_demo_ventas)
escribe >/dev/null
[ "$(errores)" = "$ANTES" ] || falla "3 · el árbol empeoró: $(cd "$R" && "$ORE" validate . 2>&1 | head -8)"
rm "$R/$RUTA"
dice "3 · Dataset en el schema: $RUTA · el árbol igual"

# ── 4 · Run ──
QN=$( cd "$HARNES" && node --experimental-strip-types --no-warnings genera.mts "$TMP/esquema.json" nombre "$VISTA" "$R/$VISTA" )
[ "$QN" = '{"ns":"ventas","schema":"rubix_demo_ventas","nombre":"clientesQuestion"}' ] || falla "4 · nombreCualificado: $QN"
TRES=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' -d '{"limite":5}' "$BASE/vistas/ventas/rubix_demo_ventas/clientesQuestion/ejecutar")
DOS=$(curl -s -o "$TMP/r2.json" -w '%{http_code}' -X POST -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' -d '{"limite":5}' "$BASE/vistas/ventas/clientesQuestion/ejecutar")
[ "$TRES" != "404" ] || falla "4 · la ruta de tres partes no la encontró: $(cat "$TMP/r.json")"
[ "$DOS" = "404" ] || falla "4 · la de dos partes la encontró ($DOS): $(cat "$TMP/r2.json")"
dice "4 · Run: $QN · /vistas/ventas/rubix_demo_ventas/clientesQuestion/ejecutar → $TRES ($(head -c 120 "$TMP/r.json")) · la de dos partes → 404"
echo "✓ lo que la consola escribe y corre en un schema, medido"
