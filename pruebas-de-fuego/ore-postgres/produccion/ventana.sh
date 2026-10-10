#!/usr/bin/env bash
# La ventana 1 de ORE Serverless Postgres en producción: P5 y P6 en una tarde (ADR 0058, «Cómo
# desplegaríamos»). El clúster despierto el mínimo: todo lo que se puede hacer antes está hecho, y
# aquí sólo se ejecuta, se mide y se decide si se da el paso siguiente.
#
#   ventana.sh            el plan, paso a paso
#   ventana.sh N          lanza las pruebas del paso N (los pasos de infraestructura y de malla piden
#                         su go y se dan a mano: aquí sólo se dicen)
#   ventana.sh numeros    los números apuntados hasta ahora ($ORE_PG_TRABAJO/numeros.jsonl)
#
# Cada paso de malla es un parche de produccion/malla/: se aplica con `git apply`, se hace commit SÓLO de
# ese fichero y se empuja; Flux lo aplica. Nunca dos a la vez (binario antes que malla, flag nuevo y
# malla nunca juntos).
AQUI_V="$(cd "$(dirname "$0")" && pwd)"
PLAN=(
  "GO · despertar: bash malla/despertar.sh --con-postgres (avisar antes a la sesión de 0060; desde aquí corre el coste)"
  "pruebas · sano.sh: ore-pg listo, ore-postgres con las migraciones 005–008, --dormir y pool apagados, los endpoints de antes vivos"
  "GO · P5·2: bash malla/87-postgres-la-entrada-gcp.sh (la IP, la cuenta del DNS-01 sólo sobre la zona, el registro A; cambios de GCP y de IAM)"
  "GO · malla 1-entrada.patch: descomentar la 87 → commit y push → Flux: el certificado entrada-pg Ready, el proxy con dos réplicas en la IP"
  "pruebas · p5.sh todo: dns, conectar, pgbench, pool150, http, ip, migrar (unos 15 min)"
  "GO · el puerto: recrear los cómputos nacidos en 55433 (postgre) matando su VM (reparar-endpoint, P7·3, la rehace con la especificación de hoy: 5432; de paso, reparar probado en producción) y, cuando no quede ninguno, malla 4-sin-55433.patch → commit y push"
  "GO · malla 2-dormir.patch → commit y push (--dormir)"
  "pruebas · p6.sh dormir y p6.sh http-corto (dormido cuesta 0; las consultas cortas por HTTP)"
  "pruebas · p6.sh despertar 10 (frío, VM de verdad: unos 20 min)"
  "GO · malla 3-pool.patch → commit y push (--pool-computos 2)"
  "pruebas · p6.sh pool 10 (desde el pool)"
  "pruebas · p5.sh limpiar y p6.sh limpiar; los números al ADR"
  "GO · dormir: bash malla/dormir.sh (salvo que la ventana 2, el soak de P7, empiece enseguida)"
)
PRUEBAS=(
  [2]="sano.sh"
  [5]="p5.sh todo"
  [8]="p6.sh dormir; p6.sh http-corto"
  [9]="p6.sh despertar 10"
  [11]="p6.sh pool 10"
  [12]="p5.sh limpiar; p6.sh limpiar"
)
case "${1:-}" in
  "")
    for i in "${!PLAN[@]}"; do printf '%2d · %s\n' $((i + 1)) "${PLAN[$i]}"; done;;
  numeros)
    source "$AQUI_V/comun.sh" >/dev/null; cat "$NUMEROS" 2>/dev/null || echo "(ninguno todavía)";;
  *)
    n=$1; [ -n "${PRUEBAS[$n]:-}" ] || { echo "el paso $n no tiene pruebas: ${PLAN[$((n - 1))]:-no existe}"; exit 64; }
    echo "── paso $n · ${PLAN[$((n - 1))]}"
    IFS=';' read -ra ordenes <<<"${PRUEBAS[$n]}"
    r=0; for o in "${ordenes[@]}"; do (cd "$AQUI_V" && eval "bash $o") || r=1; done
    exit $r;;
esac
