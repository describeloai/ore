#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LO QUE EL COPIADOR TIENE LISTO ANTES DE QUE LLEGUE UNA PASADA (0073 §9, fase H)
#
#   copiar-mantener.sh [testigos]
#
# Lo lanza `ore-copiador` al arrancar, antes de escuchar, y cada 4 min, para que
# una pasada sea el trabajo y no el montaje (0073 P8). Medido en victor el
# 2026-10-27: renovar los testigos dentro de la pasada costaba ~7,5 s (tres
# `gcloud secrets versions access` de ~1,6 s y lo que tardan en arrancar).
#
#   1. H1 · los testigos de la forja y del agente, en `$PUESTO` (memoria).
#
# Con `testigos`, sólo eso: es lo que hace la pasada si `ore-copiador` no los
# tiene al día. Sale con 0 si todo quedó listo; si no, dice qué y sale con 1.
# ⛔ Ningún valor sale a la salida.
# ═══════════════════════════════════════════════════════════════════════════
set -u

PUESTO=${PUESTO:-/puesto}
: "${SECRETOS:?falta SECRETOS}"

# ── 1 · los testigos ────────────────────────────────────────────────────────
mkdir -p "$PUESTO"
leer() {
  gcloud secrets versions access latest --secret="$SECRETOS-$1" --out-file="$PUESTO/$2.nuevo" 2>/dev/null \
    && chmod 0400 "$PUESTO/$2.nuevo"
}
if ! { leer forja-token forja && leer agente-cliente agente-cliente && leer agente-secreto agente-secreto; }; then
  rm -f "$PUESTO/forja.nuevo" "$PUESTO/agente-cliente.nuevo" "$PUESTO/agente-secreto.nuevo"
  echo "no se pudieron leer los testigos de la forja y del agente"
  exit 1
fi
for f in forja agente-cliente agente-secreto; do mv -f "$PUESTO/$f.nuevo" "$PUESTO/$f"; done
echo "testigos renovados"
[ "${1:-}" = testigos ] && exit 0
exit 0
