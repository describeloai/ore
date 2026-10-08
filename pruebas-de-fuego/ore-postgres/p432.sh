#!/usr/bin/env bash
# P4·3·2 · el cliente de Kubernetes (ADR 0058). Hecho cuando: `ore-postgres` crea, lee y borra un
# ConfigMap en `ore-pg-computo` y NO puede en ningún otro namespace (403, medido).
#
# Se mide desde dentro: el mando `kube-prueba` del `ore-postgres` desplegado, con SU cuenta, su TLS
# (sólo la CA del clúster) y su salida (la NetworkPolicy hacia el API server).
#
#   p432.sh [namespace-ajeno…]      (por defecto: ore-pg, su propio namespace, y t-demo, una celda)
source "$(dirname "$0")/entorno.sh"
AJENOS=("${@:-ore-pg}"); [ $# = 0 ] && AJENOS=(ore-pg t-demo)
echo "── lo que puede la cuenta de ore-postgres"
k exec deploy/ore-postgres -- /bin/ore-postgres kube-prueba "${AJENOS[@]}"; r=$?
echo
[ $r = 0 ] && echo "P4·3·2 ✓ todo" || { echo "P4·3·2 ✗"; exit 1; }
