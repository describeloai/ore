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
  -d '{"name":"lago","url":"bigquery://un-proyecto/ventas"}'
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
curl -sf -H "$SUJ" "$BASE/fuentes" | grep -q '"name":"lago"' \
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

echo
echo "ok · el arbol vive en la forja, y la historia dice quien decidio que"
