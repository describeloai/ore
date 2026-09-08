#!/usr/bin/env bash
# LA MUDANZA — que el cliente pueda irse, comprobado en vez de prometido.
#
# ── ⭐⭐⭐ LA REGLA QUE ESTO CONVIERTE EN PUERTA ─────────────────────────────
#
# Corremos una plataforma gestionada. Eso obliga a decir en voz alta qué se
# lleva un cliente el día que se va, y a que la respuesta no dependa de nuestra
# buena voluntad:
#
#   > **Ninguna comprobación de la plataforma puede volverse necesaria para
#   > compilar.** El día que `ore validate` necesite nuestra base, ORE deja de
#   > ser ORE — y deja de ser vendible, porque lo que se vende es un formato del
#   > que el cliente NO depende de nosotros para leer.
#
# Lo que la plataforma añade —identidad, huella, cuotas, el reconciliador— vive
# FUERA del documento y apunta a él. Nunca al revés.
#
# ── Los cuatro hechos ───────────────────────────────────────────────────────
#
#   1  el árbol se produce como lo produce la plataforma
#   2  y NO menciona nuestra infraestructura
#   3  ni lleva una credencial dentro
#   4  y compila **con el entorno vacío**, con un binario sin cliente TLS
#
# ⚠️ Lo que esto NO prueba, dicho para no vender de más: que no hubo red. Eso lo
#    prueba otra cosa y ya está probada — `ore-cli/tests/dependencias.rs` lee el
#    `Cargo.lock` y falla si una crate de red, de TLS o de FFI entra en el cierre
#    de `ore`. Aquí se comprueba que **no hace falta**, no que no se pudo.
#
# Uso:  bash pruebas-de-fuego/la-mudanza.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"

falla() { echo "✗ $*" >&2; rm -rf "$TMP"; exit 1; }
dice()  { echo "  · $*"; }
trap 'rm -rf "$TMP"' EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)" || falla "no hay binario de \`ore\` — cargo build -p ore-cli"

# ── 1 · El árbol, producido como lo produce la plataforma ───────────────────
#
# Los mismos tres verbos que corren en `malla/95-el-arbol-en-la-forja.yaml`:
# inducir desde un catálogo ya leído, cerrar las decisiones con respuestas que
# vienen de fuera, y validar. Si el árbol de esta prueba se hiciera a mano no
# probaría nada del árbol que producimos nosotros.
REPO="$TMP/inquilino"
mkdir -p "$REPO"
( cd "$REPO" && "$ORE" init . >/dev/null 2>&1 ) || falla "1 · \`ore init\` fallo"

cat > "$REPO/catalogo.json" <<'JSON'
{
  "source": "lago",
  "tables": [
    { "name": "public.clientes",
      "columns": [ { "name": "id", "type": "Integer" },
                   { "name": "email", "type": "String" } ] },
    { "name": "public.pedidos",
      "columns": [ { "name": "id", "type": "Integer" },
                   { "name": "cliente_id", "type": "Integer" } ] }
  ]
}
JSON

# La fuente, con una URL que NO lleva credencial — la que la presta la nube.
( cd "$REPO" && "$ORE" source add --name lago "bigquery://un-proyecto/ventas" >/dev/null 2>&1 ) \
  || falla "1 · \`ore source add\` fallo"
( cd "$REPO" && "$ORE" discover --from catalogo.json --out packages/ventas --name ventas >/dev/null 2>&1 ) \
  || falla "1 · \`ore discover\` fallo"

# ⛔ Y el conducto, que es lo que la plataforma declara por el inquilino en
# `malla/95-el-arbol-en-la-forja.yaml`. Aceptar la relacion del paso siguiente
# crea una `via`, y recorrerla COPIA la clave y el enlace por
# `materialization.topology`. `ore init` no lo escribe a proposito —«omitirlo no
# deja nada abierto: lo CIERRA»— asi que sin esto `OOS4011` niega el arbol.
#
# ⭐ Y viaja DENTRO del arbol, que es lo correcto: es un documento de gobierno
#   del inquilino, no una pieza de nuestra infraestructura. Se lo lleva puesto.
cat > "$REPO/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: default }
spec:
  owner: team:security
  conduits:
    materialization.topology:
      oos.maturity: DRAFT
YAML

cat > "$REPO/respuestas.json" <<'JSON'
{"answers": {
  "clave/public.clientes": ["id"],
  "clave/public.pedidos": ["id"],
  "dueno/ventas": "team:datos",
  "relacion/public.pedidos.cliente_id": "si"
}}
JSON
( cd "$REPO" && "$ORE" review packages/ventas --answers respuestas.json >/dev/null 2>&1 ) \
  || falla "1 · \`ore review\` fallo"
rm -f "$REPO/catalogo.json" "$REPO/respuestas.json"
dice "1 · arbol producido: $(find "$REPO" -name '*.yaml' | wc -l | tr -d ' ') documentos"

# ── 2 · No menciona nuestra infraestructura ─────────────────────────────────
#
# Un árbol que nombra nuestro emisor, nuestra forja o nuestro registro no es
# portable: es un puntero a nuestros servidores con forma de documento.
NUESTRO="paladio\.io|svc\.cluster\.local|pkg\.dev|project-8853a180|login\.paladio|forja\.forja"
if grep -rEl "$NUESTRO" "$REPO" --include='*.yaml' --include='*.json' 2>/dev/null | grep -q .; then
  echo "   lo que lo menciona:" >&2
  grep -rEn "$NUESTRO" "$REPO" --include='*.yaml' --include='*.json' 2>/dev/null | head -5 >&2
  falla "2 · el arbol nombra nuestra infraestructura"
fi
dice "2 · no nombra ni el emisor, ni la forja, ni el registro, ni el proyecto"

# ── 3 · No lleva una credencial dentro ──────────────────────────────────────
#
# `ore source add` manda el secreto a `.env.local` y el manifiesto sólo declara
# **de qué variable sale**. Es lo que hace publicable un repositorio ontológico,
# y aquí se comprueba en vez de creerlo.
grep -q "connectionEnv" "$REPO/ontology.config.yaml" \
  || falla "3 · el manifiesto no declara la fuente por variable"
if grep -rEl "://[^/\"' ]*:[^/\"' ]*@" "$REPO" --include='*.yaml' 2>/dev/null | grep -q .; then
  falla "3 · hay una URL con credencial dentro de un documento"
fi
# Y `.env.local` no viaja: `ore init` lo pone en el `.gitignore`.
grep -q "^\.env\.local$" "$REPO/.gitignore" 2>/dev/null \
  || falla "3 · \`.env.local\` no esta ignorado — el secreto viajaria"
dice "3 · el secreto se declara por variable, y \`.env.local\` no viaja"

# ── 4 · Y compila CON EL ENTORNO VACÍO ──────────────────────────────────────
#
# `env -i` — ni una variable. Sin `PATH`, sin `HOME`, sin las de conexión de
# ninguna fuente. Si algo de la plataforma se hubiera colado como un requisito
# tácito, aquí es donde se ve.
#
# ⚠️ En Windows se conserva `SYSTEMROOT`: sin él no arranca **ningún** proceso,
#    y eso no es una dependencia de ORE — es del sistema operativo.
VACIO="env -i"
case "$(uname -s 2>/dev/null || echo win)" in
  MINGW*|MSYS*|CYGWIN*|win) VACIO="env -i SYSTEMROOT=${SYSTEMROOT:-C:\\Windows}" ;;
esac

( cd "$REPO" && $VACIO "$ORE" validate . ) > "$TMP/validate.txt" 2>&1 \
  || falla "4 · \`ore validate\` fallo con el entorno vacio: $(tail -3 "$TMP/validate.txt")"
grep -qi "ok" "$TMP/validate.txt" || falla "4 · validate no dijo ok: $(cat "$TMP/validate.txt")"
dice "4 · \`ore validate\` con \`env -i\`: $(tr -d '\r' < "$TMP/validate.txt" | tail -1)"

# `ore view` toma una RUTA, no un nombre cualificado: informa del paquete
# entero. Es el motor de vistas —once mil lineas de algebra— compilando sin una
# sola variable de entorno.
( cd "$REPO" && $VACIO "$ORE" view . ) > "$TMP/view.txt" 2>&1 \
  || falla "4 · el motor de vistas fallo con el entorno vacio: $(tail -3 "$TMP/view.txt")"
dice '4 · `ore view .` tambien compila: el motor entero, sin una variable'

echo
echo "ok · el arbol se va entero, y sigue compilando sin nosotros"
