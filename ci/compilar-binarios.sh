#!/bin/sh
# COMPILAR LOS BINARIOS DE LAS IMÁGENES (perfil `imagen`: sin LTO, ver Cargo.toml) y dejarlos en
# `$SALIDA`. Una sola definición para los dos que compilan: la etapa `build` del
# Dockerfile (en local) y el paso `binarios` de cloudbuild.yaml, que lo corre con
# `docker run --network=cloudbuild` sobre la etapa `herramientas` —BuildKit no
# admite esa red, y sin ella sccache no llega a la identidad de la construcción—.
#
# Con `CACHE_GCS`, lo compilado de cada crate va y viene de ese bucket por su
# huella; si el bucket falla, se compila igual: la caché acelera, nunca rompe.
# (⚠️ No `SCCACHE_BUCKET`: ése es el bucket de S3 de sccache, y con él arranca
# contra S3 y no arranca —medido 2026-10-03—.)
set -eu
SALIDA=${SALIDA:-binarios}
if [ -n "${CACHE_GCS:-}" ]; then
  export SCCACHE_GCS_BUCKET="$CACHE_GCS" SCCACHE_GCS_RW_MODE=READ_WRITE \
         SCCACHE_GCS_KEY_PREFIX=imagen SCCACHE_IGNORE_SERVER_IO_ERROR=1
  # Si el servidor de sccache no arranca (credencial, configuración), sin él.
  if sccache --start-server >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
  else
    echo "aviso: sccache no arranca; se compila sin caché" >&2
  fi
fi
# `--locked`: el `Cargo.lock` del árbol, no lo que hubiera hoy en el índice.
# `-j`: con LTO completo cada enlace pedía GB y con 8 a la vez el kernel mató
# `rustc` en E2_HIGHCPU_8 (8 GB; SIGKILL en ore-store-r2, 2026-10-03). Sin LTO
# (perfil `imagen`) el enlace es ligero: todos los núcleos. `COMPILAR_JOBS` lo baja.
cargo build --profile imagen --locked -j "${COMPILAR_JOBS:-$(nproc)}" \
  -p ore-cli -p ore-serve -p ore-iam -p ore-cofre \
  -p ore-read-jsonl -p ore-read-postgres -p ore-read-bigquery -p ore-read-s3 -p ore-firmar-s3 -p ore-sts \
  -p ore-fetch -p ore-log -p ore-sign -p ore-store -p ore-invoke -p ore-medios
[ -z "${RUSTC_WRAPPER:-}" ] || sccache --show-stats || true
mkdir -p "$SALIDA"
for b in ore ore-serve ore-iam ore-cofre ore-read-jsonl ore-read-postgres ore-read-bigquery ore-read-s3 \
         ore-firmar-s3 ore-asumir-rol ore-fetch ore-log ore-sign ore-store-r2 ore-store-gcs ore-invoke ore-medios; do
  cp "target/imagen/$b" "$SALIDA/$b"
done
