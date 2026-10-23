#!/usr/bin/env bash
# Query Q4·4 / Q5·0 · Encender y apagar los trabajadores de Query en victor, SÓLO mientras se mide.
#
#   bash pruebas-de-fuego/query/q4/encender.sh estado
#   bash pruebas-de-fuego/query/q4/encender.sh encender [TECHO=2]     (en seco; APLICAR=1 para hacerlo)
#   bash pruebas-de-fuego/query/q4/encender.sh apagar                 (en seco; APLICAR=1 para hacerlo)
#
# Lo que hay en la malla es lo apagado (`58-la-consulta.yaml` con `replicas: 0`, `ore-serve` sin
# `--consultas`), y Flux lo devuelve a eso en cada vuelta. Encender para medir es, por eso:
#   1. comprobar que lo desplegado trae Q4 (y, con TECHO > 1, Q5·3: `--consultas-techo`) y que la
#      celda está convergida (la cuenta `consulta` y el Deployment existen; la cola trae su plantilla);
#   2. SUSPENDER la Kustomization `inquilino-victor` (nada más de la celda se aplica mientras tanto);
#   3. `ore-serve` con `--consultas` (y `--consultas-techo TECHO`), y el trabajador del suelo a 1.
# Y apagar es deshacerlo en el orden que no deja nada vivo:
#   1. REANUDAR la Kustomization y pedirle una vuelta ya: `ore-serve` vuelve sin `--consultas` (nadie
#      pide más trabajadores) y el Deployment a 0;
#   2. retirar de la cola los Jobs de quita y pon que queden (`empujar-plantilla.py --retirar-consultas`):
#      con el fichero en la cola, Flux volvería a crear el Job cada vez que su TTL lo borra;
#   3. borrar los Jobs de quita y pon que aún corran, y comprobar que no queda ningún pod de Query.
#
# ⚠️ Reiniciar `ore-serve` pierde lo que tiene en memoria (un Build en curso, las consultas de la cola):
#   `estado` dice si hay puestos o Builds corriendo antes de tocar nada.
set -uo pipefail
NS=t-victor
KS=inquilino-victor
aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/../../.." && pwd)"
QUE=${1:?estado|encender|apagar}; shift
for kv in "$@"; do export "$kv"; done
TECHO=${TECHO:-1}
APLICAR=${APLICAR:-}
Q4=c586b4cf   # Query Q4·1–Q4·3 en main
export MSYS_NO_PATHCONV=1

k() { kubectl -n $NS "$@"; }
hacer() {  # en seco lo dice; con APLICAR=1 lo hace
  if [ -n "$APLICAR" ]; then echo "  → $*"; "$@"; else echo "  (seco) $*"; fi
}
desplegado() { k get pods -l ore.dev/rol=control -o jsonpath='{.items[0].metadata.annotations.ore\.dev/commit}' 2>/dev/null; }
trae_el_techo() { k exec deploy/ore-serve -c ore-serve -- ore-serve --help 2>/dev/null | grep -q -- --consultas-techo; }
args() { k get deploy ore-serve -o jsonpath='{.spec.template.spec.containers[?(@.name=="ore-serve")].args}' 2>/dev/null; }
indice() { k get deploy ore-serve -o jsonpath='{range .spec.template.spec.containers[*]}{.name}{"\n"}{end}' | grep -n '^ore-serve$' | cut -d: -f1; }

estado() {
  local c; c=$(desplegado)
  echo "== victor · $(date -u +%H:%M:%SZ)"
  if [ -n "$c" ] && git -C "$raiz" merge-base --is-ancestor $Q4 "$c" 2>/dev/null; then
    echo "  ore-serve desplegado: ${c:0:8} · trae Q4 ✓"
  else
    echo "  ore-serve desplegado: ${c:0:8} · ⛔ NO trae Q4 ($Q4): esperar al despliegue (git fetch si es más nuevo que este clon)"
  fi
  if trae_el_techo; then echo "  --consultas-techo: lo trae (Q5·3) ✓"; else echo "  --consultas-techo: no (Q5·3 sin desplegar: sólo TECHO=1)"; fi
  echo "  Flux $KS: suspendida=$(kubectl -n flux-system get kustomization $KS -o jsonpath='{.spec.suspend}' 2>/dev/null || echo '?')"
  echo "  ore-serve args con consultas: $(args | tr ',' '\n' | grep -c consultas)"
  echo "  cuenta consulta: $(k get sa consulta -o name 2>/dev/null || echo '⛔ falta: converger la celda')"
  echo "  ore-consulta: $(k get deploy ore-consulta -o jsonpath='{.spec.replicas} réplicas, {.status.readyReplicas} listas' 2>/dev/null || echo '⛔ falta: converger la celda')"
  echo "  Jobs de quita y pon: $(k get jobs -l ore.dev/pieza=consulta --no-headers 2>/dev/null | wc -l)"
  echo "  pods de Query: $(k get pods -l ore.dev/pieza=consulta --no-headers 2>/dev/null | awk '{print $1" "$3}' | tr '\n' ' ')"
  echo "  puestos y Builds corriendo: $(k get pods -l ore.dev/rol=puesto --field-selector=status.phase=Running --no-headers 2>/dev/null | wc -l)"
  echo "  nodos pesados: $(kubectl get nodes -l ore.dev/pool=pesado --no-headers 2>/dev/null | wc -l)"
}

encender() {
  estado
  local c; c=$(desplegado)
  git -C "$raiz" merge-base --is-ancestor $Q4 "$c" 2>/dev/null || { echo "⛔ lo desplegado no trae Q4: no se enciende"; exit 1; }
  if [ "$TECHO" -gt 1 ] && ! trae_el_techo; then echo "⛔ TECHO=$TECHO pide Q5·3 desplegado (--consultas-techo)"; exit 1; fi
  if ! k get deploy ore-consulta >/dev/null 2>&1; then
    echo "== la celda sin converger: las plantillas de Q4 (y la de la cola) primero"
    hacer bash "$raiz/malla/converger-inquilinos.sh" victor
    [ -n "$APLICAR" ] && ! k get deploy ore-consulta >/dev/null 2>&1 && { echo "⛔ sigue sin ore-consulta tras converger"; exit 1; }
  fi
  if [ "$TECHO" -gt 1 ]; then
    echo "== la plantilla del de quita y pon, en la cola"
    hacer python "$raiz/malla/empujar-plantilla.py" victor
  fi
  echo "== 2 · Flux deja de aplicar $KS mientras se mide"
  hacer kubectl -n flux-system patch kustomization $KS --type merge -p '{"spec":{"suspend":true}}'
  echo "== 3 · ore-serve con --consultas (techo $TECHO) y el trabajador del suelo"
  local i; i=$(( $(indice) - 1 ))
  if args | grep -q -- '"--consultas"'; then
    echo "  ya lleva --consultas"
  else
    local p="[{\"op\":\"add\",\"path\":\"/spec/template/spec/containers/$i/args/-\",\"value\":\"--consultas\"}"
    [ "$TECHO" -gt 1 ] && p="$p,{\"op\":\"add\",\"path\":\"/spec/template/spec/containers/$i/args/-\",\"value\":\"--consultas-techo\"},{\"op\":\"add\",\"path\":\"/spec/template/spec/containers/$i/args/-\",\"value\":\"$TECHO\"}"
    hacer k patch deploy ore-serve --type json -p "$p]"
  fi
  hacer k scale deploy ore-consulta --replicas 1
  [ -z "$APLICAR" ] && return
  hacer k rollout status deploy ore-serve --timeout 300s
  echo "== esperando a que el trabajador pida (nodo pesado frío: ~100 s)"
  for _ in $(seq 1 60); do
    k logs deploy/ore-consulta --tail 20 2>/dev/null | grep -q "pide a" && { echo "  ✓ $(k logs deploy/ore-consulta --tail 20 | grep 'pide a' | tail -1)"; break; }
    sleep 5
  done
  estado
}

apagar() {
  echo "== 1 · Flux vuelve a mandar: ore-serve sin --consultas y el suelo a 0"
  hacer kubectl -n flux-system patch kustomization $KS --type merge -p '{"spec":{"suspend":false}}'
  hacer kubectl -n flux-system annotate kustomization $KS --overwrite "reconcile.fluxcd.io/requestedAt=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  [ -n "$APLICAR" ] && { k rollout status deploy ore-serve --timeout 300s; sleep 5; }
  echo "== 2 · fuera de la cola, los de quita y pon que queden"
  hacer python "$raiz/malla/empujar-plantilla.py" victor --retirar-consultas
  echo "== 3 · y los Jobs que aún corran"
  hacer k delete jobs -l ore.dev/pieza=consulta --ignore-not-found
  [ -n "$APLICAR" ] && { sleep 20; estado; }
  [ -n "$APLICAR" ] && args | grep -q consultas && echo "⛔ ore-serve sigue con --consultas: mirar Flux"
  true
}

case $QUE in
  estado) estado ;;
  encender) encender ;;
  apagar) apagar ;;
  *) echo "¿$QUE?"; exit 2 ;;
esac
