#!/usr/bin/env bash
# Cada paquete de `v1alpha5/emit` que emita GraphQL, contra `graphql-js`.
#
# Los que fallan al emitir NO son un fallo de esta prueba: hay casos que existen
# justamente para comprobar que la emision se NIEGA —un techo que lo poda todo,
# una clave parcial—. Lo que se comprueba aqui es que **lo que sale, vale**.
set -uo pipefail
cd "$(dirname "$0")/.."

ORE="${ORE:-cargo run --quiet -p ore-cli --}"
casos=$(ls -d vendor/oos/conformance/v1alpha5/emit/*/input 2>/dev/null)
[ -z "$casos" ] && { echo "no hay casos de emision"; exit 1; }

emitidos=0
for c in $casos; do
  sdl=$($ORE export "$c" --format graphql 2>/dev/null) || continue
  [ -z "$sdl" ] && continue
  emitidos=$((emitidos + 1))
  if ! printf '%s' "$sdl" | node pruebas-de-fuego/graphql.mjs; then
    echo "  ↑ en $c"
    exit 1
  fi
done

if [ "$emitidos" -eq 0 ]; then
  echo "ningun caso emitio SDL: la prueba no comprobo nada, que es peor que fallar"
  exit 1
fi
echo "graphql-js acepta los $emitidos esquemas emitidos"
