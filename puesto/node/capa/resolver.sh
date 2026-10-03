#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# RESOLVER LA CAPA DE NODE (ADR 0050 R3 T5b) — dentro de `capa-node:1`
#
# Lo llama `57-la-capa-node.yaml`. Espera el árbol ya clonado y deja, en
# `$TRABAJO`: `deps.txt`, `digest.txt`, `capa.tgz` y `informe.json`. No sube
# nada y no toca git: eso lo hace el contenedor que tiene la nube y el
# testigo, y esta imagen no los tiene A PROPÓSITO —lo único que necesita es
# salir al registro de npm—.
#
# ⚠️ NO FALLA NUNCA (sale 0). Si npm no puede resolver, el informe dice
#   `estado: error` con el motivo, que es lo que la consola sabe enseñar.
#
#   $1  el árbol clonado        $2  el alcance (o vacío: la celda)
# ═══════════════════════════════════════════════════════════════════════════
set -eu
ARBOL="$1"
ALCANCE="${2:-}"
TRABAJO="${TRABAJO:-/trabajo}"
CAPA="${CAPA:-}"
mkdir -p "$TRABAJO"

# ── 1 · la declaración: los package.json del alcance, como los lee `ore-serve` ──
node /opt/ore/capa.mjs declarar "$ARBOL" "$ALCANCE" "$TRABAJO"
DIGEST=$(cat "$TRABAJO/digest.txt")
if [ -z "$DIGEST" ]; then
  echo "· el árbol no declara dependencias de Node: nada que resolver"
  exit 0
fi
if [ -n "$CAPA" ] && [ "$DIGEST" != "$CAPA" ]; then
  echo "⚠️ se pidió $CAPA y el árbol declara $DIGEST (cambió entre medias): se resuelve lo que hay"
fi

# ── 2 · lo que npm resuelve: lo declarado, sin lo que la sesión ya trae ────
node /opt/ore/capa.mjs package "$TRABAJO" /opt/ore/provisto.txt
mkdir -p "$TRABAJO/capa"
cp "$TRABAJO/package.json" "$TRABAJO/capa/package.json"

# ── 3 · resolver ───────────────────────────────────────────────────────────
# `--ignore-scripts` ES LA DECISIÓN (T5b): ningún paquete ejecuta su código al
#   instalarse —ni `preinstall` ni `postinstall`—. Un Job con red no corre
#   código del registro; un paquete que compila nativo no funcionará, y el
#   informe lo dice.
# `--omit=dev`   lo que la sesión necesita para correr, y nada más.
# La caché de npm vive EN EL JOB y muere con él, como el repositorio de Maven.
ESTADO=lista
if ( cd "$TRABAJO/capa" && npm install --omit=dev --ignore-scripts --no-audit --no-fund \
       --no-update-notifier --cache "$TRABAJO/npm-cache" > "$TRABAJO/npm.log" 2>&1 ); then
  if [ -d "$TRABAJO/capa/node_modules" ] && [ -n "$(ls -A "$TRABAJO/capa/node_modules")" ]; then
    # La caja: `node_modules` y el lock, y nada más. Fechas y dueños fijos para
    # que la misma resolución dé la misma caja.
    tar --sort=name --mtime='2026-01-01 00:00Z' --owner=0 --group=0 --numeric-owner \
        -czf "$TRABAJO/capa.tgz" -C "$TRABAJO/capa" node_modules package-lock.json
  else
    echo "### nada que instalar: todo lo declarado lo pone ya la sesión"
  fi
else
  ESTADO=error
  echo "✗ npm no pudo resolver: $(tail -c 600 "$TRABAJO/npm.log")"
fi

# ── 4 · el informe: lo resuelto, su suma y sus avisos ──────────────────────
node /opt/ore/capa.mjs informe "$TRABAJO" "$ESTADO"
exit 0
