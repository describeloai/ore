#!/usr/bin/env bash
# El plano de control **con el árbol en una forja**, y sin montar una forja.
#
# Un repositorio pelado y `file://` bastan: `git clone` y `git push` recorren el
# mismo camino de `git.rs` que contra Forgejo, y así esta prueba no necesita un
# servidor, ni una imagen, ni una red. Lo que NO cubre —el HTTP contra Forgejo—
# lo cubre el clúster, y se dice para no vender lo que no es.
#
# Los cuatro hechos que fija:
#
#   1. escribir deja un COMMIT, y la respuesta lo dice
#   2. el autor del commit es **el sujeto de la petición**, y el committer el
#      servidor — `sub` y `act`, que es lo que hace que la historia sea una
#      auditoría y no una lista de cambios
#   3. leer después vuelve a clonar, así que ve lo que se escribió
#   4. un empujón RECHAZADO es un `409`, no un `500` y no un árbol a medias
#
# ⚠️ El 4 se provoca con un `pre-receive` que niega, no con dos peticiones
#    simultáneas. **No simula la carrera**: fija que un rechazo se traduce a lo
#    que el cliente tiene que ver. La carrera de verdad la resuelve git, y eso
#    no lo probamos nosotros porque no lo escribimos nosotros.
#
# Uso:  bash pruebas-de-fuego/servidor-forja.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8901}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""

falla() { echo "✗ $*" >&2; limpiar; exit 1; }
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

export GIT_AUTHOR_NAME=semilla GIT_AUTHOR_EMAIL=semilla@x
export GIT_COMMITTER_NAME=semilla GIT_COMMITTER_EMAIL=semilla@x

# ── La forja: un repositorio pelado con un árbol dentro ─────────────────────
FORJA="$TMP/arbol.git"
git init -q --bare -b main "$FORJA" || falla "no se pudo crear el repositorio pelado"

git clone -q "$FORJA" "$TMP/semilla" 2>/dev/null
( cd "$TMP/semilla" && "$ORE" init . >/dev/null 2>&1 \
  && git add -A && git commit -qm "el arbol vacio" && git push -q origin HEAD:main ) \
  || falla "no se pudo sembrar la forja"
dice "0 · forja sembrada: $(git --git-dir="$FORJA" rev-parse --short main)"

# ── El servidor, en modo forja ──────────────────────────────────────────────
#
# El testigo va por el entorno aunque aquí no haga falta: es la misma forma que
# en el clúster, y una prueba que use otra no prueba lo que se despliega.
FORJA_TOKEN=no-hace-falta-en-file "$SERVE" \
  --forja "file://$FORJA" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
  --identidad cabecera --no-es-produccion >/dev/null 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/version" | grep -q '"arbol":"forja"' \
  || falla "0 · el servidor no dice que su arbol es una forja"
dice "0 · el servidor dice que su arbol es una forja"

SUJ='x-ore-sujeto: persona:ana'

# ── 1 · Escribir deja un commit ─────────────────────────────────────────────
curl -s -o "$TMP/alta.json" -X POST -H "$SUJ" "$BASE/fuentes" \
  -d '{"name":"bq","url":"bigquery://un-proyecto/ventas"}'
grep -q '"commit"' "$TMP/alta.json" \
  || falla "1 · el alta no devolvio un commit: $(cat "$TMP/alta.json")"
dice "1 · el alta deja un commit, y lo dice: $(grep -o '"commit":"[^\"]*"' "$TMP/alta.json")"

# ── 2 · Quién firma ─────────────────────────────────────────────────────────
AUTOR=$(git --git-dir="$FORJA" log -1 --format='%an' main)
QUIEN=$(git --git-dir="$FORJA" log -1 --format='%cn' main)
[ "$AUTOR" = "persona:ana" ] || falla "2 · el autor es \`$AUTOR\` y no el sujeto"
[ "$QUIEN" = "ore-serve" ]   || falla "2 · el committer es \`$QUIEN\` y no el servidor"
dice "2 · autor=$AUTOR  committer=$QUIEN  —  \`sub\` y \`act\`, en la historia"

# ── 3 · Leer vuelve a clonar ────────────────────────────────────────────────
curl -sf -H "$SUJ" "$BASE/fuentes" | grep -q '"name":"bq"' \
  || falla "3 · una lectura nueva no ve lo que se escribio"
dice "3 · la lectura siguiente lo ve: el servidor volvio a clonar"

# Y lo que NO cambia el arbol NO deja commit: la historia es la auditoria, y un
# commit por peticion que no cambio nada la convierte en una lista de visitas.
ANTES=$(git --git-dir="$FORJA" rev-parse main)
curl -s -o /dev/null -H "$SUJ" "$BASE/fuentes"
[ "$(git --git-dir="$FORJA" rev-parse main)" = "$ANTES" ] \
  || falla "3 · una LECTURA dejo un commit"
dice "3 · y una lectura no deja commit"

# ── 4 · Un empujón rechazado es un 409 ──────────────────────────────────────
cat > "$FORJA/hooks/pre-receive" <<'HOOK'
#!/bin/sh
echo "non-fast-forward: alguien escribio antes" >&2
exit 1
HOOK
chmod +x "$FORJA/hooks/pre-receive"

COD=$(curl -s -o "$TMP/choque.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/fuentes" \
       -d '{"name":"otro","url":"bigquery://un-proyecto/otro"}')
[ "$COD" = "409" ] || falla "4 · un empujon rechazado devolvio $COD y no 409: $(cat "$TMP/choque.json")"
grep -q "NADA se perdió" "$TMP/choque.json" \
  || falla "4 · el 409 no explica que no se perdio nada"
dice "4 · un empujon rechazado es un 409, y dice que nada se perdio"

# Y el árbol de la forja NO cambió: el error no deja nada a medias.
[ "$(git --git-dir="$FORJA" log -1 --format='%s' main)" = "alta de una fuente" ] \
  || falla "4 · la forja cambio pese al rechazo"
dice "4 · y la forja no cambio: lo que falla no deja el arbol a medias"

# ── 5 · El indice de assets, de memoria por cabeza (0034 ⑤) ──────────────────
#
# `GET /assets` clona UNA vez por cabeza: la primera vez calcula (`desde_cache:
# false`), la segunda se sirve de memoria (`true`), y un push a la forja cambia
# la cabeza y el siguiente GET vuelve a calcular. Y el indice dice lo que el
# arbol tiene: la fuente dada de alta no es un item (es de Data Origins), asi
# que `items` esta vacio y `paquetes` tambien — hasta que haya un paquete.
rm -f "$FORJA/hooks/pre-receive"
curl -s -o "$TMP/a1.json" -H "$SUJ" "$BASE/assets" || falla "5 · GET /assets no contesto"
grep -q '"desde_cache":false' "$TMP/a1.json" || falla "5 · el primer GET /assets tenia que calcular: $(head -c 300 "$TMP/a1.json")"
grep -q '"cabeza":"' "$TMP/a1.json" || falla "5 · el indice no dice su cabeza: $(head -c 300 "$TMP/a1.json")"
grep -q '"items":{}' "$TMP/a1.json" || falla "5 · un arbol sin paquetes tenia que dar items vacios: $(head -c 300 "$TMP/a1.json")"
curl -s -o "$TMP/a2.json" -H "$SUJ" "$BASE/assets" || falla "5 · el segundo GET /assets no contesto"
grep -q '"desde_cache":true' "$TMP/a2.json" || falla "5 · el segundo GET /assets tenia que salir de memoria: $(head -c 300 "$TMP/a2.json")"
CAB1=$(grep -o '"cabeza":"[0-9a-f]*"' "$TMP/a1.json")
# un push mueve la cabeza: un paquete con una tabla y su dataset
( cd "$TMP/semilla" && git pull -q origin main 2>/dev/null; mkdir -p packages/v/tables packages/v/datasets \
  && printf 'apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: v, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n' > packages/v/package.yaml \
  && printf 'apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: origen, namespace: v }\nspec:\n  datasource: bq\n  object: "un-proyecto.ventas.origen"\n  columns: { id: { type: Integer } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n' > packages/v/tables/origen.yaml \
  && printf 'apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n' > packages/v/datasets/pedidos.yaml \
  && git add -A && GIT_AUTHOR_NAME=ana GIT_AUTHOR_EMAIL=persona:ana@sujeto.invalid git commit -qm "un paquete" && git push -q origin HEAD:main ) \
  || falla "5 · no se pudo empujar el paquete a la forja"
curl -s -o "$TMP/a3.json" -H "$SUJ" "$BASE/assets" || falla "5 · el tercer GET /assets no contesto"
grep -q '"desde_cache":false' "$TMP/a3.json" || falla "5 · tras el push tenia que recalcular: $(head -c 300 "$TMP/a3.json")"
CAB3=$(grep -o '"cabeza":"[0-9a-f]*"' "$TMP/a3.json")
[ "$CAB1" != "$CAB3" ] || falla "5 · la cabeza no cambio tras el push: $CAB1"
grep -q '"dataset:v.pedidos":{' "$TMP/a3.json" || falla "5 · el indice no tiene el dataset: $(head -c 400 "$TMP/a3.json")"
grep -q '"identidad":true' "$TMP/a3.json" || falla "5 · el dataset identidad no lo dice"
grep -q '"tipo":"produce"' "$TMP/a3.json" && grep -q '"tipo":"sale_de"' "$TMP/a3.json" || falla "5 · faltan las dos direcciones de la relacion"
grep -q '"sujeto":"persona:ana"' "$TMP/a3.json" || falla "5 · la version del fichero no dice quien: $(grep -o '"version":[^}]*}' "$TMP/a3.json" | head -1)"
# 0039: un paquete sin origen (ni `discover.scope.json` ni `discover.catalog.json`,
# como este, hecho a mano) es `standard`: lo que tenga solo puede vivir en el lago.
grep -q '"type":"standard"' "$TMP/a3.json" || falla "5 · el paquete no dice su clase (sin origen, standard)"
dice "5 · GET /assets: calcula una vez por cabeza ($CAB1), la segunda de memoria, y un push ($CAB3) lo recalcula con el dataset, sus dos direcciones, su version y su clase"

echo
echo "ok · el arbol vive en la forja, y la historia dice quien decidio que"
