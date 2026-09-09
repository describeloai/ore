#!/usr/bin/env bash
# LO QUE HAY Y LO QUE SE DECLARÓ, ¿son lo mismo?
#
# E2 de la `0022`. Renderiza los manifiestos de un inquilino y los compara con lo
# que hay VIVO en el clúster. Cero diferencias ⇒ el repositorio de instancia dice
# la verdad, y un agente de GitOps no tendría nada que corregir.
#
#   bash pruebas-de-fuego/la-deriva-del-inquilino.sh [nombre]
#
# ── ⭐ Por qué `kubectl diff` y no una comparación nuestra ──────────────────
#
# Porque la hace el SERVIDOR. Un `get` y un `diff` de texto compararían lo que
# escribimos con lo que el API devuelve —con sus valores por defecto, su
# `status`, sus `managedFields`— y habría que ir tachando ruido a mano hasta que
# la prueba dejara de decir nada. `kubectl diff` pregunta *«¿qué cambiaría si
# aplicase esto?»*, que es exactamente la pregunta.
#
# ── ⛔⛔ LA EXCEPCIÓN, Y ES UN HALLAZGO, NO UNA CONCESIÓN ───────────────────
#
# Hay UN objeto que siempre difiere: el `ConfigMap` `jwks`. El manifiesto declara
# la semilla —`{"keys":[]}`— y lo vivo lleva las llaves del realm, que escribe el
# `CronJob` de `50-jwks.yaml` en cada refresco.
#
# ⇒ Y eso no es una molestia de esta prueba: **es un choque con GitOps**. Un
#   agente que reconciliase ese objeto devolvería el juego a vacío cada pocos
#   minutos, y `ore-serve` dejaría de validar tokens en su siguiente arranque —
#   sin que nada fallara mientras tanto.
#
# ✔ 2026-09-09 · arreglado en la E3: ese `ConfigMap` lleva la anotación
# `kustomize.toolkit.fluxcd.io/ssa: IfNotPresent`, que le dice a Flux que lo cree
# si no está y **no lo toque nunca más**. La documentación de Flux nombra este
# caso con estas palabras: *«Flux crea los recursos con campos que otros
# controladores mutan después»*.
#
# ⇒ Aquí sigue exento porque el manifiesto declara la semilla y lo vivo lleva las
#   llaves: son distintos y tienen que serlo. Lo que cambió es que ya no es una
#   trampa esperando a que llegue un agente — el agente ya sabe.
#
# Un objeto EXENTO y nombrado es una decisión; una prueba que tolerase «alguna
# diferencia» no sería una prueba.
#
# ⭐ Y desde que Flux reconcilia, esta prueba cambia de significado: ya no busca
# a alguien que tocó el clúster a mano —eso lo corrige el agente solo— sino que
# comprueba que **la plantilla y lo que el agente aplica siguen diciendo lo
# mismo**. El día que el repositorio del inquilino se quede viejo respecto a la
# plantilla, esto se pone rojo y el agente no, porque el agente obedece al
# repositorio y no sabe de qué plantilla salió.
#
# ⚠️ Y la exención es sólo para ESE objeto. Si mañana difiere otro, esto se pone
#    rojo — que es justo lo que tiene que pasar.
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
NOMBRE="${1:-demo}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

falla() { echo "✗ $*" >&2; exit 1; }
dice()  { echo "  · $*"; }

command -v kubectl >/dev/null || falla "hace falta kubectl"
command -v python3 >/dev/null || PY=python || PY=python3
PY="$(command -v python3 || command -v python)"

# ⚠️ En Git Bash sobre Windows, `python` es el de Windows y no entiende `/c/ORE`.
#    Se traduce con `cygpath` si está; en Linux no está y no hace falta.
ruta() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else echo "$1"; fi; }

(cd "$RAIZ" && "$PY" malla/gen-inquilino.py "$NOMBRE" --a "$(ruta "$TMP/rendido")") > /dev/null \
  || falla "no se pudo renderizar \`$NOMBRE\`"
dice "renderizado \`$NOMBRE\` desde la plantilla"

# `kubectl diff` sale con 1 cuando HAY diferencias y con >1 cuando falla de
# verdad. Distinguirlo importa: un clúster inalcanzable no es «no hay deriva».
kubectl diff -f "$(ruta "$TMP/rendido")" > "$TMP/diff" 2>"$TMP/err"
CODIGO=$?
[ "$CODIGO" -le 1 ] || falla "\`kubectl diff\` no pudo correr: $(tail -3 "$TMP/err")"

# Qué OBJETOS difieren, no cuántas líneas. Una línea de más en un secreto y una
# política de red que falta pesan igual en un `wc -l` y no pesan igual aquí.
grep '^diff -u' "$TMP/diff" \
  | sed 's/.*LIVE-[0-9]*[\/\\]//; s/[[:space:]].*//' \
  | sort -u > "$TMP/objetos"

EXENTO="v1.ConfigMap.t-$NOMBRE.jwks"
INESPERADOS="$(grep -v -x -F "$EXENTO" "$TMP/objetos" || true)"

if [ -s "$TMP/objetos" ]; then
  while read -r O; do
    [ -n "$O" ] || continue
    if [ "$O" = "$EXENTO" ]; then
      dice "difiere \`$O\` — EXENTO: la semilla contra las llaves que trae el CronJob"
    else
      echo "  ⛔ difiere \`$O\`"
    fi
  done < "$TMP/objetos"
fi

if [ -n "$INESPERADOS" ]; then
  echo
  echo "⛔ LO QUE HAY EN EL CLUSTER NO ES LO QUE DICE LA PLANTILLA."
  echo "   O alguien lo cambio a mano, o la plantilla se quedo vieja. Las dos"
  echo "   se arreglan distinto, y por eso esto no dice cual es."
  echo
  sed -n '1,60p' "$TMP/diff"
  exit 1
fi

echo
echo "✓ el inquilino \`$NOMBRE\` es lo que la plantilla dice que es"
