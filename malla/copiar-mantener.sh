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
#   2. H2 · un espejo del árbol (`$TRABAJO/espejo`) al día: cada pasada saca de él
#      su `git worktree` en ~0,5 s, en vez de clonar en 3,3 s. Lo que toca el
#      espejo (fetch, worktree) va bajo `flock $TRABAJO/espejo.lock`.
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
# ⭐ 0079 ·5g·1c · Y la URL del catálogo, si la celda vive en él (`ORE_CATALOGO_ESQUEMA`, del
#   render): la lee el bloque de `copiar-worker.sh`. Sin ella, la pasada falla; nunca vuelve a
#   los ficheros.
if [ -n "${ORE_CATALOGO_ESQUEMA:-}" ]; then
  leer catalogo catalogo && mv -f "$PUESTO/catalogo.nuevo" "$PUESTO/catalogo" \
    || { rm -f "$PUESTO/catalogo.nuevo"; echo "no se pudo leer la URL del catálogo"; exit 1; }
fi
echo "testigos renovados"
[ "${1:-}" = testigos ] && exit 0

# ── 2 · el espejo del árbol ─────────────────────────────────────────────────
TRABAJO=${TRABAJO:-/trabajo}
ESPEJO="$TRABAJO/espejo"
: "${FORJA_URL:?falta FORJA_URL}"
export GIT_TERMINAL_PROMPT=0 GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader \
  GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*'
export GIT_CONFIG_VALUE_0="Authorization: token $(cat "$PUESTO/forja")"
mkdir -p "$TRABAJO"
if [ -d "$ESPEJO/.git" ]; then
  # Y los worktrees de pasadas que murieron a medias, fuera.
  flock "$ESPEJO.lock" sh -c 'git -C "$1" fetch -q --prune origin && git -C "$1" worktree prune' _ "$ESPEJO" \
    || { echo "no se pudo traer el árbol al espejo"; exit 1; }
  echo "espejo al día"
else
  # Sin `checkout`: nadie trabaja en el espejo, sólo en sus worktrees.
  rm -rf "$ESPEJO.nuevo"
  git clone -q --no-checkout "$FORJA_URL" "$ESPEJO.nuevo" 2>&1 && mv "$ESPEJO.nuevo" "$ESPEJO" \
    || { rm -rf "$ESPEJO.nuevo"; echo "no se pudo clonar el espejo del árbol"; exit 1; }
  echo "espejo clonado"
fi
