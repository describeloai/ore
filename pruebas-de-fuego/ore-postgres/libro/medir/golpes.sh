#!/usr/bin/env bash
# Los golpes del soak en GCP (ADR 0058, P7·4, «La receta del soak en GCP», paso 6). Cada golpe
# apunta en $MEDIDAS/eventos.jsonl cuándo empezó y cuándo acabó; el informe calcula con eso la
# disponibilidad dentro y fuera de las ventanas de golpe (objetivo 2) y lo que vio Libro en cada
# una (objetivos 6 y 7).
#
#   golpes.sh empieza                 apunta la hora 0 del soak ($MEDIDAS/inicio)
#   golpes.sh calendario              los golpes de la receta, cada uno a su hora desde la 0
#   golpes.sh GOLPE                   uno, ahora: matar-vm | pageserver | safekeeper | proxy |
#                                       migrar | ore-postgres
#   golpes.sh apunta TEXTO            un hecho a mano (p. ej. `pool-encendido`, tras el cambio en
#                                       la malla: es configuración y pide su go)
#
#   PROYECTO=libro-soak (el del endpoint)   ESCALA=3600 (segundos por «hora» del calendario)
#   SECO=1: dice las órdenes de kubectl sin lanzarlas (para probarlo)
#
# Corre en la máquina del cliente, con el kubectl de su cuenta de servicio (sin llaves). Matar,
# reiniciar y migrar piden permisos de escritura en `ore-pg` y `ore-pg-computo`: se dan para el
# soak y se quitan al acabar (ver la receta).
set -uo pipefail
M="${MEDIDAS:-$HOME/libro-medidas}"
PROYECTO="${PROYECTO:-libro-soak}"
ESCALA="${ESCALA:-3600}"
mkdir -p "$M"
ahora() { date -u +%Y-%m-%dT%H:%M:%S.000Z; }
k() { if [ -n "${SECO:-}" ]; then echo "    kubectl $*" >&2; else kubectl "$@"; fi; }

apunta() {   # apunta GOLPE INICIO OK DETALLE
  local d; d=$(printf '%s' "$4" | tr '\n"\\' "  '" | cut -c1-300)
  printf '{"t":"%s","fin":"%s","golpe":"%s","ok":%s,"detalle":"%s"}\n' "$2" "$(ahora)" "$1" "$3" "$d" >> "$M/eventos.jsonl"
  echo "$(ahora) · $1: $([ "$3" = true ] && echo hecho || echo FALLÓ) ${d:0:160}"
}

# El cómputo que sirve hoy al endpoint: de la base de ore-postgres (una VM del pool no lleva las
# etiquetas del proyecto). Solo lectura.
computo() {
  [ -n "${SECO:-}" ] && { echo ep-seco; return; }
  kubectl exec -n ore-pg storcon-db-0 -- psql -U postgres -d ore_postgres -Atc \
    "select coalesce(computo, vm) from plano.endpoint where proyecto = '$PROYECTO' and id = 'principal' and observado = 'listo'"
}

espera_rollout() { k rollout status "$1" -n ore-pg --timeout=15m; }

golpe() {
  local g=$1 t0; t0=$(ahora)
  local salida ok=true
  case "$g" in
    matar-vm)
      local vm pod; vm=$(computo)
      [ -n "$vm" ] || { apunta "$g" "$t0" false "el endpoint no está listo: nada que matar"; return; }
      pod=$( [ -n "${SECO:-}" ] && echo runner-seco || kubectl get neonvm "$vm" -n ore-pg-computo -o jsonpath='{.status.podName}')
      salida=$(k delete pod "$pod" -n ore-pg-computo --wait=false 2>&1) || ok=false
      salida="VM $vm, runner $pod: $salida";;
    pageserver)
      salida=$( { k delete pod pageserver-0 -n ore-pg && espera_rollout statefulset/pageserver; } 2>&1) || ok=false;;
    safekeeper)
      salida=$( { k delete pod safekeeper-1 -n ore-pg && espera_rollout statefulset/safekeeper; } 2>&1) || ok=false;;
    proxy)
      salida=$( { k rollout restart deployment/proxy-postgres -n ore-pg && espera_rollout deployment/proxy-postgres; } 2>&1) || ok=false;;
    ore-postgres)
      salida=$( { k rollout restart deployment/ore-postgres -n ore-pg && espera_rollout deployment/ore-postgres; } 2>&1) || ok=false;;
    migrar)
      # Como pruebas-de-fuego/ore-postgres/migrar.sh (D0b·4): a otro nodo, sin post-copy.
      local vm mig fase=""; vm=$(computo); mig="soak-$(date +%s)"
      [ -n "$vm" ] || { apunta "$g" "$t0" false "el endpoint no está listo: nada que migrar"; return; }
      if [ -n "${SECO:-}" ]; then
        echo "    kubectl apply VirtualMachineMigration $mig (vmName $vm)" >&2; fase=Succeeded
      else
        kubectl apply -f - >/dev/null <<EOF || ok=false
apiVersion: vm.neon.tech/v1
kind: VirtualMachineMigration
metadata: { name: $mig, namespace: ore-pg-computo }
spec: { vmName: $vm, preventMigrationToSameHost: true, allowPostCopy: false }
EOF
        local t=$(date +%s)
        until fase=$(kubectl get neonvmm "$mig" -n ore-pg-computo -o jsonpath='{.status.phase}' 2>/dev/null); \
              [ "$fase" = Succeeded ] || [ "$fase" = Failed ] || [ $(( $(date +%s) - t )) -ge 900 ]; do sleep 2; done
      fi
      [ "$fase" = Succeeded ] || ok=false
      salida="VM $vm, migración $mig: ${fase:-sin fase}";;
    *) echo "golpe desconocido: $g" >&2; return 64;;
  esac
  apunta "$g" "$t0" "$ok" "$salida"
}

# La receta: hora → golpe. A la 36 se enciende el pool (configuración en la malla, con su go):
# aquí solo se recuerda.
CALENDARIO="12 matar-vm
24 pageserver
36 recordar-pool
40 safekeeper
48 proxy
56 migrar
64 ore-postgres"

calendario() {
  [ -s "$M/inicio" ] || { echo "falta la hora 0: golpes.sh empieza" >&2; return 1; }
  local cero; cero=$(cat "$M/inicio")
  while read -r hora g; do
    local cuando=$(( cero + hora * ESCALA )) falta
    falta=$(( cuando - $(date +%s) ))
    [ "$falta" -lt 0 ] && { echo "$(ahora) · $g (hora $hora) ya pasó: se salta"; continue; }
    echo "$(ahora) · $g a la hora $hora (dentro de ${falta} s)"
    sleep "$falta"
    if [ "$g" = recordar-pool ]; then
      echo "$(ahora) · TOCA ENCENDER EL POOL: --pool-computos 2 en la malla 86 (pide go), y después: golpes.sh apunta pool-encendido"
    else
      golpe "$g"
    fi
  done <<<"$CALENDARIO"
}

case "${1:-}" in
  empieza) date +%s > "$M/inicio"; echo "hora 0: $(ahora)";;
  calendario) calendario;;
  apunta) shift; t=$(ahora); apunta "${1:?qué}" "$t" true "a mano";;
  matar-vm|pageserver|safekeeper|proxy|migrar|ore-postgres) golpe "$1";;
  *) sed -n '2,22p' "$0"; exit 64;;
esac
