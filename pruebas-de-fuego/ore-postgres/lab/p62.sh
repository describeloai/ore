#!/usr/bin/env bash
# P6·2 en el laboratorio (ADR 0058): los límites del cliente, por la API y con el reconciliador de verdad.
#   · por defecto, dormir_tras 300 s; crear un endpoint con su propio dormir_tras;
#   · POST …/endpoints/{e}/ajustes: cu_min, cu_max, dormir_tras; lo que no viene se queda; fuera de rango,
#     400; otra organización, 404; es una operación que el reconciliador da por hecha;
#   · un cómputo vivo NO se reinicia por esto: sigue con su max_connections hasta el siguiente arranque
#     (que lo pone P6·4, al despertar).
#
#   lab.sh arriba && p62.sh
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
P="p62-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p62\"}")
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
echo "── $P ($VM)"

ep=$(pide demo GET "$E/principal")
[ "$(campo "$ep" dormir_tras)" = 300 ] && bien "por defecto duerme a los 300 s" || mal "dormir_tras por defecto: $(campo "$ep" dormir_tras)"
antes=$(en_vm "$VM" "show max_connections")
[ "$antes" = 450 ] && bien "el cómputo arrancó con 1 CU: max_connections $antes" || mal "max_connections al nacer: $antes"

echo "── ajustes: lo malo, fuera"
for malo in '{"dormir_tras":"30"}' '{"dormir_tras":"999999"}' '{"cu_min":"1.5","cu_max":"1"}' '{"cu_max":"8"}' '{}'; do
  c=$(pide demo POST "$E/principal/ajustes" "$malo")
  [ -z "$(campo "$c" operacion id)" ] && bien "$malo → ${c:0:90}" || mal "$malo se aceptó"
done
c=$(pide victor POST "$E/principal/ajustes" '{"dormir_tras":"0"}')
case "$c" in *"no hay ning"*) bien "victor no toca el de demo";; *) mal "victor: ${c:0:120}";; esac

echo "── ajustes: 0,25 CU y nunca dormir"
r=$(pide demo POST "$E/principal/ajustes" '{"cu_max":"0.25","dormir_tras":"0"}')
[ "$(hecha demo "$r")" = hecha ] && bien "la operación $(campo "$r" operacion tipo), hecha por el reconciliador" || mal "ajustes: ${r:0:200}"
ep=$(pide demo GET "$E/principal")
[ "$(campo "$ep" cu min)/$(campo "$ep" cu max)/$(campo "$ep" dormir_tras)/$(campo "$ep" conexiones maximas)" = "0.25/0.25/0/112" ] \
  && bien "el endpoint: 0.25–0.25 CU, nunca duerme, 112 conexiones (las del siguiente arranque)" \
  || mal "el endpoint dice $(campo "$ep" cu min)/$(campo "$ep" cu max)/$(campo "$ep" dormir_tras)/$(campo "$ep" conexiones maximas)"
[ "$(en_vm "$VM" "show max_connections")" = "$antes" ] && [ "$(docker ps -q --filter name="$VM")" ] \
  && bien "el cómputo vivo no se reinició: sigue con $antes hasta el siguiente arranque" || mal "el cómputo cambió o se reinició"

echo "── crear con su propio dormir_tras"
pide demo POST "/v1/postgres/proyectos/$P/ramas" '{"id":"dev"}' >/dev/null
r=$(pide demo GET "/v1/postgres/proyectos/$P/ramas/dev"); for _ in $(seq 1 60); do [ "$(campo "$r" estado observado)" = lista ] && break; sleep 0.5; r=$(pide demo GET "/v1/postgres/proyectos/$P/ramas/dev"); done
r=$(pide demo POST "/v1/postgres/proyectos/$P/ramas/dev/endpoints" '{"id":"uno","dormir_tras":"60"}')
[ "$(campo "$r" endpoint dormir_tras)" = 60 ] && bien "un endpoint nuevo con dormir_tras 60" || mal "crear con dormir_tras: ${r:0:200}"
hecha demo "$r" >/dev/null
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P/ramas/dev/endpoints/uno")" >/dev/null
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P/ramas/dev")" >/dev/null

hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
echo
[ $fallos = 0 ] && echo "P6·2 (laboratorio) ✓ todo" || { echo "P6·2 (laboratorio) ✗ $fallos fallos"; exit 1; }
