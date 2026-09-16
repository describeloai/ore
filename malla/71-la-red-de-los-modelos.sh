#!/usr/bin/env bash
# LA RED DE LOS MODELOS (0027 ④, E2): la otra mitad de la regla de clase.
#
# ── Por qué esto es un guion y no un manifiesto ─────────────────────────────
# La `NetworkPolicy` `salida-al-modelo` (11) dice que un Job de la celda puede
# salir a MODELOS:8000 y `ore-serve` a MODELOS:9000. Eso es la mitad que el
# clúster obedece. La otra mitad la obedece la VPC: que una máquina que NO es
# del clúster acepte lo que le llega de los pods, y de nadie más. Son objetos
# de Google —una reserva de IP, dos reglas de firewall— y se escriben con
# `gcloud`, como el papel del aprovisionador o el KMS: aquí, con nombre, y
# comprobables, en vez de en un historial de comandos de alguien.
#
# ── Lo que hay, y lo que NO se crea aquí ────────────────────────────────────
#   la reserva `modelos`        10.10.0.100 en `ore-mesh-europe-west1`: la
#                               constante MODELOS de `gen-inquilino.py`. Una
#                               NetworkPolicy no sabe de nombres, y una IP que
#                               cambiara sin que 11 cambiase sería una regla
#                               que ya no abre nada
#   `ore-modelos-desde-la-malla` pods (10.20.0.0/16) y nodos (10.10.0.0/20) →
#                               tag `modelos`, tcp 8000 (datos) y 9000 (control)
#   `ore-modelos-iap-ssh`       35.235.240.0/20 → tag `modelos`, tcp 22: ssh
#                               por IAP a una máquina sin IP pública
#   la máquina                  NO se crea aquí. Es de Bastion (B1/B3: hoy
#                               `modelos-e0`, una e2-micro; mañana un G4 con
#                               etiqueta eu-dc). Aquí solo se comprueba que la
#                               que use la IP lleve el tag, porque sin él la
#                               firewall no la alcanza y el síntoma es un
#                               timeout que manda a mirar la NetworkPolicy.
#
# Medido antes de escribirlo (E0, 2026-09-16): sin la firewall los pods no
# llegan; con ella y sin abrir INPUT en la máquina (COS tira lo que entra)
# tampoco — esa parte es del arranque de la máquina, en Bastion.
#
# Uso:  bash malla/71-la-red-de-los-modelos.sh [--seco]
set -u

PROYECTO="project-8853a180-450d-47be-b83"
LUGAR="europe-west1"
RED="ore-mesh"
SUBRED="ore-mesh-europe-west1"
MODELOS="10.10.0.100"            # = gen-inquilino.py::MODELOS, cotejado abajo
PODS="10.20.0.0/16"
NODOS="10.10.0.0/20"
IAP="35.235.240.0/20"
TAG="modelos"

SECO=""; [ "${1:-}" = "--seco" ] && SECO="1"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
GCLOUD=$(command -v gcloud.cmd || command -v gcloud) || { echo "✗ hace falta gcloud" >&2; exit 1; }

hecho() { echo "  ✓ $*"; }
ya()    { echo "  · $* — ya estaba"; }
haria() { echo "  ~ $*"; }
falla() { echo "✗ $*" >&2; exit 1; }
g() { "$GCLOUD" "$@" --project "$PROYECTO" --quiet 2>/dev/null; }

echo "LA RED DE LOS MODELOS${SECO:+   (EN SECO: no se escribe nada)}"

# ── 0 · la constante es la misma que la plantilla ───────────────────────────
EN_GEN=$(grep -o 'MODELOS = "[0-9.]*"' "$RAIZ/malla/gen-inquilino.py" | grep -o '[0-9.]*')
[ "$EN_GEN" = "$MODELOS" ] || falla "MODELOS aqui es $MODELOS y en gen-inquilino.py es $EN_GEN: una regla que no abre nada"
hecho "MODELOS = $MODELOS, la misma en 11, 40 y aqui"

# ── 1 · la reserva ──────────────────────────────────────────────────────────
TIENE=$(g compute addresses describe modelos --region "$LUGAR" --format='value(address)')
if [ "$TIENE" = "$MODELOS" ]; then
  ya "la reserva modelos = $MODELOS"
elif [ -n "$TIENE" ]; then
  falla "la reserva modelos es $TIENE y no $MODELOS: cambia la constante o la reserva, y las dos a la vez"
elif [ -n "$SECO" ]; then
  haria "reservar $MODELOS en $SUBRED como modelos"
else
  g compute addresses create modelos --region "$LUGAR" --subnet "$SUBRED" --addresses "$MODELOS" \
    --description "0027 (4): la puerta a los modelos, el gateway de Bastion. Constante MODELOS de la regla de clase" \
    && hecho "reserva modelos = $MODELOS" || falla "no se pudo reservar $MODELOS"
fi

# ── 2 · las dos reglas de firewall ──────────────────────────────────────────
firewall() { # <nombre> <origenes> <permitido> <que es>
  local N="$1" DE="$2" PERMITE="$3" QUE="$4" VIVO
  VIVO=$(g compute firewall-rules describe "$N" --format='value(sourceRanges.list(),allowed[].map().firewall_rule().list(),targetTags.list())' | tr -d ' ')
  if [ -n "$VIVO" ]; then
    # lo que hay tiene que ser exactamente esto: mas ancho seria abrir la maquina a mas
    case "$VIVO" in
      *"$TAG"*) ya "$N ($QUE)" ;;
      *) falla "$N existe y no lleva el tag $TAG: $VIVO" ;;
    esac
  elif [ -n "$SECO" ]; then
    haria "crear $N: $DE → tag $TAG, $PERMITE"
  else
    g compute firewall-rules create "$N" --network "$RED" --direction INGRESS \
      --source-ranges "$DE" --allow "$PERMITE" --target-tags "$TAG" --description "$QUE" \
      && hecho "$N: $DE → tag $TAG, $PERMITE" || falla "no se pudo crear $N"
  fi
}
firewall ore-modelos-desde-la-malla "$PODS,$NODOS" "tcp:8000,tcp:9000" "pods y nodos de ore-mesh alcanzan el gateway de modelos: 8000 datos, 9000 control (0027 4)"
firewall ore-modelos-iap-ssh        "$IAP"         "tcp:22"            "ssh por IAP a la maquina de modelos, que no tiene IP publica"

# ── 3 · la maquina que usa la IP lleva el tag (se mira, no se crea) ─────────
MAQ=$(g compute instances list --filter="networkInterfaces.networkIP=$MODELOS" --format='value(name,status,tags.items)')
if [ -z "$MAQ" ]; then
  echo "  ⚠ ninguna maquina usa $MODELOS: la regla de clase abre una puerta que hoy no contesta (Bastion la pone: modelos-e0 o un G4)"
else
  case "$MAQ" in
    *"$TAG"*) hecho "la maquina en $MODELOS: $(echo "$MAQ" | tr '\t' ' ')" ;;
    *) falla "la maquina en $MODELOS no lleva el tag $TAG: la firewall no la alcanza ($MAQ)" ;;
  esac
fi

echo "✓ la red de los modelos${SECO:+ (en seco)}"
