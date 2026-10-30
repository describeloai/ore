#!/usr/bin/env bash
# El corpus de la vara (ORE 0081 §7.4, S4): los `.slt` de apache/datafusion en el tag EXACTO de la versión
# del árbol, con los dos submódulos de datos que citan (arrow-testing en `testing/`, parquet-testing en
# `parquet-testing/`). Fuera quedan `datafusion-testing` (sqlite y pg_compat: horas) y lo demás (D-S11).
#
#   ci/corpus-slt.sh <directorio>
#
# Deja el árbol en <directorio>/datafusion. Si ya está en el tag, no hace nada. El tag sale de Cargo.lock:
# subir DataFusion sube el corpus, sin tocar este script.
set -euo pipefail

DESTINO="${1:?uso: ci/corpus-slt.sh <directorio>}"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TAG="$(awk '/^name = "datafusion"$/ { getline; gsub(/"/, "", $3); print $3; exit }' "$RAIZ/Cargo.lock")"
[ -n "$TAG" ] || { echo "corpus-slt: no encuentro la versión de datafusion en Cargo.lock" >&2; exit 1; }
ARBOL="$DESTINO/datafusion"

if [ -f "$ARBOL/.ore-tag" ] && [ "$(cat "$ARBOL/.ore-tag")" = "$TAG" ]; then
  echo "corpus-slt: $ARBOL ya está en $TAG"
  exit 0
fi

rm -rf "$ARBOL"
mkdir -p "$DESTINO"
git clone --quiet --depth 1 --branch "$TAG" https://github.com/apache/datafusion "$ARBOL"
git -C "$ARBOL" submodule update --quiet --init --depth 1 testing parquet-testing
echo "$TAG" > "$ARBOL/.ore-tag"
echo "corpus-slt: $ARBOL en $TAG ($(find "$ARBOL/datafusion/sqllogictest/test_files" -name '*.slt' | wc -l) ficheros .slt, $(du -sh "$ARBOL" | cut -f1))"
