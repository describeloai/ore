#!/bin/sh
# COMPILAR LOS BINARIOS DE LAS IMÁGENES (perfil `imagen`: sin LTO, ver Cargo.toml) y dejarlos en
# `$SALIDA`. Una sola definición para todo lo que compila: la etapa `deps` del
# Dockerfile (con `RECETA`: sólo las dependencias, por cargo-chef), la etapa
# `build` (en local) y el paso `binarios` de cloudbuild.yaml, que lo corre con
# `docker run` sobre `ore-deps:h-<huella de la receta>`.
#
# ⚠️ Las dependencias se compilan con los MISMOS paquetes, perfil y opciones que
#   los binarios: si no, cargo unifica otras features y las recompila.
set -eu
SALIDA=${SALIDA:-binarios}
OBJETIVO=${CARGO_TARGET_DIR:-target}
PAQUETES="-p ore-cli -p ore-serve -p ore-iam -p ore-cofre
  -p ore-read-jsonl -p ore-read-postgres -p ore-read-bigquery -p ore-read-s3 -p ore-firmar-s3 -p ore-sts
  -p ore-fetch -p ore-log -p ore-sign -p ore-store -p ore-invoke -p ore-medios -p ore-packages -p ore-federation"
# `-j`: con LTO completo cada enlace pedía GB y con 8 a la vez el kernel mató
# `rustc` en E2_HIGHCPU_8 (8 GB; SIGKILL en ore-store-r2, 2026-10-03). Sin LTO
# (perfil `imagen`) el enlace es ligero: todos los núcleos. `COMPILAR_JOBS` lo baja.
JOBS=${COMPILAR_JOBS:-$(nproc)}

if [ -n "${RECETA:-}" ]; then
  # shellcheck disable=SC2086
  cargo chef cook --profile imagen --locked -j "$JOBS" $PAQUETES --recipe-path "$RECETA"
  exit 0
fi

# `--locked`: el `Cargo.lock` del árbol, no lo que hubiera hoy en el índice.
# shellcheck disable=SC2086
cargo build --profile imagen --locked -j "$JOBS" $PAQUETES
mkdir -p "$SALIDA"
for b in ore ore-serve ore-iam ore-cofre ore-read-jsonl ore-read-postgres ore-read-bigquery ore-read-s3 \
         ore-firmar-s3 ore-asumir-rol ore-fetch ore-log ore-sign ore-store-r2 ore-store-gcs ore-invoke ore-medios ore-packages \
         ore-federation; do
  cp "$OBJETIVO/imagen/$b" "$SALIDA/$b"
done

# ⛔ LA PUERTA: cada servidor ARRANCA. El 2026-10-03 `ore-cofre` y `ore-medios`
#   murieron al arrancar (SIGSEGV) y lo vio t-demo, caído 10 min. Aquí, 3 s
#   cada uno: que salga por su configuración (código < 128) o que siga vivo
#   (lo para `timeout`: 124 o 143) está bien; una señal (SIGSEGV 139, abort
#   134…) para la construcción antes de publicar nada.
SALIDA_ABS=$(cd "$SALIDA" && pwd)
for b in ore-serve ore-iam ore-cofre ore-medios ore-federation; do
  set +e
  (cd /tmp && ORE_STORE=gcs ORE_GCS_BUCKET=x ORE_MEDIOS_PUERTO=8097 ORE_MEDIOS_PUERTO_CONTENIDO=8098 \
     timeout 3 "$SALIDA_ABS/$b" >/dev/null 2>&1)
  c=$?
  set -e
  case $c in
    124|143) echo "  $b arranca (vivo a los 3 s)" ;;
    *) if [ "$c" -lt 128 ]; then echo "  $b arranca (sale $c: su configuración)"
       else echo "⛔ $b muere al arrancar (exit $c)" >&2; exit 1; fi ;;
  esac
done
