#!/bin/sh
# ORE 0072 F2·3 · el `ore-teselas` de la rama, estático (musl), para el banco (`banco.py BANCO_BINARIO=…`).
# En rust:1.90-alpine, con /src = C:\ORE-pg y el destino en el volumen `ore-target-alpine`. Sin --cpus (40 min).
set -e
apk add -q musl-dev openssl-dev openssl-libs-static pkgconfig krb5-dev clang-dev mold
cd /src
RUSTFLAGS="-C link-arg=-fuse-ld=mold" cargo build --release -p ore-teselas --bin ore-teselas
cp /target/release/ore-teselas /src/pruebas-de-fuego/geo/f2/motor/ore-teselas
ls -la /src/pruebas-de-fuego/geo/f2/motor/ore-teselas
