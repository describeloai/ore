#!/usr/bin/env bash
# 0038 P6c · Medida: lo que Ontology Forge pinta de una base con DOS `clientes`
# en dos schemas, con el código de la consola (rubix-platform
# `components/ontology/datos.ts` → `deLaCelda`, ejecutado por Node quitando
# tipos) sobre lo que un `ore-serve` de verdad sirve en `/documentos/*`.
#
#   1  dos entidades `Clientes`, dos ids: `ventas.espana.Clientes` y
#      `ventas.francia.Clientes` (con `namespace.name` eran uno)
#   2  la relación de `ventas.espana.Pedidos` —escrita por discover en tres
#      partes— apunta a la de `espana`
#   3  las vistas llevan su schema, y cada entidad dice a qué vista se sostiene
#      por su nombre entero (`backedById`)
#
# No va en CI: necesita la consola al lado (`CONSOLA`, por defecto C:/rubix-platform).
#
# Uso: bash pruebas-de-fuego/medida-forge-con-schema.sh
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
CONSOLA="${CONSOLA:-C:/rubix-platform}"
PUERTO="${PUERTO:-8918}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""
HARNES="$CONSOLA/.medida-forge"
falla() { echo "✗ $*" >&2; limpiar; exit 1; }
dice() { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$TMP" "$HARNES"; }
trap limpiar EXIT
ORE="$RAIZ/target/release/ore"; SERVE="$RAIZ/target/release/ore-serve"
[ -x "$ORE" ] || ORE="$ORE.exe"; [ -x "$SERVE" ] || SERVE="$SERVE.exe"

# ── un origen con `clientes` en dos schemas y una foránea dentro de `espana` ──
R="$TMP/arbol"; mkdir -p "$R"
cat > "$TMP/cat.json" <<'J'
{ "source": "pg",
  "tables": [
    { "name": "espana.clientes", "kind": "table", "primaryKey": ["id"],
      "columns": [ { "name": "id", "type": "Integer", "required": true }, { "name": "nombre", "type": "String" } ] },
    { "name": "francia.clientes", "kind": "table", "primaryKey": ["id"],
      "columns": [ { "name": "id", "type": "Integer", "required": true }, { "name": "nom", "type": "String" } ] },
    { "name": "espana.pedidos", "kind": "table", "primaryKey": ["id"],
      "columns": [ { "name": "id", "type": "Integer", "required": true }, { "name": "cliente", "type": "Integer", "required": true } ],
      "foreignKeys": [ { "columns": ["cliente"], "references": "espana.clientes", "toColumns": ["id"] } ] }
  ] }
J
( cd "$R" && "$ORE" init . --name demo >/dev/null 2>&1 && cp "$TMP/cat.json" cat.json \
  && "$ORE" discover --from cat.json --out packages/ventas --owner team:ventas >/dev/null 2>&1 ) || falla "0 · discover"
"$SERVE" --repo "$R" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --identidad cabecera --no-es-produccion > "$TMP/srv.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
for k in Entity View Table; do
  curl -sf -H 'x-ore-sujeto: persona:ana' "$BASE/documentos/$k" > "$TMP/$k.json" || falla "0 · /documentos/$k"
done
dice "0 · base \`ventas\` descubierta: $(cd "$R" && find packages/ventas -name '*.yaml' -path '*entities*' | sort | tr '\n' ' ')"

# ── el código de la consola, tal cual, con los especificadores para Node ──
mkdir -p "$HARNES"
cp "$CONSOLA/lib/ejecucion/kind.ts" "$HARNES/kind.ts"
cp "$CONSOLA/components/ontology/acme.ts" "$HARNES/acme.ts"
sed -e "s#from '@/lib/ejecucion/kind'#from './kind.ts'#" -e "s#from './acme'#from './acme.ts'#" \
  "$CONSOLA/components/ontology/datos.ts" > "$HARNES/datos.ts"
cat > "$HARNES/pinta.mts" <<'JS'
import { readFileSync } from 'node:fs';
import { deLaCelda } from './datos.ts';
const [dir] = process.argv.slice(2);
const lee = (k: string) => JSON.parse(readFileSync(`${dir}/${k}.json`, 'utf8')).documentos;
const d = deLaCelda({
  celda: 'demo', fuentes: [], paquetes: [], esquemas: {},
  documentos: lee('Entity'), vistas: lee('View'), tablas: lee('Table'),
  conceptos: [], interfaces: [], aviso: null,
});
console.log(JSON.stringify({
  entidades: d.entidades.map((e) => ({ id: e.id, backedBy: e.backedBy, backedById: e.backedById })),
  links: d.links,
  vistas: d.vistas?.map((v) => `${v.ns}|${v.schema}|${v.name}`),
}));
JS
( cd "$HARNES" && node --experimental-strip-types --no-warnings pinta.mts "$TMP" ) > "$TMP/forge.json" || falla "0 · deLaCelda"
py() { python -c "import json,sys; d=json.load(open(sys.argv[1],encoding='utf-8')); $1" "$TMP/forge.json"; }

# ── 1 ──
IDS=$(py "print(' '.join(sorted(e['id'] for e in d['entidades'])))")
echo " $IDS " | grep -q ' ventas.espana.Clientes ' && echo " $IDS " | grep -q ' ventas.francia.Clientes ' \
  || falla "1 · los ids: $IDS"
[ "$(py "print(len(set(e['id'] for e in d['entidades'])) == len(d['entidades']))")" = "True" ] || falla "1 · ids repetidos: $IDS"
dice "1 · entidades: $IDS"

# ── 2 ──
L=$(py "print(' '.join(l['from']+'->'+l['to'] for l in d['links']))")
echo "$L" | grep -q 'ventas.espana.Pedidos->ventas.espana.Clientes' || falla "2 · el link: $L"
dice "2 · links: $L"

# ── 3 ──
V=$(py "print(' '.join(sorted(d['vistas'])))")
echo "$V" | grep -q 'ventas|espana|clientes' && echo "$V" | grep -q 'ventas|francia|clientes' || falla "3 · las vistas: $V"
B=$(py "print(' '.join(sorted(e['id']+'='+str(e.get('backedById')) for e in d['entidades'])))")
echo "$B" | grep -q 'ventas.espana.Clientes=ventas.espana.clientes' && echo "$B" | grep -q 'ventas.francia.Clientes=ventas.francia.clientes' \
  || falla "3 · backedById: $B"
dice "3 · vistas: $V · se sostienen en: $B"
echo "✓ Forge distingue lo de dos schemas, medido"
