#!/usr/bin/env bash
# ROTAR LA BASE DEL COFRE DE UNA CELDA — sin dejar a nadie fuera (0054 I3)
#
#   bash malla/rotar-base-del-cofre.sh <celda> [--seco]
#
# Lo corre UNA PERSONA, con el cluster y la nube en la mano. No es un paso del
# aprovisionador, a proposito: converger solo crea (0054 I2), y rotar es un
# acto que alguien decide.
#
# ── ⭐⭐ LA INVARIANTE: la version viva del secreto abre la base, SIEMPRE ────
#
# Cada celda tiene dos papeles alternos (`cofre_<c>` y `cofre_<c>_b`, migracion
# 049). Uno es el vigente —el que dice el secreto—; rotar prepara el OTRO:
#
#   paso                                       si se corta aqui
#   ① preparar: clave nueva al no vigente      el vigente sigue; el secreto, igual
#   ② comprobar que la clave nueva entra       igual
#   ③ version nueva del secreto                entran las dos: el custodio, con cualquiera
#   ④ el custodio arranca con ella             igual
#   ⑤ confirmar: solo si el papel nuevo YA     —
#      tiene sesion abierta; el otro, sin login
#   ⑥ las versiones viejas, deshabilitadas     solo higiene
#
# La base hace cumplir lo que importa: `preparar` NO PUEDE tocar el vigente, y
# `confirmar` se niega si nadie ha entrado con el nuevo.
#
# ⛔ La clave no pasa por la salida, ni por `argv` (tampoco el de `kubectl
#   exec`: la comprobacion la lee de la entrada estandar del pod), ni por la
#   huella. Vive en un fichero 0600 de un directorio temporal que se borra.
#
# ⚠️ Mientras el custodio sea `Recreate` (41-el-cofre.yaml), ④ lo deja unos
#   segundos sin servir. 0054 I5 lo pasa a `RollingUpdate`.
set -u

CELDA="${1:-}"
SECO=""
for a in "$@"; do [ "$a" = "--seco" ] && SECO="1"; done
[ -n "$CELDA" ] && [ "${CELDA#--}" = "$CELDA" ] || {
  echo "uso: bash malla/rotar-base-del-cofre.sh <celda> [--seco]" >&2
  exit 64
}
printf %s "$CELDA" | grep -qE '^[a-z0-9][a-z0-9-]{0,40}$' || { echo "✗ celda con forma rara: $CELDA" >&2; exit 64; }

NS="t-$CELDA"
SECRETO="$NS-base-del-cofre"
KUBECTL="${KUBECTL:-kubectl}"
GCLOUD="$(command -v gcloud.cmd || command -v gcloud)" || { echo "✗ hace falta gcloud" >&2; exit 1; }
TMP="$(mktemp -d)"
chmod 700 "$TMP"
trap 'rm -rf "$TMP"' EXIT

falla() { echo "✗ $*" >&2; exit 1; }
paso()  { echo; echo "── $* ──"; }
hecho() { echo "  ✓ $*"; }
ruta()  { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }

# Las tres respuestas, como en `aprovisionar-inquilino.sh` (0054 I1).
preguntar() { # <orden…> → 0 contesto · 1 no existe · 2 no se sabe
  RESPUESTA=""; NO_SE=""
  if RESPUESTA=$("$@" 2>"$TMP/pregunta.err"); then
    RESPUESTA=$(printf '%s' "$RESPUESTA" | tr -d '\r')
    return 0
  fi
  RESPUESTA=""
  grep -qE 'NOT_FOUND|HTTPError 404|not found: 404' "$TMP/pregunta.err" && return 1
  NO_SE=$(tr '\r\n' '  ' < "$TMP/pregunta.err" | cut -c1-240)
  return 2
}
# SQL como el dueño de la base. La celda va validada arriba: no hay comillas.
sql() { $KUBECTL exec -n identidad idp-db-0 -- psql -U keycloak -d iam -v ON_ERROR_STOP=1 -tAc "$1" 2>"$TMP/sql.err" | tr -d '\r'; }

echo "ROTAR LA BASE DEL COFRE DE \`$CELDA\`${SECO:+   (EN SECO: no se toca nada)}"

# ══════════════════════════════════════════════════════════════════════════
paso "⓪ ANTES DE TOCAR NADA — lo que tiene que ser verdad"
preguntar "$GCLOUD" secrets describe "$SECRETO" --format="value(name)"
case $? in
  0) hecho "el secreto $SECRETO existe" ;;
  1) falla "$SECRETO no existe: eso es CREAR, y lo hace el aprovisionador (0054 I2)" ;;
  *) falla "no se sabe si $SECRETO existe ($NO_SE)" ;;
esac
VIGENTE=$(sql "select p.papel from iam.papel_de_celda p join iam.celda c on c.id = p.celda where c.nombre = '$CELDA' and c.estado <> 'retirada' and p.vigente")
[ -n "$VIGENTE" ] || falla "la celda no tiene papel vigente en la base: $(head -c 300 "$TMP/sql.err")"
hecho "papel vigente: $VIGENTE"
ORG=$(sql "select organizacion from iam.celda where nombre = '$CELDA' and estado <> 'retirada'")
[ -n "$ORG" ] || falla "no se leyo la organizacion de la celda: $(head -c 300 "$TMP/sql.err")"
$KUBECTL -n "$NS" get deployment ore-cofre -o name >/dev/null 2>"$TMP/k.err" \
  || falla "no se ve el custodio de $NS: $(head -c 300 "$TMP/k.err")"
hecho "el custodio de $NS existe"
if [ -n "$SECO" ]; then
  echo
  echo "  ~ prepararia el papel que no es $VIGENTE, comprobaria que entra, lo guardaria en"
  echo "    $SECRETO, reiniciaria ore-cofre en $NS y confirmaria cuando entre con el."
  exit 0
fi

# ══════════════════════════════════════════════════════════════════════════
paso "① PREPARAR — clave nueva al papel que NO es el vigente"
PAPEL=$(sql "select iam.preparar_papel_de_celda('$CELDA')" | tr -d ' \n')
printf %s "$PAPEL" | grep -qE '^cofre_[a-z0-9_]+:[0-9a-f]{64}$' \
  || falla "preparar no devolvio papel:clave ($(head -c 300 "$TMP/sql.err")). Nada cambio para el custodio: el vigente sigue."
NUEVO="${PAPEL%%:*}"
[ "$NUEVO" != "$VIGENTE" ] || falla "preparar devolvio el vigente: eso no puede pasar (049)"
umask 077
printf 'postgres://%s@idp-db.identidad.svc.cluster.local:5432/iam' "$PAPEL" > "$TMP/url"
printf 'postgres://%s@127.0.0.1:5432/iam\n' "$PAPEL" > "$TMP/url-local"
unset PAPEL
hecho "preparado $NUEVO (la clave, en un fichero 0600; en ningun otro sitio)"

# ══════════════════════════════════════════════════════════════════════════
paso "② COMPROBAR — la clave nueva entra, y sabe de que organizacion es"
# Por la entrada estandar del pod: ni `argv` ni el registro de auditoria.
LEIDA=$($KUBECTL exec -i -n identidad idp-db-0 -- sh -c 'read -r U; psql "$U" -tAc "select iam.mi_organizacion()"' \
  < "$TMP/url-local" 2>"$TMP/k.err" | tr -d ' \r\n')
[ "$LEIDA" = "$ORG" ] \
  || falla "la clave nueva no entra, o no sabe su organizacion ($(head -c 200 "$TMP/k.err")). El secreto no se toco: el vigente sigue."
hecho "$NUEVO entra, y es de $ORG"

# ══════════════════════════════════════════════════════════════════════════
paso "③ GUARDAR — version nueva de $SECRETO"
VERSION=$("$GCLOUD" secrets versions add "$SECRETO" --data-file="$(ruta "$TMP/url")" --format="value(name)" 2>"$TMP/g.err" | tr -d '\r')
[ -n "$VERSION" ] \
  || falla "no se guardo ($(head -c 300 "$TMP/g.err")). El secreto sigue con la de antes y el vigente entra: repetir este guion."
rm -f "$TMP/url" "$TMP/url-local"
hecho "guardada: $VERSION"

# ══════════════════════════════════════════════════════════════════════════
paso "④ EL CUSTODIO — arranca con la version nueva"
# ⛔ Borrar el POD, no `rollout restart`: esa orden anota la plantilla del
#   Deployment, Flux quita la anotacion en su siguiente pase y el custodio se
#   reinicia OTRA vez (medido el 2026-10-05 en victor). El pod nuevo trae la
#   version `latest` del secreto en su init, igual.
$KUBECTL -n "$NS" delete pod -l ore.dev/rol=cofre --wait=true >/dev/null 2>"$TMP/k.err" \
  || falla "no se pudo reiniciar el custodio ($(head -c 300 "$TMP/k.err")). Las dos claves entran: nada roto; reiniciarlo y confirmar."
LISTO=""
for _ in $(seq 1 60); do
  [ "$($KUBECTL -n "$NS" get deployment ore-cofre -o jsonpath='{.status.readyReplicas}' 2>/dev/null)" = "1" ] && { LISTO=1; break; }
  sleep 5
done
[ -n "$LISTO" ] || falla "el custodio no quedo listo en 5 min. Las dos claves entran: mirarlo antes de confirmar."
hecho "ore-cofre de $NS, listo"

# ══════════════════════════════════════════════════════════════════════════
paso "⑤ CONFIRMAR — sólo si el custodio ya entra con $NUEVO"
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
  R=$(sql "select iam.confirmar_papel_de_celda('$CELDA', '$NUEVO')" | tr -d ' \n')
  [ "$R" = "$NUEVO" ] && break
  grep -q "nadie ha entrado" "$TMP/sql.err" || falla "confirmar fallo: $(head -c 300 "$TMP/sql.err")"
  sleep 5
done
[ "$R" = "$NUEVO" ] || falla "el custodio no entro con $NUEVO en un minuto: no se confirma (el vigente sigue siendo $VIGENTE y entra)."
hecho "vigente: $NUEVO · $VIGENTE, sin login"

# ══════════════════════════════════════════════════════════════════════════
paso "⑥ HIGIENE — las versiones viejas del secreto, deshabilitadas"
if preguntar "$GCLOUD" secrets versions list "$SECRETO" --filter=state:enabled --format="value(name)"; then
  for V in $RESPUESTA; do
    [ "${V##*/}" = "${VERSION##*/}" ] && continue
    "$GCLOUD" secrets versions disable "${V##*/}" --secret="$SECRETO" >/dev/null 2>"$TMP/g.err" \
      && hecho "version ${V##*/} deshabilitada" \
      || echo "  ⚠ la version ${V##*/} no se deshabilito ($(head -c 200 "$TMP/g.err")): su clave ya no entra igualmente"
  done
else
  echo "  ⚠ no se pudieron listar las versiones ($NO_SE): su clave vieja ya no entra igualmente"
fi

echo
echo "✓ \`$CELDA\`: el custodio entra con $NUEVO y $SECRETO lo dice (0054 I3)"
