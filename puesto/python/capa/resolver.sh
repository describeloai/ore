#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# RESOLVER LA CAPA DE PYTHON (ADR 0050 P2) — dentro de `capa-python:1`
#
# El gemelo de `puesto/node/capa/resolver.sh`. Lo llama `52-la-capa.yaml`.
# Espera el árbol ya clonado y deja, en `$TRABAJO`: `deps.txt`, `digest.txt`,
# `capa.tgz` (lo que corre), `dev.tgz` (lo de desarrollo), `informe.json` y
# `lock-del-repositorio.toml` (el `pylock.toml` del repositorio). No sube nada
# y no toca git: eso lo hace el contenedor que tiene la nube y el testigo, y
# esta imagen no los tiene A PROPÓSITO —lo único que necesita es PyPI—.
#
# ⚠️ NO FALLA NUNCA (sale 0). Si uv no puede resolver, el informe dice
#   `estado: error` con el motivo, que es lo que la consola sabe enseñar.
#
#   $1  el árbol clonado        $2  el alcance (o vacío: la celda)
# ═══════════════════════════════════════════════════════════════════════════
set -eu
ARBOL="$1"
ALCANCE="${2:-}"
TRABAJO="${TRABAJO:-/trabajo}"
CAPA="${CAPA:-}"
PROVISTO="${PROVISTO:-/opt/ore/provisto.txt}"
export PROVISTO
mkdir -p "$TRABAJO"
# uv: su caché vive EN EL JOB y muere con él; nunca baja un Python (usa el de
# esta imagen, el mismo que el del puesto) ni pinta barras.
export UV_CACHE_DIR="$TRABAJO/uv-cache" UV_PYTHON_DOWNLOADS=never UV_NO_PROGRESS=1 UV_LINK_MODE=copy
C="python3 /opt/ore/capa.py"

# ── 1 · la declaración: los pyproject.toml del alcance, como los lee `ore-serve` ──
$C declarar "$ARBOL" "$ALCANCE" "$TRABAJO"
DIGEST=$(cat "$TRABAJO/digest.txt")
if [ -z "$DIGEST" ]; then
  echo "· el árbol no declara dependencias de Python: nada que resolver"
  exit 0
fi
if [ -n "$CAPA" ] && [ "$DIGEST" != "$CAPA" ]; then
  echo "⚠️ se pidió $CAPA y el árbol declara $DIGEST (cambió entre medias): se resuelve lo que hay"
fi

# ── 2 · el proyecto que uv resuelve, con lo de la sesión como restricción ──
$C proyecto "$TRABAJO" "$PROVISTO"
P="$TRABAJO/proyecto"

# ── 3 · resolver: UNA resolución, DOS cajas ──────────────────────────────
# ⭐ Como en Node (L3·1): `uv lock` resuelve TODO —con el grupo `dev`— y de ESE
#   lock salen, sin volver a resolver, lo que corre (`--no-dev`) y lo de
#   desarrollo (`--only-group dev`, sin lo que ya va en la primera). Ninguna de
#   las dos lleva lo que la sesión trae (`--no-emit-package`).
#   - `capa.tgz` lo que la sesión necesita para correr, y nada más; es lo
#                único que recibe la ejecución de una función;
#   - `dev.tgz`  lo de desarrollo (pytest y sus plugins, hypothesis, stubs):
#                lo usan las pruebas y el servidor de lenguaje de la sesión.
# `no-build` ES LA DECISIÓN (P2, la de `--ignore-scripts` en Node): sólo
#   ruedas, ningún paquete compila ni ejecuta código al instalarse. Uno que
#   sólo publica fuentes no se resuelve, y el informe lo dice.
ESTADO=lista
TAR="--sort=name --mtime=2026-01-01T00:00Z --owner=0 --group=0 --numeric-owner"
if ( cd "$P" && uv lock > "$TRABAJO/uv.log" 2>&1 \
     && uv export -q --format pylock.toml --no-emit-project -o pylock.toml >> "$TRABAJO/uv.log" 2>&1 ); then
  FUERA=$($C fuera "$TRABAJO" - "$PROVISTO")
  # shellcheck disable=SC2086
  if ( cd "$P" && uv export -q --format pylock.toml --no-emit-project --no-dev $FUERA -o pylock.run.toml >> "$TRABAJO/uv.log" 2>&1 ); then
    FUERA_DEV=$($C fuera "$TRABAJO" "$P/pylock.run.toml" "$PROVISTO")
    # shellcheck disable=SC2086
    ( cd "$P" && uv export -q --format pylock.toml --no-emit-project --only-group dev $FUERA_DEV -o pylock.dev.toml >> "$TRABAJO/uv.log" 2>&1 ) || ESTADO=error
  else
    ESTADO=error
  fi
  for CAJA in run dev; do
    [ "$ESTADO" = lista ] || break
    L="$P/pylock.$CAJA.toml"
    if grep -q '^\[\[packages\]\]' "$L" 2>/dev/null; then
      if uv pip install -q --no-deps --target "$TRABAJO/$CAJA" -r "$L" >> "$TRABAJO/uv.log" 2>&1; then
        rm -f "$TRABAJO/$CAJA/.lock"   # el cerrojo de uv, no un paquete
        NOMBRE=$([ "$CAJA" = run ] && echo capa.tgz || echo dev.tgz)
        # Fechas y dueños fijos: la misma resolución da la misma caja, byte a byte.
        # Sus entradas, sin `./`: abrirla en `/capa` no toca el directorio montado.
        # shellcheck disable=SC2046
        tar $TAR -czf "$TRABAJO/$NOMBRE" -C "$TRABAJO/$CAJA" $(ls -A "$TRABAJO/$CAJA")
      else
        ESTADO=error
      fi
    elif [ "$CAJA" = run ]; then
      echo "### nada que correr: lo declarado para correr lo pone ya la sesión"
    fi
  done
  [ "$ESTADO" = lista ] || echo "✗ uv no pudo instalar lo resuelto: $(tail -c 600 "$TRABAJO/uv.log")"
else
  ESTADO=error
  echo "✗ uv no pudo resolver: $(tail -c 600 "$TRABAJO/uv.log")"
fi

# ── 4 · el informe: lo resuelto, sus sumas y sus avisos ────────────────────
$C informe "$TRABAJO" "$ESTADO"
exit 0
