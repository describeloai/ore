#!/usr/bin/env bash
# ORE 0068 6c · Dentro del banco: los tres repositorios Heavyweight de la consola (`consola/`), tal cual
# y con los puntos rebajados (PUNTOS), en `packages/geo/…`; `ore transforms generate` y el commit.
set -euo pipefail
B=/repo/target/release
A=/tmp/banco/arbol
C=/repo/pruebas-de-fuego/geo/c2/celda-victor/consola
cd "$A"
git -c user.name=b -c user.email=b@x add -A >/dev/null 2>&1 || true
git -c user.name=b -c user.email=b@x commit -qm "lo que dejó el banco" >/dev/null 2>&1 || true
for r in python sql java; do
  d=packages/geo/pesado-$r
  mkdir -p $d
  printf -- '---\nnombre: Pesado %s\nplantilla: transforms-%s\nplantillaVersion: 1\nmotor: heavyweight\n---\n6c.\n' "$r" "$r" > $d/README.md
  cp -r $C/$r/transforms $d/
done
sed -i "s/range(20000000)/range(${PUNTOS:-200000})/" packages/geo/pesado-python/transforms/sembrar.py
echo "== ore transforms generate"
"$B/ore" transforms generate . 2>&1 | tail -8 || true
git -c user.name=b -c user.email=b@x add -A && git -c user.name=b -c user.email=b@x commit -qm "6c: tres repositorios Heavyweight"
git log --oneline -1
