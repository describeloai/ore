#!/usr/bin/env bash
# CONVERGER SIN ROMPER (0054 I1) — lo que corre solo no confunde «no lo sé»
# con «no existe».
#
#   bash pruebas-de-fuego/converger-sin-romper.sh
#
# El 2026-10-04 una pasada del aprovisionador pregunto a gcloud con
# `2>/dev/null`, la respuesta vacia se leyo como «no hay version», y roto la
# clave de dos custodios sin guardarla. Esta prueba fija las dos mitades:
#
#   ① que la forma no vuelva: en los guiones que corren SOLOS, ninguna llamada
#     a gcloud tira su error (`2>/dev/null`, `2>&1`, `|| true`);
#   ② que `preguntar` distinga las tres respuestas — y en especial que un
#     fallo de credencial que dice «not found» NO sea «no existe».
#
# Sin red, sin nube y sin base: se ejecuta en cualquier sitio.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }

# ── ① la forma, prohibida ─────────────────────────────────────────────────
#
# Por LINEA LOGICA: las continuaciones (`\` al final) se juntan, porque un
# `2>&1 || true` dos lineas por debajo del `"$GCLOUD"` es la misma llamada.
# Los comentarios no cuentan.
SOLOS="malla/aprovisionar-inquilino.sh malla/converger-inquilinos.sh malla/rotar-base-del-cofre.sh"
for f in $SOLOS; do
  [ -f "$RAIZ/$f" ] || { falla "$f no existe"; continue; }
  HALLADO=$(awk '
    { linea = linea $0; if (sub(/\\$/, "", linea)) { if (!inicio) inicio = NR; next } }
    {
      n = inicio ? inicio : NR; inicio = 0
      l = linea; linea = ""
      if (l ~ /^[ \t]*#/) next
      if (l ~ /("\$GCLOUD"|(^|[ ;(])gcloud )/ && l ~ /(2>\/dev\/null|2>&1|\|\| *true)/) print n ": " substr(l, 1, 140)
    }' "$RAIZ/$f")
  if [ -n "$HALLADO" ]; then
    falla "$f tira el error de gcloud (0054 I1):"
    printf '%s\n' "$HALLADO" | sed 's/^/        /'
  else
    dice "$f: ninguna llamada a gcloud tira su error"
  fi
done

# Y una cuenta de servicio se pregunta listando: por una que no existe, Google
# le dice al aprovisionador PERMISSION_DENIED, y `describe` no sabria nunca.
for f in $SOLOS; do
  if grep -nE '^[^#]*service-accounts describe' "$RAIZ/$f" >/dev/null 2>&1; then
    falla "$f pregunta por una cuenta con \`describe\`: es \`preguntar_cuenta\` (lista)"
  fi
done
dice "las cuentas de servicio se preguntan listando"

# Y la funcion que rotaba sin querer no vuelve a llamarse desde ningun sitio.
if grep -rn "dar_papel_de_celda(" "$RAIZ/malla" "$RAIZ/crates" --include=*.sh --include=*.rs --include=*.py 2>/dev/null \
    | awk -F: '$3 !~ /^[ \t]*(#|--|\/\/)/' | grep -q .; then
  falla "alguien llama todavia a iam.dar_papel_de_celda (049 la retiro)"
else
  dice "nadie llama a iam.dar_papel_de_celda"
fi

# ── ② las tres respuestas ─────────────────────────────────────────────────
#
# Las funciones se sacan del guion tal cual (no una copia que pueda divergir).
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT

# Ordenes de mentira: lo que dice gcloud, medido recurso por recurso (0054).
contesta()      { echo "projects/1/secrets/x/versions/3"; }
contesta_nada() { :; }
no_existe_sm()  { echo "ERROR: (gcloud.secrets.versions.list) NOT_FOUND: Secret [projects/1/secrets/x] not found." >&2; return 1; }
no_existe_gcs() { echo "ERROR: (gcloud.storage.buckets.describe) gs://x not found: 404." >&2; return 1; }
no_existe_dns() { echo "ERROR: (gcloud.dns.record-sets.describe) HTTPError 404: The 'parameters.name' resource named 'x.' does not exist." >&2; return 1; }
sin_permiso()   { echo "ERROR: (gcloud.iam.roles.describe) PERMISSION_DENIED: Permission 'iam.roles.get' denied on resource" >&2; return 1; }
sin_red()       { echo "ERROR: (gcloud.secrets.versions.list) There was a problem refreshing your current auth tokens: connection reset" >&2; return 1; }
sin_credencial(){ echo "ERROR: Your default credentials were not found." >&2; return 1; }
lista_con()     { echo "x@y"; }
lista_sin()     { :; }
lista_rota()    { echo "ERROR: (gcloud.iam.service-accounts.list) PERMISSION_DENIED: denied" >&2; return 1; }

caso() { # <esperado> <orden>
  preguntar "$2"; local r=$?
  [ "$r" = "$1" ] && dice "$G · $2 → $r" || falla "$G · $2 → $r, y tenia que ser $1"
}
# Las funciones se sacan de cada guion tal cual (no una copia que pueda divergir).
for G in malla/aprovisionar-inquilino.sh malla/rotar-base-del-cofre.sh; do
  unset -f preguntar duda 2>/dev/null
  eval "$(sed -n '/^preguntar() {/,/^}/p; /^duda() {/,/^}/p' "$RAIZ/$G")"
  command -v preguntar >/dev/null || { falla "no se encontro preguntar() en $G"; continue; }
  caso 0 contesta
  [ "$RESPUESTA" = "projects/1/secrets/x/versions/3" ] || falla "$G · la salida no quedo en \$RESPUESTA"
  caso 0 contesta_nada
  [ -z "$RESPUESTA" ] || falla "$G · una respuesta vacia tiene que quedar vacia"
  caso 1 no_existe_sm
  caso 1 no_existe_gcs
  caso 1 no_existe_dns
  caso 2 sin_permiso
  caso 2 sin_red
  # ⛔ La trampa: «not found» sin NOT_FOUND ni 404 es una credencial, no un recurso.
  caso 2 sin_credencial
done
G=malla/aprovisionar-inquilino.sh
unset -f preguntar duda
eval "$(sed -n '/^preguntar() {/,/^}/p; /^duda() {/,/^}/p' "$RAIZ/$G")"
eval "$(sed -n '/^preguntar_cuenta() {/,/^}/p' "$RAIZ/$G")"
# Lo que dice la lista: una linea si existe, nada si no, y un error si no se sabe.
GCLOUD=lista_con;  preguntar_cuenta x@y; r=$?; [ "$r" = 0 ] && [ "$RESPUESTA" = "x@y" ] && dice "cuenta listada → 0" || falla "cuenta listada → $r"
GCLOUD=lista_sin;  preguntar_cuenta x@y; r=$?; [ "$r" = 1 ] && dice "lista vacia → 1 (no existe)" || falla "lista vacia → $r"
GCLOUD=lista_rota; preguntar_cuenta x@y; r=$?; [ "$r" = 2 ] && dice "lista que falla → 2" || falla "lista que falla → $r"
DUDAS=""
preguntar sin_red; duda "lo de prueba" 2>/dev/null
[ -n "$DUDAS" ] && dice "duda deja la pasada en rojo" || falla "duda no anoto nada"

echo
[ "$MAL" = 0 ] && echo "✓ converger sin romper (0054 I1)" || { echo "✗ converger sin romper (0054 I1)"; exit 1; }
