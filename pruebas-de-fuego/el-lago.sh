#!/usr/bin/env bash
# ══════════════════════════════════════════════════════════════════════════════
# EL LAGO — W3.6b (0031 §10): el swap del puntero por `ore-serve`, el
# `datasource: lago`, la ficha con la historia, y el mantenimiento — de punta a
# punta y sin red de verdad: una forja pelada (`file://`), el S3 de mentira y
# `ore-store-r2` haciendo de escritor de un puesto.
#
# Lo que afirma:
#   0  el árbol nace con el lago (`ore init` declara `datasource: lago`)
#   1  CONFIRMAR: `POST /datasets/{ns}/{n}/confirmar {metadata_location, columnas}`
#      deja en UN commit —firmado por el sujeto— la `Table` del lago tipada, el
#      puntero `datasets/<ns>_<n>.json`, y el árbol compila
#   2  la lista y la ficha: `GET /datasets`, `GET /datasets/{ns}/{n}` con los
#      snapshots de la tabla (`ore-store historia`) y su esquema
#   3  el CAS semántico: con `esperado` al día, 200 y el puntero se mueve; con
#      `esperado` viejo, 409 y `actual`; sin `esperado` sobre un puntero que
#      existe, 409; lo que no está en el bucket, 422; una Table de otra fuente,
#      422 — y nada de eso deja commit
#   4  la carrera: cuatro escritores a la vez con el mismo `esperado`: UNO gana
#      y tres reciben 409 (la forja decide, y ahora se dice)
#   5  la historia del puntero es la historia de la tabla (`GET /arbol/historia`)
#   6  el mantenimiento: `ore datasets . --recoger --edad 0` expira lo superado,
#      retira lo que nadie nombra y mueve el puntero; `--seco` no toca nada
#
# Y el verbo escribir sobre el árbol (W3.6c c2, 0031 §11), con `ore-store
# escribir` (la tabla Arrow por IPC) haciendo de `write()`:
#   7  `ore datasets --commit`: la tabla NACE del cuerpo que `escribir` devolvió
#      (`assert-create` + sus cambios): metadata.json, la `Table` del lago con
#      las columnas de OOS traducidas de Iceberg (`Integer`, `String`,
#      `Decimal`, `DateTimeTz`), el puntero con `uuid` y `operacion`; compila
#   8  la clave de operación: la MISMA escritura otra vez no deja snapshot ni
#      mueve nada (`repetida`); otra clave anexa; un cuerpo con la base vieja es
#      código 75 con `actual`; y una columna nueva regenera la `Table`
#   9  dos tablas en un commit (`table-changes`): las dos nacen o se mueven en
#      la misma pasada; una `View` como destino se niega sin tocar nada
#  10  la retención declarada en la tabla (`--retencion p.t --edad 0`) es la que
#      `--recoger` obedece SIN `--edad`; la que nació con `--retencion-defecto
#      7d` conserva; la que no tiene ninguna no expira
#
# Y el catálogo REST de Iceberg en `ore-serve` (W3.6c c3, 0031 §11 ①③), con
# PyIceberg y DuckDB DE VERDAD como clientes (lo de la medida, como prueba):
#  11  PyIceberg: `create_table` + dos `append` contra `/v1/…` → la Table y el
#      puntero nacen en el árbol firmados por el sujeto, y se lee de vuelta; dos
#      manos con la misma base: 409 y el cliente refresca y reintenta solo
#  12  DuckDB: `ATTACH … TYPE iceberg`, lee lo de PyIceberg, `INSERT` (por
#      `transactions/commit`), `CREATE TABLE … AS` (`stage-create` +
#      `assert-create`); PyIceberg lee lo de DuckDB
#  13  la credencial prestada: con `X-Iceberg-Access-Delegation` el
#      `LoadTableResult` trae `config` y `storage-credentials` acotadas al
#      prefijo de la tabla; sin ella, nada; una View como destino es 400, una
#      tabla que no existe 404, la base vieja 409 `CommitFailedException` con
#      `actual` — todos con la forma de error de la spec
#
# Necesita `ore`, `ore-serve`, `ore-store-r2` en target/{release,debug}, git y
# python3 con pyarrow (para 7–10), pyiceberg y duckdb (11–13); sin ellos se
# saltan y se dice.
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
STORE="$(buscar ore-store-r2)" || { echo "no hay binario de \`ore-store-r2\` — cargo build -p ore-store"; exit 2; }
export PATH="$(dirname "$STORE"):$PATH"

fallos=0
ok()   { printf '  \xe2\x9c\x93 %s\n' "$1"; }
falla() { printf '\xe2\x9c\x97 %s\n' "$1"; fallos=$((fallos + 1)); }
dice() { printf '  \xc2\xb7 %s\n' "$1"; }

TMP="${TMPDIR:-/tmp}/ore-lago-$$"
rm -rf "$TMP"; mkdir -p "$TMP"
SRV=""; S3_PID=""
limpiar() { kill "$S3_PID" "${SRV:-}" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT
export GIT_AUTHOR_NAME=semilla GIT_AUTHOR_EMAIL=semilla@x
export GIT_COMMITTER_NAME=semilla GIT_COMMITTER_EMAIL=semilla@x

# ── el S3 de mentira ─────────────────────────────────────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for i in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log")
[ -n "$S3_PUERTO" ] || { echo "el S3 de mentira no arrancó"; cat "$TMP/s3.log"; exit 2; }
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia"

# ── 0 · la forja, sembrada con un árbol que nace con el lago ─────────────────
FORJA="$TMP/arbol.git"
git init -q --bare -b main "$FORJA" || { echo "no se pudo crear la forja"; exit 2; }
git clone -q "$FORJA" "$TMP/semilla" 2>/dev/null
A="$TMP/semilla"
( cd "$A" && "$ORE" init . --name lago >/dev/null 2>&1 ) || { "$ORE" init "$A" --name lago; falla "0 · ore init"; exit 1; }
grep -q "name: lago" "$A/ontology.config.yaml" && grep -q "connectionEnv: LAGO_URL" "$A/ontology.config.yaml" \
  || falla "0 · ore init no declaró el datasource lago"
# una fuente de ficheros, para tener una Table que NO es del lago
mkdir -p "$A/packages/ventas/tables" "$A/packages/ventas/views"
cat >> "$A/ontology.config.yaml" <<'Y'
  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR }
Y
cat > "$A/packages/ventas/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: ventas, version: 1.0.0, status: active, domain: sales }
spec: { owner: team:data }
Y
cat > "$A/packages/ventas/tables/pedidos.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: pedidos, namespace: ventas }
spec:
  datasource: ficheros
  object: "pedidos.jsonl"
  columns: { order_id: {}, pais: {} }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$A"; falla "0 · el árbol semilla no compila"; exit 1; }
( cd "$A" && git add -A && git commit -qm "el arbol con su lago" && git push -q origin HEAD:main ) || { falla "0 · no se pudo sembrar la forja"; exit 1; }
ok "0 · el árbol nace con \`datasource: lago\` (ore init) y compila"

# ── el servidor, en modo forja ───────────────────────────────────────────────
PUERTO=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
BASE="http://127.0.0.1:$PUERTO"
FORJA_TOKEN=no-hace-falta "$SERVE" --forja "file://$FORJA" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
  --identidad cabecera --no-es-produccion --organizacion lago >"$TMP/serve.log" 2>&1 &
SRV=$!
for _ in $(seq 1 60); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
SUJ='x-ore-sujeto: persona:ana'
pide() { # metodo ruta [cuerpo] → código; cuerpo en $TMP/out.json
  local m=$1 r=$2 c=${3:-}
  if [ -n "$c" ]; then curl -s -o "$TMP/out.json" -w '%{http_code}' -X "$m" -H "$SUJ" -H 'content-type: application/json' "$BASE$r" -d "$c"
  else curl -s -o "$TMP/out.json" -w '%{http_code}' -X "$m" -H "$SUJ" "$BASE$r"; fi
}
campo() { "$PY" -c 'import json,sys; v=json.load(open(sys.argv[1]));
for k in sys.argv[2].split("."): v = v[int(k)] if k.isdigit() else v[k]
print(v if not isinstance(v,bool) else str(v).lower())' "$TMP/out.json" "$1"; }

# ── lo que un puesto escribiría: una tabla Iceberg en el bucket ───────────────
# `ore-store-r2 sellar` hace aquí de `write()` (W3.6c): datos + metadata.json
# en `datasets/ventas_salida`, y NADIE lo apunta todavía.
escribe() { # base → metadata_location
  local extra='"dataset":"datasets/ventas_salida","fundir":false,'
  [ -n "${1:-}" ] && extra="$extra\"base\":\"$1\","
  { echo "{$extra\"clave\":[],\"conducto\":\"puesto:ana\",\"esquema\":{\"id\":\"Integer\",\"pais\":\"String\",\"total\":\"Decimal\"},\"plan\":\"sha256:escrito\",\"testigo\":{\"modo\":\"snapshot\",\"valor\":\"${2:-1}\"}}"
    echo '{"id":"1","pais":"ES","total":"10.50"}'; echo '{"id":"2","pais":"PT","total":"7.25"}'; } \
    | "$STORE" sellar > "$TMP/sellado.json" 2>"$TMP/sellado.err" || { cat "$TMP/sellado.err" >&2; return 1; }
  "$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$TMP/sellado.json"
}
ML1=$(escribe "" 1); [ -n "$ML1" ] || { falla "el escritor no dejó metadata_location"; exit 1; }

# ── 1 · confirmar: la Table nace con el puntero, en un commit del sujeto ──────
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"$ML1\",\"snapshot\":\"1\",\"filas\":2,\"columnas\":{\"id\":\"Integer\",\"pais\":\"String\",\"total\":\"Decimal\"}}")
[ "$COD" = "201" ] || falla "1 · confirmar no dio 201: $COD $(cat "$TMP/out.json")"
[ "$(campo tabla_nueva)" = "true" ] && [ "$(campo puntero_nuevo)" = "true" ] || falla "1 · no dijo que la Table y el puntero nacen: $(cat "$TMP/out.json")"
[ "$(campo lago_declarado)" = "false" ] || falla "1 · el lago ya estaba declarado y dijo que lo declaró"
COMMIT1=$(campo commit)
[ -n "$COMMIT1" ] || falla "1 · sin commit"
git clone -q "$FORJA" "$TMP/mira1"
[ -f "$TMP/mira1/packages/ventas/tables/salida.yaml" ] || falla "1 · la Table del lago no está en la forja"
grep -q "datasource: lago" "$TMP/mira1/packages/ventas/tables/salida.yaml" || falla "1 · la Table no es del lago"
grep -q "total: { type: Decimal }" "$TMP/mira1/packages/ventas/tables/salida.yaml" || falla "1 · la Table no lleva las columnas tipadas: $(cat "$TMP/mira1/packages/ventas/tables/salida.yaml")"
"$PY" - "$TMP/mira1/datasets/ventas_salida.json" "$ML1" <<'EOF' || falla "1 · el puntero"
import json, sys
p = json.load(open(sys.argv[1]))
assert p["estado"] == "copiada" and p["metadata_location"] == sys.argv[2] and p["dataset"] == "datasets/ventas_salida" and p["tabla"] == "ventas.salida" and p["filas"] == 2 and p["escrito_por"] == "persona:ana", p
EOF
[ "$(git --git-dir="$FORJA" log -1 --format='%an' main)" = "persona:ana" ] || falla "1 · el autor del commit no es el sujeto"
( cd "$TMP/mira1" && "$ORE" validate . >/dev/null 2>&1 ) || falla "1 · el árbol con la Table del lago no compila: $(cd "$TMP/mira1" && "$ORE" validate . 2>&1 | head -3)"
ok "1 · confirmar: la Table del lago (tipada) y el puntero nacen en UN commit firmado por el sujeto, y el árbol compila"

# ── 2 · la lista y la ficha ──────────────────────────────────────────────────
[ "$(pide GET /datasets)" = "200" ] || falla "2 · GET /datasets: $(cat "$TMP/out.json")"
[ "$(campo datasets.0.clase)" = "dataset" ] && [ "$(campo datasets.0.nombre)" = "ventas.salida" ] || falla "2 · la lista no trae ventas.salida como dataset: $(cat "$TMP/out.json")"
[ "$(pide GET /datasets/ventas/salida)" = "200" ] || falla "2 · GET /datasets/ventas/salida: $(cat "$TMP/out.json")"
[ "$(campo snapshots.0.operacion)" = "append" ] && [ "$(campo snapshots.0.filas)" = "2" ] && [ "$(campo snapshots.0.vigente)" = "true" ] || falla "2 · la ficha no trae el snapshot: $(cat "$TMP/out.json")"
[ "$(campo esquema.total)" = "decimal(38, 18)" ] || falla "2 · la ficha no trae el esquema de Iceberg: $(campo esquema.total)"
[ "$(pide GET /datasets/ventas/nadie)" = "404" ] || falla "2 · un dataset que no existe no dio 404"
ok "2 · GET /datasets lista el dataset; GET /datasets/ventas/salida trae sus snapshots y su esquema"

# ── 3 · el CAS semántico ─────────────────────────────────────────────────────
ML2=$(escribe "$ML1" 2)
ANTES=$(git --git-dir="$FORJA" rev-parse main)
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"$ML2\",\"esperado\":\"$ML1\",\"snapshot\":\"2\",\"filas\":2}")
[ "$COD" = "200" ] || falla "3 · con esperado al día no dio 200: $COD $(cat "$TMP/out.json")"
[ "$(campo puntero_nuevo)" = "false" ] && [ "$(campo tabla_nueva)" = "false" ] || falla "3 · el puntero se movió pero dijo que nace"
[ "$(git --git-dir="$FORJA" rev-parse main)" != "$ANTES" ] || falla "3 · mover el puntero no dejó commit"
ANTES=$(git --git-dir="$FORJA" rev-parse main)
ML3=$(escribe "$ML2" 3)
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"$ML3\",\"esperado\":\"$ML1\"}")
[ "$COD" = "409" ] || falla "3 · con esperado viejo no dio 409: $COD $(cat "$TMP/out.json")"
[ "$(campo actual)" = "$ML2" ] || falla "3 · el 409 no dice cuál es el puntero actual: $(cat "$TMP/out.json")"
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"$ML3\"}")
[ "$COD" = "409" ] || falla "3 · sin esperado sobre un puntero que existe no dio 409: $COD"
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"s3://copia/ore/v2/datasets/ventas_salida/metadata/99999-nadie.metadata.json\",\"esperado\":\"$ML2\"}")
[ "$COD" = "422" ] || falla "3 · lo que no está en el bucket no dio 422: $COD $(cat "$TMP/out.json")"
COD=$(pide POST /datasets/ventas/pedidos/confirmar "{\"metadata_location\":\"$ML3\"}")
[ "$COD" = "422" ] || falla "3 · una Table de otra fuente no dio 422: $COD $(cat "$TMP/out.json")"
grep -q "no del lago" "$TMP/out.json" || falla "3 · no dijo que pedidos es de otra fuente: $(cat "$TMP/out.json")"
COD=$(pide POST /datasets/ventas/nueva/confirmar "{\"metadata_location\":\"$ML3\"}")
[ "$COD" = "422" ] || falla "3 · una Table nueva sin columnas no dio 422: $COD"
COD=$(pide POST /datasets/nadie/x/confirmar "{\"metadata_location\":\"$ML3\"}")
[ "$COD" = "422" ] || falla "3 · un paquete que no existe no dio 422: $COD"
[ "$(git --git-dir="$FORJA" rev-parse main)" = "$ANTES" ] || falla "3 · algún rechazo dejó commit"
# y el mismo puntero otra vez: idempotente, sin commit
COD=$(pide POST /datasets/ventas/salida/confirmar "{\"metadata_location\":\"$ML2\",\"esperado\":\"$ML2\"}")
[ "$COD" = "200" ] || falla "3 · confirmar el mismo puntero no dio 200: $COD"
[ "$(git --git-dir="$FORJA" rev-parse main)" = "$ANTES" ] || falla "3 · confirmar lo mismo dejó commit"
ok "3 · el CAS semántico: al día 200 y commit; viejo 409 con \`actual\`; sin esperado 409; no está en el bucket 422; otra fuente 422; sin columnas 422 — y nada de eso deja commit"

# ── 4 · la carrera ───────────────────────────────────────────────────────────
HILOS=""
for i in 1 2 3 4; do
  MLi=$(escribe "$ML2" "c$i")
  ( curl -s -o "$TMP/c$i.json" -w '%{http_code}' -X POST -H "$SUJ" -H 'content-type: application/json' "$BASE/datasets/ventas/salida/confirmar" -d "{\"metadata_location\":\"$MLi\",\"esperado\":\"$ML2\",\"snapshot\":\"c$i\"}" > "$TMP/c$i.cod" ) &
  HILOS="$HILOS $!"
done
# shellcheck disable=SC2086
wait $HILOS
GANAN=0; PIERDEN=0; OTROS=0
for i in 1 2 3 4; do
  case "$(cat "$TMP/c$i.cod")" in 200) GANAN=$((GANAN+1));; 409) PIERDEN=$((PIERDEN+1));; *) OTROS=$((OTROS+1)); echo "    hilo $i: $(cat "$TMP/c$i.cod") $(cat "$TMP/c$i.json")";; esac
done
[ "$GANAN" = 1 ] && [ "$PIERDEN" = 3 ] && [ "$OTROS" = 0 ] || falla "4 · la carrera: $GANAN ganan, $PIERDEN 409, $OTROS otros"
ok "4 · cuatro escritores a la vez sobre el mismo puntero: uno gana, tres reciben 409"

# ── 5 · la historia del puntero ──────────────────────────────────────────────
[ "$(pide GET /arbol/historia/datasets/ventas_salida.json)" = "200" ] || falla "5 · GET /arbol/historia: $(cat "$TMP/out.json")"
N=$("$PY" -c 'import json,sys;print(len(json.load(open(sys.argv[1]))["versiones"]))' "$TMP/out.json")
[ "$N" = "3" ] || falla "5 · el puntero tiene $N versiones y no 3 (nace, se mueve, gana la carrera)"
[ "$(campo versiones.0.autor)" = "persona:ana" ] || falla "5 · la versión no dice quién: $(campo versiones.0.autor)"
ok "5 · git log del puntero = la historia de la tabla: 3 versiones, con quién"

# ── 6 · el mantenimiento, sobre punteros ─────────────────────────────────────
git clone -q "$FORJA" "$TMP/mant"
ANTES_OBJ=$(curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/datasets/ventas_salida/" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' ')
salida=$("$ORE" datasets "$TMP/mant" --recoger --edad 0 --seco 2>&1) || { echo "$salida"; falla "6 · --recoger --seco"; }
case "$salida" in *"en seco"*"expirado(s)"*) ;; *) falla "6 · --seco no dijo qué haría: $salida";; esac
[ "$(curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/datasets/ventas_salida/" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' ')" = "$ANTES_OBJ" ] || falla "6 · --seco tocó el bucket"
MLV=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$TMP/mant/datasets/ventas_salida.json")
salida=$("$ORE" datasets "$TMP/mant" --recoger --edad 0 2>&1) || { echo "$salida"; falla "6 · --recoger"; }
case "$salida" in *"1 dataset"*"1 puntero(s) movido(s)"*) ;; *) falla "6 · recoger no expiró ni movió: $salida";; esac
MLN=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata_location"])' "$TMP/mant/datasets/ventas_salida.json")
[ "$MLN" != "$MLV" ] || falla "6 · el puntero no se movió al metadata.json de expirar"
DESPUES_OBJ=$(curl -s "$ORE_R2_S3_ENDPOINT/copia?list-type=2&prefix=ore/v2/datasets/ventas_salida/" | grep -o '<Key>[^<]*</Key>' | wc -l | tr -d ' ')
[ "$DESPUES_OBJ" -lt "$ANTES_OBJ" ] || falla "6 · recoger no retiró nada ($ANTES_OBJ → $DESPUES_OBJ)"
# lo que quedó legible: la vigente, entera
n=$(printf '{"metadata_location":"%s"}\n' "$MLN" | "$STORE" leer | grep -c '^{"id"'); [ "$n" = "2" ] || falla "6 · tras recoger la vigente no se lee entera ($n)"
# y las escrituras que perdieron la carrera (tres tablas sin puntero... no: tres
# snapshots que nadie apunta en la MISMA tabla) se fueron con los huérfanos
# y el puntero movido se empuja, como hace el CronJob de mantenimiento
( cd "$TMP/mant" && git add -A datasets && git commit -qm "mantenimiento" && git push -q origin HEAD:main ) || falla "6 · el mantenimiento no pudo empujar"
salida=$("$ORE" datasets "$TMP/mant" 2>&1) || falla "6 · ore datasets"
case "$salida" in *"dataset  ventas.salida"*"copiada"*) ;; *) falla "6 · la lista no enseña el dataset: $salida";; esac
ok "6 · ore datasets --recoger: en seco no toca; de verdad expira, retira ($ANTES_OBJ → $DESPUES_OBJ objetos) y mueve el puntero; la vigente sigue entera"

# ══ el verbo escribir sobre el árbol (c2) ════════════════════════════════════
if ! "$PY" -c 'import pyarrow' 2>/dev/null; then
  dice "sin pyarrow: 7–10 (el verbo escribir) se saltan"
  if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 el lago: 0\xe2\x80\x936\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
  exit 0
fi
# La tabla Arrow que un SDK mandaría: pyarrow la escribe por IPC en stdout.
cat > "$TMP/ipc.py" <<'PY'
import sys
import pyarrow as pa
desde, n = int(sys.argv[1]), int(sys.argv[2])
extra = len(sys.argv) > 3
cols = {
    "id": pa.array(range(desde, desde + n), pa.int64()),
    "pais": pa.array(["ES", "PT"][i % 2] for i in range(n)),
    "total": pa.array([(desde + i) * 10 + 0.5 for i in range(n)], pa.float64()).cast(pa.decimal128(18, 2)),
    "cuando": pa.array([1_700_000_000_000_000 + desde + i for i in range(n)], pa.timestamp("us", tz="UTC")),
}
if extra:
    cols["canal"] = pa.array(["web"] * n)
t = pa.table(cols)
w = pa.ipc.new_stream(sys.stdout.buffer, t.schema)
w.write_table(t)
w.close()
PY
# escribe <dataset> <modo> <base> <clave> <desde> <n> [extra] → $TMP/escrito.json
escribe() {
  { printf '{"dataset":"datasets/%s","modo":"%s","base":"%s","operacion":"%s"}\n' "$1" "$2" "$3" "$4"
    "$PY" "$TMP/ipc.py" "$5" "$6" ${7:-}; } | "$STORE" escribir > "$TMP/escrito.json" 2>"$TMP/escrito.err" \
    || { cat "$TMP/escrito.err" >&2; return 1; }
}
jq_() { "$PY" -c 'import json,sys
v=json.load(open(sys.argv[1]))
for k in sys.argv[2].split("."):
    v = v[int(k)] if k.isdigit() else v[k]
print(v if not isinstance(v,bool) else str(v).lower())' "$1" "$2"; }
CL="$TMP/c2"; git clone -q "$FORJA" "$CL"

# ── 7 · la tabla nace del cuerpo de `escribir` ───────────────────────────────
escribe ventas_escrita sobrescribir "" op-1 0 5 || falla "7 · ore-store escribir"
[ "$(jq_ "$TMP/escrito.json" requirements.0.type)" = "assert-create" ] || falla "7 · sin base, el requisito es assert-create"
"$ORE" datasets "$CL" --commit --tabla ventas.escrita --peticion "@$TMP/escrito.json" --sujeto persona:ana --retencion-defecto 7d --json > "$TMP/commit.json" 2>"$TMP/commit.err" \
  || { cat "$TMP/commit.err"; falla "7 · ore datasets --commit"; }
[ "$(jq_ "$TMP/commit.json" tablas.0.tabla_nueva)" = "true" ] || falla "7 · la Table tenía que nacer: $(cat "$TMP/commit.json")"
[ "$(jq_ "$TMP/commit.json" tablas.0.puntero_nuevo)" = "true" ] || falla "7 · el puntero tenía que nacer"
[ "$(jq_ "$TMP/commit.json" tablas.0.filas)" = "5" ] || falla "7 · 5 filas: $(jq_ "$TMP/commit.json" tablas.0.filas)"
ML7=$(jq_ "$TMP/commit.json" tablas.0.metadata_location)
grep -q "cuando: { type: DateTimeTz }" "$CL/packages/ventas/tables/escrita.yaml" || falla "7 · la Table no lleva DateTimeTz: $(cat "$CL/packages/ventas/tables/escrita.yaml")"
grep -q "total: { type: Decimal }" "$CL/packages/ventas/tables/escrita.yaml" || falla "7 · la Table no lleva Decimal"
[ "$(jq_ "$CL/datasets/ventas_escrita.json" operacion)" = "op-1" ] || falla "7 · el puntero no lleva la operación"
[ -n "$(jq_ "$CL/datasets/ventas_escrita.json" uuid)" ] || falla "7 · el puntero no lleva el uuid"
( cd "$CL" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$CL"; falla "7 · el árbol no compila con la Table que nació"; }
( cd "$CL" && git add -A && git commit -qm "escrita nace" && git push -q origin HEAD:main ) || falla "7 · no se pudo empujar"
n=$(printf '{"metadata_location":"%s","dataset":"datasets/ventas_escrita"}\n' "$ML7" | "$STORE" leer | grep -c '^{"'); [ "$n" = "6" ] || falla "7 · leer sin cabecera de copia: $n líneas y no 6"
ok "7 · --commit: la tabla nace del cuerpo de \`escribir\` (assert-create), con su Table tipada desde Iceberg y su puntero; compila y se lee"

# ── 8 · la clave de operación, la base vieja, el esquema que evoluciona ──────
escribe ventas_escrita sobrescribir "$ML7" op-1 0 5 || falla "8 · escribir (repetida)"
"$ORE" datasets "$CL" --commit --tabla ventas.escrita --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>&1 || { cat "$TMP/commit.json"; falla "8 · commit repetido"; }
[ "$(jq_ "$TMP/commit.json" tablas.0.repetida)" = "true" ] || falla "8 · la misma operación tenía que ser \`repetida\`: $(cat "$TMP/commit.json")"
[ "$(jq_ "$CL/datasets/ventas_escrita.json" metadata_location)" = "$ML7" ] || falla "8 · la repetida movió el puntero"
[ -z "$(cd "$CL" && git status --porcelain)" ] || falla "8 · la repetida tocó el árbol: $(cd "$CL" && git status --porcelain)"
escribe ventas_escrita anexar "$ML7" op-2 5 3 || falla "8 · escribir (anexar)"
"$ORE" datasets "$CL" --commit --tabla ventas.escrita --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>&1 || { cat "$TMP/commit.json"; falla "8 · commit anexar"; }
[ "$(jq_ "$TMP/commit.json" tablas.0.filas)" = "8" ] || falla "8 · tras anexar, 8 filas: $(cat "$TMP/commit.json")"
ML8=$(jq_ "$TMP/commit.json" tablas.0.metadata_location)
# la base vieja: alguien escribió mientras tanto → 75 con actual
escribe ventas_escrita anexar "$ML7" op-3 100 1 || falla "8 · escribir (base vieja)"
"$ORE" datasets "$CL" --commit --tabla ventas.escrita --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>/dev/null; c=$?
[ "$c" = "75" ] || falla "8 · la base vieja tenía que ser código 75 y fue $c: $(cat "$TMP/commit.json")"
[ "$(jq_ "$TMP/commit.json" actual.metadata_location)" = "$ML8" ] || falla "8 · el 75 no dice \`actual\`: $(cat "$TMP/commit.json")"
[ "$(jq_ "$CL/datasets/ventas_escrita.json" metadata_location)" = "$ML8" ] || falla "8 · el 75 movió el puntero"
# una columna más: la Table del árbol sigue el esquema
escribe ventas_escrita sobrescribir "$ML8" op-4 0 2 extra || falla "8 · escribir (columna nueva)"
[ "$(jq_ "$TMP/escrito.json" esquema_cambiado)" = "true" ] || falla "8 · escribir no vio el esquema nuevo"
"$ORE" datasets "$CL" --commit --tabla ventas.escrita --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>&1 || { cat "$TMP/commit.json"; falla "8 · commit con columna nueva"; }
[ "$(jq_ "$TMP/commit.json" tablas.0.tabla_regenerada)" = "true" ] || falla "8 · la Table tenía que regenerarse: $(cat "$TMP/commit.json")"
grep -q "canal: { type: String }" "$CL/packages/ventas/tables/escrita.yaml" || falla "8 · la Table no lleva la columna nueva"
( cd "$CL" && "$ORE" validate . >/dev/null 2>&1 ) || { "$ORE" validate "$CL"; falla "8 · el árbol no compila con la Table regenerada"; }
ML8b=$(jq_ "$TMP/commit.json" tablas.0.metadata_location)
"$ORE" datasets "$CL" --ficha ventas.escrita --json > "$TMP/ficha.json" 2>&1 || falla "8 · ficha"
[ "$(jq_ "$TMP/ficha.json" snapshots.0.idempotencia)" = "op-4" ] || falla "8 · la ficha no enseña la clave de operación: $(cat "$TMP/ficha.json")"
[ "$(jq_ "$TMP/ficha.json" retencion.edad_ms)" = "604800000" ] || falla "8 · la ficha no enseña la retención con la que nació (7d): $(jq_ "$TMP/ficha.json" retencion.edad_ms)"
ok "8 · la misma operación no deja snapshot ni toca el árbol; otra anexa; la base vieja es 75 con \`actual\`; una columna nueva regenera la Table; la ficha lo cuenta"

# ── 9 · dos tablas en un commit, y lo que no se escribe ──────────────────────
escribe ventas_escrita anexar "$ML8b" op-5 200 1 || falla "9 · escribir escrita"; cp "$TMP/escrito.json" "$TMP/e1.json"
escribe ventas_otra sobrescribir "" op-6 0 4 || falla "9 · escribir otra"; cp "$TMP/escrito.json" "$TMP/e2.json"
"$PY" -c 'import json,sys
e1=json.load(open(sys.argv[1])); e2=json.load(open(sys.argv[2]))
def cambio(ns,n,e): return {"identifier":{"namespace":[ns],"name":n},"requirements":e["requirements"],"updates":e["updates"]}
json.dump({"table-changes":[cambio("ventas","escrita",e1),cambio("ventas","otra",e2)]}, open(sys.argv[3],"w"))' "$TMP/e1.json" "$TMP/e2.json" "$TMP/tx.json"
"$ORE" datasets "$CL" --commit --peticion "@$TMP/tx.json" --json > "$TMP/commit.json" 2>&1 || { cat "$TMP/commit.json"; falla "9 · commitTransaction"; }
[ "$(jq_ "$TMP/commit.json" tablas.0.filas)" = "3" ] || falla "9 · escrita: 3 filas (2 + 1): $(cat "$TMP/commit.json")"
[ "$(jq_ "$TMP/commit.json" tablas.1.tabla_nueva)" = "true" ] || falla "9 · otra tenía que nacer"
[ -f "$CL/datasets/ventas_otra.json" ] && [ -f "$CL/packages/ventas/tables/otra.yaml" ] || falla "9 · otra no dejó puntero y Table"
( cd "$CL" && git add -A && git commit -qm "dos tablas" && git push -q origin HEAD:main ) || falla "9 · no se pudo empujar"
# una View como destino: se niega, y no queda nada a medias
mkdir -p "$CL/packages/ventas/views"
printf 'apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: vista, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos }\n  fields: { id: order_id }\n' > "$CL/packages/ventas/views/vista.yaml"
escribe ventas_vista sobrescribir "" op-7 0 1 || falla "9 · escribir vista"
"$ORE" datasets "$CL" --commit --tabla ventas.vista --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>/dev/null; c=$?
[ "$c" = "65" ] || falla "9 · escribir una View tenía que ser 65 y fue $c: $(cat "$TMP/commit.json")"
[ ! -f "$CL/datasets/ventas_vista.json" ] || falla "9 · la View dejó puntero"
( cd "$CL" && git add -A && git commit -qm "una vista" && git push -q origin HEAD:main ) || falla "9 · no se pudo empujar la vista"
# una Table de otra fuente: lo mismo
escribe ventas_pedidos sobrescribir "" op-8 0 1 || falla "9 · escribir pedidos"
"$ORE" datasets "$CL" --commit --tabla ventas.pedidos --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>/dev/null; c=$?
[ "$c" = "65" ] || falla "9 · escribir una Table de ficheros tenía que ser 65 y fue $c"
ok "9 · dos tablas en un commit (una nace, otra anexa); una View y una Table de otra fuente se niegan sin dejar nada"

# ── 10 · la retención declarada es la que --recoger obedece ──────────────────
"$ORE" datasets "$CL" --retencion ventas.escrita --edad 0 --json > "$TMP/ret.json" 2>&1 || { cat "$TMP/ret.json"; falla "10 · --retencion"; }
[ "$(jq_ "$TMP/ret.json" edad_ms)" = "0" ] || falla "10 · --retencion no dejó 0"
escribe ventas_libre sobrescribir "" op-9 0 1 || falla "10 · escribir libre"
"$ORE" datasets "$CL" --commit --tabla ventas.libre --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>&1 || { cat "$TMP/commit.json"; falla "10 · commit libre (sin retención)"; }
MLL=$(jq_ "$TMP/commit.json" tablas.0.metadata_location)
escribe ventas_libre anexar "$MLL" op-10 1 1 || falla "10 · escribir libre 2"
"$ORE" datasets "$CL" --commit --tabla ventas.libre --peticion "@$TMP/escrito.json" --json > "$TMP/commit.json" 2>&1 || falla "10 · commit libre 2"
"$ORE" datasets "$CL" --recoger --json > "$TMP/rec.json" 2>&1 || { cat "$TMP/rec.json"; falla "10 · --recoger sin --edad"; }
"$PY" -c 'import json,sys
r=json.load(open(sys.argv[1])); por={d["nombre"]:d for d in r["por_dataset"]}
assert por["ventas.escrita"]["expirados"]>=1, ("escrita (retencion 0) tenia que expirar", por)
assert por["ventas.otra"]["expirados"]==0, ("otra (7d) no tenia que expirar", por)
assert por["ventas.libre"]["expirados"]==0, ("libre (sin retencion) no tenia que expirar", por)
assert por["ventas.escrita"]["movido"] is True, ("el puntero de escrita tenia que moverse", por)' "$TMP/rec.json" || falla "10 · recoger no obedeció la retención de cada tabla: $(cat "$TMP/rec.json")"
ok "10 · --recoger sin --edad obedece la retención de cada tabla: 0 expira, 7d conserva, ninguna no expira"

# ══ el catálogo REST en ore-serve (c3) ═══════════════════════════════════════
if ! "$PY" -c 'import pyiceberg, duckdb' 2>/dev/null; then
  dice "sin pyiceberg/duckdb: 11–13 (el catálogo REST) se saltan"
  if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 el lago: 0\xe2\x80\x9310\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
  exit 0
fi
export ORE_RETENCION=7d
# ── 11 · PyIceberg escribe por el catálogo ───────────────────────────────────
cat > "$TMP/py11.py" <<'PY'
import sys, json
import pyarrow as pa
from pyiceberg.catalog import load_catalog
from pyiceberg.exceptions import CommitFailedException
base = sys.argv[1]
cat = load_catalog("ore", **{"type": "rest", "uri": base, "header.x-ore-sujeto": "persona:ana"})
def datos(desde, n):
    return pa.table({"id": pa.array(range(desde, desde + n), pa.int64()), "pais": pa.array(["ES"] * n),
                     "total": pa.array([i * 1.5 for i in range(n)], pa.float64()).cast(pa.decimal128(18, 2)),
                     "cuando": pa.array([1_700_000_000_000_000 + i for i in range(n)], pa.timestamp("us", tz="UTC"))})
t = cat.create_table(("ventas", "py"), schema=datos(0, 1).schema)
t.append(datos(0, 100))
t.append(datos(100, 100))
n = cat.load_table(("ventas", "py")).scan().to_arrow().num_rows
# dos manos con la misma base: la segunda pierde, refresca y reintenta sola
a = cat.load_table(("ventas", "py")); b = cat.load_table(("ventas", "py"))
a.append(datos(200, 10))
b.append(datos(300, 10))
n2 = cat.load_table(("ventas", "py")).scan().to_arrow().num_rows
snaps = len(cat.load_table(("ventas", "py")).metadata.snapshots)
print(json.dumps({"filas": n, "filas2": n2, "snapshots": snaps, "tablas": [i[1] for i in cat.list_tables("ventas")], "ns": cat.list_namespaces()}))
PY
"$PY" "$TMP/py11.py" "$BASE" > "$TMP/py11.json" 2> "$TMP/py11.err" || { tail -5 "$TMP/py11.err"; falla "11 · PyIceberg contra ore-serve"; }
[ "$(jq_ "$TMP/py11.json" filas)" = "200" ] || falla "11 · PyIceberg leyó $(jq_ "$TMP/py11.json" filas) filas y no 200"
[ "$(jq_ "$TMP/py11.json" filas2)" = "220" ] || falla "11 · tras la carrera tenía que haber 220 filas: $(cat "$TMP/py11.json")"
[ "$(jq_ "$TMP/py11.json" snapshots)" = "4" ] || falla "11 · 4 snapshots: $(cat "$TMP/py11.json")"
grep -q "Commit failed due to a concurrent update, retrying" "$TMP/py11.err" || falla "11 · la segunda mano no vio el 409 ni reintentó: $(tail -3 "$TMP/py11.err")"
[ "$(pide GET /arbol/packages/ventas/tables/py.yaml)" = "200" ] || falla "11 · la Table no está en el árbol"
grep -q '"total: { type: Decimal }' "$TMP/out.json" || grep -q 'total: { type: Decimal }' "$TMP/out.json" || falla "11 · la Table no lleva Decimal: $(cat "$TMP/out.json" | head -c 300)"
[ "$(pide GET /arbol/historia/datasets/ventas_py.json)" = "200" ] || falla "11 · GET /arbol/historia del puntero"
[ "$(campo versiones.0.autor)" = "persona:ana" ] || falla "11 · el commit no es del sujeto: $(campo versiones.0.autor)"
NV=$("$PY" -c 'import json,sys;print(len(json.load(open(sys.argv[1]))["versiones"]))' "$TMP/out.json")
[ "$NV" = "5" ] || falla "11 · el puntero tenía que tener 5 versiones (nace, 2 append, 2 de la carrera) y tiene $NV"
ok "11 · PyIceberg contra ore-serve: la Table y el puntero nacen firmados por el sujeto, 200 + 20 filas, la carrera es 409 y el cliente reintenta solo"

# ── 12 · DuckDB escribe por el catálogo ──────────────────────────────────────
cat > "$TMP/duck12.py" <<'PY'
import sys, json, duckdb
base, s3 = sys.argv[1], sys.argv[2]
con = duckdb.connect()
# en CI no están preinstaladas: se bajan (hay red); en el puesto sí lo están (W3.5b)
con.execute("install iceberg; install httpfs; load iceberg; load httpfs;")
con.execute("create secret s3 (type s3, key_id 'de', secret 'mentira', endpoint '%s', url_style 'path', use_ssl false, region 'auto')" % s3.replace("http://", ""))
con.execute("create secret ice (type iceberg, token 'persona:ana')")
con.execute("attach '' as lago (type iceberg, endpoint '%s', secret ice)" % base)
n0 = con.execute("select count(*) from lago.ventas.py").fetchone()[0]
con.execute("insert into lago.ventas.py select 1000 + i as id, 'PT' as pais, (i * 2.5)::decimal(18,2) as total, (timestamp '2023-11-14 22:13:20' + interval (i) second)::timestamptz as cuando from range(50) r(i)")
n1 = con.execute("select count(*) from lago.ventas.py").fetchone()[0]
con.execute("create table lago.ventas.pato as select i as id, 'PT' as pais from range(7) r(i)")
n2 = con.execute("select count(*) from lago.ventas.pato").fetchone()[0]
print(json.dumps({"antes": n0, "despues": n1, "pato": n2, "tablas": sorted(r[0] for r in con.execute("select table_name from information_schema.tables where table_catalog='lago'").fetchall())}))
PY
"$PY" "$TMP/duck12.py" "$BASE" "$ORE_R2_S3_ENDPOINT" > "$TMP/duck12.json" 2> "$TMP/duck12.err" || { tail -5 "$TMP/duck12.err"; falla "12 · DuckDB contra ore-serve"; }
[ "$(jq_ "$TMP/duck12.json" antes)" = "220" ] || falla "12 · DuckDB tenía que leer 220: $(cat "$TMP/duck12.json")"
[ "$(jq_ "$TMP/duck12.json" despues)" = "270" ] || falla "12 · tras el INSERT, 270: $(cat "$TMP/duck12.json")"
[ "$(jq_ "$TMP/duck12.json" pato)" = "7" ] || falla "12 · CREATE TABLE AS: 7 filas: $(cat "$TMP/duck12.json")"
[ "$(pide GET /arbol/packages/ventas/tables/pato.yaml)" = "200" ] || falla "12 · la Table de DuckDB no está en el árbol"
"$PY" - "$BASE" > "$TMP/py12.json" 2>&1 <<'PY' || { cat "$TMP/py12.json"; falla "12 · PyIceberg lee lo de DuckDB"; }
import sys, json
from pyiceberg.catalog import load_catalog
cat = load_catalog("ore", **{"type": "rest", "uri": sys.argv[1], "header.x-ore-sujeto": "persona:ana"})
print(json.dumps({"py": cat.load_table(("ventas", "py")).scan().to_arrow().num_rows, "pato": cat.load_table(("ventas", "pato")).scan().to_arrow().num_rows}))
PY
[ "$(jq_ "$TMP/py12.json" pato)" = "7" ] && [ "$(jq_ "$TMP/py12.json" py)" = "270" ] || falla "12 · PyIceberg no lee lo de DuckDB: $(cat "$TMP/py12.json")"
ok "12 · DuckDB contra ore-serve: lee lo de PyIceberg, INSERT por transactions/commit, CREATE TABLE AS por stage-create; PyIceberg lee lo de DuckDB"

# ── 13 · la credencial prestada y los errores de la spec ─────────────────────
curl -s -o "$TMP/out.json" -H "$SUJ" -H 'X-Iceberg-Access-Delegation: vended-credentials' "$BASE/v1/namespaces/ventas/tables/py" > /dev/null
[ "$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["config"].get("s3.access-key-id",""))' "$TMP/out.json")" = "de" ] || falla "13 · con delegación, config sin la credencial: $(head -c 200 "$TMP/out.json")"
[ "$(campo storage-credentials.0.prefix)" = "s3://copia/ore/v2/datasets/ventas_py/" ] || falla "13 · el prefijo de la credencial: $(campo storage-credentials.0.prefix)"
[ "$(campo metadata-location)" != "" ] || falla "13 · sin metadata-location"
curl -s -o "$TMP/out.json" -H "$SUJ" "$BASE/v1/namespaces/ventas/tables/py" > /dev/null
[ "$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["config"])' "$TMP/out.json")" = "{}" ] || falla "13 · sin delegación tenía que venir config vacío"
c=$(curl -s -o "$TMP/out.json" -w '%{http_code}' -H "$SUJ" "$BASE/v1/namespaces/ventas/tables/nadie"); [ "$c" = "404" ] || falla "13 · una tabla que no existe: $c"
[ "$(campo error.type)" = "NoSuchTableException" ] || falla "13 · el 404 no es NoSuchTableException: $(cat "$TMP/out.json")"
c=$(curl -s -o "$TMP/out.json" -w '%{http_code}' -X HEAD -I -H "$SUJ" "$BASE/v1/namespaces/ventas/tables/py" | head -c 3); [ "$c" = "204" ] || falla "13 · HEAD de una tabla que existe: $c"
# una View como destino: 400 con la forma de la spec, y nada en el árbol
c=$(pide POST /v1/namespaces/ventas/tables '{"name":"vista","schema":{"type":"struct","fields":[{"id":1,"name":"id","type":"long","required":false}]}}'); [ "$c" = "400" ] || falla "13 · escribir una View tenía que ser 400 y fue $c: $(cat "$TMP/out.json")"
[ "$(campo error.type)" = "BadRequestException" ] || falla "13 · el 400 no lleva la forma de la spec: $(cat "$TMP/out.json")"
# la base vieja: 409 CommitFailedException con `actual`
UUID=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata"]["table-uuid"])' "$TMP/out.json" 2>/dev/null || true)
curl -s -o "$TMP/out.json" -H "$SUJ" "$BASE/v1/namespaces/ventas/tables/py" > /dev/null
UUID=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["metadata"]["table-uuid"])' "$TMP/out.json")
c=$(pide POST /v1/namespaces/ventas/tables/py "{\"requirements\":[{\"type\":\"assert-table-uuid\",\"uuid\":\"$UUID\"},{\"type\":\"assert-ref-snapshot-id\",\"ref\":\"main\",\"snapshot-id\":1}],\"updates\":[{\"action\":\"set-properties\",\"updates\":{\"x\":\"y\"}}]}")
[ "$c" = "409" ] || falla "13 · la base vieja tenía que ser 409 y fue $c: $(cat "$TMP/out.json")"
[ "$(campo error.type)" = "CommitFailedException" ] || falla "13 · el 409 no es CommitFailedException: $(cat "$TMP/out.json")"
[ "$(campo actual.actual.metadata_location)" != "" ] || falla "13 · el 409 no trae actual: $(cat "$TMP/out.json")"
# y un cuerpo sin assert-create sobre una tabla que no existe: 404
c=$(pide POST /v1/namespaces/ventas/tables/nadie '{"requirements":[],"updates":[{"action":"set-properties","updates":{"x":"y"}}]}'); [ "$c" = "404" ] || falla "13 · commit sobre lo que no existe: $c"
ok "13 · la credencial prestada acotada al prefijo (y sin pedirla, nada); 404, 400, 409 con \`actual\` y la forma de error de la spec"

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 el lago: 0\xe2\x80\x9313\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
