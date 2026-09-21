#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# LA PREGUNTA SE CONTESTA — W1 ③ (ADR 0030): `ore ask` sobre la copia, de punta
# a punta y sin red de verdad.
#
#   `ore materialize` (jsonl → S3 de mentira)  →  `ore ask`:
#       compila la vista, decide qué copia la contesta (View Matcher),
#       `ore-store-r2 leer` trae la copia, `hoja` la tipa por la cabecera,
#       `recomputar` ejecuta el plan reescrito, y las filas salen por stdout.
#
# Lo que afirma:
#   1  una vista con copia se contesta con la suya: todas las filas, tipadas
#      (el entero es número, el decimal va como dígitos, el nulo no está)
#   2  una vista SIN copia sobre la misma tabla la contesta la copia vecina, con
#      compensación: `where` de igualdad… y de PERTENENCIA, que materialize niega
#   3  un agregado —groupBy, count, sum, avg, having— sobre la copia plana
#   4  `--limite N` recorta la respuesta, no lo que se lee
#   5  `--seco` decide sin traer
#   6  lo que se niega: sin ninguna copia que conteste; copia declarada y no hecha
#   7  SERVIDO (W1 ④): `POST /vistas/{ns}/{n}/ejecutar` en ore-serve devuelve la
#      cabecera con `datos`; `{"limite": N}`; 404 / 409 / 422 como `ore ask`
#   8  REHACER: el puntero manda —el origen cambia sin mover el testigo y la
#      copia no se entera—; `--rehacer --vista` lee entero, sobrescribe el
#      dataset (snapshot nuevo, la historia se queda) y `ask` contesta lo nuevo
#   9  LO HUÉRFANO: la vista se va y su dataset sale del bucket con `--recoger`
#
# La copia es un DATASET (W3.6a, 0031 §10; 0033: `kind: Dataset` con `from`,
# el documento que lleva el plan): una tabla Iceberg en el bucket y un puntero
# en el árbol (`datasets/<p>_<n>.json` → `metadata_location`). El S3 de mentira
# no sabe nada de Iceberg y no le hace falta: son objetos con nombre.
#
# Necesita `ore`, `ore-serve`, `ore-store-r2` y `ore-read-jsonl` en el PATH o en
# target/{release,debug}, y python3.
# ══════════════════════════════════════════════════════════════════════════════
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PY=$(command -v python3 || command -v python) || { echo "hace falta python"; exit 2; }

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  command -v "$1"
}
ORE="$(buscar ore)" || { echo "no hay binario de \`ore\`"; exit 2; }
SERVE="$(buscar ore-serve)" || { echo "no hay binario de \`ore-serve\` — cargo build -p ore-serve"; exit 2; }
for b in ore-store-r2 ore-read-jsonl; do
  B="$(buscar $b)" || { echo "no hay binario de \`$b\` — cargo build -p ore-store -p ore-read-jsonl"; exit 2; }
  export PATH="$(dirname "$B"):$PATH"
done

fallos=0
ok()   { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '\xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice() { printf '  \xc2\xb7 %s\n' "$1"; }

TMP="${TMPDIR:-/tmp}/ore-pregunta-$$"
rm -rf "$TMP"; mkdir -p "$TMP/datos" "$TMP/arbol"
limpiar() { kill "$S3_PID" "${SRV:-}" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT

# ── el S3 de mentira ─────────────────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for i in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; cat "$TMP/s3.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira

# ── el árbol: una tabla tipada por su entidad, su copia, y las preguntas ─────
A="$TMP/arbol"
mkdir -p "$A/packages/ventas/tables" "$A/packages/ventas/views" "$A/packages/ventas/entities"
cat > "$TMP/datos/pedidos.jsonl" <<'J'
{"order_id":"p1","pais":"ES","total":"10.50","unidades":"2"}
{"order_id":"p2","pais":"ES","total":"4.25","unidades":"1"}
{"order_id":"p3","pais":"PT","total":"7","unidades":"3"}
{"order_id":"p4","pais":"FR","total":"1.10","unidades":"1"}
{"order_id":"p5","pais":"ES","total":"0.05"}
J
cat > "$TMP/datos/clientes.jsonl" <<'J'
{"cliente_id":"c1","pais":"ES"}
J
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: FICHEROS_DIR }
  - { name: copia, type: jsonl, connectionEnv: FICHEROS_DIR }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/ventas/tables/pedidos_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: pedidos_t, namespace: ventas }
spec:
  datasource: erp
  object: "pedidos.jsonl"
  columns:
    order_id: {}
    pais: {}
    total: {}
    unidades: {}
  reads: { fullScan: cheap }
  changes: { mode: upsert, key: [order_id], witness: snapshot }
Y
cat > "$A/packages/ventas/tables/clientes_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: clientes_t, namespace: ventas }
spec:
  datasource: erp
  object: "clientes.jsonl"
  columns:
    cliente_id: {}
    pais: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
# la copia: el dataset con el plan (0033), la tabla entera con sus nombres
mkdir -p "$A/packages/ventas/datasets"
cat > "$A/packages/ventas/datasets/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, pais: pais, total: total, unidades: unidades }
Y
# la entidad tipa las columnas raíz: total es Decimal, unidades Integer
cat > "$A/packages/ventas/entities/Pedido.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Entity
metadata: { name: Pedido, namespace: ventas }
spec:
  nature: entity
  primaryKey: [id]
  backedBy: pedidos
  properties:
    id: { type: String }
    pais: { type: String }
    total: { type: Decimal }
    unidades: { type: Integer }
Y
# las preguntas, sin copia propia
cat > "$A/packages/ventas/views/espana.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: espana, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, total: total }
  where: { pais: ES }
Y
cat > "$A/packages/ventas/views/iberia.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: iberia, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, pais: pais }
  where: { pais: [ES, PT] }
Y
cat > "$A/packages/ventas/views/porPais.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: porPais, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { pais: pais, n: "count()", masa: "sum(total)", media: "avg(total)" }
  groupBy: [pais]
  having: { n: ">= 2" }
Y
cat > "$A/packages/ventas/views/clientes.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: clientes, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.clientes_t }
  fields: { id: cliente_id }
Y
cat > "$A/packages/ventas/tables/proveedores_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: proveedores_t, namespace: ventas }
spec:
  datasource: erp
  object: "proveedores.jsonl"
  columns:
    proveedor_id: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/ventas/views/proveedores.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: proveedores, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.proveedores_t }
  fields: { id: proveedor_id }
Y
cat > "$A/packages/ventas/datasets/declarada.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: declarada, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.clientes_t }
  fields: { id: cliente_id, pais: pais }
Y
export FICHEROS_DIR="$TMP/datos"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$A"; falla "el árbol no compila"; exit 1; }

# la copia de `pedidos`, y solo esa
salida=$("$ORE" materialize "$A" --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "0 · materialize"; exit 1; }
rm -f "$A/datasets/ventas_declarada.json"
puntero() { "$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$A/datasets/ventas_pedidos.json" "$1"; }
ML=$(puntero metadata_location)
[ -n "$ML" ] || { falla "0 · el puntero de la copia no tiene metadata_location"; exit 1; }
case "$ML" in s3://copia/ore/v2/datasets/ventas_pedidos/metadata/00000-*.metadata.json) ;; *) falla "0 · el puntero no apunta a la tabla del dataset: $ML";; esac
[ "$(puntero operacion)" = "creada" ] || falla "0 · la primera copia tenía que decir creada: $(puntero operacion)"
[ "$(puntero dataset)" = "datasets/ventas_pedidos" ] || falla "0 · el puntero no nombra el dataset"
# lo que hay en el bucket es una tabla Iceberg: metadata.json, la lista de
# manifiestos, un manifiesto y un fichero de datos — y NADA de ore/v1/
OBJETOS=$(curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/datasets/ventas_pedidos/" | grep -o '<Key>[^<]*</Key>' | sed 's/<[^>]*>//g')
[ "$(echo "$OBJETOS" | grep -c '/metadata/.*\.metadata\.json$')" = "1" ] || falla "0 · no hay UN metadata.json: $OBJETOS"
[ "$(echo "$OBJETOS" | grep -c '/metadata/snap-.*\.avro$')" = "1" ] || falla "0 · no hay UNA lista de manifiestos: $OBJETOS"
[ "$(echo "$OBJETOS" | grep -c '/data/.*\.parquet$')" = "1" ] || falla "0 · no hay UN fichero de datos: $OBJETOS"
[ "$(curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v1/" | grep -c '<Key>')" = "0" ] || falla "0 · sigue escribiendo sobres en ore/v1/"
dice "0 · copia de ventas.pedidos hecha · $ML"

pregunta() { "$ORE" ask "$A" "$@" 2>"$TMP/err.txt"; }
cumple() { # expresión python sobre `cab` (cabecera) y `filas` (lista de dicts) de $TMP/out.txt
  "$PY" - "$TMP/out.txt" "$1" <<'EOF' || { falla "$2"; return 1; }
import json, sys
lineas = [l for l in open(sys.argv[1], encoding="utf-8").read().splitlines() if l.strip()]
cab = json.loads(lineas[0]); filas = [json.loads(l) for l in lineas[1:]]
assert eval(sys.argv[2]), (cab, filas)
EOF
}

# ── 1 · la copia propia, entera y tipada ─────────────────────────────────────
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "1 · ore ask falló"; }
cumple "cab['copia']['de']=='ventas.pedidos' and cab['copia']['metadata_location']=='$ML' and cab['compensacion']==0" "1 · contesta su copia (el puntero del dataset), sin compensación" && \
cumple "cab['filas']==5 and cab['leidas']==5 and len(filas)==5" "1 · las cinco filas" && \
cumple "cab['columnas']=={'id':'String','pais':'String','total':'Decimal','unidades':'Integer'}" "1 · las columnas con su tipo" && \
cumple "[f for f in filas if f['id']=='p1'][0]=={'id':'p1','pais':'ES','total':'10.5','unidades':2}" "1 · el decimal como dígitos, el entero como número" && \
cumple "'unidades' not in [f for f in filas if f['id']=='p5'][0]" "1 · el nulo es la propiedad ausente" && \
ok "1 · una vista con copia se contesta con la suya: 5 filas tipadas"

# ── 2 · sin copia propia: la vecina, con compensación ────────────────────────
pregunta --vista ventas.espana > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "2 · ore ask espana falló"; }
cumple "cab['copia']['de']=='ventas.pedidos' and cab['compensacion']==1" "2 · la contesta la copia de pedidos con 1 conyunto" && \
cumple "sorted(f['id'] for f in filas)==['p1','p2','p5'] and all(set(f)=={'id','total'} for f in filas)" "2 · solo España, solo id y total" && \
ok "2 · una vista sin copia la contesta la vecina, con compensación (where de igualdad)"
grep -q "contesta \`ventas.pedidos\`" "$TMP/err.txt" || falla "2 · no dijo quién contesta: $(cat "$TMP/err.txt")"

pregunta --vista ventas.iberia > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "2 · ore ask iberia falló"; }
cumple "sorted(f['id'] for f in filas)==['p1','p2','p3','p5']" "2 · la pertenencia [ES, PT] recorta sobre la copia" && \
ok "2 · …y con pertenencia, que materialize se niega a copiar"

# ── 3 · el agregado sobre la copia plana ─────────────────────────────────────
pregunta --vista ventas.porPais > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "3 · ore ask porPais falló"; }
cumple "cab['copia']['de']=='ventas.pedidos' and cab['columnas']=={'pais':'String','n':'Integer','masa':'Decimal','media':'Decimal'}" "3 · agrega sobre la copia y tipa la salida" && \
cumple "filas==[{'masa':'14.8','media':'4.933333','n':3,'pais':'ES'}]" "3 · groupBy + count + sum + avg + having: solo ES (3 pedidos), 14.8, media a seis decimales" && \
ok "3 · groupBy, count, sum, avg y having sobre la copia plana"

# ── 4 · el límite es de la respuesta ─────────────────────────────────────────
pregunta --vista ventas.pedidos --limite 2 > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "4 · --limite falló"; }
cumple "cab['filas']==2 and cab['leidas']==5 and cab['limite']==2 and len(filas)==2" "4 · 2 filas de respuesta, 5 leídas" && \
ok "4 · --limite recorta la respuesta y no lo que se lee"

# ── 5 · en seco ──────────────────────────────────────────────────────────────
antes=$(grep -c "GET" "$TMP/s3.log" || true)
pregunta --vista ventas.espana --seco > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "5 · --seco falló"; }
cumple "len(filas)==0 and cab['copia']['de']=='ventas.pedidos' and cab['plan'].startswith('sha256:')" "5 · cabecera sin filas" && \
[ "$(grep -c "GET" "$TMP/s3.log" || true)" = "$antes" ] && ok "5 · --seco decide quién contesta sin traer la copia" || falla "5 · --seco tocó el almacén"

# ── 6 · lo que se niega ──────────────────────────────────────────────────────
s=$("$ORE" ask "$A" --vista ventas.proveedores 2>&1) && falla "6 · sin copia que conteste no falló: $s"
case "$s" in *"ningún dataset contesta"*) ;; *) falla "6 · no dijo que ningún dataset contesta: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.declarada 2>&1) && falla "6 · con copia declarada y no hecha no falló: $s"
case "$s" in *"no está hecha"*) ;; *) falla "6 · no dijo que la copia no está hecha: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.clientes 2>&1) && falla "6 · con una vecina declarada y no hecha no falló: $s"
case "$s" in *'`ventas.declarada` la contesta, pero'*"no está hecha"*) ;; *) falla "6 · no dijo que la vecina contesta pero no está hecha: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.noExiste 2>&1) && falla "6 · una vista que no existe no falló"
ok "6 · se niega: sin copia que conteste · declarada y no hecha (propia, y vecina) · vista que no existe"

# ── 8 · rehacer: cuando el puntero miente ─────────────────────────────────────
# El caso de demo (medida W1 §B): la copia se hizo con un lector que callaba
# —aquí, una fila de menos— bajo la cabecera VERDADERA (mismo plan, esquema y
# testigo). Se reproduce sellando 4 filas sobre el dataset con la cabecera real
# (un snapshot más, que el puntero pasa a nombrar). Copiar otra vez dice «ya
# está»; `ask` sirve la mentira; `--rehacer` lee entero y sobrescribe: snapshot
# nuevo, la historia se queda, y `ask` ve las 5.
CAB=$(printf '{"metadata_location":"%s"}
' "$ML" | ore-store-r2 leer | head -1)
MALA=$( { printf '{"dataset":"datasets/ventas_pedidos","base":"%s","fundir":false,%s' "$ML" "${CAB#\{}"; echo; printf '{"id":"p1","pais":"ES","total":"10.50","unidades":"2"}
{"id":"p2","pais":"ES","total":"4.25","unidades":"1"}
{"id":"p3","pais":"PT","total":"7","unidades":"3"}
{"id":"p4","pais":"FR","total":"1.10","unidades":"1"}
'; } | ore-store-r2 sellar) || falla "8 · no se pudo sellar la copia mala"
ML_MALA=$("$PY" -c 'import json,sys;print(json.loads(sys.argv[1])["metadata_location"])' "$MALA")
[ -n "$ML_MALA" ] && [ "$ML_MALA" != "$ML" ] || falla "8 · la copia mala no se selló: $MALA"
# el puntero pasa a nombrarla, como si el Job la hubiera confirmado
"$PY" - "$A/datasets/ventas_pedidos.json" "$ML_MALA" <<'EOF'
import json, sys
p = json.load(open(sys.argv[1])); p["metadata_location"] = sys.argv[2]; p["filas"] = 4
json.dump(p, open(sys.argv[1], "w"), indent=2)
EOF
salida=$("$ORE" materialize "$A" --vista ventas.pedidos --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "8 · materialize"; }
case "$salida" in *"ya está · $ML_MALA"*) ;; *) falla "8 · sin --rehacer tenía que decir «ya está» con la mala: $salida";; esac
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "8 · ask con la mala"; }
cumple "cab['copia']['metadata_location']=='$ML_MALA' and cab['filas']==4" "8 · ask sirve la copia que hay: 4 filas (la mentira)"
salida=$("$ORE" materialize "$A" --seco --rehacer --vista ventas.pedidos 2>&1) || { echo "$salida"; falla "8 · --seco --rehacer"; }
case "$salida" in *"se rehará entera"*) ;; *) falla "8 · --seco --rehacer no dijo que se rehará: $salida";; esac
[ "$(curl -s -o /dev/null -w '%{http_code}' "$ORE_R2_S3_ENDPOINT/copia/${ML_MALA#s3://copia/}")" = "200" ] || falla "8 · --seco tocó el almacén"
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.pedidos --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "8 · --rehacer"; }
case "$salida" in *"rehecha: sobrescrita entera"*) ;; *) falla "8 · --rehacer no sobrescribió: $salida";; esac
ML2=$(puntero metadata_location)
"$PY" - "$A/datasets/ventas_pedidos.json" "$ML_MALA" "$ML" <<'EOF' || falla "8 · el puntero de la rehecha"
import json, sys
i = json.load(open(sys.argv[1]))
assert i["estado"] == "copiada" and i["rehecha"] is True and i["operacion"] == "sobrescrita", i
assert i["metadata_location"] not in (sys.argv[2], sys.argv[3]) and i["filas"] == 5 and i["leidas"] == 5, i
assert i["metadata_location"].split("/metadata/")[1].startswith("00002-"), i["metadata_location"]
EOF
# la historia se queda: el snapshot de la mentira sigue siendo legible tal cual
n=$(printf '{"metadata_location":"%s"}\n' "$ML_MALA" | ore-store-r2 leer | grep -c '^{"id"'); [ "$n" = "4" ] || falla "8 · el snapshot anterior ya no se lee entero ($n)"
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.pedidos --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "8 · --rehacer otra vez"; }
case "$salida" in *"rehecha: sobrescrita entera"*) ;; *) falla "8 · rehacer otra vez tenía que sobrescribir otra vez: $salida";; esac
[ "$(puntero metadata_location)" != "$ML2" ] || falla "8 · rehacer otra vez no movió el puntero"
ML=$(puntero metadata_location)
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.noExiste 2>&1) && falla "8 · una vista que no declara copia no falló"
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "8 · ask tras rehacer"; }
cumple "cab['copia']['metadata_location']=='$ML' and cab['filas']==5" "8 · ask contesta con la copia rehecha (5 filas)" && ok "8 · rehacer: el puntero decía «ya está» de una copia que mentía; --rehacer lee entero, sobrescribe (snapshot nuevo, la historia se queda) y ask ve las 5"

# ── 8b · un commit en OTRO paquete no cambia la cabecera de esta copia ───────
# Medido en victor el 18 de septiembre: la cabecera llevaba el digest del árbol
# ENTERO y 16 de 19 commits (altas, catálogos, retiradas de otras bases) dejaban
# sin recibo a todas las vistas, que releían el origen sin que nada suyo
# cambiara. Aquí: otro paquete nace, y `ventas.pedidos` sigue diciendo «ya está»;
# y el informe dice de qué árbol salió (`bundle`), que es procedencia, no llave.
mkdir -p "$A/packages/otro/tables"
cat > "$A/packages/otro/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: otro, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/otro/tables/clientes_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: clientes_t, namespace: otro }
spec:
  datasource: erp
  object: "clientes.jsonl"
  columns:
    cliente_id: {}
    pais: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
"$ORE" validate "$A" >/dev/null 2>&1 || falla "8b · el árbol con el paquete nuevo no compila"
salida=$("$ORE" materialize "$A" --vista ventas.pedidos --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "8b · materialize tras el paquete nuevo"; }
case "$salida" in *"ya está · $ML"*) ;; *) falla "8b · otro paquete en el árbol cambió la cabecera de ventas.pedidos: $salida";; esac
"$PY" - "$A/datasets/ventas_pedidos.json" <<'EOF' || falla "8b · el informe no dice de qué árbol salió"
import json, sys
i = json.load(open(sys.argv[1]))
assert i["estado"] == "al-dia" and i["bundle"].startswith("sha256:"), i
EOF
rm -rf "$A/packages/otro"
ok "8b · un paquete nuevo en el árbol no toca la cabecera de ventas.pedidos («ya está»), y el puntero lleva el bundle como procedencia"

# ── 7 · servido: ore-serve delante, contra el mismo S3 de mentira ─────────────
( cd "$A" && git init -q && git config core.autocrlf false && git -c user.name=banco -c user.email=banco@invalido add -A   && git -c user.name=banco -c user.email=banco@invalido commit -q -m "el arbol con su copia" ) || falla "7 · no se pudo dar historia al arbol"
PUERTO=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
BASE="http://127.0.0.1:$PUERTO"
FORJA_TOKEN=no-hace-falta "$SERVE" --repo "$A" --ore "$ORE" --bind "127.0.0.1:$PUERTO"   --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/serve.log" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
SUJ='x-ore-sujeto: persona:ana'
sirve() { curl -s -o "$TMP/out.json" -w '%{http_code}' -X POST -H "$SUJ" -H 'content-type: application/json' "$BASE/vistas/$1/ejecutar" -d "${2:-}"; }
servido() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); assert eval(sys.argv[2]), d' "$TMP/out.json" "$1" || falla "$2"; }
[ "$(sirve ventas/porPais)" = "200" ] || falla "7 · POST /ejecutar porPais · $(cat "$TMP/out.json")"
servido "d['view']=='ventas.porPais' and d['copia']['de']=='ventas.pedidos' and d['datos']==[{'masa':'14.8','media':'4.933333','n':3,'pais':'ES'}] and d['columnas']['media']=='Decimal' and d['limite']==200" "7 · la respuesta servida trae la cabecera y los datos"
[ "$(sirve ventas/pedidos '{"limite": 2}')" = "200" ] || falla "7 · con limite · $(cat "$TMP/out.json")"
servido "len(d['datos'])==2 and d['filas']==2 and d['leidas']==5 and d['limite']==2" "7 · {limite: 2} recorta la respuesta"
[ "$(sirve ventas/noExiste)" = "404" ] || falla "7 · una vista que no existe no dio 404: $(cat "$TMP/out.json")"
[ "$(sirve ventas/declarada)" = "409" ] || falla "7 · copia declarada y no hecha no dio 409: $(cat "$TMP/out.json")"
[ "$(sirve ventas/proveedores)" = "422" ] || falla "7 · sin copia que conteste no dio 422: $(cat "$TMP/out.json")"
[ "$(sirve ventas/pedidos '{"limite": 0}')" = "422" ] || falla "7 · limite 0 no dio 422"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/vistas/ventas/pedidos/ejecutar")" = "401" ] || falla "7 · sin identidad no dio 401"
ok "7 · servido: POST /vistas/{ns}/{n}/ejecutar → 200 con datos · limite · 404 · 409 · 422 · 401"

# ── 9 · lo huérfano: la vista se va, su dataset también ──────────────────────
# Retirar una base deja su dataset en el bucket y su puntero en `copias/`, y
# `recoger` no lo ve (limpia DENTRO de un dataset vigente). Medido en demo el
# 2026-09-18. La pasada con `--recoger` recoge lo que ningún puntero del árbol
# reclama —y los sobres heredados de `ore/v1/` que ningún puntero nombra—, y
# retira el puntero de la vista que ya no está.
kill "$SRV" 2>/dev/null; SRV=""
en_bucket() { curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=$1" | grep -c '<Key>'; }
# un sobre heredado que nadie nombra, y un dataset de una vista que no existe
curl -s -o /dev/null -X PUT "$ORE_R2_S3_ENDPOINT/copia/ore/v1/deadbeef" -d "bytes de nadie"
curl -s -o /dev/null -X PUT "$ORE_R2_S3_ENDPOINT/copia/ore/v2/datasets/nadie_nada/metadata/00000-x.metadata.json" -d "{}"
# y con --recoger DENTRO del dataset vigente: los snapshots superados (8 dejó
# tres tras el vigente) se expiran y sus ficheros se van; el puntero se mueve al metadata nuevo
# Sin edad —ni en la tabla ni en `ORE_RECOGER_EDAD`— no se expira nada (0031 §11
# ⑥, W3.6c): aquí se pide expirar todo lo superado.
salida=$(ORE_RECOGER_EDAD=0 "$ORE" materialize "$A" --recoger --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "9 · materialize --recoger con todo vigente"; }
case "$salida" in *"recogidos 3 snapshot(s) superado(s)"*) ;; *) falla "9 · tenía que expirar los 3 snapshots superados de pedidos: $salida";; esac
case "$salida" in *"huérfanas: 1 dataset(s)"*"1 objeto(s) heredado(s)"*) ;; *) falla "9 · con todo vigente tenía que recoger 1 dataset huérfano y 1 heredado: $salida";; esac
[ "$(en_bucket ore/v1/deadbeef)" = "0" ] || falla "9 · el sobre heredado sigue"
[ "$(en_bucket ore/v2/datasets/nadie_nada/)" = "0" ] || falla "9 · el dataset huérfano sigue"
[ "$(puntero metadata_location)" != "$ML" ] || falla "9 · expirar tenía que mover el puntero a un metadata.json nuevo"
ML=$(puntero metadata_location)
[ "$(en_bucket ore/v2/datasets/ventas_pedidos/data/)" = "1" ] || falla "9 · tras recoger tenía que quedar UN fichero de datos: $(en_bucket ore/v2/datasets/ventas_pedidos/data/)"
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "9 · ask tras recoger"; }
cumple "cab['copia']['metadata_location']=='$ML' and cab['filas']==5" "9 · ask sigue contestando las 5 tras recoger"
# la vista deja de declarar copia (la entidad la respalda: quitarla rompería
# OOS2018); para el almacén es lo mismo que si su base se hubiera retirado
# el dataset se retira y en su lugar queda la pregunta (una View con el mismo
# plan), para que la entidad que lo respaldaba siga teniendo de dónde salir
rm -f "$A/packages/ventas/datasets/pedidos.yaml"
cat > "$A/packages/ventas/views/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, pais: pais, total: total, unidades: unidades }
Y
[ -f "$A/packages/ventas/datasets/pedidos.yaml" ] && falla "9 · pedidos sigue siendo un dataset"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "9 · el árbol sin pedidos no compila: $(cd "$A" && "$ORE" validate . 2>&1 | grep -A1 '^error' | head -4)"
salida=$("$ORE" materialize "$A" --recoger --informe "$A/datasets" 2>&1) || { echo "$salida"; falla "9 · materialize --recoger sin pedidos"; }
case "$salida" in *"huérfanas: 1 dataset(s)"*) ;; *) falla "9 · sin pedidos tenía que recoger 1 dataset huérfano: $salida";; esac
case "$salida" in *"informe de \`ventas.pedidos\` retirado"*) ;; *) falla "9 · no retiró el puntero de la vista que ya no está: $salida";; esac
[ ! -f "$A/datasets/ventas_pedidos.json" ] || falla "9 · datasets/ventas_pedidos.json sigue en el árbol"
[ "$(en_bucket ore/v2/datasets/ventas_pedidos/)" = "0" ] || falla "9 · el dataset de la vista retirada sigue en el almacén"
ok "9 · lo huérfano: con todo vigente, recoger expira los snapshots superados y borra lo que nadie nombra; retirada la vista, su dataset sale del almacén y su puntero del árbol"

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 la pregunta se contesta: 0\xe2\x80\x939\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
