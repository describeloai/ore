#!/usr/bin/env bash
# ORE 0068 C3·c0 · Punto 7: dos Builds grandes de dos celdas a la vez. A arranca solo y toma los dos nodos;
# cuando tiene dos ejecutores, arranca B en la otra celda. Con KUEUE=1, los ejecutores van por la cola de
# su celda: B tiene que recuperar su nodo garantizado echando el prestado de A.
#   KUEUE=1 bash pruebas-de-fuego/geo/c3/c0/dos-celdas.sh
set -uo pipefail
aqui="$(cd "$(dirname "$0")" && pwd)"
sal=${SAL:-/tmp}
NS=c3c0 bash "$aqui/c0.sh" grande MAX=2 > "$sal/c0-celda-a.log" 2>&1 &
until [ "$(kubectl -n c3c0 get pods -l spark-role=executor --field-selector=status.phase=Running --no-headers 2>/dev/null | wc -l)" -ge 2 ]; do sleep 3; done
echo "== A tiene dos ejecutores: arranca B $(date -u +%T)"
NS=c3c0b bash "$aqui/c0.sh" grande MAX=2 > "$sal/c0-celda-b.log" 2>&1
wait
for c in a b; do echo "--- celda $c"; grep -E "^==|C3C|Scheduled|ScaleUp|Preempt|Evict|Killing" "$sal/c0-celda-$c.log" | cut -c1-200; done
echo "--- lo que hizo Kueue"
kubectl get events -A --field-selector reason=Preempted -o custom-columns=NS:.involvedObject.namespace,QUE:.involvedObject.name,MSG:.message 2>/dev/null | cut -c1-200 | tail -8
