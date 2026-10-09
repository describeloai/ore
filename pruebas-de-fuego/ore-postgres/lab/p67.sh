#!/usr/bin/env bash
# P6·7 en el laboratorio (ADR 0058): el interruptor `--dormir`. En producción, el código de P6 llega
# antes que el proxy (P5·3); un endpoint dormido sólo lo despierta una conexión por el proxy, así que
# dormir va APAGADO hasta que el proxy esté desplegado:
#   · sin `--dormir` (DORMIR=): con dormir_tras 60 y quieto más de un minuto, NO duerme, ninguna
#     operación dormir-endpoint, y la actividad se sigue apuntando;
#   · al encenderlo (el compose lo trae encendido por defecto), ese mismo endpoint duerme enseguida.
#
#   lab.sh arriba && p67.sh        (unos 2 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
plano_con() { DORMIR="$1" dc up -d plano >/dev/null 2>&1; sleep 3
  dc exec -T cliente bash -c 'for i in $(seq 1 30); do exec 3<>/dev/tcp/172.29.51.7/8100 && break; sleep 1; done' 2>/dev/null; }

P="p67-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
observado() { campo "$(pide demo GET "$E")" estado observado; }

echo "── sin --dormir"
plano_con ""
dc logs plano 2>&1 | grep "LABORATORIO" | tail -1 | grep -q "dormir false" \
  && bien "el plano arranca con dormir apagado" || mal "el plano no dice dormir false"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p67\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"60"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
A0=$(sql_plano "select ultima_actividad from plano.endpoint where vm = '$VM'")
dc exec -T -e PGPASSWORD="$CLAVE" cliente psql \
  "host=$VM.$DOMINIO hostaddr=172.29.51.20 port=4432 user=$ROL dbname=$P sslmode=verify-full sslrootcert=/llaves/tls/tls.crt" \
  -Atqc "select pg_sleep(2)" >/dev/null && bien "una consulta de 2 s por el proxy (más que el muestreo de 500 ms de compute_ctl)" || mal "la consulta"
sleep 80
[ "$(observado)" = listo ] && bien "a los 80 s quieto, con dormir_tras 60, sigue despierto" || mal "sin --dormir quedó $(observado)"
n=$(sql_plano "select count(*) from plano.operacion where proyecto = '$P' and tipo = 'dormir-endpoint'")
[ "$n" = 0 ] && bien "ninguna operación dormir-endpoint" || mal "$n operaciones dormir-endpoint"
A1=$(sql_plano "select ultima_actividad from plano.endpoint where vm = '$VM'")
[ -n "$A1" ] && [ "$A1" != "$A0" ] && bien "la actividad se sigue apuntando ($A0 → $A1)" || mal "la actividad no se apuntó ($A0 → $A1)"

echo "── con --dormir"
plano_con "--dormir"
T0=$(date +%s)
until [ "$(observado)" = dormido ] || [ $(( $(date +%s) - T0 )) -ge 60 ]; do sleep 1; done
[ "$(observado)" = dormido ] && bien "al encenderlo, duerme a los $(( $(date +%s) - T0 )) s" || mal "con --dormir sigue $(observado)"

hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P6·7 (laboratorio) ✓ todo" || { echo "P6·7 (laboratorio) ✗ $fallos fallos"; exit 1; }
