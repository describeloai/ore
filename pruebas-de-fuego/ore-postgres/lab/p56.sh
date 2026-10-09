#!/usr/bin/env bash
# P5·6 en el laboratorio (ADR 0058): quién entra. Por el proxy de Neon de verdad, con lo que
# `ore-postgres` le contesta en get_endpoint_access_control (`POST …/acceso`):
#   · ips_permitidas: una IP fuera de la lista no entra (TCP y HTTP); dentro, sí; subred y rango;
#     vacía, todas. Una entrada mala, 400 (el proxy la convertiría en «nadie»);
#   · bloquear_publico: nadie entra por la entrada pública, y al quitarlo, vuelve;
#   · limites: con una cubeta pequeña, una ráfaga de intentos se corta («too many connections»);
#   · cada cambio vale en cuanto la operación queda hecha (el proxy olvida por Redis), y otra
#     organización no lo puede tocar.
# El cliente TCP es `cliente` (172.29.51.30); el HTTP, curl desde 172.29.51.31.
#
#   lab.sh arriba && p56.sh
source "$(dirname "$0")/lab.sh"
fallos=0
P="p56-$(date +%s | tail -c 6)"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:p56\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(campo "$(pide demo GET "/v1/postgres/proyectos/$P/ramas/main/endpoints/principal")" vm)
[ -n "$VM" ] || { echo "✗ no crea: ${r:0:300}"; exit 1; }
reconcilia "$VM" computo-a
echo "── $P ($VM), rol $ROL"

tcp() { entra "$ROL" "${1:-$CLAVE}" "$P" "$VM"; }
http() {   # desde 172.29.51.31, el driver por HTTP como lo hace @neondatabase/serverless
  docker run --rm --network p5lab_lab --ip 172.29.51.31 --add-host "api.$DOMINIO:172.29.51.20" \
    -v "$LAB/.secretos/tls:/t:ro" curlimages/curl:8.10.1 -s --cacert /t/tls.crt -X POST "https://api.$DOMINIO/sql" \
    -H "Neon-Connection-String: postgresql://$ROL:$CLAVE@$VM.$DOMINIO/$P" -d '{"query":"select current_user as u","params":[]}' \
    | sed -n 's/.*"u":"\([^"]*\)".*/\1/p; s/.*"message":"\([^"]*\)".*/\1/p' | head -1; }
cambia() {   # cambia JSON [celda] → el código HTTP; deja la operación hecha
  local r; r=$(pide "${2:-demo}" POST "/v1/postgres/proyectos/$P/acceso" "$1")
  hechas; [ -n "$(campo "$r" operacion id)" ] && echo 202 || echo "${r:0:160}"; }
mira() { local que=$1 quiere=$2 r=$3
  case "$quiere:$r" in si:"$ROL") echo "  ✓ $que: entra";; no:"$ROL") echo "  ✗ $que: entra"; fallos=$((fallos+1));;
    si:*) echo "  ✗ $que: no entra (${r:0:110})"; fallos=$((fallos+1));; no:*) echo "  ✓ $que, no: ${r:0:100}";; esac; }

echo "── por defecto, todas las IPs"
mira "TCP desde .30" si "$(tcp)"; mira "HTTP desde .31" si "$(http)"

echo "── ips_permitidas"
c=$(pide demo POST "/v1/postgres/proyectos/$P/acceso" '{"ips_permitidas":["172.29.51.0/24","203.0.113"]}')
case "$c" in *"no es una IP"*) echo "  ✓ una entrada mala, 400: ${c:0:90}";; *) echo "  ✗ una entrada mala: ${c:0:120}"; fallos=$((fallos+1));; esac
c=$(pide victor POST "/v1/postgres/proyectos/$P/acceso" '{"bloquear_publico":true}')
case "$c" in *operacion*) echo "  ✗ victor cambia el de demo"; fallos=$((fallos+1));; *) echo "  ✓ victor no puede tocar el de demo: ${c:0:80}";; esac
cambia '{"ips_permitidas":["172.29.51.31"]}' >/dev/null
mira "sólo .31 · TCP desde .30" no "$(tcp)"; mira "sólo .31 · HTTP desde .31" si "$(http)"
cambia '{"ips_permitidas":["172.29.51.28-172.29.51.30"]}' >/dev/null
mira "rango .28-.30 · TCP desde .30" si "$(tcp)"; mira "rango .28-.30 · HTTP desde .31" no "$(http)"
cambia '{"ips_permitidas":["172.29.51.0/24"]}' >/dev/null
mira "subred /24 · TCP" si "$(tcp)"; mira "subred /24 · HTTP" si "$(http)"
cambia '{"ips_permitidas":["10.0.0.0/8","2001:db8::/32"]}' >/dev/null
mira "otra subred · TCP" no "$(tcp)"; mira "otra subred · HTTP" no "$(http)"
cambia '{"ips_permitidas":[]}' >/dev/null
mira "vacía otra vez · TCP" si "$(tcp)"

echo "── bloquear_publico"
cambia '{"bloquear_publico":true}' >/dev/null
mira "bloqueado · TCP" no "$(tcp)"; mira "bloqueado · HTTP" no "$(http)"
cambia '{"bloquear_publico":false}' >/dev/null
mira "desbloqueado · TCP" si "$(tcp)"

echo "── limites: la fuerza bruta se corta"
cambia '{"limites":{"tcp":{"por_segundo":1,"rafaga":3}}}' >/dev/null
T=$(mktemp -d); for i in $(seq 1 12); do tcp "mala$i" > "$T/$i" & done; wait
cortados=$(cat "$T"/* | grep -ci "too many"); rm -rf "$T"
[ "$cortados" -ge 6 ] && echo "  ✓ 12 intentos a la vez con cubeta 1/s ráfaga 3: $cortados cortados por el límite" \
  || { echo "  ✗ sólo $cortados de 12 cortados por el límite"; fallos=$((fallos+1)); }
r=$(tcp); case "$r" in *"too many"*|*"Too many"*) echo "  ✓ y mientras la cubeta está llena, tampoco la buena: ${r:0:80}";; *) echo "  · justo después, la buena: ${r:0:80}";; esac
cambia '{"limites":{"tcp":{"por_segundo":100,"rafaga":1000}}}' >/dev/null
mira "con el límite de siempre" si "$(tcp)"

pide demo DELETE "/v1/postgres/proyectos/$P" >/dev/null; barre
echo
[ $fallos = 0 ] && echo "P5·6 (laboratorio) ✓ todo" || { echo "P5·6 (laboratorio) ✗ $fallos fallos"; exit 1; }
