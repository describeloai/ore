#!/usr/bin/env bash
# S3 DE VERDAD (0046 E4): `ore-read-s3` contra el bucket de la medida F1, SOLO
# LECTURA. Lo que se fija es lo que F1 midió con boto3 y el catálogo tiene que
# decir igual:
#
#   1  check        las tres acciones, cada una sobre su ARN; y con la región
#                   equivocada, el motivo es la región —no un permiso—
#   2  explorar     las carpetas, con su URL SIN credencial
#   3  catalogo     12 tablas y 5 conjuntos de objetos:
#                   · `ventas/pedidos/fecha=*` es UNA tabla Parquet, `fecha` es
#                     columna, 1500 filas del pie, `total` Decimal<12, 2>
#                   · los CSV sueltos de `Nueva carpeta/` son una tabla CADA UNO
#                     (esquemas distintos), y el código postal se queda en texto
#                   · `logs/*.jsonl` es una tabla, con `usuario` (el segundo día)
#                   · el BOM de `product_category_name_translation.csv` se quita
#                   · contratos (document, 4), fotos (image, 3, jpg y png), los
#                     zips (archive, con su patrón) y la raíz
#                   · se lee poco: menos de 1 MB para 171
#   4  testigo      el listado de un prefijo, estable entre dos lecturas
#   5  por `ore`    `source add` + `source check` + `source catalog`: el mismo
#                   catálogo, y la credencial no está ni en el manifiesto ni en
#                   el catálogo
#
# Uso:  ORE_S3_URL='s3://<bucket>/?region=…&access_key_id=…&secret_access_key=…' \
#         bash pruebas-de-fuego/s3-real.sh
# La URL no se imprime nunca: va por stdin a los drivers.
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
PY=$(command -v python3 || command -v python)
trap 'rm -rf "$TMP"' EXIT

falla() { echo "✗ $*" >&2; exit 1; }
dice()  { echo "  · $*"; }
[ -n "${ORE_S3_URL:-}" ] || falla "falta ORE_S3_URL (la URL del bucket con su credencial)"

buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/debug/$1" "$RAIZ/target/debug/$1.exe" \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
S3="$(buscar ore-read-s3)" || falla "no hay binario de \`ore-read-s3\`"
ORE="$(buscar ore)"        || falla "no hay binario de \`ore\`"

# Lo que salga no puede llevar la credencial: se busca el VALOR de la clave y
# del secreto de la URL en cada salida (cómo los tape quien los tape —`ore`
# usa un carácter propio— no importa; que no estén, sí).
limpio() { "$PY" - "$1" <<'PYEOF'
import os, sys, urllib.parse
q = urllib.parse.parse_qs(os.environ["ORE_S3_URL"].split("?", 1)[1])
t = open(sys.argv[1], encoding="utf-8", errors="replace").read()
sys.exit(1 if any(v and v in t for k in ("access_key_id", "secret_access_key") for v in q.get(k, [])) else 0)
PYEOF
}
coord() { "$PY" -c "import json,os,sys; print(json.dumps({'url': os.environ['ORE_S3_URL'], 'objeto': sys.argv[1] if len(sys.argv)>1 else ''}))" "$@"; }
en() { "$PY" -c "import json,sys; d=json.load(open(sys.argv[1],encoding='utf-8')); assert ($2), sys.argv[2]" "$1" "$3" \
  || falla "$3 · $(head -c 400 "$1")"; }

# ── 1 · check ────────────────────────────────────────────────────────────────
coord | "$S3" check > "$TMP/check.json" 2>&1 || falla "1 · check no respondió: $(cat "$TMP/check.json")"
limpio "$TMP/check.json" || falla "1 · check imprimió la credencial"
en "$TMP/check.json" "d['ok'] and all(d['permisos'][k]['ok'] for k in ('listar','leer','versiones'))" "1 · las tres acciones"
en "$TMP/check.json" "d['permisos']['listar']['rol']=='s3:ListBucket' and d['permisos']['leer']['donde'].endswith('/*')" "1 · cada acción sobre su ARN"
MALA=$("$PY" -c "import os,re; print(re.sub(r'region=[^&]*','region=us-east-1',os.environ['ORE_S3_URL']))")
ORE_S3_URL="$MALA" coord | "$S3" check > "$TMP/mala.json" 2>&1
en "$TMP/mala.json" "not d['ok'] and d['porque'].startswith('el bucket está en') and 'falta' not in d['porque']" "1 · la región equivocada se dice como región"
dice "1 · check: ListBucket, GetObject y ListBucketVersions, cada uno sobre su ARN; la región equivocada es la región"

# ── 2 · explorar ─────────────────────────────────────────────────────────────
coord | "$S3" explorar > "$TMP/explorar.json" 2>&1 || falla "2 · explorar: $(cat "$TMP/explorar.json")"
limpio "$TMP/explorar.json" || falla "2 · explorar imprimió la credencial"
en "$TMP/explorar.json" "any(c['nombre']=='Nueva carpeta/' for c in d['contiene'])" "2 · la carpeta de F1"
dice "2 · explorar: las carpetas con su URL, sin credencial"

# ── 3 · catalogo ─────────────────────────────────────────────────────────────
"$PY" -c "import os,sys; sys.stdout.write(os.environ['ORE_S3_URL'])" | "$S3" catalogo s3_ventas > "$TMP/cat.json" 2> "$TMP/avisos.txt" \
  || falla "3 · catalogo: $(cat "$TMP/avisos.txt")"
limpio "$TMP/cat.json" && limpio "$TMP/avisos.txt" || falla "3 · el catálogo lleva la credencial"
T="{t['name']: t for t in d['tables']}"
O="{o['name']: o for o in d.get('objects', [])}"
en "$TMP/cat.json" "len(d['tables'])==12 and len(d['objects'])==5" "3 · 12 tablas y 5 conjuntos"
en "$TMP/cat.json" "(lambda t: t['object']=='Nueva carpeta/ventas/pedidos/' and t['format']['partitions']==['fecha'] and t['rows']==1500 and {c['name']: c.get('type') for c in t['columns']}['total']=='Decimal<12, 2>' and any(c['name']=='fecha' for c in t['columns']))($T['nueva_carpeta.pedidos'])" "3 · la tabla Hive"
en "$TMP/cat.json" "(lambda t: t['object'].endswith('olist_customers_dataset.csv') and {c['name']: c.get('type') for c in t['columns']}['customer_zip_code_prefix']=='String')($T['nueva_carpeta.olist_customers_dataset'])" "3 · un CSV suelto, y su código postal en texto"
en "$TMP/cat.json" "sum(1 for n in $T if n.startswith('nueva_carpeta.olist_'))==8" "3 · los ocho CSV de Olist, cada uno su tabla"
en "$TMP/cat.json" "any(c['name']=='usuario' for c in $T['nueva_carpeta.logs']['columns'])" "3 · el JSONL, con la unión de sus claves"
en "$TMP/cat.json" "$T['nueva_carpeta.product_category_name_translation']['columns'][0]['name']=='product_category_name'" "3 · sin BOM"
en "$TMP/cat.json" "(lambda o: o['nueva_carpeta.contratos']['media']=='document' and o['nueva_carpeta.contratos']['count']==4 and o['nueva_carpeta.fotos']['media']=='image' and o['nueva_carpeta.fotos']['extensions']==['jpg','png'] and o['nueva_carpeta.nueva_carpeta_zip']['match']=='*.zip')($O)" "3 · los conjuntos de objetos"
grep -q "catalogado con" "$TMP/avisos.txt" || falla "3 · no dice cuánto leyó"
KB=$(grep -oE '[0-9]+ KB leídos' "$TMP/avisos.txt" | grep -oE '^[0-9]+')
[ "${KB:-99999}" -lt 1024 ] || falla "3 · leyó ${KB} KB: tiene que ser menos de 1 MB"
dice "3 · catalogo: 12 tablas y 5 conjuntos (Hive, CSV sueltos, JSONL unido, sin BOM; contratos, fotos, zips); ${KB} KB leídos"

# ── 4 · testigo ──────────────────────────────────────────────────────────────
A=$(coord "Nueva carpeta/contratos/" | "$S3" testigo) || falla "4 · testigo"
B=$(coord "Nueva carpeta/contratos/" | "$S3" testigo)
[ "$A" = "$B" ] || falla "4 · el testigo de un listado que no cambió cambió"
echo "$A" | grep -q '"modo":"listing"' || falla "4 · el testigo no es de listado: $A"
dice "4 · testigo: la huella del listado, estable"

# ── 5 · por ore ──────────────────────────────────────────────────────────────
mkdir -p "$TMP/arbol" && cd "$TMP/arbol" && "$ORE" init . >/dev/null 2>&1 || falla "5 · ore init"
PATH="$(dirname "$S3"):$PATH" "$ORE" source add --name s3_ventas "$ORE_S3_URL" > "$TMP/add.txt" 2>&1 || falla "5 · source add"
limpio "$TMP/add.txt" || falla "5 · source add imprimió la credencial"
limpio ontology.config.yaml || falla "5 · el manifiesto lleva la credencial"
PATH="$(dirname "$S3"):$PATH" "$ORE" source check s3_ventas > "$TMP/chk.txt" 2>&1 || falla "5 · source check: $(cat "$TMP/chk.txt")"
PATH="$(dirname "$S3"):$PATH" "$ORE" source catalog s3_ventas --out "$TMP/cat2.json" > /dev/null 2>&1 || falla "5 · source catalog"
limpio "$TMP/cat2.json" || falla "5 · el catálogo de ore lleva la credencial"
en "$TMP/cat2.json" "len(d['tables'])==12 and len(d['objects'])==5" "5 · el mismo catálogo por ore"
dice "5 · por ore: source add / check / catalog, y la credencial no está ni en el manifiesto ni en el catálogo"

echo
echo "ok · ore-read-s3 contra un bucket real: check por acción y ARN, explorar, el catálogo de tablas y objetos que F1 midió, el testigo de listado, y ore de punta a punta"
