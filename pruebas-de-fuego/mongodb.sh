#!/usr/bin/env bash
# MONGODB, DE PUNTA A PUNTA (ADR 0069 M6) — contra un Atlas de verdad, con el usuario de SOLO
# LECTURA, y rompiendo cada protección a propósito para ver que muerde.
#
#   MONGO_ATLAS_URL='mongodb+srv://lector:clave@cluster0.xxxxx.mongodb.net/ore_prueba' \
#   [MONGO_URL='mongodb://localhost:27017/?directConnection=true']   # un replica set LOCAL que escribe
#   [MONGO_CADUCAR=1]                                                 # trunca su oplog (1,1 GB)
#     bash pruebas-de-fuego/mongodb.sh
#
# `ore`, `ore-read-mongodb` y `ore-federation` en el PATH (o junto a `ORE=…`). Python con
# `pymongo`, `pyarrow` y `duckdb`. En Windows, `SSL_CERT_FILE` con un bundle de CA (OpenSSL no
# usa el almacén del sistema).
#
# ── Qué hace que esto sea una prueba y no una demostración ──────────────────────────────
#
# Cada protección se prueba por el lado en que se rompe, no por el que funciona: una URL con
# TLS apagado, una contraseña mala, un operador que el conector no declaró, una columna de tipos
# mezclados que no se puede filtrar como SQL, un tipo declarado que el dato no lleva. Una guarda
# que nunca se ha visto negarse tiene exactamente el mismo aspecto que una que no existe.
#
# Y lo que se compara con el origen lo dice OTRO cliente (`mongodb/origen.py`, pymongo): una
# copia que "casa" porque el conector se compara consigo mismo no demuestra nada.
#
# El usuario es el de la tarjeta «Un usuario que sólo lee»: si algo de aquí necesitara escribir,
# no pasaría, y eso es lo que se quiere saber.
set -u
ORE="${ORE:-ore}"
command -v "$ORE" >/dev/null 2>&1 || { echo "sin \`ore\` (ORE=$ORE)"; exit 2; }
export PATH="$(dirname "$(command -v "$ORE")"):$PATH"
command -v ore-read-mongodb >/dev/null 2>&1 || { echo "sin \`ore-read-mongodb\` junto a \`ore\`"; exit 2; }
: "${MONGO_ATLAS_URL:?falta MONGO_ATLAS_URL (el usuario de solo lectura de un Atlas)}"
for c in python3 python; do "$c" -c 'import pymongo, pyarrow, duckdb' 2>/dev/null && PY=$c && break; done
[ -n "${PY:-}" ] || { echo "falta Python con pymongo, pyarrow y duckdb"; exit 2; }

# `SOLO=4` (o `SOLO=1,4`) corre sólo esas secciones: las protecciones (4) tardan segundos, la copia
# (2) y el replica set local (5), minutos. Sin él, todas.
seccion() { [ -z "${SOLO:-}" ] || [[ ",$SOLO," == *",$1,"* ]]; }

AQUI="$(cd "$(dirname "$0")" && pwd)"
RAIZ="$(cd "$AQUI/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"; [ -n "${FPID:-}" ] && kill "$FPID" 2>/dev/null' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }
# Todo lo que cualquier orden escribe, para buscar en ello la credencial al final.
TODO="$TMP/todo.txt"; : > "$TODO"

# La URL sin base (para leer cualquier base del clúster; CONSERVA el usuario y la contraseña) y
# la contraseña, para buscarla.
SIN_BASE=$(echo "$MONGO_ATLAS_URL" | sed -E 's#^(mongodb(\+srv)?://[^/]+)/.*#\1#')
SECRETO=$(echo "$MONGO_ATLAS_URL" | sed -E 's#^[a-z+]+://[^:]+:([^@]*)@.*#\1#')
[ -n "$SECRETO" ] && [ "$SECRETO" != "$MONGO_ATLAS_URL" ] || { echo "la URL no lleva usuario y contraseña"; exit 2; }
codigo_de() { sed -n 's/.*"codigo":"\([a-z]*\)".*/\1/p' "$1" | head -1; }
leer() { # json  → stdout, stderr a $2
  printf '%s' "$1" | ore-read-mongodb leer 2>"$2"
}
pet() { # url objeto proyeccion-json [extra-json]
  printf '{"url":"%s","objeto":"%s","proyeccion":%s,"formato":"arrow"%s}' "$1" "$2" "$3" "${4:+,$4}"
}

# ═══ 1 · EL ALTA Y EL DESCUBRIMIENTO ═════════════════════════════════════════════════════
if seccion 1; then
echo "── 1 · el alta y el descubrimiento (ore source add + ore discover)"
D="$TMP/descubre"; mkdir -p "$D"
( cd "$D" && "$ORE" init --name m6 . ) >>"$TODO" 2>&1
( cd "$D" && "$ORE" source add --name atlas "$MONGO_ATLAS_URL" ) >"$TMP/alta.txt" 2>&1; cat "$TMP/alta.txt" >>"$TODO"
grep -q "type: mongodb" "$D/ontology.config.yaml" \
  && dice "1 · \`mongodb+srv://\` se da de alta como tipo \`mongodb\` (el \`+srv\` no es otro origen)" \
  || falla "1 · el tipo no es mongodb: $(grep -A3 datasources "$D/ontology.config.yaml")"
grep -q "connectionEnv:" "$D/ontology.config.yaml" && ! grep -qF "$SECRETO" "$D/ontology.config.yaml" \
  && dice "1 · el manifiesto declara DE QUÉ variable sale la credencial, no cuál es" \
  || falla "1 · el manifiesto lleva la contraseña o no dice la variable"
grep -qF "$SECRETO" "$D/.env.local" && dice "1 · la credencial está en un sitio, \`.env.local\`" || falla "1 · la credencial no está en .env.local"
set -a; . "$D/.env.local"; set +a
( cd "$D" && "$ORE" discover --source atlas --out inducido ) >"$TMP/disc.txt" 2>&1; RC=$?; cat "$TMP/disc.txt" >>"$TODO"
[ "$RC" = 0 ] && grep -q "7 tablas" "$TMP/disc.txt" \
  && dice "1 · \`discover\` lee el clúster y propone 7 tablas, todas en DRAFT" \
  || falla "1 · discover rc=$RC: $(tail -5 "$TMP/disc.txt")"
T="$D/inducido/ore_prueba/tables"
grep -q "witness: log" "$T/Polimorfica__polimorfica.yaml" && grep -q "mode: upsert" "$T/Polimorfica__polimorfica.yaml" \
  && dice "1 · la cara de cambios se SONDEÓ: \`upsert\` por \`_id\`, con el \`operationTime\` como testigo" \
  || falla "1 · la cara de cambios: $(grep -A4 changes "$T/Polimorfica__polimorfica.yaml")"
! grep -q "witness: log" "$T/Serie__serie.yaml" \
  && dice "1 · una serie temporal NO declara cambios (no tiene change stream)" \
  || falla "1 · la serie temporal declara cambios que no puede dar"
grep -q '"cliente.direccion.ciudad"' "$T/Anidada__anidada.yaml" && grep -q "cliente_direccion_ciudad" "$D/inducido/ore_prueba/entities/Anidada.yaml" \
  && dice "1 · el anidado se aplana en la entidad y la RUTA FÍSICA se conserva en la tabla" \
  || falla "1 · el anidado no se aplanó como dice la spec"
grep -q "ore_prueba.vacia — ninguna columna tiene tipo" "$TMP/disc.txt" && [ ! -e "$T/Vacia__vacia.yaml" ] \
  && dice "1 · una colección VACÍA no se inventa: queda como decisión, sin entidad" \
  || falla "1 · la colección vacía: $(ls "$T")"
grep -q "ore_prueba.vista_tipos — el origen la declara \`view\`" "$TMP/disc.txt" \
  && dice "1 · una VISTA se pregunta (¿entidad o informe?), no se emite sola" || falla "1 · la vista no se preguntó"
grep -q "tipos mezclados" "$TMP/disc.txt" && dice "1 · los tipos mezclados se DICEN, no se esconden" \
  || { grep -q "mixed" "$D/inducido/ore_prueba/tables/Polimorfica__polimorfica.yaml" && dice "1 · los tipos mezclados quedan como texto (\`mixed\`)" || falla "1 · los tipos mezclados no se dicen"; }

fi

# ═══ 2 · LA COPIA: una vista SQL sobre una colección de Atlas, calculada en DuckDB ═══════
if seccion 2; then
echo "── 2 · la copia (ore materialize --preparar + DuckDB) contra sample_mflix"
A="$TMP/arbol"; mkdir -p "$A/packages/atlas/sample_mflix/tables" "$A/packages/informes/views" "$A/packages/informes/datasets"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: m6, version: 0.1.0 }
datasources:
  - name: atlas
    type: mongodb
    connectionEnv: M6_ATLAS_COPIA_URL
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: m6 }
spec:
  owner: team:m6
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
    federation.read: { oos.maturity: DRAFT }
YAML
cat > "$A/packages/atlas/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: atlas, version: 0.1.0, status: draft, domain: atlas }
spec: { owner: "team:m6", exports: [atlas.sample_mflix.comments] }
YAML
cat > "$A/packages/atlas/sample_mflix/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: sample_mflix, namespace: atlas }
spec: { owner: team:m6 }
YAML
# La tabla como la dejaría `discover`; el tipo de `date` es el que la TABLA declara y el dato no
# lleva (en BSON es un instante en milisegundos): viaja en la petición (`tipos`, ADR 0069 M3).
cat > "$A/packages/atlas/sample_mflix/tables/comments.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: comments, namespace: atlas, schema: sample_mflix }
spec:
  datasource: atlas
  object: "sample_mflix.comments"
  columns:
    _id: { type: String, physicalType: objectId, required: true }
    name: { type: String, physicalType: string }
    email: { type: String, physicalType: string }
    movie_id: { type: String, physicalType: objectId }
    text: { type: String, physicalType: string }
    date: { type: DateTimeTz, physicalType: date }
  reads:
    fullScan: cheap
    predicatePushdown: [eq, neq, in, range, isNull]
  changes:
    key: [_id]
    mode: upsert
    witness: log
YAML
cat > "$A/packages/informes/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: informes, version: 0.1.0, status: draft, domain: informes }
spec: { owner: "team:m6", dependencies: [{ name: atlas, version: "0.1.0" }] }
YAML
cat > "$A/packages/informes/views/comentarios_por_anio.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: comentarios_por_anio, namespace: informes }
spec:
  owner: team:m6
  dialect: duckdb
  sql: |
    SELECT year(c."date" AT TIME ZONE 'UTC') AS anio, count(*) AS comentarios
    FROM atlas.sample_mflix.comments c
    WHERE c."date" >= TIMESTAMPTZ '2010-01-01 00:00:00+00'
    GROUP BY 1
  columns:
    anio: { type: Integer }
    comentarios: { type: Integer }
YAML
cat > "$A/packages/informes/datasets/comentarios_copia.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: comentarios_copia, namespace: informes }
spec:
  owner: team:m6
  from: { view: informes.comentarios_por_anio }
YAML
export M6_ATLAS_COPIA_URL="$SIN_BASE"

"$ORE" validate "$A" >"$TMP/v.txt" 2>&1 && dice "2 · el árbol de MongoDB compila" || falla "2 · no compila: $(tail -6 "$TMP/v.txt")"
DC="$TMP/calc"; mkdir -p "$DC"
( cd "$A" && "$ORE" materialize . --preparar "$DC" --vista informes.comentarios_copia ) >"$TMP/prep.txt" 2>&1; RC=$?; cat "$TMP/prep.txt" >>"$TODO"
C="$DC/informes.comentarios_copia"
[ "$RC" = 0 ] && [ -f "$C/entradas/atlas.sample_mflix.comments.arrow" ] \
  && dice "2 · preparar leyó la colección de Atlas a entradas/" || falla "2 · preparar rc=$RC: $(tail -8 "$TMP/prep.txt")"
grep -q "1 filtros empujados" "$TMP/prep.txt" && dice "2 · el rango de fecha se EMPUJÓ al clúster (no se trae todo para filtrar aquí)" \
  || falla "2 · lo empujado: $(grep leída "$TMP/prep.txt")"
TOTAL=$("$PY" "$AQUI/mongodb/origen.py" cuenta "$SIN_BASE" 1970-01-01T00:00:00Z)
ESPERADAS=$("$PY" "$AQUI/mongodb/origen.py" cuenta "$SIN_BASE" 2010-01-01T00:00:00Z)
LEIDAS=$("$PY" -c 'import pyarrow.ipc as i,sys;print(i.open_stream(sys.argv[1]).read_all().num_rows)' "$C/entradas/atlas.sample_mflix.comments.arrow" 2>&1)
[ "$LEIDAS" = "$ESPERADAS" ] && [ "$LEIDAS" -lt "$TOTAL" ] \
  && dice "2 · el clúster devolvió $LEIDAS de $TOTAL comentarios: filtrados allí, y las mismas que cuenta pymongo" \
  || falla "2 · leídas $LEIDAS, el origen dice $ESPERADAS de $TOTAL"
( cd "$RAIZ/puesto/python" && "$PY" -m ore.calcular "$DC" ) >"$TMP/calc.txt" 2>&1 || falla "2 · calcular: $(tail -5 "$TMP/calc.txt")"
SALE=$("$PY" -c '
import pyarrow.ipc as i, sys
t = i.open_stream(sys.argv[1]).read_all().to_pylist()
print("\n".join("%s|%s" % (r["anio"], r["comentarios"]) for r in sorted(t, key=lambda r: r["anio"])))' "$C/salida.arrow" 2>&1)
ESPERA=$("$PY" "$AQUI/mongodb/origen.py" anios "$SIN_BASE" 2010-01-01T00:00:00Z)
[ "$SALE" = "$ESPERA" ] && dice "2 · la vista calculada casa con el origen, año a año ($(echo "$SALE" | wc -l) años)" \
  || falla "2 · sale «$(echo $SALE | tr '\n' ' ')», el origen dice «$(echo $ESPERA | tr '\n' ' ')»"

fi

# ═══ 3 · LO MISMO, POR LA PASARELA ═══════════════════════════════════════════════════════
if seccion 3; then
echo "── 3 · por la pasarela (ore-federation)"
if command -v ore-federation >/dev/null 2>&1; then
  PF=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
  # El directorio con los conectores SIN extensión, que es como los busca la pasarela.
  mkdir -p "$TMP/conectores"; cp "$(command -v ore-read-mongodb)" "$TMP/conectores/ore-read-mongodb"
  ore-federation --escucha "127.0.0.1:$PF" --conectores "$TMP/conectores" --tipos mongodb >"$TMP/fed.log" 2>&1 &
  FPID=$!
  for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done
  D2="$TMP/calc2"; mkdir -p "$D2"
  ( cd "$A" && ORE_PASARELA="127.0.0.1:$PF" "$ORE" materialize . --preparar "$D2" --vista informes.comentarios_copia ) >"$TMP/prep2.txt" 2>&1; RC=$?; cat "$TMP/prep2.txt" >>"$TODO"
  grep -q "por la pasarela" "$TMP/prep2.txt" && [ "$RC" = 0 ] && dice "3 · preparar leyó POR la pasarela" || falla "3 · rc=$RC: $(tail -6 "$TMP/prep2.txt")"
  F2=$("$PY" -c 'import pyarrow.ipc as i,sys;print(i.open_stream(sys.argv[1]).read_all().num_rows)' "$D2/informes.comentarios_copia/entradas/atlas.sample_mflix.comments.arrow" 2>&1)
  [ "$F2" = "$LEIDAS" ] && dice "3 · las mismas $F2 filas que el conector directo" || falla "3 · $F2 por la pasarela, $LEIDAS directas"
  curl -s "http://127.0.0.1:$PF/v1/origins" | grep -q '"copias":1' && dice "3 · la pasarela la cuenta como una copia" || falla "3 · origins: $(curl -s "http://127.0.0.1:$PF/v1/origins")"
  kill "$FPID" 2>/dev/null; FPID=
else
  echo "  · sin \`ore-federation\` junto a \`ore\`: la pasarela no corre"
fi

fi

# ═══ 4 · LAS PROTECCIONES, ROTAS A PROPÓSITO ═════════════════════════════════════════════
echo "── 4 · cada protección, rota a propósito"
OBJ="ore_prueba.polimorfica"
PROY='{"id":"_id","nombre":"nombre"}'

# 4a · TLS: una URL que lo apaga NO se acepta.
for opcion in tlsInsecure tlsAllowInvalidCertificates tlsAllowInvalidHostnames; do
  MALA="$SIN_BASE/?$opcion=true"
  echo "{\"url\":\"$MALA\"}" | ore-read-mongodb check >"$TMP/tls.json" 2>&1; cat "$TMP/tls.json" >>"$TODO"
  grep -q '"ok":false' "$TMP/tls.json" && grep -q "verificación de TLS" "$TMP/tls.json" \
    && dice "4a · \`$opcion=true\` se rechaza, y dice por qué (\`check\`)" || falla "4a · $opcion pasó: $(cat "$TMP/tls.json")"
done
leer "$(pet "$SIN_BASE/?tlsInsecure=true" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && dice "4a · y \`leer\` con TLS apagado no lee nada (rc=$RC)" || falla "4a · leer con tlsInsecure leyó"

# 4b · la contraseña mala: `credencial`, y la contraseña (buena ni mala) no sale en el error.
MALA_CLAVE=$(echo "$SIN_BASE" | sed -E 's#^(mongodb(\+srv)?://[^:]+:)[^@]*@#\1clave-que-no-es-xyz@#')
# La guarda de la prueba: que la URL mala esté BIEN FORMADA. Si no, «credencial» saldría por una URL
# rota y no por una contraseña que Atlas rechaza, y la prueba pasaría sin probar nada.
case "$MALA_CLAVE" in mongodb+srv://*:clave-que-no-es-xyz@*) ;; *) falla "4b · la URL de la prueba está mal armada: ${MALA_CLAVE%%@*}@…";; esac
echo "{\"url\":\"$MALA_CLAVE\"}" | ore-read-mongodb check >"$TMP/c.json" 2>&1
grep -q '"ok":false' "$TMP/c.json" && ! grep -q "la URL de MongoDB es" "$TMP/c.json" \
  && dice "4b · Atlas rechaza la contraseña (no es una URL rota): \`check\` → $(sed -n 's/.*"porque":"\([^"]\{0,60\}\).*/\1/p' "$TMP/c.json")…" \
  || falla "4b · check con la contraseña mala: $(cat "$TMP/c.json")"
leer "$(pet "$MALA_CLAVE" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null; cat "$TMP/e.txt" >>"$TODO"
[ "$(codigo_de "$TMP/e.txt")" = credencial ] && dice "4b · una contraseña mala es \`credencial\` (no \`conexion\`, no un error genérico)" \
  || falla "4b · código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"
! grep -q "clave-que-no-es-xyz" "$TMP/e.txt" && dice "4b · y el error no repite la contraseña" || falla "4b · el error repite la contraseña"

# 4c · un clúster que no existe: `conexion`, reintentable.
leer "$(pet "mongodb://u:p@no-existe.invalid:27017/?serverSelectionTimeoutMS=1500" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null
[ "$(codigo_de "$TMP/e.txt")" = conexion ] && grep -q '"reintentable":true' "$TMP/e.txt" \
  && dice "4c · un servidor al que no se llega es \`conexion\` y REINTENTABLE" || falla "4c · $(head -c 200 "$TMP/e.txt")"

# 4d · el vocabulario de operadores es cerrado: uno desconocido no se ignora (devolvería de más).
leer "$(pet "$SIN_BASE" "$OBJ" "$PROY" '"filtros":[{"columna":"nombre","operador":"regex","valor":"a"}]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && dice "4d · un operador que no está en el vocabulario es \`operador\`, no un filtro ignorado" \
  || falla "4d · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4e · una columna de tipos mezclados NO se filtra como SQL (un \`eq\` de Mongo buscaría dentro de arrays).
leer "$(pet "$SIN_BASE" "$OBJ" '{"edad":"edad"}' '"filtros":[{"columna":"edad","operador":"eq","valor":"30"}]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && grep -q "no se empuja" "$TMP/e.txt" \
  && dice "4e · filtrar una columna de tipos mezclados se rechaza diciendo por qué, no devuelve «0 filas»" \
  || falla "4e · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4f · un tipo declarado que el dato no lleva: \`esquema\`, nombrando la columna.
leer "$(pet "$SIN_BASE" "$OBJ" '{"nombre":"nombre"}' '"tipos":[["nombre","Integer"]]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = esquema ] && grep -q '`nombre`' "$TMP/e.txt" \
  && dice "4f · la tabla dice Integer y el dato es texto: \`esquema\` con la columna, no una conversión a escondidas" \
  || falla "4f · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4g · la proyección es cerrada: salen EXACTAMENTE las columnas pedidas.
leer "$(pet "$SIN_BASE" "$OBJ" "$PROY")" "$TMP/e.txt" | "$PY" "$AQUI/mongodb/arrow.py" --filas 2 >"$TMP/a.json" 2>&1
"$PY" -c '
import json,sys
a=json.load(open(sys.argv[1]))
nombres=[c["nombre"] for c in a["columnas"]]
sys.exit(0 if nombres==["id","nombre"] and a["filas"]>0 else 1)' "$TMP/a.json" \
  && dice "4g · pedí 2 columnas de un documento con 5 y salieron esas 2, con el nombre del MODELO (no el físico)" \
  || falla "4g · $(cat "$TMP/a.json")"

# 4h · el tiempo se corta EN EL ORIGEN: \`tiempo\`, no un cuelgue.
leer "$(pet "$SIN_BASE" "sample_mflix.comments" '{"t":"text"}' '"timeoutMs":1,"orderBy":[{"columna":"text","direccion":"desc"}]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = tiempo ] && dice "4h · \`timeoutMs: 1\` es \`tiempo\` (maxTimeMS en el servidor)" \
  || falla "4h · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4i · el rol de la credencial NO escribe: es lo que hace que «solo lectura» sea de verdad.
R=$("$PY" "$AQUI/mongodb/origen.py" escribir "$MONGO_ATLAS_URL" 2>&1)
[ "$R" = denegada ] && dice "4i · el usuario de la fuente intenta escribir y Atlas se lo NIEGA" || falla "4i · $R"

# 4j · la posición: ordinal, y legible por un usuario de solo lectura.
echo "{\"url\":\"$SIN_BASE\"}" | ore-read-mongodb testigo >"$TMP/t.json" 2>&1
VALOR=$(sed -n 's/.*"valor":"\([0-9:]*\)".*/\1/p' "$TMP/t.json")
echo "$VALOR" | grep -qE '^[0-9]{10}:[0-9]{10}$' && grep -q '"modo":"log"' "$TMP/t.json" \
  && dice "4j · el testigo es \`log\`, de ancho fijo ($VALOR): comparar el texto es comparar el instante" || falla "4j · $(cat "$TMP/t.json")"
HACE30=$("$PY" -c 'import sys;t,i=sys.argv[1].split(":");print("%010d:%010d"%(int(t)-1800,0))' "$VALOR")
leer "$(pet "$SIN_BASE" "sample_mflix.comments" '{"id":"_id"}' "\"start\":\"$HACE30\",\"end\":\"$VALOR\"")" "$TMP/e.txt" | "$PY" "$AQUI/mongodb/arrow.py" >"$TMP/a.json" 2>&1
grep -q '"_ore_op"' "$TMP/a.json" && dice "4j · un rango por posición abre el change stream con el usuario de solo lectura (\`_ore_op\`, provisional)" \
  || falla "4j · $(head -c 200 "$TMP/e.txt") $(cat "$TMP/a.json")"
leer "$(pet "$SIN_BASE" "sample_mflix.comments" '{"id":"_id"}' "\"start\":\"$HACE30\",\"filtros\":[{\"columna\":\"_id\",\"operador\":\"eq\",\"valor\":\"x\"}]")" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && dice "4j · un changelog no se filtra en el origen (un borrado no trae el documento): se rechaza" \
  || falla "4j · rc=$RC: $(head -c 200 "$TMP/e.txt")"

# 4k · NADA de lo que escribió ninguna orden lleva la contraseña.
! grep -qF "$SECRETO" "$TODO" && dice "4k · ninguna orden de esta prueba escribió la contraseña (alta, descubrimiento, copia, pasarela, errores)" \
  || falla "4k · la contraseña aparece en la salida de alguna orden: $(grep -n -F "$SECRETO" "$TODO" | head -2 | cut -c1-100)"
[ ! -d "${D:-/no/hay}/inducido" ] || ! grep -rqF "$SECRETO" "$D" --exclude=.env.local && dice "4k · y en el árbol que escribió \`discover\` solo está en \`.env.local\`" \
  || falla "4k · la contraseña está en el árbol: $(grep -rlF "$SECRETO" "$D" | head -3)"

# ═══ 5 · UN REPLICA SET LOCAL QUE ESCRIBE: el kit, la pasarela y los cambios de verdad ═══
if seccion 5; then
echo "── 5 · un replica set local (el kit, la pasarela y los cambios)"
if [ -n "${MONGO_URL:-}" ] && command -v ore-kit >/dev/null 2>&1; then
  export MONGO_KIT_URL="$MONGO_URL"
  ore-kit --conector "$(command -v ore-read-mongodb)" --banco mongodb --exige 1,2,3,4,5,6,8,11,12,13 >"$TMP/kit.txt" 2>&1 \
    && dice "5 · el kit de conformidad: $(grep -E 'pasan' "$TMP/kit.txt" | tail -1)" || falla "5 · el kit: $(tail -6 "$TMP/kit.txt")"
  if command -v ore-federation >/dev/null 2>&1; then
    # Los conectores SIN extensión, que es como los busca la pasarela (también si sólo corre esta sección).
    mkdir -p "$TMP/conectores"; cp "$(command -v ore-read-mongodb)" "$TMP/conectores/ore-read-mongodb"
    ore-kit --conector "$TMP/conectores/ore-read-mongodb" --banco mongodb --pasarela "$(command -v ore-federation)" --exige todos >"$TMP/kitp.txt" 2>&1 \
      && dice "5 · el kit POR LA PASARELA: $(grep -E 'pasan' "$TMP/kitp.txt" | tail -1)" || falla "5 · el kit por la pasarela: $(tail -6 "$TMP/kitp.txt")"
  fi
  # Cambios de verdad: alta, actualización y borrado entre dos posiciones.
  "$PY" - "$MONGO_URL" "$AQUI/mongodb/arrow.py" <<'PY' >"$TMP/cambios.txt" 2>&1
import json, subprocess, sys, pymongo
url, arrow = sys.argv[1], sys.argv[2]
c = pymongo.MongoClient(url)
c.drop_database("m6_cambios")
col = c.m6_cambios.c
col.insert_many([{"_id": 1, "n": "a"}, {"_id": 2, "n": "b"}, {"_id": 3, "n": "c"}])
def ord_():
    t = c.admin.command("hello")["operationTime"]; return "%010d:%010d" % (t.time, t.inc)
t0 = ord_()
col.insert_one({"_id": 4, "n": "d"}); col.update_one({"_id": 1}, {"$set": {"n": "a2"}}); col.delete_one({"_id": 2})
t1 = ord_()
pet = {"url": url, "objeto": "m6_cambios.c", "proyeccion": {"id": "_id", "n": "n"}, "start": t0, "end": t1, "formato": "arrow"}
sal = subprocess.run(["ore-read-mongodb", "leer"], input=json.dumps(pet).encode(), capture_output=True)
r = subprocess.run([sys.executable, arrow, "--filas", "10"], input=sal.stdout, capture_output=True)
a = json.loads(r.stdout)
print(sorted((f["id"], f["_ore_op"]) for f in a["primeras"]))
c.drop_database("m6_cambios")
PY
  grep -q "\[(1, 'upsert'), (2, 'delete'), (4, 'upsert')\]" "$TMP/cambios.txt" \
    && dice "5 · un alta, una actualización y un borrado entre dos posiciones salen como \`upsert\`, \`upsert\` y \`delete\`" \
    || falla "5 · los cambios: $(head -c 300 "$TMP/cambios.txt")"
  if [ "${MONGO_CADUCAR:-}" = 1 ]; then
    MONGO_URL="$MONGO_URL" "$PY" "$AQUI/mongodb/caduca-el-oplog.py" >"$TMP/cad.txt" 2>&1
    VIEJA=$(sed -n 's/^T_OLD=//p' "$TMP/cad.txt")
    leer "$(pet "$MONGO_URL" "ore_caduca.semilla" '{"id":"_id"}' "\"start\":\"$VIEJA\"")" "$TMP/e.txt" | wc -c | grep -q '^0$' \
      && grep -q "posición caducada" "$TMP/e.txt" && grep -q '"reintentable":false' "$TMP/e.txt" \
      && dice "5 · con el oplog TRUNCADO de verdad, una posición vieja es «posición caducada», no reintentable, y antes del primer byte" \
      || falla "5 · la caducidad: $(head -c 300 "$TMP/e.txt")"
  else
    echo "  · \`MONGO_CADUCAR=1\` trunca el oplog de MONGO_URL (1,1 GB): sin él, la caducidad real no corre"
  fi
else
  echo "  · sin MONGO_URL (un replica set local) o sin \`ore-kit\`: el kit, la pasarela y los cambios de verdad no corren"
fi

fi

echo
[ "$MAL" = 0 ] && echo "✓ MongoDB de punta a punta (ADR 0069 M6)" || { echo "✗ MongoDB de punta a punta"; exit 1; }
