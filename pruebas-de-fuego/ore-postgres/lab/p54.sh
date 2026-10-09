#!/usr/bin/env bash
# P5·4 en el laboratorio (ADR 0058): HTTP y WebSocket por el proxy de Neon, con el driver de
# Neon (`@neondatabase/serverless`) desde Node. El proxy los sirve en 443 (`--wss`) con el mismo
# certificado y la misma autenticación que P5·1: el verificador del rol, de ore-postgres.
# El nombre `ep-….europe-west1.pg.paladio.io` resuelve al proxy con --add-host (en producción, el
# DNS comodín de P5·2); Node confía en el certificado autofirmado con NODE_EXTRA_CA_CERTS.
# ⚠️ Desde la 1.0 el driver manda el HTTP a `api.europe-west1.pg.paladio.io/sql` (el endpoint va en
#   la cabecera Neon-Connection-String): `api.` también tiene que resolver al proxy.
#
#   lab.sh arriba && p54.sh
source "$(dirname "$0")/lab.sh"
P="p54-$(date +%s | tail -c 6)"
crea() {   # crea CELDA → «vm rol clave»
  local r vm
  r=$(pide "$1" POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p54\"}")
  vm=$(campo "$(pide "$1" GET "/v1/postgres/proyectos/$P/ramas/main/endpoints/principal")" vm)
  [ -n "$vm" ] || { echo "  ✗ $1 no crea: ${r:0:300}" >&2; exit 1; }
  echo "$vm $(campo "$r" rol nombre) $(campo "$r" rol contrasena)"
}
echo "── demo y victor crean $P; el reconciliador de verdad levanta sus cómputos"
read -r VM ROL CLAVE < <(crea demo)
read -r VM_B _ _ < <(crea victor)
[ "$(listo demo "$P")" = "$VM" ] && [ "$(listo victor "$P")" = "$VM_B" ] || { echo "  ✗ no quedan listos"; exit 1; }
echo "  ✓ demo: $VM · victor: $VM_B"

echo "── @neondatabase/serverless 1.1.0 desde Node 22"
docker volume create p5lab-npm >/dev/null
docker run --rm --network p5lab_lab \
  --add-host "$VM.$DOMINIO:172.29.51.20" --add-host "$VM_B.$DOMINIO:172.29.51.20" --add-host "api.$DOMINIO:172.29.51.20" \
  -e NODE_EXTRA_CA_CERTS=/llaves/tls/tls.crt \
  -v "$LAB/.secretos/tls:/llaves/tls:ro" -v "$LAB/p54.mjs:/w/p54.mjs:ro" -v p5lab-npm:/w/node_modules \
  -w /w node:22-slim sh -c '[ -d node_modules/@neondatabase/serverless ] || npm i -s --no-save --no-package-lock @neondatabase/serverless@1.1.0 ws@8.18.3 >/dev/null 2>&1; node p54.mjs "$@"' \
  -- "$ROL" "$CLAVE" "$P" "$VM" "$VM_B"
r=$?

for c in demo victor; do hecha "$c" "$(pide "$c" DELETE "/v1/postgres/proyectos/$P")" >/dev/null; done
echo
[ $r = 0 ] && echo "P5·4 (laboratorio) ✓ todo" || { echo "P5·4 (laboratorio) ✗"; exit 1; }
