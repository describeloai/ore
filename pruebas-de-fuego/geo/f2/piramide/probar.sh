#!/bin/sh
# ORE 0072 F2·3·2 · fmt, clippy y pruebas de los crates de la pirámide (sólo -p de lo tocado: el workspace
# entero es del CI). En rust:1.90, con /src = C:\ORE-pg y los volúmenes ore-cargo y ore-target.
#   sh pruebas-de-fuego/geo/f2/piramide/probar.sh
cd /src
rustup component add rustfmt clippy >/dev/null 2>&1
cargo fmt -p ore-piramide -p ore-teselas -p ore-store -p ore-serve
echo "== clippy"
cargo clippy -p ore-piramide -p ore-teselas -p ore-serve --all-targets -- -D warnings 2>&1 | grep -E '^(error|warning)' -A8 | head -100
echo "== pruebas"
cargo test -p ore-piramide -p ore-teselas 2>&1; cargo test -p ore-serve teselas 2>&1 | grep -E '^test .*(FAILED|ok)$|test result|panicked|^error' | grep -v ' ok$' | head -60
