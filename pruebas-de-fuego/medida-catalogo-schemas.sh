#!/usr/bin/env bash
# 0038 P6d · Medida: los schemas que el CATÁLOGO de la consola pinta son los
# declarados (más `default`), no cada carpeta. Con el código de la consola
# (rubix-platform `lib/server/assets.ts` → `comoDatabaseDesdeAssets`, por Node
# quitando tipos) sobre el `GET /assets` de un `ore-serve` de verdad.
#
# La base: el schema del origen (`rubix_demo_ventas`), uno creado y vacío
# (`espana`), una carpeta que NO es schema (`transforms/`, con un `.sql`) y una
# View en `default`.
#
#   1  los schemas pintados: default, espana, rubix_demo_ventas — y no transforms
#   2  cada ítem en el suyo: la View de default en default, las del origen en el
#      suyo, espana vacío; ninguno se pierde
#
# No va en CI: necesita la consola al lado (`CONSOLA`, por defecto C:/rubix-platform).
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
CONSOLA="${CONSOLA:-C:/rubix-platform}"
PUERTO="${PUERTO:-8919}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""
HARNES="$CONSOLA/.medida-catalogo"
falla() { echo "✗ $*" >&2; limpiar; exit 1; }
dice() { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$TMP" "$HARNES"; }
trap limpiar EXIT
ORE="$RAIZ/target/release/ore"; SERVE="$RAIZ/target/release/ore-serve"
[ -x "$ORE" ] || ORE="$ORE.exe"; [ -x "$SERVE" ] || SERVE="$SERVE.exe"

R="$TMP/arbol"; mkdir -p "$R"
( cd "$R" && "$ORE" init . --name demo >/dev/null 2>&1 \
  && cp "$RAIZ/crates/ore-cli/tests/catalogos/bigquery-rubix-demo-ventas.json" cat.json \
  && "$ORE" discover --from cat.json --out packages/ventas \
       --only rubix_demo_ventas.clientes --only rubix_demo_ventas.Pedidos \
       --owner team:ventas --no-model --type foreign >/dev/null 2>&1 \
  && "$ORE" package schema new ventas espana >/dev/null 2>&1 ) || falla "0 · la base"
mkdir -p "$R/packages/ventas/transforms" "$R/packages/ventas/views"
printf 'create or replace table ventas.cuenta as select 1 as n\n' > "$R/packages/ventas/transforms/cuenta.sql"
printf 'apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: resumen, namespace: ventas }\nspec:\n  owner: "team:ventas"\n  from: { table: ventas.rubix_demo_ventas.clientes }\n  fields: { id: id }\n' > "$R/packages/ventas/views/resumen.yaml"
"$SERVE" --repo "$R" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --identidad cabecera --no-es-produccion > "$TMP/srv.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
curl -sf -H 'x-ore-sujeto: persona:ana' "$BASE/assets" > "$TMP/assets.json" || falla "0 · /assets"

mkdir -p "$HARNES"
# `server-only` y la consulta fuera: lo que se mide es la función pura.
grep -v "^import 'server-only';" "$CONSOLA/lib/server/assets.ts" \
  | sed -e "s#^import { consultarJson } from './query';#const consultarJson = (_: unknown): never => { throw new Error('sin red'); };#" \
  > "$HARNES/assets.ts"
cat > "$HARNES/pinta.mts" <<'JS'
import { readFileSync } from 'node:fs';
import { comoDatabaseDesdeAssets } from './assets.ts';
const i = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const p = i.paquetes.find((x: { name: string }) => x.name === 'ventas');
const db = comoDatabaseDesdeAssets(p, Object.values(i.items));
console.log(JSON.stringify(db.schemas.map((s) => ({
  id: s.id, name: s.name,
  items: [...s.datasets, ...s.views, ...(s.otros ?? [])].map((x) => x.id).sort(),
}))));
JS
( cd "$HARNES" && node --experimental-strip-types --no-warnings pinta.mts "$TMP/assets.json" ) > "$TMP/db.json" || falla "0 · comoDatabaseDesdeAssets"
py() { python -c "import json,sys; d=json.load(open(sys.argv[1],encoding='utf-8')); $1" "$TMP/db.json"; }

N=$(py "print(' '.join(s['name'] for s in d))")
[ "$N" = "default espana rubix_demo_ventas" ] || falla "1 · los schemas pintados: [$N]"
C=$(python -c "import json,sys; d=json.load(open(sys.argv[1],encoding='utf-8')); print(' '.join([p for p in d['paquetes'] if p['name']=='ventas'][0]['carpetas']))" "$TMP/assets.json")
dice "1 · schemas pintados [$N] · carpetas del índice [$C]"

py "
por = {s['name']: s['items'] for s in d}
assert por['default'] == ['view:ventas.resumen'], por['default']
assert por['espana'] == [], por['espana']
assert por['rubix_demo_ventas'] and all('ventas.rubix_demo_ventas.' in x for x in por['rubix_demo_ventas']), por['rubix_demo_ventas']
assert [s['id'] for s in d] == ['paquete:ventas/', 'paquete:ventas/espana', 'paquete:ventas/rubix_demo_ventas'], [s['id'] for s in d]
" || falla "2 · los ítems por schema: $(cat "$TMP/db.json")"
dice "2 · $(py "print(' · '.join(s['name']+': '+str(len(s['items'])) for s in d))") ítems; ids paquete:ventas/, …/espana, …/rubix_demo_ventas"
echo "✓ el catálogo pinta los schemas declarados, medido"
