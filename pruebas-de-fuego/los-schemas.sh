#!/usr/bin/env bash
# LOS SCHEMAS DE UNA BASE (0038 P6): crear y renombrar, contra un `ore-serve`
# de verdad y con el arbol en una forja.
#
# La consola tenia «Create schema» y el doble clic de renombrar, y los dos se
# quedaban en el estado del navegador. Esto fija que van al arbol: un clon,
# `ore package schema new|rename`, un commit del sujeto — y que la puerta es
# la de siempre, «el arbol no empeora».
#
# El arbol: una base `ventas` descubierta con alcance (lo que crea el modal de
# la consola) —schema del origen `rubix_demo_ventas`— y lo que la nombra desde
# fuera en tres partes: una View de `default`, un paquete `eu`, un `.sql`, un
# programa y el puntero de una copia.
#
#   1  el indice de assets          la carpeta del schema del origen
#   2  POST /paquetes/ventas/schemas     201 · commit del sujeto · el indice la
#                                   trae aunque este vacia
#   3  lo que se niega              la misma otra vez (y en mayusculas) 409;
#                                   `default` 422; sin `name` 422; un paquete
#                                   que no hay 404; la forja no se mueve
#   4  POST …/schemas/rubix_demo_ventas/renombrar   200 · la carpeta, lo que lo
#                                   nombra (yaml y sql), el puntero, `moved` y
#                                   el alcance, en UN commit; el programa se
#                                   dice y no se toca; el indice ya no ve el
#                                   viejo
#   5  lo que se niega al renombrar a uno que ya esta 409; uno que no hay 404;
#                                   sin `to` 422; `default` 422
#   6  la siguiente induccion       copiar una tabla re-induce el paquete: la
#                                   copia nace en el schema renombrado y nada
#                                   vuelve a la carpeta del origen
#
# Uso:  bash pruebas-de-fuego/los-schemas.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8913}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""
PY=$(command -v python3 || command -v python)

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  limpiar; exit 1
}
dice()  { echo "  · $*"; }
limpiar() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$TMP"
}
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\`"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\`"
[ -n "$PY" ] || falla "hace falta python"

export GIT_AUTHOR_NAME=semilla GIT_AUTHOR_EMAIL=semilla@x
export GIT_COMMITTER_NAME=semilla GIT_COMMITTER_EMAIL=semilla@x

# ── La forja, sembrada con una base descubierta ─────────────────────────────
FORJA="$TMP/arbol.git"
git init -q --bare -b main "$FORJA" || falla "no se pudo crear el repositorio pelado"
git clone -q "$FORJA" "$TMP/semilla" 2>/dev/null
S="$TMP/semilla"
(
  cd "$S" && git config core.autocrlf false \
  && "$ORE" init . --name demo >/dev/null 2>&1 \
  && cp "$RAIZ/crates/ore-cli/tests/catalogos/bigquery-rubix-demo-ventas.json" cat.json \
  && "$ORE" discover --from cat.json --out packages/ventas \
       --only rubix_demo_ventas.clientes --only rubix_demo_ventas.Pedidos \
       --owner team:ventas >/dev/null 2>&1 \
  && "$ORE" package new eu --owner team:eu >/dev/null 2>&1
) || falla "0 · no se pudo descubrir la base"
[ -f "$S/packages/ventas/rubix_demo_ventas/schema.yaml" ] || falla "0 · discover no dejo el schema del origen"
mkdir -p "$S/packages/ventas/views" "$S/packages/eu/views" "$S/packages/ventas/transforms" "$S/datasets/ventas/rubix_demo_ventas"
printf 'apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: resumen, namespace: ventas }\nspec:\n  owner: "team:ventas"\n  from: { table: ventas.rubix_demo_ventas.clientes }\n  fields: { id: id }\n' > "$S/packages/ventas/views/resumen.yaml"
printf 'apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: copia, namespace: eu }\nspec:\n  owner: "team:eu"\n  from: { table: ventas.rubix_demo_ventas.clientes }\n  fields: { id: id }\n' > "$S/packages/eu/views/copia.yaml"
printf 'create or replace table ventas.cuenta as\nselect count(*) as n from ventas.rubix_demo_ventas.clientes\n' > "$S/packages/ventas/transforms/cuenta.sql"
printf 'df = ore.read("ventas.rubix_demo_ventas.clientes")\n' > "$S/packages/ventas/transforms/lee.py"
printf '{"metadata_location":"s3://b/ore/v2/catalogo/ventas/rubix_demo_ventas/clientes/metadata/00001-x.metadata.json"}\n' > "$S/datasets/ventas/rubix_demo_ventas/clientes.json"
( cd "$S" && git add -A && git commit -qm "una base descubierta" && git push -q origin HEAD:main ) \
  || falla "0 · no se pudo sembrar la forja"
ERRORES_0=$(cd "$S" && "$ORE" validate . 2>&1 | tail -1)
cabeza() { git --git-dir="$FORJA" rev-parse main; }
asunto() { git --git-dir="$FORJA" log -1 --format='%s' main; }
autor()  { git --git-dir="$FORJA" log -1 --format='%an' main; }
fichero() { git --git-dir="$FORJA" show "main:$1" 2>/dev/null; }
hay()    { git --git-dir="$FORJA" cat-file -e "main:$1" 2>/dev/null; }
dice "0 · forja sembrada con \`ventas\` (schema del origen \`rubix_demo_ventas\`): $ERRORES_0"

FORJA_TOKEN=no-hace-falta-en-file "$SERVE" \
  --forja "file://$FORJA" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
  --identidad cabecera --no-es-produccion > "$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/version" | grep -q '"arbol":"forja"' || falla "0 · el servidor no arranco en modo forja"

SUJ='x-ore-sujeto: persona:ana'
pide() { # metodo ruta [cuerpo]
  local m="$1" r="$2" c="${3:-}"
  if [ -n "$c" ]; then
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$SUJ" \
      -H 'Content-Type: application/json' -d "$c" "$BASE$r"
  else
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$m" -H "$SUJ" "$BASE$r"
  fi
}
cumple() { "$PY" -c "import json,sys; d=json.load(open(sys.argv[1], encoding='utf-8')); assert ($1), sys.argv[2]" "$TMP/r.json" "$2" \
  || falla "$2 · $(head -c 600 "$TMP/r.json")"; }
carpetas() { # las carpetas de `ventas` en el indice de assets
  pide GET /assets >/dev/null
  "$PY" -c "import json,sys; d=json.load(open(sys.argv[1], encoding='utf-8')); print(' '.join(sorted([p for p in d['paquetes'] if p['name']=='ventas'][0]['carpetas'])))" "$TMP/r.json"
}

# ── 1 · el indice ───────────────────────────────────────────────────────────
C=$(carpetas)
echo " $C " | grep -q ' rubix_demo_ventas ' || falla "1 · el indice no trae la carpeta del origen: [$C]"
dice "1 · el indice de assets: ventas › [$C]"

# ── 2 · crear ───────────────────────────────────────────────────────────────
ANTES=$(cabeza)
[ "$(pide POST /paquetes/ventas/schemas '{"name":"espana","description":"Lo de Espa\u00f1a"}')" = "201" ] \
  || falla "2 · crear · $(cat "$TMP/r.json")"
cumple "d['schema']=='espana' and d['package']=='ventas' and d['fichero']=='packages/ventas/espana/schema.yaml'" "2 · lo que contesta"
cumple "len(d.get('commit',''))>=7" "2 · con su commit"
[ "$(cabeza)" != "$ANTES" ] || falla "2 · la forja no se movio"
autor | grep -q ana || falla "2 · el commit no es del sujeto: $(autor)"
asunto | grep -q 'crear el schema `espana`' || falla "2 · el asunto: $(asunto)"
fichero packages/ventas/espana/schema.yaml | grep -q 'description: "Lo de España"' || falla "2 · el documento: $(fichero packages/ventas/espana/schema.yaml)"
C=$(carpetas)
echo " $C " | grep -q ' espana ' || falla "2 · el indice no trae el schema recien creado (vacio): [$C]"
dice "2 · \`espana\` creado: 201 · commit de ana · el indice lo trae vacio: [$C]"

# ── 3 · lo que se niega al crear ────────────────────────────────────────────
ANTES=$(cabeza)
[ "$(pide POST /paquetes/ventas/schemas '{"name":"espana"}')" = "409" ] || falla "3 · otra vez · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/ventas/schemas '{"name":"ESPANA"}')" = "409" ] || falla "3 · en mayusculas · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/ventas/schemas '{"name":"default"}')" = "422" ] || falla "3 · default · $(cat "$TMP/r.json")"
cumple "'default' in d['error']" "3 · default dice por que"
[ "$(pide POST /paquetes/ventas/schemas '{"description":"x"}')" = "422" ] || falla "3 · sin name · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/nadie/schemas '{"name":"x"}')" = "404" ] || falla "3 · paquete que no hay · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "3 · una negativa movio la forja"
dice "3 · repetido 409 (tambien en mayusculas) · default 422 · sin name 422 · sin paquete 404 · la forja quieta"

# ── 4 · renombrar ───────────────────────────────────────────────────────────
ANTES=$(cabeza)
[ "$(pide POST /paquetes/ventas/schemas/rubix_demo_ventas/renombrar '{"to":"ventas_es"}')" = "200" ] \
  || falla "4 · renombrar · $(cat "$TMP/r.json")"
cumple "d['from']=='rubix_demo_ventas' and d['to']=='ventas_es' and d['movidos']>=7 and d['punteros']==1" "4 · lo que contesta"
cumple "set(d['reapuntados']) >= {'packages/ventas/views/resumen.yaml','packages/eu/views/copia.yaml','packages/ventas/transforms/cuenta.sql'}" "4 · lo reapuntado"
cumple "d['aMano']==['packages/ventas/transforms/lee.py']" "4 · el programa se dice"
cumple "d['anunciados']>=4" "4 · los anunciados"
[ "$(git --git-dir="$FORJA" rev-list --count "$ANTES..main")" = "1" ] || falla "4 · no fue UN commit"
asunto | grep -q 'el schema `rubix_demo_ventas` pasa a llamarse `ventas_es`' || falla "4 · el asunto: $(asunto)"
autor | grep -q ana || falla "4 · el commit no es del sujeto"
hay packages/ventas/rubix_demo_ventas/schema.yaml && falla "4 · la carpeta vieja sigue"
fichero packages/ventas/ventas_es/schema.yaml | grep -q 'name: ventas_es' || falla "4 · el schema nuevo"
fichero packages/eu/views/copia.yaml | grep -q 'table: ventas.ventas_es.clientes' || falla "4 · eu no se reapunto"
fichero packages/ventas/transforms/cuenta.sql | grep -q 'from ventas.ventas_es.clientes' || falla "4 · el .sql"
fichero packages/ventas/transforms/lee.py | grep -q 'ventas.rubix_demo_ventas.clientes' || falla "4 · el programa se toco"
fichero datasets/ventas/ventas_es/clientes.json | grep -q 'catalogo/ventas/rubix_demo_ventas/clientes' || falla "4 · el puntero"
hay datasets/ventas/rubix_demo_ventas/clientes.json && falla "4 · el puntero viejo sigue"
fichero packages/ventas/package.yaml | grep -q 'from: ventas.rubix_demo_ventas.Clientes, to: ventas.ventas_es.Clientes' || falla "4 · el moved: $(fichero packages/ventas/package.yaml)"
fichero packages/ventas/discover.scope.json | grep -q '"rubix_demo_ventas": "ventas_es"' || falla "4 · el alcance"
C=$(carpetas)
echo " $C " | grep -q ' ventas_es ' || falla "4 · el indice no trae el nombre nuevo: [$C]"
echo " $C " | grep -q ' rubix_demo_ventas ' && falla "4 · el indice sigue viendo el viejo: [$C]"
git clone -q "$FORJA" "$TMP/tras" 2>/dev/null
ERRORES_4=$(cd "$TMP/tras" && "$ORE" validate . 2>&1 | tail -1)
[ "$ERRORES_4" = "$ERRORES_0" ] || falla "4 · el arbol cambio: antes «$ERRORES_0», despues «$ERRORES_4»"
dice "4 · renombrado en UN commit: carpeta, yaml+sql de fuera, puntero, moved y alcance; lee.py se dice; el arbol igual («$ERRORES_4»); indice: [$C]"

# ── 5 · lo que se niega al renombrar ────────────────────────────────────────
ANTES=$(cabeza)
[ "$(pide POST /paquetes/ventas/schemas/ventas_es/renombrar '{"to":"espana"}')" = "409" ] || falla "5 · a uno que esta · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/ventas/schemas/no_hay/renombrar '{"to":"otro"}')" = "404" ] || falla "5 · uno que no hay · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/ventas/schemas/ventas_es/renombrar '{"since":"1.0.0"}')" = "422" ] || falla "5 · sin to · $(cat "$TMP/r.json")"
[ "$(pide POST /paquetes/ventas/schemas/default/renombrar '{"to":"otro"}')" = "422" ] || falla "5 · default · $(cat "$TMP/r.json")"
[ "$(cabeza)" = "$ANTES" ] || falla "5 · una negativa movio la forja"
dice "5 · a uno que esta 409 · uno que no hay 404 · sin to 422 · default 422 · la forja quieta"

# ── 6 · la siguiente induccion no lo deshace ────────────────────────────────
# copiar una tabla re-induce el paquete entero (`ore copy`)
[ "$(pide POST /paquetes/ventas/tablas/rubix_demo_ventas.clientes/copiar)" -lt 300 ] || falla "6 · copiar · $(cat "$TMP/r.json")"
hay packages/ventas/rubix_demo_ventas/schema.yaml && falla "6 · la re-induccion volvio a la carpeta del origen"
git --git-dir="$FORJA" ls-tree -r --name-only main packages/ventas | grep -q '^packages/ventas/rubix_demo_ventas/' && falla "6 · queda algo en la carpeta del origen"
fichero packages/ventas/ventas_es/datasets/Clientes__clientes.yaml | grep -q 'schema: ventas_es' || falla "6 · la copia no nacio en ventas_es: $(git --git-dir="$FORJA" ls-tree -r --name-only main packages/ventas)"
fichero packages/ventas/ventas_es/entities/Pedidos.yaml | grep -q 'schema: ventas_es' || falla "6 · la entidad re-inducida no esta en ventas_es"
dice "6 · copiar \`clientes\` re-induce: el Dataset nace en \`ventas_es\` y nada vuelve a \`rubix_demo_ventas\`"

echo "✓ los schemas se crean y se renombran de verdad (0038 P6)"
