#!/usr/bin/env bash
# CASSANDRA, DE PUNTA A PUNTA (ADR 0070 M6) — contra un Cassandra de verdad, con un rol de SOLO
# LECTURA, y rompiendo cada protección a propósito para ver que muerde.
#
#   CASSANDRA_ADMIN_URL='cassandra://admin:clave@127.0.0.1:9042/?contacto=solo' \
#   [CASSANDRA_KIT_URL='cassandra://admin:clave@127.0.0.1:9042/kit?contacto=solo']   # el kit de conformidad
#   [CASSANDRA_TLS_ADMIN_URL=… CASSANDRA_TLS_CA=…]   # TLS de verdad: `bash pruebas-de-fuego/cassandra/tls-lab.sh` los da
#     bash pruebas-de-fuego/cassandra.sh
#
# `ore`, `ore-read-cassandra` y `ore-federation` en el PATH (o junto a `ORE=…`); para la sección 5,
# `ore-kit` compilado con `--features ore-conector-kit/cassandra`. Python con `cassandra-driver`,
# `pyarrow` y `duckdb` (y, desde Python 3.12, `pyasyncore pyasynchat`).
#
# La URL de administración SOLO se usa para cargar la semilla (`ore_m6`, `ore_m6_ajeno`) y crear el rol
# `m6_lector`, con `SELECT` sobre `ore_m6` y nada más. Lo demás —el alta, el descubrimiento, la copia, la
# pasarela, los errores— se hace con ESE rol: si algo de aquí necesitara escribir, no pasaría, y eso es lo
# que se quiere saber. Y se comprueba que el keyspace al que NO se le dio nada no sale en ninguna parte.
#
# ── Qué hace que esto sea una prueba y no una demostración ──────────────────────────────
#
# Cada protección se prueba por el lado en que se rompe, no por el que funciona: una URL con la
# verificación de TLS apagada, una contraseña mala, un operador que el conector no declaró, un filtro
# que Cassandra no sabe y tiene que hacer el conector, un tipo declarado que el dato no lleva. Una guarda
# que nunca se ha visto negarse tiene exactamente el mismo aspecto que una que no existe.
#
# Y lo que se compara con el origen lo dice OTRO cliente (`cassandra/origen.py`, el driver de DataStax
# para Python): una copia que "casa" porque el conector se compara consigo mismo no demuestra nada.
set -u
ORE="${ORE:-ore}"
command -v "$ORE" >/dev/null 2>&1 || { echo "sin \`ore\` (ORE=$ORE)"; exit 2; }
export PATH="$(dirname "$(command -v "$ORE")"):$PATH"
command -v ore-read-cassandra >/dev/null 2>&1 || { echo "sin \`ore-read-cassandra\` junto a \`ore\`"; exit 2; }
: "${CASSANDRA_ADMIN_URL:?falta CASSANDRA_ADMIN_URL (un rol que escribe: carga la semilla y crea el de solo lectura)}"
for c in python3 python; do "$c" -c 'import cassandra, pyarrow, duckdb' 2>/dev/null && PY=$c && break; done
[ -n "${PY:-}" ] || { echo "falta Python con cassandra-driver, pyarrow y duckdb"; exit 2; }
export PYTHONIOENCODING=utf-8

# `SOLO=4` (o `SOLO=1,4`) corre sólo esas secciones: las protecciones (4) tardan segundos, la copia (2) y
# el kit (5), minutos. Sin él, todas.
seccion() { [ -z "${SOLO:-}" ] || [[ ",$SOLO," == *",$1,"* ]]; }

AQUI="$(cd "$(dirname "$0")" && pwd)"
RAIZ="$(cd "$AQUI/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"; [ -n "${FPID:-}" ] && kill "$FPID" 2>/dev/null' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }
# Todo lo que cualquier orden escribe, para buscar en ello las credenciales al final.
TODO="$TMP/todo.txt"; : > "$TODO"

# El rol de solo lectura: su contraseña es única (no se parece a nada de lo que ORE escribe).
ROL="m6_lector"
SECRETO="m6-lector-xq7"
# La URL del rol: la de administración con otro usuario. Sin keyspace, para que `discover` recorra los
# que el rol VE y se vea que no ve más.
URL_ADMIN="$CASSANDRA_ADMIN_URL"
SECRETO_ADMIN=$(echo "$URL_ADMIN" | sed -E 's#^[a-z]+://[^:]+:([^@]*)@.*#\1#')
[ -n "$SECRETO_ADMIN" ] && [ "$SECRETO_ADMIN" != "$URL_ADMIN" ] || { echo "la URL de administración no lleva usuario y contraseña"; exit 2; }
URL=$(echo "$URL_ADMIN" | sed -E "s#^(cassandra://)[^@]*@#\1$ROL:$SECRETO@#; s#^(cassandra://[^/]+)/[^?]*#\1/#")
echo "── semilla (ore_m6, ore_m6_ajeno y el rol $ROL)"
"$PY" "$AQUI/cassandra/origen.py" semilla "$URL_ADMIN" "$ROL" "$SECRETO" >>"$TMP/semilla.txt" 2>&1 \
  && dice "0 · semilla lista: 24 000 pedidos, 150 000 lecturas y un keyspace ajeno" \
  || { echo "la semilla no se carga: $(tail -5 "$TMP/semilla.txt")"; exit 2; }

codigo_de() { sed -n 's/.*"codigo":"\([a-z]*\)".*/\1/p' "$1" | head -1; }
leer() { # json  → stdout, stderr a $2
  printf '%s' "$1" | ore-read-cassandra leer 2>"$2"
}
pet() { # url objeto proyeccion-json [extra-json]
  printf '{"url":"%s","objeto":"%s","proyeccion":%s,"formato":"arrow"%s}' "$1" "$2" "$3" "${4:+,$4}"
}
filas() { "$PY" "$AQUI/cassandra/arrow.py" | "$PY" -c 'import json,sys;print(json.load(sys.stdin).get("filas",-1))'; }

# ═══ 1 · EL ALTA Y EL DESCUBRIMIENTO ═════════════════════════════════════════════════════
if seccion 1; then
echo "── 1 · el alta y el descubrimiento (ore source add + ore discover)"
D="$TMP/descubre"; mkdir -p "$D"
( cd "$D" && "$ORE" init --name m6 . ) >>"$TODO" 2>&1
( cd "$D" && "$ORE" source add --name cass "$URL" ) >"$TMP/alta.txt" 2>&1; cat "$TMP/alta.txt" >>"$TODO"
grep -q "type: cassandra" "$D/ontology.config.yaml" \
  && dice "1 · \`cassandra://\` se da de alta como tipo \`cassandra\`" \
  || falla "1 · el tipo no es cassandra: $(grep -A3 datasources "$D/ontology.config.yaml")"
grep -q "connectionEnv:" "$D/ontology.config.yaml" && ! grep -qF "$SECRETO" "$D/ontology.config.yaml" \
  && dice "1 · el manifiesto declara DE QUÉ variable sale la credencial, no cuál es" \
  || falla "1 · el manifiesto lleva la contraseña o no dice la variable"
grep -qF "$SECRETO" "$D/.env.local" && dice "1 · la credencial está en un sitio, \`.env.local\`" || falla "1 · la credencial no está en .env.local"
set -a; . "$D/.env.local"; set +a
( cd "$D" && "$ORE" discover --source cass --out inducido ) >"$TMP/disc.txt" 2>&1; RC=$?; cat "$TMP/disc.txt" >>"$TODO"
[ "$RC" = 0 ] && grep -q "6 tablas" "$TMP/disc.txt" \
  && dice "1 · \`discover\` lee el clúster y propone 6 tablas, todas en DRAFT" \
  || falla "1 · discover rc=$RC: $(tail -5 "$TMP/disc.txt")"
T="$D/inducido/ore_m6/tables"
CAT="$D/inducido/discover.catalog.json"
# El permiso: `system_schema` enseña los nombres de TODOS los keyspaces, y el conector solo cataloga los que el rol lee.
[ ! -d "$D/inducido/ore_m6_ajeno" ] && ! grep -qE "ajeno|secreta" "$CAT" \
  && dice "1 · el keyspace AJENO no sale, aunque \`system_schema\` lo liste: el catálogo filtra por lo que el rol puede leer" \
  || falla "1 · el keyspace ajeno está en el catálogo"
! grep -qE '"name": *"(system|system_)' "$CAT" && dice "1 · ni el sistema de Cassandra (\`system\`, \`system_schema\`…)" || falla "1 · salen keyspaces del sistema"
grep -q "key: \[cliente, fecha, id\]" "$T/Pedidos__pedidos.yaml" \
  && dice "1 · la clave es la de Cassandra, en orden: partición (cliente) y clustering (fecha, id)" \
  || falla "1 · la clave de pedidos: $(grep key "$T/Pedidos__pedidos.yaml")"
grep -q "importe: { type: String, physicalType: decimal }" "$T/Pedidos__pedidos.yaml" \
  && dice "1 · un \`decimal\` (escala libre) llega como texto EXACTO, no como un float" || falla "1 · el decimal: $(grep importe "$T/Pedidos__pedidos.yaml")"
grep -q "bin: { type: Opaque, physicalType: blob }" "$T/Mezcla__mezcla.yaml" && grep -q 'physicalType: "frozen<direccion>"' "$T/Mezcla__mezcla.yaml" \
  && dice "1 · un blob es Opaque, y un tipo del usuario conserva su tipo de CQL (\`frozen<direccion>\`)" || falla "1 · mezcla: $(grep -E 'bin|dir' "$T/Mezcla__mezcla.yaml")"
grep -q "c: { type: Integer, physicalType: counter }" "$T/Contadores__contadores.yaml" \
  && dice "1 · un contador se lee como entero" || falla "1 · contador: $(grep ' c:' "$T/Contadores__contadores.yaml")"
[ "$(grep -l "fullScan: expensive" "$T"/*.yaml | wc -l | tr -d ' ')" = 6 ] \
  && dice "1 · recorrer una tabla de Cassandra entera se declara CARO (\`fullScan: expensive\`), en las seis" || falla "1 · fullScan"
[ "$(grep -l "mode: none" "$T"/*.yaml | wc -l | tr -d ' ')" = 6 ] && ! grep -q "witness: log\|mode: upsert" "$T"/*.yaml \
  && dice "1 · NINGUNA tabla declara cambios: Cassandra no tiene change streams y no se inventan" || falla "1 · cambios: $(grep -h "mode:" "$T"/*.yaml | sort | uniq -c)"

fi

# ═══ 2 · LA COPIA: una vista SQL sobre tablas de Cassandra, calculada en DuckDB ══════════
if seccion 2; then
echo "── 2 · la copia (ore materialize --preparar + DuckDB) contra ore_m6"
A="$TMP/arbol"; mkdir -p "$A/packages/cass/ore_m6/tables" "$A/packages/informes/views" "$A/packages/informes/datasets"
cat > "$A/ontology.config.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: m6, version: 0.1.0 }
datasources:
  - name: cass
    type: cassandra
    connectionEnv: M6_CASS_COPIA_URL
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
cat > "$A/packages/cass/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: cass, version: 0.1.0, status: draft, domain: cass }
spec: { owner: "team:m6", exports: [cass.ore_m6.pedidos, cass.ore_m6.lecturas] }
YAML
cat > "$A/packages/cass/ore_m6/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: ore_m6, namespace: cass }
spec: { owner: team:m6 }
YAML
# Las tablas como las dejaría `discover`, con lo que la TABLA declara y el dato no lleva: un `decimal` de
# Cassandra no tiene precisión (la escala es de cada valor), y la tabla dice `Decimal<12, 2>`. Viaja en la
# petición (`tipos`) y el conector lo escribe EXACTO, o dice que no cabe.
cat > "$A/packages/cass/ore_m6/tables/pedidos.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: pedidos, namespace: cass, schema: ore_m6 }
spec:
  datasource: cass
  object: "ore_m6.pedidos"
  columns:
    cliente: { type: Integer, physicalType: int, required: true }
    fecha: { type: DateTimeTz, physicalType: timestamp, required: true }
    id: { type: String, physicalType: timeuuid, required: true }
    importe: { type: "Decimal<12, 2>", physicalType: decimal }
    estado: { type: String, physicalType: text }
    nota: { type: String, physicalType: text }
  reads:
    fullScan: expensive
    predicatePushdown: [eq, neq, in, range, isNull]
  changes:
    key: [cliente, fecha, id]
    mode: none
    witness: none
YAML
cat > "$A/packages/cass/ore_m6/tables/lecturas.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha22
kind: Table
metadata: { name: lecturas, namespace: cass, schema: ore_m6 }
spec:
  datasource: cass
  object: "ore_m6.lecturas"
  columns:
    id: { type: Integer, physicalType: bigint, required: true }
    sensor: { type: Integer, physicalType: int }
    valor: { type: Float, physicalType: double }
    instante: { type: DateTimeTz, physicalType: timestamp }
  reads:
    fullScan: expensive
    predicatePushdown: [eq, neq, in, range, isNull]
  changes:
    key: [id]
    mode: none
    witness: none
YAML
cat > "$A/packages/informes/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: informes, version: 0.1.0, status: draft, domain: informes }
spec: { owner: "team:m6", dependencies: [{ name: cass, version: "0.1.0" }] }
YAML
cat > "$A/packages/informes/views/pedidos_por_mes.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: pedidos_por_mes, namespace: informes }
spec:
  owner: team:m6
  dialect: duckdb
  sql: |
    SELECT year(p.fecha AT TIME ZONE 'UTC') AS anio, month(p.fecha AT TIME ZONE 'UTC') AS mes,
           CAST(count(*) AS BIGINT) AS pedidos, CAST(sum(p.importe) AS DECIMAL(18, 2)) AS total
    FROM cass.ore_m6.pedidos p
    WHERE p.cliente = 7 AND p.fecha >= TIMESTAMPTZ '2026-01-01 00:00:00+00'
    GROUP BY 1, 2
  columns:
    anio: { type: Integer }
    mes: { type: Integer }
    pedidos: { type: Integer }
    total: { type: "Decimal<18, 2>" }
YAML
cat > "$A/packages/informes/datasets/pedidos_copia.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: pedidos_copia, namespace: informes }
spec:
  owner: team:m6
  from: { view: informes.pedidos_por_mes }
YAML
cat > "$A/packages/informes/views/lecturas_resumen.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha24
kind: View
metadata: { name: lecturas_resumen, namespace: informes }
spec:
  owner: team:m6
  dialect: duckdb
  sql: |
    SELECT CAST(count(*) AS BIGINT) AS filas, CAST(sum(l.sensor) AS BIGINT) AS suma
    FROM cass.ore_m6.lecturas l
  columns:
    filas: { type: Integer }
    suma: { type: Integer }
YAML
cat > "$A/packages/informes/datasets/lecturas_copia.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Dataset
metadata: { name: lecturas_copia, namespace: informes }
spec:
  owner: team:m6
  from: { view: informes.lecturas_resumen }
YAML
export M6_CASS_COPIA_URL="$URL"

"$ORE" validate "$A" >"$TMP/v.txt" 2>&1 && dice "2 · el árbol de Cassandra compila" || falla "2 · no compila: $(tail -6 "$TMP/v.txt")"
rows_de() { "$PY" -c 'import pyarrow.ipc as i,sys;print(i.open_stream(sys.argv[1]).read_all().num_rows)' "$1" 2>&1; }
salida_de() { # dir → una línea por fila, `col|col|…`, ordenadas
  "$PY" -c '
import pyarrow.ipc as i, sys, decimal
t = i.open_stream(sys.argv[1]).read_all().to_pylist()
cols = sys.argv[2:]
def f(v): return "%.2f" % v if isinstance(v, decimal.Decimal) else str(v)
for r in sorted(t, key=lambda r: tuple(r[c] for c in cols)):
    print("|".join(f(r[c]) for c in cols))' "$1" "${@:2}"; }

# 2a · los pedidos de UN cliente desde una fecha: la partición y el rango de clustering se EMPUJAN al servidor.
DC="$TMP/calc"; mkdir -p "$DC"
( cd "$A" && "$ORE" materialize . --preparar "$DC" --vista informes.pedidos_copia ) >"$TMP/prep.txt" 2>&1; RC=$?; cat "$TMP/prep.txt" >>"$TODO"
C="$DC/informes.pedidos_copia"
[ "$RC" = 0 ] && [ -f "$C/entradas/cass.ore_m6.pedidos.arrow" ] \
  && dice "2 · preparar leyó la tabla de Cassandra a entradas/" || falla "2 · preparar rc=$RC: $(tail -8 "$TMP/prep.txt")"
grep -qE "[2-9] filtros empujados" "$TMP/prep.txt" && dice "2 · el cliente y la fecha se EMPUJARON al servidor (no se trae la tabla entera para filtrar aquí)" \
  || falla "2 · lo empujado: $(grep -i "empuj\|leída" "$TMP/prep.txt" | head -3)"
TOTAL=$("$PY" "$AQUI/cassandra/origen.py" cuenta "$URL" 7 2025-01-01T00:00:00Z)
ESPERADAS=$("$PY" "$AQUI/cassandra/origen.py" cuenta "$URL" 7 2026-01-01T00:00:00Z)
LEIDAS=$(rows_de "$C/entradas/cass.ore_m6.pedidos.arrow")
[ "$LEIDAS" = "$ESPERADAS" ] && [ "$LEIDAS" -lt "$TOTAL" ] \
  && dice "2 · el servidor devolvió $LEIDAS de los $TOTAL pedidos del cliente 7: filtrados allí, y los mismos que cuenta el driver de Python" \
  || falla "2 · leídos $LEIDAS, el origen dice $ESPERADAS de $TOTAL"
( cd "$RAIZ/puesto/python" && "$PY" -m ore.calcular "$DC" ) >"$TMP/calc.txt" 2>&1 || falla "2 · calcular: $(tail -5 "$TMP/calc.txt")"
SALE=$(salida_de "$C/salida.arrow" anio mes pedidos total 2>&1)
ESPERA=$("$PY" "$AQUI/cassandra/origen.py" meses "$URL" 7 2026-01-01T00:00:00Z)
[ "$SALE" = "$ESPERA" ] && [ -n "$SALE" ] && dice "2 · la vista calculada casa con el origen, mes a mes, con el IMPORTE EXACTO ($(echo "$SALE" | wc -l | tr -d ' ') meses)" \
  || falla "2 · sale «$(echo $SALE | tr '\n' ' ')», el origen dice «$(echo $ESPERA | tr '\n' ' ')»"

# 2b · una tabla ENTERA (150 000 filas), por rangos de token en paralelo: ni una perdida ni una repetida.
DL="$TMP/calcl"; mkdir -p "$DL"
( cd "$A" && "$ORE" materialize . --preparar "$DL" --vista informes.lecturas_copia ) >"$TMP/prepl.txt" 2>&1; RC=$?; cat "$TMP/prepl.txt" >>"$TODO"
CL="$DL/informes.lecturas_copia"
[ "$RC" = 0 ] && [ -f "$CL/entradas/cass.ore_m6.lecturas.arrow" ] && dice "2 · y la tabla entera, a entradas/" || falla "2 · preparar lecturas rc=$RC: $(tail -6 "$TMP/prepl.txt")"
( cd "$RAIZ/puesto/python" && "$PY" -m ore.calcular "$DL" ) >"$TMP/calcl.txt" 2>&1 || falla "2 · calcular lecturas: $(tail -5 "$TMP/calcl.txt")"
SALEL=$(salida_de "$CL/salida.arrow" filas suma 2>&1)
ESPERAL=$("$PY" "$AQUI/cassandra/origen.py" lecturas "$URL")
[ "$SALEL" = "$ESPERAL" ] && dice "2 · 150 000 filas leídas por rangos de token: ni una menos ni una repetida (${SALEL})" || falla "2 · sale «$SALEL», el origen dice «$ESPERAL»"

fi

# ═══ 3 · LO MISMO, POR LA PASARELA ═══════════════════════════════════════════════════════
if seccion 3 && seccion 2; then
echo "── 3 · por la pasarela (ore-federation)"
if command -v ore-federation >/dev/null 2>&1; then
  PF=$("$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
  # SIN `--tipos`: la pasarela carga lo que hay en la carpeta de conectores.
  ore-federation --escucha "127.0.0.1:$PF" --conectores "$(dirname "$(command -v ore-read-cassandra)")" >"$TMP/fed.log" 2>&1 &
  FPID=$!
  for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done
  CON=$(curl -s "http://127.0.0.1:$PF/v1/connectors")
  echo "$CON" | grep -q "ore-read-cassandra" && dice "3 · la pasarela DESCUBRIÓ el conector de su carpeta, sin \`--tipos\`" || falla "3 · /v1/connectors: $(echo "$CON" | head -c 200)"
  ! echo "$CON" | grep -q "ore-read-jsonl" && dice "3 · y no ofrece por la red el que lee del disco local (\`jsonl\`)" || falla "3 · la pasarela ofrece jsonl"
  D2="$TMP/calc2"; mkdir -p "$D2"
  ( cd "$A" && ORE_PASARELA="127.0.0.1:$PF" "$ORE" materialize . --preparar "$D2" --vista informes.pedidos_copia ) >"$TMP/prep2.txt" 2>&1; RC=$?; cat "$TMP/prep2.txt" >>"$TODO"
  grep -q "por la pasarela" "$TMP/prep2.txt" && [ "$RC" = 0 ] && dice "3 · preparar leyó POR la pasarela" || falla "3 · rc=$RC: $(tail -6 "$TMP/prep2.txt")"
  F2=$(rows_de "$D2/informes.pedidos_copia/entradas/cass.ore_m6.pedidos.arrow")
  [ "$F2" = "$LEIDAS" ] && dice "3 · las mismas $F2 filas que el conector directo" || falla "3 · $F2 por la pasarela, $LEIDAS directas"
  curl -s "http://127.0.0.1:$PF/v1/origins" | grep -q '"copias":1' && dice "3 · la pasarela la cuenta como una copia" || falla "3 · origins: $(curl -s "http://127.0.0.1:$PF/v1/origins" | head -c 300)"
  kill "$FPID" 2>/dev/null; FPID=
else
  echo "  · sin \`ore-federation\` junto a \`ore\`: la pasarela no corre"
fi

fi

# ═══ 4 · LAS PROTECCIONES, ROTAS A PROPÓSITO ═════════════════════════════════════════════
echo "── 4 · cada protección, rota a propósito"
OBJ="ore_m6.pedidos"
PROY='{"id":"cliente","estado":"estado"}'
SIN_CRED=$(echo "$URL" | sed -E 's#^(cassandra://)[^@]*@#\1#')   # la URL sin credencial, para las que no deben llegar a autenticar

# 4a · TLS: una dirección que lo apaga NO se acepta, y no hay forma de dar una CA por fichero.
for opcion in "verify=false" "tlsInsecure=true" "tlsAllowInvalidCertificates=true" "tlsCAFile=/etc/ca.pem"; do
  MALA="${URL}&$opcion"
  echo "{\"url\":\"$MALA\"}" | ore-read-cassandra check >"$TMP/tls.json" 2>&1; cat "$TMP/tls.json" >>"$TODO"
  grep -q '"ok":false' "$TMP/tls.json" && grep -q "no se admite" "$TMP/tls.json" \
    && dice "4a · \`$opcion\` se rechaza, y dice por qué (\`check\`)" || falla "4a · $opcion pasó: $(cat "$TMP/tls.json")"
done
leer "$(pet "${URL}&tlsInsecure=true" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && dice "4a · y \`leer\` con la verificación apagada no lee nada (\`operador\`, rc=$RC)" || falla "4a · leer con tlsInsecure: rc=$RC código «$(codigo_de "$TMP/e.txt")»"

# 4b · la contraseña mala: `credencial`, y la contraseña no sale en el error.
MALA_CLAVE=$(echo "$URL" | sed -E "s#^(cassandra://$ROL:)[^@]*@#\1clave-que-no-es-xyz@#")
# La guarda de la prueba: que la URL mala esté BIEN FORMADA. Si no, «credencial» saldría de una URL rota y
# no de una contraseña que Cassandra rechaza, y la prueba pasaría sin probar nada.
case "$MALA_CLAVE" in cassandra://$ROL:clave-que-no-es-xyz@*) ;; *) falla "4b · la URL de la prueba está mal armada: ${MALA_CLAVE%%@*}@…";; esac
echo "{\"url\":\"$MALA_CLAVE\"}" | ore-read-cassandra check >"$TMP/c.json" 2>&1; cat "$TMP/c.json" >>"$TODO"
grep -q '"ok":false' "$TMP/c.json" && ! grep -q "la URL de Cassandra es" "$TMP/c.json" && grep -q "incorrect" "$TMP/c.json" \
  && dice "4b · Cassandra rechaza la contraseña (no es una URL rota): \`check\` → …$(sed -n 's/.*\(Provided username[^"]\{0,50\}\).*/\1/p' "$TMP/c.json")…" \
  || falla "4b · check con la contraseña mala: $(cat "$TMP/c.json")"
leer "$(pet "$MALA_CLAVE" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null; cat "$TMP/e.txt" >>"$TODO"
[ "$(codigo_de "$TMP/e.txt")" = credencial ] && dice "4b · una contraseña mala es \`credencial\` (no \`conexion\`, no un error genérico)" \
  || falla "4b · código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"
! grep -q "clave-que-no-es-xyz" "$TMP/e.txt" "$TMP/c.json" && dice "4b · y ni el error ni el \`check\` repiten la contraseña" || falla "4b · el error repite la contraseña"
leer "$(pet "$SIN_CRED" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null
[ "$(codigo_de "$TMP/e.txt")" = credencial ] && dice "4b · y entrar SIN credencial en un servidor con autenticación también es \`credencial\`" || falla "4b · sin credencial: «$(codigo_de "$TMP/e.txt")»"

# 4c · un clúster al que no se llega: `conexion`, reintentable.
leer "$(pet "cassandra://u:p@127.0.0.1:1/?contacto=solo" "$OBJ" "$PROY")" "$TMP/e.txt" >/dev/null
[ "$(codigo_de "$TMP/e.txt")" = conexion ] && grep -q '"reintentable":true' "$TMP/e.txt" \
  && dice "4c · un servidor al que no se llega es \`conexion\` y REINTENTABLE" || falla "4c · $(head -c 200 "$TMP/e.txt")"

# 4d · el vocabulario de operadores es cerrado: uno desconocido no se ignora (devolvería de más).
leer "$(pet "$URL" "$OBJ" "$PROY" '"filtros":[{"columna":"estado","operador":"regex","valor":"a"}]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && dice "4d · un operador que no está en el vocabulario es \`operador\`, no un filtro ignorado" \
  || falla "4d · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4e · bytes, colecciones y duraciones NO se filtran como SQL: se dice, no devuelve «0 filas».
for col in bin etiquetas dur; do
  leer "$(pet "$URL" "ore_m6.mezcla" '{"id":"id"}' "\"filtros\":[{\"columna\":\"$col\",\"operador\":\"eq\",\"valor\":\"x\"}]")" "$TMP/e.txt" >/dev/null; RC=$?
  [ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && grep -q "no se compara como SQL" "$TMP/e.txt" \
    && dice "4e · filtrar \`$col\` se rechaza diciendo por qué, no devuelve «0 filas»" || falla "4e · $col: rc=$RC «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"
done

# 4f · un tipo declarado que el dato no lleva: `esquema`, nombrando la columna.
leer "$(pet "$URL" "$OBJ" '{"estado":"estado"}' '"tipos":[["estado","Integer"]]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = esquema ] && grep -q '`estado`' "$TMP/e.txt" \
  && dice "4f · la tabla dice Integer y el dato es texto: \`esquema\` con la columna, no una conversión a escondidas" \
  || falla "4f · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"
leer "$(pet "$URL" "$OBJ" '{"importe":"importe"}' '"tipos":[["importe","Decimal<12, 1>"]]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = esquema ] && grep -q "sin perder cifras" "$TMP/e.txt" \
  && dice "4f · un decimal que perdería cifras en la escala que la tabla declara NO se redondea: \`esquema\`, con el valor" \
  || falla "4f · decimal: rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4g · la proyección es cerrada: salen EXACTAMENTE las columnas pedidas, con el nombre del MODELO.
leer "$(pet "$URL" "$OBJ" '{"c":"cliente","e":"estado"}' '"filtros":[{"columna":"cliente","operador":"eq","valor":"7"}],"limit":3')" "$TMP/e.txt" | "$PY" "$AQUI/cassandra/arrow.py" --filas 3 >"$TMP/a.json" 2>&1
"$PY" -c '
import json,sys
a=json.load(open(sys.argv[1]))
nombres=[c["nombre"] for c in a["columnas"]]
sys.exit(0 if nombres==["c","e"] and a["filas"]==3 and all(r["c"]==7 for r in a["primeras"]) else 1)' "$TMP/a.json" \
  && dice "4g · pedí 2 columnas de una tabla de 6 y salieron esas 2, con el nombre del MODELO (no el físico), y el \`limit\` exacto" \
  || falla "4g · $(cat "$TMP/a.json" | head -c 300)"

# 4h · el tiempo se corta, y es `tiempo`, no un cuelgue.
leer "$(pet "$URL" "ore_m6.lecturas" '{"id":"id"}' '"timeoutMs":1')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = tiempo ] && dice "4h · \`timeoutMs: 1\` sobre 150 000 filas es \`tiempo\` (del cliente: Cassandra no tiene tiempo por consulta)" \
  || falla "4h · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4i · el rol NO escribe: es lo que hace que «solo lectura» sea de verdad.
R=$("$PY" "$AQUI/cassandra/origen.py" escribir "$URL" 2>&1 | tr -d '\r')
[ "$R" = denegada ] && dice "4i · el rol de la fuente intenta escribir y Cassandra se lo NIEGA" || falla "4i · $R"

# 4j · el permiso: un keyspace al que el rol no llega, no se lee, no se cataloga y no se explora.
R=$("$PY" "$AQUI/cassandra/origen.py" ajeno "$URL" 2>&1 | tr -d '\r')
[ "$R" = denegada ] && dice "4j · el rol no lee \`ore_m6_ajeno\` (ni el otro cliente)" || falla "4j · $R"
leer "$(pet "$URL" "ore_m6_ajeno.secreta" '{"id":"id"}')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = credencial ] \
  && dice "4j · y \`leer\` de ahí es \`credencial\`: el permiso lo pone Cassandra, no el conector" || falla "4j · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"
printf '%s' "$URL" | ore-read-cassandra catalogo cass >"$TMP/cat.json" 2>>"$TODO"
! grep -qE "ajeno|secreta" "$TMP/cat.json" && grep -q '"ore_m6.pedidos"' "$TMP/cat.json" \
  && dice "4j · \`catalogo\` lista \`ore_m6\` y NO el ajeno, aunque \`system_schema\` lo enseñe" || falla "4j · el catálogo: $(head -c 200 "$TMP/cat.json")"
echo "{\"url\":\"$URL\"}" | ore-read-cassandra explorar >"$TMP/ex.json" 2>&1
grep -q '"ore_m6"' "$TMP/ex.json" && ! grep -q "ajeno" "$TMP/ex.json" && dice "4j · y \`explorar\` tampoco" || falla "4j · explorar: $(head -c 200 "$TMP/ex.json")"

# 4k · lo que Cassandra NO sabe filtrar lo filtra el conector, con la semántica de SQL, y casa con el otro cliente.
leer "$(pet "$URL" "$OBJ" '{"e":"estado"}' '"filtros":[{"columna":"estado","operador":"neq","valor":"pagado"}]')" "$TMP/e.txt" | filas >"$TMP/n.txt"
N=$(cat "$TMP/n.txt"); E=$("$PY" "$AQUI/cassandra/origen.py" no_pagados "$URL")
[ "$N" = "$E" ] && [ "$N" -gt 0 ] && dice "4k · \`estado <> 'pagado'\` (que Cassandra no sabe ni con ALLOW FILTERING) da $N filas: las mismas que cuenta el driver de Python" \
  || falla "4k · neq: $N por el conector, $E por el driver"
leer "$(pet "$URL" "$OBJ" '{"n":"nota"}' '"filtros":[{"columna":"nota","operador":"isNull"}]')" "$TMP/e.txt" | filas >"$TMP/n.txt"
N=$(cat "$TMP/n.txt"); E=$("$PY" "$AQUI/cassandra/origen.py" sin_nota "$URL")
[ "$N" = "$E" ] && [ "$N" -gt 0 ] && dice "4k · \`nota IS NULL\` (que CQL ni tiene) da $N filas, las mismas que cuenta el driver" || falla "4k · isNull: $N por el conector, $E por el driver"
leer "$(pet "$URL" "$OBJ" '{"n":"nota"}' '"filtros":[{"columna":"nota","operador":"like","valor":"n7-%"}]')" "$TMP/e.txt" | filas >"$TMP/n.txt"
[ "$(cat "$TMP/n.txt")" = 480 ] && dice "4k · \`nota LIKE 'n7-%'\` da 480: las 600 del cliente 7 menos las 120 con la nota nula (un nulo no casa con nada)" || falla "4k · like: $(cat "$TMP/n.txt") filas, se esperaban 480"
leer "$(pet "$URL" "$OBJ" '{"c":"cliente"}' '"filtros":[{"columna":"cliente","operador":"in","valor":["3","5"]}]')" "$TMP/e.txt" | filas >"$TMP/n.txt"
[ "$(cat "$TMP/n.txt")" = 1200 ] && dice "4k · \`cliente IN (3, 5)\` (la clave de partición: una consulta por partición) da 1 200 filas" || falla "4k · in: $(cat "$TMP/n.txt") filas, se esperaban 1200"

# 4l · \`orderBy\`: el servidor solo ordena dentro de una partición; este conector no sabe ordenar, y lo dice.
leer "$(pet "$URL" "$OBJ" "$PROY" '"orderBy":[{"columna":"estado","direccion":"asc"}]')" "$TMP/e.txt" >/dev/null; RC=$?
[ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e.txt")" = operador ] && grep -q "orderBy" "$TMP/e.txt" && dice "4l · \`orderBy\` se rechaza (\`operador\`): no se devuelve un orden que no es el pedido" \
  || falla "4l · rc=$RC código «$(codigo_de "$TMP/e.txt")»: $(head -c 200 "$TMP/e.txt")"

# 4m · la salida es DETERMINISTA: la misma petición, el mismo flujo, aunque cuatro rangos lean a la vez.
H1=$(leer "$(pet "$URL" "$OBJ" '{"c":"cliente","f":"fecha"}')" "$TMP/e.txt" | md5sum | cut -d' ' -f1)
H2=$(leer "$(pet "$URL" "$OBJ" '{"c":"cliente","f":"fecha"}')" "$TMP/e.txt" | md5sum | cut -d' ' -f1)
[ "$H1" = "$H2" ] && [ "$H1" != "d41d8cd98f00b204e9800998ecf8427e" ] && dice "4m · dos lecturas de 24 000 filas por 17 rangos son BYTE A BYTE la misma ($H1)" || falla "4m · dos lecturas distintas: $H1 / $H2"

# 4n · una partición que solo tiene la columna estática no es una fila de la tabla.
leer "$(pet "$URL" "ore_m6.estaticas" '{"p":"p","s":"s","v":"v"}')" "$TMP/e.txt" | filas >"$TMP/n.txt"
[ "$(cat "$TMP/n.txt")" = 2 ] && dice "4n · la partición que solo tiene la estática no sale: 2 filas, no 3 (Cassandra devuelve una con la clave de clustering a null)" || falla "4n · estaticas: $(cat "$TMP/n.txt") filas"

# 4o · un `limit` que no se puede empujar lo pone el conector, y es exacto.
leer "$(pet "$URL" "$OBJ" '{"e":"estado"}' '"filtros":[{"columna":"estado","operador":"neq","valor":"pagado"}],"limit":7')" "$TMP/e.txt" | filas >"$TMP/n.txt"
[ "$(cat "$TMP/n.txt")" = 7 ] && dice "4o · \`limit 7\` con un filtro que filtra el conector da 7, ni una más" || falla "4o · limit: $(cat "$TMP/n.txt")"

# 4p · NADA de lo que escribió ninguna orden lleva las contraseñas.
! grep -qF "$SECRETO" "$TODO" && dice "4p · ninguna orden de esta prueba escribió la contraseña del rol (alta, descubrimiento, copia, pasarela, errores)" \
  || falla "4p · la contraseña aparece en la salida de alguna orden: $(grep -n -F "$SECRETO" "$TODO" | head -2 | cut -c1-100)"
! grep -qF "$SECRETO_ADMIN" "$TODO" "$TMP/semilla.txt" && dice "4p · ni la de administración, que solo cargó la semilla" || falla "4p · la contraseña de administración sale"
[ ! -d "${D:-/no/hay}/inducido" ] || ! grep -rqF "$SECRETO" "$D" --exclude=.env.local && dice "4p · y en el árbol que escribió \`discover\` solo está en \`.env.local\`" \
  || falla "4p · la contraseña está en el árbol: $(grep -rlF "$SECRETO" "$D" | head -3)"

# ═══ 5 · EL KIT DE CONFORMIDAD ═══════════════════════════════════════════════════════════
if seccion 5; then
echo "── 5 · el kit de conformidad (14 casos, directo y por la pasarela)"
if [ -n "${CASSANDRA_KIT_URL:-}" ] && command -v ore-kit >/dev/null 2>&1; then
  export CASSANDRA_KIT_URL
  ore-kit --conector "$(command -v ore-read-cassandra)" --banco cassandra --exige 1,2,3,4,5,6,7,8,11,12,13 >"$TMP/kit.txt" 2>&1 \
    && dice "5 · el kit de conformidad: $(grep -E 'pasan' "$TMP/kit.txt" | tail -1)" || falla "5 · el kit: $(tail -6 "$TMP/kit.txt")"
  if command -v ore-federation >/dev/null 2>&1; then
    ore-kit --conector "$(command -v ore-read-cassandra)" --banco cassandra --pasarela "$(command -v ore-federation)" --exige todos >"$TMP/kitp.txt" 2>&1 \
      && dice "5 · el kit POR LA PASARELA: $(grep -E 'pasan' "$TMP/kitp.txt" | tail -1)" || falla "5 · el kit por la pasarela: $(tail -6 "$TMP/kitp.txt")"
  fi
else
  echo "  · sin CASSANDRA_KIT_URL o sin \`ore-kit\` (con \`--features ore-conector-kit/cassandra\`): el kit no corre"
fi

fi

# ═══ 6 · TLS DE VERDAD ═══════════════════════════════════════════════════════════════════
if seccion 6; then
echo "── 6 · TLS de verdad (un Cassandra con certificado propio: cassandra/tls-lab.sh)"
if [ -n "${CASSANDRA_TLS_ADMIN_URL:-}" ] && [ -n "${CASSANDRA_TLS_CA:-}" ]; then
  URL_TLS_ADMIN="$CASSANDRA_TLS_ADMIN_URL"
  URL_TLS=$(echo "$URL_TLS_ADMIN" | sed -E "s#^(cassandra://)[^@]*@#\1$ROL:$SECRETO@#")
  TODO_TLS="$TMP/todo_tls.txt"; : > "$TODO_TLS"
  con_ca() { SSL_CERT_FILE="$CASSANDRA_TLS_CA" "$@"; }                     # confía en la CA del laboratorio
  con_otra_ca() { SSL_CERT_FILE="${CASSANDRA_TLS_OTRA_CA:-$CASSANDRA_TLS_CA.otra}" "$@"; }   # y en otra que NO firmó el servidor
  leer_tls() { printf '%s' "$1" | con_ca ore-read-cassandra leer 2>"$2"; }
  SSL_CERT_FILE="$CASSANDRA_TLS_CA" "$PY" "$AQUI/cassandra/origen.py" semilla "$URL_TLS_ADMIN" "$ROL" "$SECRETO" >>"$TMP/semilla_tls.txt" 2>&1 \
    && dice "6 · semilla lista en el servidor con TLS (el otro cliente, el de Python, también verifica el certificado)" \
    || { falla "6 · la semilla no se carga por TLS: $(tail -3 "$TMP/semilla_tls.txt")"; }

  # 6a · con la CA de confianza: TLS 1.2+, autenticación y lectura, por IP y por nombre.
  echo "{\"url\":\"$URL_TLS\"}" | con_ca ore-read-cassandra check >"$TMP/t6.json" 2>&1; cat "$TMP/t6.json" >>"$TODO_TLS"
  grep -q '"ok":true' "$TMP/t6.json" && dice "6a · con la CA de confianza, \`check\` entra: TLS, autenticación y una lectura, por la IP" || falla "6a · $(cat "$TMP/t6.json" | head -c 300)"
  echo "{\"url\":\"${URL_TLS/127.0.0.1/localhost}\"}" | con_ca ore-read-cassandra check >"$TMP/t6.json" 2>&1; cat "$TMP/t6.json" >>"$TODO_TLS"
  grep -q '"ok":true' "$TMP/t6.json" && dice "6a · y por el NOMBRE de la URL (\`localhost\`): el certificado lleva las dos cosas" || falla "6a · por nombre: $(cat "$TMP/t6.json" | head -c 300)"

  # 6b · el catálogo y la copia, por TLS, casan con lo que dice el otro cliente (que también verifica).
  printf '%s' "$URL_TLS" | con_ca ore-read-cassandra catalogo cass >"$TMP/cat6.json" 2>>"$TODO_TLS"
  grep -q '"ore_m6.pedidos"' "$TMP/cat6.json" && ! grep -qE "ajeno|secreta" "$TMP/cat6.json" \
    && dice "6b · \`catalogo\` por TLS lista \`ore_m6\` y NO el keyspace ajeno" || falla "6b · catálogo: $(head -c 200 "$TMP/cat6.json")"
  leer_tls "$(pet "$URL_TLS" "ore_m6.pedidos" '{"c":"cliente"}' '"filtros":[{"columna":"cliente","operador":"eq","valor":"7"}]')" "$TMP/e6.txt" | filas >"$TMP/n6.txt"
  N=$(cat "$TMP/n6.txt"); E=$(SSL_CERT_FILE="$CASSANDRA_TLS_CA" "$PY" "$AQUI/cassandra/origen.py" cuenta "$URL_TLS" 7 2025-01-01T00:00:00Z)
  [ "$N" = "$E" ] && [ "$N" = 600 ] && dice "6b · y leer los pedidos del cliente 7 por TLS da $N filas: las mismas que cuenta el driver de Python" || falla "6b · $N por el conector, $E por el driver: $(head -c 200 "$TMP/e6.txt")"

  # 6c · una CA que NO firmó el certificado: no se acepta, se dice por qué, y la credencial no se envía.
  echo "{\"url\":\"$URL_TLS\"}" | con_otra_ca ore-read-cassandra check >"$TMP/t6.json" 2>&1; cat "$TMP/t6.json" >>"$TODO_TLS"
  grep -q '"ok":false' "$TMP/t6.json" && grep -q "no se pudo verificar" "$TMP/t6.json" && grep -q "no se envió la credencial" "$TMP/t6.json" \
    && dice "6c · con otra CA el certificado NO se acepta, dice por qué ($(sed -n 's/.*verificar (\([^)]*\)).*/\1/p' "$TMP/t6.json")) y que no se envió la credencial" \
    || falla "6c · con otra CA: $(head -c 300 "$TMP/t6.json")"
  printf '%s' "$(pet "$URL_TLS" "ore_m6.pedidos" '{"c":"cliente"}')" | con_otra_ca ore-read-cassandra leer >/dev/null 2>"$TMP/e6.txt"; RC=$?; cat "$TMP/e6.txt" >>"$TODO_TLS"
  [ "$RC" != 0 ] && [ "$(codigo_de "$TMP/e6.txt")" = conexion ] && grep -q '"reintentable":false' "$TMP/e6.txt" \
    && dice "6c · y \`leer\` con otra CA es \`conexion\` NO reintentable (un certificado malo no mejora a los tres segundos)" \
    || falla "6c · leer con otra CA: rc=$RC «$(codigo_de "$TMP/e6.txt")»: $(head -c 200 "$TMP/e6.txt")"

  # 6d · un cliente SIN TLS contra un servidor que lo exige: se dice qué probar.
  echo "{\"url\":\"${URL_TLS/&tls=true/}\"}" | con_ca ore-read-cassandra check >"$TMP/t6.json" 2>&1; cat "$TMP/t6.json" >>"$TODO_TLS"
  grep -q '"ok":false' "$TMP/t6.json" && grep -q 'tls=true' "$TMP/t6.json" \
    && dice "6d · sin \`tls=true\` contra un servidor que exige TLS: falla y dice «¿El servidor exige TLS? Prueba con tls=true»" || falla "6d · $(head -c 300 "$TMP/t6.json")"

  # 6e · ni la contraseña del rol ni la de administración salen en nada de esta sección.
  ! grep -qF "$SECRETO" "$TODO_TLS" "$TMP/e6.txt" && ! grep -qF "$(echo "$URL_TLS_ADMIN" | sed -E 's#^[a-z]+://[^:]+:([^@]*)@.*#\1#')x" "$TODO_TLS" \
    && dice "6e · ninguna orden de esta sección escribió la contraseña del rol" || falla "6e · la contraseña sale: $(grep -n -F "$SECRETO" "$TODO_TLS" | head -2 | cut -c1-100)"
else
  echo "  · sin CASSANDRA_TLS_ADMIN_URL y CASSANDRA_TLS_CA (\`bash pruebas-de-fuego/cassandra/tls-lab.sh\` los da): el TLS real no corre"
fi

fi

echo
[ "$MAL" = 0 ] && echo "✓ Cassandra de punta a punta (ADR 0070 M6)" || { echo "✗ Cassandra de punta a punta"; exit 1; }
