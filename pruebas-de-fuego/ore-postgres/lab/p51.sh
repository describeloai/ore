#!/usr/bin/env bash
# P5·1 en el laboratorio (ADR 0058): el contrato de ore-postgres con el proxy de Neon DE VERDAD.
# Las mismas comprobaciones que ../p51.sh en el clúster: `psql` por el proxy, con el SNI de
# `ep-….europe-west1.pg.paladio.io` y `verify-full`, entra con la contraseña del rol que dio la
# API y no con otra; un rol que no existe, un endpoint que no existe y la contraseña de una
# organización en el endpoint de otra, fuera.
#
#   lab.sh arriba && p51.sh
source "$(dirname "$0")/lab.sh"
fallos=0
P="p51-$(date +%s | tail -c 6)"
crea() {   # crea CELDA → «vm rol clave»
  local r vm
  r=$(pide "$1" POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p51\"}")
  vm=$(campo "$(pide "$1" GET "/v1/postgres/proyectos/$P/ramas/main/endpoints/principal")" vm)
  [ -n "$vm" ] || { echo "  ✗ $1 no crea: ${r:0:300}" >&2; exit 1; }
  echo "$vm $(campo "$r" rol nombre) $(campo "$r" rol contrasena)"
}

echo "── demo y victor crean $P (dueño user:p51); el laboratorio hace de reconciliador"
read -r VM ROL CLAVE < <(crea demo)
read -r VM_B ROL_B CLAVE_B < <(crea victor)
reconcilia "$VM" computo-a; reconcilia "$VM_B" computo-b
echo "  ✓ demo: $VM en computo-a · victor: $VM_B en computo-b · rol $ROL"

echo "── las preguntas del proxy, sin su token: nada"
c=$(pide demo GET "/proxy/wake_compute?endpointish=$VM" | head -c 200)
case "$c" in *address*) echo "  ✗ con el token de una celda contesta: $c"; fallos=$((fallos+1));; *) echo "  ✓ con el token de una celda, no";; esac

echo "── la primera conexión, arrancando: wake_compute contesta RUNNING_OPERATIONS y el proxy reintenta (8 veces, ~7 s)"
sql_plano "update plano.endpoint set observado = 'arrancando' where vm = '$VM'" >/dev/null
( sleep 3; sql_plano "update plano.endpoint set observado = 'listo' where vm = '$VM'" >/dev/null ) &
r=$(entra "$ROL" "$CLAVE" "$P" "$VM"); wait
[ "$r" = "$ROL" ] && echo "  ✓ listo a los 3 s: el proxy esperó y entra" || { echo "  ✗ no espera: ${r:0:140}"; fallos=$((fallos+1)); }

echo "── por el proxy de Neon"
r=$(entra "$ROL" "$CLAVE" "$P" "$VM")
[ "$r" = "$ROL" ] && echo "  ✓ $ROL entra en $P por $VM.$DOMINIO (verify-full)" || { echo "  ✗ no entra: $r"; fallos=$((fallos+1)); }
r=$(entra "$ROL" "$CLAVE" "$P" "$VM-pooler")
[ "$r" = "$ROL" ] && echo "  ✓ por -pooler, el mismo cómputo" || echo "  · por -pooler: ${r:0:140} (el pool es P5·5)"
r=$(entra "$ROL" mala "$P" "$VM")
[ "$r" = "$ROL" ] && { echo "  ✗ entra con otra contraseña"; fallos=$((fallos+1)); } || echo "  ✓ con otra contraseña, no: ${r:0:110}"
r=$(entra nadie "$CLAVE" "$P" "$VM")
[ "$r" = nadie ] && { echo "  ✗ entra un rol que no existe"; fallos=$((fallos+1)); } || echo "  ✓ un rol que no existe, no: ${r:0:110}"
r=$(entra "$ROL" "$CLAVE" "$P" ep-00000000000000000000)
[ "$r" = "$ROL" ] && { echo "  ✗ entra en un endpoint que no existe"; fallos=$((fallos+1)); } || echo "  ✓ un endpoint que no existe, no: ${r:0:110}"
r=$(entra "$ROL" "$CLAVE" "$P" "$VM_B")
[ "$r" = "$ROL" ] && { echo "  ✗ la contraseña de demo entra en el de victor"; fallos=$((fallos+1)); } || echo "  ✓ la contraseña de demo en el endpoint de victor, no"
r=$(entra "$ROL_B" "$CLAVE_B" "$P" "$VM_B")
[ "$r" = "$ROL_B" ] && echo "  ✓ y victor entra en el suyo con la suya" || { echo "  ✗ victor no entra en el suyo: $r"; fallos=$((fallos+1)); }
r=$(dc exec -T -e PGPASSWORD="$CLAVE" -e PGCONNECT_TIMEOUT=10 cliente psql "host=172.29.51.20 port=4432 user=$ROL dbname=$P sslmode=require" -Atc 'select 1' 2>&1 | tail -1)
[ "$r" = 1 ] && { echo "  ✗ sin SNI entra"; fallos=$((fallos+1)); } || echo "  ✓ sin SNI (sin endpoint), no: ${r:0:110}"

echo "── una contraseña nueva (Reset password): la nueva entra, la vieja no"
r=$(pide demo POST "/v1/postgres/proyectos/$P/ramas/main/roles/$ROL/contrasena")
NUEVA=$(campo "$r" rol contrasena); reconcilia "$VM" computo-a
t0=$(date +%s)
until [ "$(entra "$ROL" "$NUEVA" "$P" "$VM")" = "$ROL" ] || [ $(( $(date +%s) - t0 )) -ge 300 ]; do sleep 5; done
t=$(( $(date +%s) - t0 ))
[ $t -lt 10 ] && echo "  ✓ la nueva entra a la primera" || echo "  ✗ la nueva tarda ${t}s en entrar (la caché del proxy: project-info-cache ttl=4m)"
[ $t -lt 10 ] || fallos=$((fallos+1))
r=$(entra "$ROL" "$CLAVE" "$P" "$VM")
[ "$r" = "$ROL" ] && { echo "  ✗ la vieja aún entra"; fallos=$((fallos+1)); } || echo "  ✓ la vieja, no: ${r:0:110}"
CLAVE=$NUEVA

echo "── se borra"
for c in demo victor; do
  r=$(pide "$c" DELETE "/v1/postgres/proyectos/$P")
  [ -n "$(campo "$r" operacion id)" ] && echo "  ✓ $c lo borra por la API" || { echo "  ✗ $c: ${r:0:200}"; fallos=$((fallos+1)); }
done
barre
r=$(entra "$ROL" "$CLAVE" "$P" "$VM")
[ "$r" = "$ROL" ] && { echo "  ✗ borrado, y aún entra"; fallos=$((fallos+1)); } || echo "  ✓ borrado, ya no entra: ${r:0:110}"
echo
[ $fallos = 0 ] && echo "P5·1 (laboratorio) ✓ todo" || { echo "P5·1 (laboratorio) ✗ $fallos fallos"; exit 1; }
