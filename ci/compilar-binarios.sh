#!/bin/sh
# COMPILAR LOS BINARIOS DE LAS IMÁGENES (perfil `imagen`: sin LTO, ver Cargo.toml) y dejarlos en
# `$SALIDA`. Una sola definición para todo lo que compila: la etapa `deps` del
# Dockerfile (con `RECETA`: sólo las dependencias, por cargo-chef), la etapa
# `build` (en local) y el job `construir` de ci.yml, que lo corre con
# `docker run` sobre `ore-deps:h-<huella de la receta>`.
#
# ⚠️ Las dependencias se compilan con los MISMOS paquetes, perfil y opciones que
#   los binarios: si no, cargo unifica otras features y las recompila.
set -eu
SALIDA=${SALIDA:-binarios}
OBJETIVO=${CARGO_TARGET_DIR:-target}
PAQUETES="-p ore-cli -p ore-serve -p ore-iam -p ore-cofre
  -p ore-read-jsonl -p ore-read-postgres -p ore-read-mongodb -p ore-read-bigquery -p ore-read-s3 -p ore-read-gcs -p ore-read-azure -p ore-read-sftp -p ore-read-sharepoint -p ore-firmar-s3 -p ore-sts
  -p ore-fetch -p ore-log -p ore-sign -p ore-store -p ore-invoke -p ore-medios -p ore-packages -p ore-federation -p ore-postgres"
# `-j`: con LTO completo cada enlace pedía GB y con 8 a la vez el kernel mató
# `rustc` en E2_HIGHCPU_8 (8 GB; SIGKILL en ore-store-r2, 2026-10-03). Sin LTO
# (perfil `imagen`) el enlace es ligero: todos los núcleos. `COMPILAR_JOBS` lo baja.
JOBS=${COMPILAR_JOBS:-$(nproc)}

# ⭐ UNA HUELLA POR BINARIO (ADR 0060 B2). `TODOS` son los que el Dockerfile copia
#   (`ci/huella-de-los-binarios.py --comprobar` mira que cuadren). El CI pasa en
#   `BINARIOS` sólo los que su caché no tiene; sin él, todos, como en la etapa
#   `build` del Dockerfile.
#
# ⛔ Con LOS MISMOS `-p` que al cocinar las dependencias, y `--bin` para elegir.
#   Con `-p <sólo su paquete>` cargo unifica otras features y recompila lo de
#   terceros: medido en H0 (2026-10-11, `ore-serve` solo), 78 dependencias de
#   nuevo y 404 s; con todos los `-p` y `--bin ore-serve`, ninguna y 304 s (los 24,
#   409 s, en la misma máquina).
TODOS="ore ore-serve ore-iam ore-cofre ore-read-jsonl ore-read-postgres ore-read-mongodb ore-read-bigquery ore-read-s3 ore-read-gcs ore-read-azure ore-read-sftp ore-read-sharepoint ore-firmar-s3 ore-asumir-rol ore-fetch ore-log ore-sign ore-store-r2 ore-store-gcs ore-invoke ore-medios ore-packages ore-federation ore-postgres"
BINARIOS=${BINARIOS:-$TODOS}

if [ -n "${RECETA:-}" ]; then
  # shellcheck disable=SC2086
  cargo chef cook --profile imagen --locked -j "$JOBS" $PAQUETES --recipe-path "$RECETA"
  exit 0
fi

CONSTRUIR=$PAQUETES
if [ "$BINARIOS" != "$TODOS" ]; then
  for b in $BINARIOS; do CONSTRUIR="$CONSTRUIR --bin $b"; done
fi
echo "  se compilan: $BINARIOS"

# `--locked`: el `Cargo.lock` del árbol, no lo que hubiera hoy en el índice.
# shellcheck disable=SC2086
cargo build --profile imagen --locked -j "$JOBS" $CONSTRUIR
mkdir -p "$SALIDA"
for b in $BINARIOS; do
  cp "$OBJETIVO/imagen/$b" "$SALIDA/$b"
done

# ⛔ LA PUERTA: cada servidor ARRANCA. El 2026-10-03 `ore-cofre` y `ore-medios`
#   murieron al arrancar (SIGSEGV) y lo vio t-demo, caído 10 min. Aquí, 3 s
#   cada uno: que salga por su configuración (código < 128) o que siga vivo
#   (lo para `timeout`: 124 o 143) está bien; una señal (SIGSEGV 139, abort
#   134…) para la construcción antes de publicar nada.
SALIDA_ABS=$(cd "$SALIDA" && pwd)
for b in ore-serve ore-iam ore-cofre ore-medios ore-federation ore-postgres; do
  # Sólo los compilados en esta vuelta: los de la caché ya pasaron la suya.
  case " $BINARIOS " in *" $b "*) ;; *) continue ;; esac
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
