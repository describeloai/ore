#!/usr/bin/env bash
# MEDIDA · LA SEMILLA DE `transforms-sql` (antes de cambiarla).
#
# Una instancia de `transforms-sql` nace con `transforms/ejemplo.py` y un
# `pyproject.toml` (clases.rs): la consulta dentro de una cadena de Python,
# porque cuando se sembro "una celda SQL a secas lee pero no escribe" (0036).
# Eso ya no es verdad: un `.sql` es la unidad (`celda_de_sql`), corre como
# trabajo y, desde 6d451da, escribe desde la sesion. Queremos una semilla
# SIMPLE y FUNCIONAL: esta medida dice que hace la de hoy y que haria cada
# candidata, con el stack de verdad (ore-serve, el agente, el S3 de mentira).
#
#   §1  HOY              POST /repositorios con la plantilla: que siembra, y si
#                        corre (en la sesion y como trabajo) tal como nace
#   §2  LAS CANDIDATAS   un `.sql` en `transforms/`: `ore sql` (el analisis del
#                        arbol), la sesion (con `fichero`) y el trabajo
#   §3  EL PAQUETE       el nombre del paquete de un proyecto lleva guion: como
#                        se escribe en SQL
#   §4  LO QUE MUEVE     subir la version de la clase, y quien lee la semilla
#
#   bash pruebas-de-fuego/medida-la-semilla-sql.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8951}"
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
"$PY" -c 'import pyarrow, duckdb' 2>/dev/null || falla "hacen falta pyarrow y duckdb"
SERVE="$(buscar ore-serve)" || falla "no hay ore-serve (cargo build --release --workspace)"
ORE="$(buscar ore)" || falla "no hay ore"
export PYTHONIOENCODING=utf-8
pide() { # <metodo> <ruta> [cuerpo]
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H 'x-ore-sujeto: persona:ana' -H 'content-type: application/json' ${3:+--data-binary "$3"} "$BASE$2"
}
jq_() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1], encoding="utf-8")); print(eval(sys.argv[2]))' "$TMP/r.json" "$1"; }

# ── el arbol: el paquete de un proyecto, con guion, como los que hace la consola ──
A="$TMP/arbol"
mkdir -p "$A/packages/test-project" "$A/packages/hr" "$A/datasets"
cat > "$A/ontology.config.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
Y
cat > "$A/packages/test-project/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: test-project, version: 0.1.0, status: active, domain: proyectos }
spec: { owner: team:datos }
Y
cat > "$A/packages/hr/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: hr, version: 1.0.0, status: active, domain: people }
spec: { owner: team:hr }
Y
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol no compila: $(cd "$A" && "$ORE" validate . 2>&1 | head -3)"
ALMACEN="$TMP/almacen"; mkdir -p "$ALMACEN"
ALMACEN_PY="$ALMACEN"; TMP_PY="$TMP"; A_W="$A"
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) ALMACEN_PY="$(cd "$ALMACEN" && pwd -W)"; TMP_PY="$(cd "$TMP" && pwd -W)"; A_W="$(cd "$A" && pwd -W)";; esac

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

celda() { # <lenguaje> <texto> [fichero]
  local cuerpo n
  cuerpo=$("$PY" -c 'import json,sys; d={"texto": sys.argv[2], "lenguaje": sys.argv[1]}
if len(sys.argv) > 3: d["fichero"] = sys.argv[3]
print(json.dumps(d))' "$@")
  [ "$(pide POST /puestos/$P/ejecutar "$cuerpo")" = "202" ] || { echo "     ejecutar: $(cat "$TMP/r.json")"; return 1; }
  n=$(jq_ "d['celda']")
  for _ in $(seq 1 5); do pide GET "/puestos/$P/celdas/$n" >/dev/null; [ "$(jq_ "d['estado']")" = "hecha" ] && return 0; done
  return 1
}
resumen() {
  jq_ "(lambda s: s['tipo'] + ' · ' + (('%s · %s' % ([c['name'] for c in s['columnas']], s['filas'][:3])) if s['tipo']=='tabla' else ((s.get('nombre','') + ': ' + s.get('mensaje','')[:150]) if s['tipo']=='error' else repr(s.get('texto','')))))(d['salida'])"
}
trabajo() { # <codigo>: lanza el trabajo, lo corre el agente como en el Job, y dice el informe
  local t
  [ "$(pide POST /trabajos "{\"codigo\":\"$1\"}")" = "202" ] || { echo "$(pide POST /trabajos "{\"codigo\":\"$1\"}") $(jq_ "d.get('error', d)" | cut -c1-160)"; return; }
  t=$(jq_ "d['id']")
  ORE_SERVE="$BASE" PUESTO="$t" TRABAJO="$1@local" ORE_SUJETO=agente:local ORE_ALMACEN="dir:$ALMACEN_PY" TTL=600 \
    "$PY" "$RAIZ/puesto/python/agente.py" >"$TMP/trabajo.txt" 2>&1
  pide GET "/trabajos/$t" >/dev/null
  jq_ "d['informe']['estado'] + ' · ' + (lambda s: s.get('texto','').strip() if s['tipo']!='error' else s.get('nombre','')+': '+s.get('mensaje','')[:140])(d['informe']['salida'])"
}

titulo "§1 HOY: lo que siembra \`transforms-sql\` y si corre tal como nace"
C=$(pide POST /repositorios '{"paquete":"test-project","carpeta":"transforms_sql_test","nombre":"transforms_sql_test","plantilla":"transforms-sql"}')
echo "     POST /repositorios → $C"
R="$A/packages/test-project/transforms_sql_test"
( cd "$R" && find . -type f | sort | sed 's/^/       /' )
TEXTO=$(cat "$R/transforms/ejemplo.py")
celda python "$TEXTO"
echo "     ejemplo.py en la sesion:   $(resumen)"
echo "     ejemplo.py como trabajo:   $(trabajo packages/test-project/transforms_sql_test/transforms/ejemplo.py)"
echo "     pyproject.toml: dependencies = [] → no construye capa (0036 ③); en SQL no hay nada que declarar"

titulo "§2 LAS CANDIDATAS: un \`.sql\` en \`transforms/\`"
candidata() { # <etiqueta> <fichero> <texto>
  local f="$R/transforms/$2" fp
  printf '%s\n' "$3" > "$f"
  fp="$A_W/packages/test-project/transforms_sql_test/transforms/$2"
  echo "     $1"
  echo "       ore sql:  $("$ORE" sql --json --arbol "$A_W" "$fp" 2>/dev/null | "$PY" -c 'import json,sys; s=json.load(sys.stdin); print(("✗ "+s["fallos"][0]["mensaje"][:120]) if s["fallos"] else ("escribe %s (%s), lee %s" % (s["escribe"]["ref"], s["escribe"]["modo"], [x["ref"] for x in s["lee"]]) if s.get("escribe") else "lee %s" % [x["ref"] for x in s["lee"]]))')"
  celda sql "$3" "packages/test-project/transforms_sql_test/transforms/$2"
  echo "       sesion:   $(resumen)"
  echo "       trabajo:  $(trabajo "packages/test-project/transforms_sql_test/transforms/$2")"
}
candidata "A · lee un dataset que hay que cambiar (como hoy)" a.sql \
'create or replace table mi_paquete.mi_resumen as
select pais, count(*) as n
from mi_paquete.mi_dataset
group by pais'
candidata "B · no lee nada (range), escribe en SU paquete, sin comillas" b.sql \
'create or replace table test-project.ejemplo as
select range as n, range % 3 as grupo
from range(10)'
candidata "C · lo mismo, con el paquete entre comillas" c.sql \
'create or replace table "test-project".ejemplo as
select range as n, range % 3 as grupo
from range(10)'
candidata "D · y el paso siguiente: leer lo que C escribio" d.sql \
'create or replace table "test-project".ejemplo_por_grupo as
select grupo, count(*) as n
from "test-project".ejemplo
group by grupo
order by grupo'
candidata "E · un analisis: solo lee" e.sql \
'select * from "test-project".ejemplo_por_grupo'
echo "     y lo que dejaron: $(ls "$A/datasets" | tr '\n' ' ')"
[ -f "$A/datasets/test-project_ejemplo.json" ] && echo "     procedencia de C: $("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["procedencia"])' "$A/datasets/test-project_ejemplo.json")"
[ -f "$A/datasets/test-project_ejemplo_por_grupo.json" ] && echo "     procedencia de D: $("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["procedencia"])' "$A/datasets/test-project_ejemplo_por_grupo.json")"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) && echo "     el arbol sigue compilando: si" || echo "     el arbol sigue compilando: NO · $(cd "$A" && "$ORE" validate . 2>&1 | head -2)"

titulo "§2b LAS MISMAS, en un paquete SIN guion (\`hr\`): ¿el resto de la cadena anda?"
[ "$(pide POST /repositorios '{"paquete":"hr","carpeta":"transforms_sql","nombre":"transforms_sql","plantilla":"transforms-sql"}')" = "201" ] || falla "el repositorio en hr: $(cat "$TMP/r.json")"
R="$A/packages/hr/transforms_sql"
candidata2() { # <etiqueta> <fichero> <texto>
  local fp
  printf '%s\n' "$3" > "$R/transforms/$2"
  fp="$A_W/packages/hr/transforms_sql/transforms/$2"
  echo "     $1"
  echo "       ore sql:  $("$ORE" sql --json --arbol "$A_W" "$fp" 2>/dev/null | "$PY" -c 'import json,sys; s=json.load(sys.stdin); print(("✗ "+s["fallos"][0]["mensaje"][:120]) if s["fallos"] else ("escribe %s (%s), lee %s" % (s["escribe"]["ref"], s["escribe"]["modo"], [x["ref"] for x in s["lee"]]) if s.get("escribe") else "lee %s" % [x["ref"] for x in s["lee"]]))')"
  celda sql "$3" "packages/hr/transforms_sql/transforms/$2"
  echo "       sesion:   $(resumen)"
  echo "       trabajo:  $(trabajo "packages/hr/transforms_sql/transforms/$2")"
}
candidata2 "C' · no lee nada (range), escribe en SU paquete" ejemplo.sql \
'create or replace table hr.ejemplo as
select range as n, range % 3 as grupo
from range(10)'
candidata2 "D' · el paso siguiente: leer lo que C' escribio" por_grupo.sql \
'create or replace table hr.ejemplo_por_grupo as
select grupo, count(*) as n
from hr.ejemplo
group by grupo
order by grupo'
candidata2 "E' · un analisis: solo lee" mira.sql \
'select * from hr.ejemplo_por_grupo order by grupo'
[ -f "$A/datasets/hr_ejemplo.json" ] && echo "     procedencia de C': $("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["procedencia"])' "$A/datasets/hr_ejemplo.json")"
[ -f "$A/datasets/hr_ejemplo_por_grupo.json" ] && echo "     procedencia de D': $("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["procedencia"])' "$A/datasets/hr_ejemplo_por_grupo.json")"
( cd "$A" && "$ORE" validate . >/dev/null 2>&1 ) && echo "     el arbol sigue compilando: si" || echo "     el arbol sigue compilando: NO · $(cd "$A" && "$ORE" validate . 2>&1 | head -2)"

titulo "§3 EL PAQUETE: como se llaman los de un proyecto"
grep -n "c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'" "$RAIZ/crates/ore-serve/src/proyectos.rs" | head -1 | sed 's/^/     proyectos.rs (id del proyecto = su paquete):/'
grep -n "paquete y tabla llevan" "$RAIZ/crates/ore-cli/src/datasets.rs" | sed 's/^/     datasets.rs (write):/'
grep -n "fn segmento_valido" -A 12 "$RAIZ/crates/ore-serve/src/repositorios.rs" | grep -E "is_ascii|'-'|'_'" | head -3 | sed 's/^/     repositorios.rs:/'
grep -rn "fn slug\|fn nombre_de_paquete\|to_lowercase().replace" "$RAIZ/crates/ore-serve/src/proyectos.rs" 2>/dev/null | head -3 | sed 's/^/     proyectos.rs:/'

titulo "§4 LO QUE MUEVE"
grep -n 'id: "transforms-sql"' -A 12 "$RAIZ/crates/ore-core/src/clases.rs" | grep -E "version|semilla|\(\"" | sed 's/^/     clases.rs:/'
echo "     subir \`version\` ofrece a los que ya estan «Propose upgrade to vN»: la propuesta ESCRIBE la"
echo "     semilla nueva y el manifiesto (repositorios.rs, actualizar) y NO borra lo que habia:"
echo "     ejemplo.py y pyproject.toml se quedarian al lado del .sql."
grep -rn "transforms-sql" "$RAIZ/pruebas-de-fuego/"*.sh "$RAIZ/crates/ore-serve/src/"*.rs 2>/dev/null | grep -v "^.*://" | grep -v medida-la-semilla | head -5 | sed 's/^/     /'

titulo "§5 LO QUE DICE (2026-09-24)"
cat <<'FIN'
     HOY NO ES FUNCIONAL: `ejemplo.py` falla tal como nace (en la sesion y como trabajo:
       `mi_paquete` no existe), la consulta vive en una cadena (sin editor de SQL) y el
       `pyproject.toml` no declara nada: en SQL no hay dependencias. La razon de 0036
       ("una celda SQL a secas lee pero no escribe") ya no es verdad.
     UNA SEMILLA FUNCIONAL YA EXISTE: un `.sql` que no lee nada (`range`, generadora: el
       arbol la deja) y escribe en EL PAQUETE DEL REPOSITORIO corre tal como nace, en la
       sesion y como trabajo, con procedencia {inputs: [], transform: <fichero>}; y el paso
       siguiente (leerlo, agruparlo) y un analisis encadenan. La semilla tiene que saber
       su paquete: se siembra en `packages/{paquete}/…`, hoy como texto fijo.
     ⛔ EL PAQUETE DE UN PROYECTO NO SE PUEDE ESCRIBIR: su id lleva guion
       (`id_de("Test project")` = `test-project`, 0035 ⑦.1) y `write()` solo acepta
       letras, digitos y `_` (datasets.rs): 400 desde Python y desde SQL. Sin comillas ni
       analiza (`-`). Es de todo transform de un proyecto, no solo de la semilla.
     LA VERSION: subirla ofrece la propuesta a las instancias viejas, que ESCRIBE lo nuevo
       y no borra `ejemplo.py` ni `pyproject.toml`.
FIN
echo
