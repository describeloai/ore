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
# Necesita `ore`, `ore-serve`, `ore-store-r2` en target/{release,debug}, git y
# python3.
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
salida=$("$ORE" datasets "$TMP/mant" 2>&1) || falla "6 · ore datasets"
case "$salida" in *"dataset  ventas.salida"*"copiada"*) ;; *) falla "6 · la lista no enseña el dataset: $salida";; esac
ok "6 · ore datasets --recoger: en seco no toca; de verdad expira, retira ($ANTES_OBJ → $DESPUES_OBJ objetos) y mueve el puntero; la vigente sigue entera"

if [ "$fallos" = 0 ]; then printf '\xe2\x9c\x93 el lago: 0\xe2\x80\x936\n'; else printf '\xe2\x9c\x97 %s fallos\n' "$fallos"; exit 1; fi
