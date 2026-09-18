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
#   8  REHACER: el recibo manda —el origen cambia sin mover el testigo y la
#      copia no se entera—; `--rehacer --vista` lee entero, el recibo apunta a la
#      nueva, la superada se borra, y `ask` contesta lo nuevo
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
# la copia: la tabla entera, con sus nombres
cat > "$A/packages/ventas/views/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.pedidos_t }
  fields: { id: order_id, pais: pais, total: total, unidades: unidades }
  materialized: { datasource: copia, table: "copia.pedidos" }
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
cat > "$A/packages/ventas/views/declarada.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: declarada, namespace: ventas }
spec:
  owner: team:data
  from: { table: ventas.clientes_t }
  fields: { id: cliente_id, pais: pais }
  materialized: { datasource: copia, table: "copia.clientes" }
Y
export FICHEROS_DIR="$TMP/datos"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$A"; falla "el árbol no compila"; exit 1; }

# la copia de `pedidos`, y solo esa
salida=$("$ORE" materialize "$A" --informe "$A/copias" 2>&1) || { echo "$salida"; falla "0 · materialize"; exit 1; }
rm -f "$A/copias/ventas_declarada.json"
CLAVE=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["clave"])' "$A/copias/ventas_pedidos.json")
[ -n "$CLAVE" ] || { falla "0 · el informe de la copia no tiene clave"; exit 1; }
dice "0 · copia de ventas.pedidos hecha · $CLAVE"

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
cumple "cab['copia']['de']=='ventas.pedidos' and cab['copia']['clave']=='$CLAVE' and cab['compensacion']==0" "1 · contesta su copia, sin compensación" && \
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
case "$s" in *"ninguna copia contesta"*) ;; *) falla "6 · no dijo que ninguna copia contesta: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.declarada 2>&1) && falla "6 · con copia declarada y no hecha no falló: $s"
case "$s" in *"no está hecha"*) ;; *) falla "6 · no dijo que la copia no está hecha: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.clientes 2>&1) && falla "6 · con una vecina declarada y no hecha no falló: $s"
case "$s" in *'`ventas.declarada` la contesta, pero'*"no está hecha"*) ;; *) falla "6 · no dijo que la vecina contesta pero no está hecha: $s";; esac
s=$("$ORE" ask "$A" --vista ventas.noExiste 2>&1) && falla "6 · una vista que no existe no falló"
ok "6 · se niega: sin copia que conteste · declarada y no hecha (propia, y vecina) · vista que no existe"

# ── 8 · rehacer: cuando el recibo miente ──────────────────────────────────────
# El caso de demo (medida W1 §B): la copia se hizo con un lector que callaba
# —aquí, una fila de menos— bajo la cabecera VERDADERA (mismo plan, esquema y
# testigo). Se reproduce sellando 4 filas con la cabecera real y apuntando el
# recibo a ese artefacto. Copiar otra vez dice «ya está»; `ask` sirve la
# mentira; `--rehacer` lee entero, mueve el recibo y borra la superada.
CAB=$(printf '{"clave":"%s"}
' "$CLAVE" | ore-store-r2 leer | head -1)
MALA=$( { echo "$CAB"; printf '{"id":"p1","pais":"ES","total":"10.50","unidades":"2"}
{"id":"p2","pais":"ES","total":"4.25","unidades":"1"}
{"id":"p3","pais":"PT","total":"7","unidades":"3"}
{"id":"p4","pais":"FR","total":"1.10","unidades":"1"}
'; } | ore-store-r2 sellar) || falla "8 · no se pudo sellar la copia mala"
K_MALA=$("$PY" -c 'import json,sys;print(json.loads(sys.argv[1])["clave"])' "$MALA")
RECIBO=$("$PY" -c 'import json,sys;print(json.loads(sys.argv[1])["recibo"])' "$MALA")
[ -n "$K_MALA" ] && [ "$K_MALA" != "$CLAVE" ] && [ -n "$RECIBO" ] || falla "8 · la copia mala no se selló: $MALA"
curl -s -o /dev/null -X DELETE "$ORE_R2_S3_ENDPOINT/copia/$RECIBO"
[ "$(curl -s -o /dev/null -w '%{http_code}' -X PUT "$ORE_R2_S3_ENDPOINT/copia/$RECIBO" -d "$K_MALA")" = "200" ] || falla "8 · no se pudo apuntar el recibo a la copia mala"
salida=$("$ORE" materialize "$A" --vista ventas.pedidos --informe "$A/copias" 2>&1) || { echo "$salida"; falla "8 · materialize"; }
case "$salida" in *"ya está · $K_MALA"*) ;; *) falla "8 · sin --rehacer tenía que decir «ya está» con la mala: $salida";; esac
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "8 · ask con la mala"; }
cumple "cab['copia']['clave']=='$K_MALA' and cab['filas']==4" "8 · ask sirve la copia que hay: 4 filas (la mentira)"
salida=$("$ORE" materialize "$A" --seco --rehacer --vista ventas.pedidos 2>&1) || { echo "$salida"; falla "8 · --seco --rehacer"; }
case "$salida" in *"se rehará entera"*) ;; *) falla "8 · --seco --rehacer no dijo que se rehará: $salida";; esac
[ "$(curl -s -o /dev/null -w '%{http_code}' "$ORE_R2_S3_ENDPOINT/copia/$K_MALA")" = "200" ] || falla "8 · --seco tocó el almacén"
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.pedidos --informe "$A/copias" 2>&1) || { echo "$salida"; falla "8 · --rehacer"; }
case "$salida" in *"rehecha: el recibo apunta a la nueva y se borró $K_MALA"*) ;; *) falla "8 · --rehacer no movió el recibo ni borró la superada: $salida";; esac
"$PY" - "$A/copias/ventas_pedidos.json" "$K_MALA" "$CLAVE" <<'EOF' || falla "8 · el informe de la rehecha"
import json, sys
i = json.load(open(sys.argv[1]))
assert i["estado"] == "copiada" and i["rehecha"] is True and i["superada"] == sys.argv[2] and i["clave"] == sys.argv[3] and i["filas"] == 5 and i["leidas"] == 5, i
EOF
[ "$(curl -s -o /dev/null -w '%{http_code}' "$ORE_R2_S3_ENDPOINT/copia/$K_MALA")" = "404" ] || falla "8 · la copia superada sigue en el almacén"
[ "$(curl -s "$ORE_R2_S3_ENDPOINT/copia/$RECIBO")" = "$CLAVE" ] || falla "8 · el recibo no apunta a la buena"
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.pedidos --informe "$A/copias" 2>&1) || { echo "$salida"; falla "8 · --rehacer otra vez"; }
case "$salida" in *"los mismos bytes, el recibo no se movió"*) ;; *) falla "8 · rehacer con los mismos bytes tenía que decirlo: $salida";; esac
salida=$("$ORE" materialize "$A" --rehacer --vista ventas.noExiste 2>&1) && falla "8 · una vista que no declara copia no falló"
pregunta --vista ventas.pedidos > "$TMP/out.txt" || { cat "$TMP/err.txt"; falla "8 · ask tras rehacer"; }
cumple "cab['copia']['clave']=='$CLAVE' and cab['filas']==5" "8 · ask contesta con la copia rehecha (5 filas)" && ok "8 · rehacer: el recibo decía «ya está» de una copia que mentía; --rehacer lee entero, mueve el recibo, borra la superada y ask ve las 5"

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

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 la pregunta se contesta: 0\xe2\x80\x938\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
