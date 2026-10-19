#!/bin/bash
# ORE 0068 C3·c0 · El `preStop` del ejecutor (`spark.kubernetes.decommission.script`): como el `decom.sh`
# de Spark, sin `ps` (la imagen slim no lo trae). La java del ejecutor es el PID 1 (`exec` en ejecutor.sh):
# SIGPWR le dice que se retire (migra lo que le dé tiempo), y se espera a que salga.
echo "decom: $(date -u +%T) SIGPWR al ejecutor"
kill -s SIGPWR 1
while [ -d /proc/1 ]; do sleep 1; done
