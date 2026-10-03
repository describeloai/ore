#!/bin/sh
# LA HUELLA DE LOS BINARIOS: 16 hex de todo lo que decide lo que `cargo` compila
# para las imágenes —las etapas `herramientas` y `build` del Dockerfile, el guion
# que compila, `Cargo.*`, la toolchain,
# `crates/` y `vendor/oos` (que `ore-cli` lee al compilar)—. Si no cambia, los
# binarios son los mismos: la imagen `ore-binarios:h-<huella>` se reutiliza y
# no se compila nada (cloudbuild.yaml, paso `binarios`).
#
# Por el CONTENIDO de los ficheros y no por git: Cloud Build no recibe `.git`, y
# así la calcula igual el job de GitHub que una construcción a mano.
set -eu
cd "$(dirname "$0")/.."
{
  awk '/^FROM rust:.* AS herramientas$/ {p=1} /^FROM scratch AS binarios$/ {p=0} p' Dockerfile
  find Cargo.toml Cargo.lock rust-toolchain.toml ci/compilar-binarios.sh crates vendor/oos -type f \
    ! -path '*/target/*' ! -path '*/.git' ! -path '*/.git/*' ! -name '*.md' -print0 \
    | LC_ALL=C sort -z | xargs -0 sha256sum
} | sha256sum | cut -c1-16
