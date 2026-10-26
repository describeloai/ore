#!/bin/sh
# ORE 0072 F2·3 · binarios de la rama, estáticos (musl), para el banco: `ore-teselas` (`banco.py
# BANCO_BINARIO=…`) y `ore-piramide` (`piramide/banco_piramide.py`). En rust:1.90-alpine, con /src = C:\ORE-pg y
# el destino en el volumen `ore-target-alpine`. Sin --cpus (40 min).
#   sh compilar.sh [ore-teselas|ore-piramide …]
set -e
apk add -q musl-dev openssl-dev openssl-libs-static pkgconfig krb5-dev clang-dev mold
cd /src
for b in ${*:-ore-teselas}; do
  RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build --release -p "$b" --bin "$b"
  cp "/target/release/$b" "/src/pruebas-de-fuego/geo/f2/motor/$b"
  ls -la "/src/pruebas-de-fuego/geo/f2/motor/$b"
done
