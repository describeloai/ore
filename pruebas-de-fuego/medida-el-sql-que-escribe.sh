#!/usr/bin/env bash
# MEDIDA · UN `.sql` QUE ESCRIBE, EJECUTADO EN LA SESION (antes de tocar ore-serve).
#
# Decidido: en la sesion, una frase SQL que escribe ESCRIBE de verdad —crea el
# dataset, como `write()` desde Python y como el editor de Databricks—. Hoy una
# celda `sql` va entera a `ore.sql()` (agente.py) y DuckDB la ejecuta en su
# memoria. Esta medida dice, con el stack de verdad (ore-serve, el agente de
# Python, el S3 de mentira y un dataset Iceberg), que pasa hoy y que pasaria por
# el camino que ya existe para el trabajo (`celda_de_sql` → `celda_de_unidad`:
# la frase como un `@transform` de Python que llama a `write()`).
#
#   §1  HOY                 las tres frases en una celda `sql`: que devuelve, que
#                           queda en el lago y en el arbol, y que ve la celda siguiente
#   §2  EL CAMINO DEL TRABAJO, EN LA SESION
#                           la celda que ore-serve ya genera para un trabajo, corrida
#                           en la sesion: los tres modos, el puntero, el Dataset, la
#                           procedencia, repetida, y lo que cuesta
#   §3  LA FRONTERA         lo que una celda de sesion puede traer, por `ore sql`
#                           (el mismo analisis que `celda_de_sql`) y por DuckDB: que
#                           es una unidad, que es de la sesion, y que se perderia
#   §4  LA RAMA             donde escribe una sesion abierta en una rama (del codigo)
#
#   bash pruebas-de-fuego/medida-el-sql-que-escribe.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8941}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""; AGENTE=""; S3_PID=""

limpiar() {
  [ -n "$S3_PID" ] && kill "$S3_PID" 2>/dev/null
  [ -n "$AGENTE" ] && kill "$AGENTE" 2>/dev/null
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$TMP"
}
falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── el servidor ──" >&2; tail -15 "$TMP/arranque.txt" >&2; }
  [ -s "$TMP/agente.txt" ] && { echo "── el agente ──" >&2; tail -15 "$TMP/agente.txt" >&2; }
  limpiar; exit 1
}
trap limpiar EXIT
titulo() { echo; echo "  $1"; echo "  $(echo "$1" | sed 's/./-/g')"; }
buscar() {
  for c in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" "$RAIZ/target/debug/$1" "$RAIZ/target/debug/$1.exe"; do
    [ -x "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}
PY=$(command -v python3 || command -v python) || falla "hace falta python"
"$PY" -c 'import pyarrow, pandas, pyiceberg, duckdb' 2>/dev/null || falla "hacen falta pyarrow, pandas, pyiceberg y duckdb"
SERVE="$(buscar ore-serve)" || falla "no hay ore-serve (cargo build --release --workspace)"
ORE="$(buscar ore)" || falla "no hay ore"
export PYTHONIOENCODING=utf-8
pide() { # <metodo> <ruta> [cuerpo]
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' ${3:+--data-binary "$3"} "$BASE$2"
}
jq_() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1], encoding="utf-8")); print(eval(sys.argv[2]))' "$TMP/r.json" "$1"; }

# ── el arbol: hr con una tabla, una vista, un mantenido y un dataset Iceberg ──
A="$TMP/arbol"
mkdir -p "$A/packages/hr/tables" "$A/packages/hr/views" "$A/packages/hr/datasets" "$A/datasets"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: erp, type: jsonl, connectionEnv: ERP_URL }
Y
cat > "$A/packages/hr/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: hr, version: 1.0.0, status: active, domain: people }
spec: { owner: team:hr }
Y
cat > "$A/conduits.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:security
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
Y
cat > "$A/packages/hr/tables/empleados_t.yaml" <<'Y'
apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: empleados_t, namespace: hr }
spec:
  datasource: erp
  object: "empleados.jsonl"
  columns:
    id: {}
    pais: {}
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
Y
cat > "$A/packages/hr/views/empleados.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: empleados, namespace: hr, labels: { oos.maturity: DRAFT } }
spec:
  owner: team:data
  from: { table: hr.empleados_t }
  fields: { id: id, pais: pais }
Y
cat > "$A/packages/hr/datasets/espanoles.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: espanoles, namespace: hr }
spec:
  owner: team:data
  from: { view: hr.empleados }
  where: { pais: ES }
  fields: { id: id }
Y
cat > "$A/packages/hr/datasets/lago.yaml" <<'Y'
apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: lago, namespace: hr }
spec:
  owner: team:data
  from: { view: hr.empleados }
  fields: { id: id }
Y
ALMACEN="$TMP/almacen"; mkdir -p "$ALMACEN"
ALMACEN_PY="$ALMACEN"; TMP_PY="$TMP"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) ALMACEN_PY="$(cd "$ALMACEN" && pwd -W)"; TMP_PY="$(cd "$TMP" && pwd -W)";; esac
META=$("$PY" - "$ALMACEN_PY" "$RAIZ" <<'EOF'
import decimal, importlib.util, os, sys, tempfile
import pyarrow as pa
sp = importlib.util.spec_from_file_location("ice", os.path.join(sys.argv[2], "pruebas-de-fuego", "medida-w3-iceberg.py"))
ice = importlib.util.module_from_spec(sp); sp.loader.exec_module(ice)
Arbol = ice.catalogo_arbol()
cat = Arbol("prueba", tempfile.mkdtemp(prefix="ore-lago-").replace("\\", "/"), warehouse=os.path.join(sys.argv[1], "lago").replace("\\", "/"))
cat.create_namespace("hr")
t = pa.table({"n": pa.array([1, 2, 3, 4], pa.int64()), "letra": pa.array(["a", "b", "a", "b"], pa.string()),
              "importe": pa.array([decimal.Decimal("1.50"), decimal.Decimal("2.25"), decimal.Decimal("3.00"), None], pa.decimal128(10, 2))})
tb = cat.create_table("hr.lago", t.schema); tb.append(t)
print(tb.metadata_location)
EOF
)
[ -n "$META" ] || falla "pyiceberg no escribio hr.lago"
"$PY" -c 'import json,sys; json.dump({"estado":"copiada","metadata_location":sys.argv[1],"snapshot":"1","plan":"x","filas":"4"}, open(sys.argv[2],"w"))' "$META" "$A/datasets/hr_lago.json"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol no compila"

# ── la cola, el S3 de mentira, ore-serve y el agente (como el-puesto.sh) ──────
COLA="$TMP/cola.git"
git init -q --bare -b main "$COLA"
mkdir -p "$TMP/semilla" && ( cd "$TMP/semilla" && git init -q -b main && git config core.autocrlf false )
"$PY" "$RAIZ/malla/gen-inquilino.py" demo --a "$TMP/rendido" >/dev/null 2>&1 || falla "gen-inquilino"
cp "$TMP/rendido/plantilla-puesto.txt" "$TMP/rendido/plantilla-capa.txt" "$TMP/rendido/plantilla-capa-jvm.txt" "$TMP/semilla/"
( cd "$TMP/semilla" && git add -A && git -c user.name=m -c user.email=m@invalido commit -q -m p && git remote add origin "$COLA" && git push -q origin HEAD:main ) || falla "la cola"
COLA_URL="file://$COLA"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) COLA_URL="file:///$(cd "$COLA" && pwd -W)";; esac
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & S3_PID=$!
for _ in $(seq 1 50); do grep -q listo "$TMP/s3.log" 2>/dev/null && break; sleep 0.2; done
S3_PUERTO=$(awk '{print $2}' "$TMP/s3.log"); [ -n "$S3_PUERTO" ] || falla "el S3 de mentira"
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT="http://127.0.0.1:$S3_PUERTO" ORE_R2_BUCKET=copia \
       ORE_R2_ACCESS_KEY_ID=de ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL="s3://copia" ORE_RETENCION=7d
export ORE_STORE_DIR="$(dirname "$ORE")"; export PATH="$ORE_STORE_DIR:$PATH"
FORJA_TOKEN=no-hace-falta "$SERVE" --repo "$A" --ore "$ORE" --bind "127.0.0.1:$PUERTO" --cola "$COLA_URL" \
  --identidad cabecera --no-es-produccion --organizacion demo >"$TMP/arranque.txt" 2>&1 & SRV=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
[ "$(pide POST /puestos '{}')" = "201" ] || falla "abrir el puesto: $(cat "$TMP/r.json")"
P=puesto-ana-python
ORE_SERVE="$BASE" PUESTO=$P ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 TRABAJO_DIR="$TMP_PY" \
  "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/agente.txt" 2>&1 & AGENTE=$!
for _ in $(seq 1 40); do pide GET /puestos/$P >/dev/null; [ "$(jq_ "d['estado']")" = "vivo" ] && break; sleep 0.25; done
[ "$(jq_ "d['estado']")" = "vivo" ] || falla "el puesto no pasa a vivo"

# celda <lenguaje> <texto> → r.json con la celda hecha; MS = lo que tardo (de la salida)
celda() {
  local cuerpo n
  cuerpo=$("$PY" -c 'import json,sys; print(json.dumps({"texto": sys.argv[2], "lenguaje": sys.argv[1]}))' "$1" "$2")
  [ "$(pide POST /puestos/$P/ejecutar "$cuerpo")" = "202" ] || { echo "     ejecutar: $(cat "$TMP/r.json")"; return 1; }
  n=$(jq_ "d['celda']")
  for _ in $(seq 1 5); do pide GET "/puestos/$P/celdas/$n" >/dev/null; [ "$(jq_ "d['estado']")" = "hecha" ] && return 0; done
  return 1
}
resumen() { # la salida de la celda, en una linea
  jq_ "(lambda s: s['tipo'] + ' · ' + (('%s · %s' % ([c['name'] for c in s['columnas']], s['filas'][:4])) if s['tipo']=='tabla' else ((s.get('nombre','') + ': ' + s.get('mensaje','')) if s['tipo']=='error' else repr(s.get('texto','')))) + ' · %s ms' % s.get('ms'))(d['salida'])"
}
en_lago() { # <tabla>: que dice /v1, el puntero y el documento
  local c1 p y
  c1=$(pide GET "/v1/namespaces/hr/tables/$1")
  [ -f "$A/datasets/hr_$1.json" ] && p="puntero si" || p="puntero no"
  [ -f "$A/packages/hr/datasets/$1.yaml" ] && y="Dataset si" || y="Dataset no"
  echo "/v1 loadTable $c1 · $p · $y"
}

titulo "§1 HOY: las tres frases en una celda \`sql\` de la sesion"
celda sql 'create or replace table hr.resumen as select letra, sum(importe) as total from hr.lago group by letra'
echo "     create or replace table hr.resumen as select … from hr.lago"
echo "       salida:   $(resumen)"
echo "       el lago:  $(en_lago resumen)"
celda sql 'select * from hr.resumen order by letra'
echo "     la celda siguiente, sql  select * from hr.resumen"
echo "       salida:   $(resumen)"
celda python 'over("hr.resumen")'
echo "     la celda siguiente, python  over(\"hr.resumen\")"
echo "       salida:   $(resumen)"
celda sql 'insert into hr.resumen select letra, 0 from hr.lago'
echo "     insert into hr.resumen select …"
echo "       salida:   $(resumen)"
celda sql 'insert or replace into hr.resumen select letra, 9 from hr.lago'
echo "     insert or replace into hr.resumen select …"
echo "       salida:   $(resumen)"
celda sql 'create or replace table hr.nuevo as select 1 as x'
echo "     create or replace table hr.nuevo as select 1 as x   (sin leer nada)"
echo "       salida:   $(resumen)"
echo "       el lago:  $(en_lago nuevo)"
celda sql 'create schema tmp; create table tmp.t as select 1 as x; select count(*) as n from tmp.t'
echo "     create schema tmp; create table tmp.t …; select …   (un esquema de la sesion: el caso 7)"
echo "       salida:   $(resumen)"

titulo "§2 EL CAMINO DEL TRABAJO, EN LA SESION (la celda de \`celda_de_unidad\`)"
# La celda, tal cual la escribe `celda_de_unidad` (puestos.rs) para `consulta.sql`
# (en la sesion no hay fichero: el nombre del transform es `consulta`).
generada() { # <destino> <consulta> <modo>
  "$PY" - "$1" "$2" "$3" <<'EOF'
import json, sys
d, q, m = sys.argv[1:4]
print("from ore import transform, sql, write\n\n\n@transform(inputs=%s, output=%s)\ndef consulta():\n    return write(%s, sql(%s, como=\"arrow\"), modo=%s)\n\n\nprint(\"filas\", consulta()[\"filas\"])\n"
      % (json.dumps(["hr.lago"]), json.dumps(d), json.dumps(d), json.dumps(q), json.dumps(m)))
EOF
}
Q='select letra, sum(importe) as total from hr.lago group by letra'
celda python "$(generada hr.resumen2 "$Q" sobrescribir)"
echo "     sobrescribir  hr.resumen2   $(resumen)"
echo "       el lago:  $(en_lago resumen2)"
[ -f "$A/datasets/hr_resumen2.json" ] && echo "       procedencia: $("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["procedencia"])' "$A/datasets/hr_resumen2.json")"
[ -f "$A/packages/hr/datasets/resumen2.yaml" ] && echo "       el Dataset nace: $(tr '\n' ' ' < "$A/packages/hr/datasets/resumen2.yaml" | sed 's/  */ /g' | cut -c1-200)"
celda sql 'select * from hr.resumen2 order by letra'
echo "       lo escrito: $(resumen)"
celda sql "$Q"
echo "       la frase, leida sin escribir: $(resumen)"
celda python 'import pyarrow as pa; t = sql("select letra, sum(importe) as total from hr.lago group by letra", como="arrow"); print(t.schema)'
echo "       el tipo que write() recibe: $(resumen)"
celda python "$(generada hr.resumen2 "$Q" sobrescribir)"
echo "     la misma otra vez  $(resumen)"
pide GET /datasets/hr/resumen2 >/dev/null; echo "       snapshots: $(jq_ "len(d['snapshots'])")"
celda python "$(generada hr.resumen2 'select letra, 0.5 as total from hr.lago' anexar)"
echo "     anexar        hr.resumen2   $(resumen)"
celda sql 'select * from hr.resumen2 order by letra, total'
echo "       lo que hay: $(resumen)"
Q3='select letra, sum(importe) as total from hr.lago group by letra order by letra'
celda python "$(generada hr.orden "$Q3" sobrescribir)"; celda python "$(generada hr.orden "$Q3" sobrescribir)"
pide GET /datasets/hr/orden >/dev/null; echo "     con order by, dos veces: snapshots $(jq_ "len(d['snapshots'])")"
celda python "$(generada hr.resumen2 'select letra, 7.0 as total from hr.lago' upsert)"
echo "     upsert (sin clave: la frase no la dice)   $(resumen)"
celda sql 'select count(*) as n, sum(total) as s from hr.resumen2'
echo "     y la celda sql que lo lee   $(resumen)"
celda python "$(generada hr.espanoles "$Q" sobrescribir)"
echo "     a un Dataset mantenido      $(resumen)"
celda python "$(generada hr.empleados "$Q" sobrescribir)"
echo "     a una View                  $(resumen)"
# coste: la frase en sql (DuckDB en memoria) frente a la celda del trabajo (lago)
for i in 1 2 3; do celda sql "create or replace table hr.coste as $Q"; A1="${A1:-} $(jq_ "d['salida']['ms']")"; done
for i in 1 2 3; do celda python "$(generada hr.coste$i "$Q" sobrescribir)"; A2="${A2:-} $(jq_ "d['salida']['ms']")"; done
echo "     coste (ms, tres veces): hoy, DuckDB en memoria:$A1 · al lago (dataset nuevo):$A2"

titulo "§3 LA FRONTERA: lo que una celda de sesion puede traer"
echo "     ore sql = el analisis de \`celda_de_sql\` (sqlparser + cotejo con el arbol);"
echo "     duckdb = como la parte DuckDB; ¿paquete? = el destino es un paquete del arbol"
i=0
while IFS= read -r q; do
  [ -z "$q" ] && continue
  i=$((i+1)); f="$TMP/c$i.sql"; printf '%s\n' "$q" > "$f"
  FP="$f"; case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) FP="$(cd "$TMP" && pwd -W)/c$i.sql"; A_W="$(cd "$A" && pwd -W)";; *) A_W="$A";; esac
  "$ORE" sql --json --arbol "$A_W" "$FP" > "$TMP/s.json" 2>/dev/null
  "$PY" - "$TMP/s.json" "$q" <<'EOF'
import json, sys, duckdb
try:
    s = json.load(open(sys.argv[1], encoding="utf-8"))
except Exception:
    s = {"fallos": [{"mensaje": "(ore sql no dio JSON)"}]}
q = sys.argv[2]
try:
    tipos = ",".join(x.type.name for x in duckdb.extract_statements(q))
except Exception as e:
    tipos = "NO PARSEA"
if s.get("fallos"):
    o = "✗ " + s["fallos"][0]["mensaje"][:70]
elif s.get("escribe"):
    o = "ESCRIBE %s (%s)" % (s["escribe"]["ref"], s["escribe"]["modo"])
else:
    o = "lee (analisis)"
print("     %-62s %-12s %s" % (q[:62], tipos[:12], o))
EOF
done <<'CASOS'
create or replace table hr.x as select letra from hr.lago
insert into hr.x select letra from hr.lago
insert or replace into hr.x select letra from hr.lago
create table hr.x as select letra from hr.lago
create or replace table hr.lago as select * from hr.lago where n > 1
create or replace table hr.espanoles as select 1 as id
create or replace table hr.empleados as select 1 as id
create or replace table nada.x as select 1 as a
create or replace table x as select 1 as a
create temp table t as select * from hr.lago
create table tmp.t as select 1 as x
create schema tmp; create table tmp.t as select 1 as x
create or replace table hr.x as select * from hr.lago; select * from hr.x
create or replace view hr.v as select * from hr.lago
insert into hr.x by name select letra from hr.lago
create or replace table hr.x as pivot hr.lago on letra using sum(importe)
create or replace table hr.x as select * from hr.lago using sample 10%
delete from hr.lago where n = 1
update hr.lago set letra = 'z'
copy (select * from hr.lago) to 'f.parquet'
with a as (select * from hr.lago) select * from a
select * from hr.lago
CASOS

titulo "§4 LA RAMA (del codigo)"
grep -n "una sesión en una rama escribe en la suya" "$RAIZ/crates/ore-serve/src/catalogo.rs" | sed 's/^/     catalogo.rs:/'
grep -n "x-ore-rama" "$RAIZ/puesto/python/ore/__init__.py" | head -3 | sed 's/^/     ore\/__init__.py:/'
echo "     (aqui el arbol es un directorio: sin ramas; la rama del puesto la pone ore-serve por x-ore-puesto)"

titulo "§5 LO QUE DICE (2026-09-24)"
cat <<'FIN'
     HOY ENGAÑA DOS VECES: `create or replace table hr.x as …` en una celda sql da
       `Count` (parece un exito), no deja nada en el lago ni en el arbol, y la celda
       siguiente NI SIQUIERA lo ve (`LookupError`: ore resuelve `hr.x` contra el arbol
       antes que DuckDB). `insert into` cuenta filas que no van a ningun sitio;
       `insert or replace` es un Binder Error de DuckDB que no dice nada de ORE.
     EL CAMINO YA EXISTE: la celda de `celda_de_unidad`, corrida en la sesion, escribe
       el dataset (puntero, Dataset tipado, procedencia {inputs, transform, puesto}),
       niega el mantenido y la View con su porque, y la celda sql siguiente lo lee.
       ~300 ms frente a ~80 en memoria: es lo que cuesta escribir de verdad.
     LA FRONTERA ES EL DESTINO: si lo que se crea o inserta es `paquete.nombre` de un
       paquete del arbol, es del lago (y se analiza como una unidad: sus errores, 422
       con posicion); si no (`tmp.t`, `x`, una temp), es de DuckDB, como hoy (caso 7).
       Lo que sqlparser no analiza (BY NAME, PIVOT, USING SAMPLE) ya es 422 en un
       trabajo: en la sesion tambien, en vez de perderse en memoria.
     ⛔ ANEXAR CON OTRO DECIMAL BORRA LO QUE HABIA: `0.5 as total` (decimal(2,1)) sobre
       una columna decimal(38,2) deja las filas anteriores en NULL (6 filas, suma 2.0).
       Es de `write(modo="anexar")`, tambien desde Python: arreglarlo antes.
     ⛔ LA MISMA FRASE, DOS SNAPSHOTS: sin `order by`, un `group by` sale en otro orden y
       la clave de operacion (del contenido, en orden) cambia; con `order by`, uno.
     `insert or replace` (upsert) necesita una clave que la frase no puede decir: sobre
       un dataset nuevo es error; sobre uno que la declara, vale.
     LA RAMA: una sesion en una rama escribe en la suya (catalogo.rs, §11 ⑦).
FIN
echo
