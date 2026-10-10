#!/usr/bin/env bash
# P7·1 en el laboratorio (ADR 0058): Libro, una aplicación real, encima de ORE Serverless Postgres.
#   A · carga: las cuatro piezas trabajan a la vez por el proxy (Prisma por el pool, el driver de
#       Neon por HTTP, psycopg y JDBC directos), y los tres invariantes cuadran;
#   B · despertar: con la base dormida, cada pieza la despierta por su camino y su operación sale
#       bien sin que la pieza lo note más que como espera (Prisma con connect_timeout, como
#       recomienda Neon);
#   C · otra vez carga, y la conciliación final: ningún error atribuible en todo el recorrido.
#
#   lab.sh arriba && p71.sh        (unos 10 min; la primera vez, más: construye las imágenes)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
LIBRO="$(cd "$LAB/../libro" && pwd -W 2>/dev/null || pwd)"
lc() { docker compose -f "$LIBRO/compose.yaml" --env-file "$LIBRO/.env" "$@"; }
datos() { docker run --rm -v libro-datos:/datos alpine:3.20 sh -c "$1"; }

P="libro-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
observado() { campo "$(pide demo GET "$E")" estado observado; }
dormida() { local t0; t0=$(date +%s)
  until [ "$(observado)" = dormido ] || [ $(( $(date +%s) - t0 )) -ge 180 ]; do sleep 2; done
  [ "$(observado)" = dormido ]; }

echo "── el proyecto $P (demo), dormir_tras 60 s"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:libro\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"60"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
ep=$(pide demo GET "$E")
cat > "$LIBRO/.env" <<EOF
EP_HOST=$(campo "$ep" host)
EP_POOL=$(campo "$ep" host_pool)
BASE=$P
ROL=$ROL
CLAVE=$CLAVE
PUERTO_PG=4432
INTERVALO_CONCILIADOR=3600
INTERVALO_INFORMES=3600
EOF
bien "$(campo "$ep" host) y $(campo "$ep" host_pool)"

echo "── las imágenes"
lc build -q 2>&1 | tail -3
docker volume rm libro-datos >/dev/null 2>&1

echo "── A · carga: las cuatro piezas a la vez"
lc up -d >/dev/null 2>&1
T0=$(date +%s)
until lc logs libro-api 2>&1 | grep -q "libro-api ·" || [ $(( $(date +%s) - T0 )) -ge 180 ]; do sleep 2; done
lc logs libro-api 2>&1 | grep -qE "applied|No pending" && bien "Prisma migró por DIRECT_URL: $(lc logs libro-api 2>&1 | grep -oE '[0-9]+ migrations? found' | head -1)" \
  || mal "las migraciones: $(lc logs libro-api 2>&1 | tail -3 | tr '\n' ' ' | cut -c1-200)"
sleep 90
n_api=$(datos "grep -c '\"pieza\":\"libro-api\".*\"resultado\":\"hecha\"' /datos/operaciones.jsonl")
n_web=$(datos "grep -c '\"pieza\":\"libro-webhooks\".*\"resultado\":\"hecha\"' /datos/operaciones.jsonl")
n_mal=$(datos "grep -cE '\"resultado\":\"(fallo|duplicado-mal|mala)\"' /datos/operaciones.jsonl")
[ "${n_api:-0}" -gt 50 ] && bien "libro-api (Prisma, pool): $n_api operaciones hechas" || mal "libro-api: ${n_api:-0} hechas"
[ "${n_web:-0}" -gt 10 ] && bien "libro-webhooks (HTTP): $n_web avisos apuntados, con duplicados" || mal "libro-webhooks: ${n_web:-0}"
[ "${n_mal:-0}" = 0 ] && bien "ningún fallo ni duplicado mal apuntado" || mal "$n_mal operaciones mal: $(datos "grep -E '\"resultado\":\"(fallo|duplicado-mal|mala)\"' /datos/operaciones.jsonl | head -2")"
inf=$(datos "head -1 /datos/informes.jsonl")
case "$inf" in *'"ok":true'*) bien "libro-informes (JDBC): el cierre y la exportación ($(echo "$inf" | grep -oE '"exportadas":[0-9]+'))";;
  *) mal "libro-informes: ${inf:0:250}";; esac
c=$(lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | tail -1)
case "$c" in *'"ok":true'*) bien "libro-conciliador (psycopg): los tres invariantes cuadran $(echo "$c" | grep -oE '"(transferencias|avisos|confirmadas|entregados)":[0-9]+' | tr '\n' ' ')";;
  *) mal "la conciliación: ${c:0:300}";; esac

echo "── B · despertar: cada pieza, contra la base dormida"
lc stop usuarios banco >/dev/null 2>&1
despierta() {   # despierta NOMBRE ORDEN…
  local n=$1; shift
  dormida || { mal "$n: la base no se durmió ($(observado))"; return; }
  local t0 s; t0=$(date +%s%3N); s=$("$@" 2>&1 | tail -1); local ms=$(( $(date +%s%3N) - t0 ))
  [ "${PIPESTATUS[0]}" = 0 ] || true
  case "$s" in *'"resultado":"hecha"'*|*'"resultado":"sin-fondos"'*|*'"nuevo":true'*|*'"ok":true'*)
    bien "$n despierta la base y sale bien en $ms ms (con el arranque del contenedor): ${s:0:120}";;
    *) mal "$n: ${s:0:300}";; esac
}
despierta "libro-api (Prisma por el pool)" lc run --rm --no-deps -T usuarios python -u /libro/usuarios/usuarios.py --una
despierta "libro-webhooks (HTTP)" lc run --rm --no-deps -T banco python -u /libro/banco/banco.py --uno
despierta "libro-conciliador (psycopg)" lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una
dormida && { lc restart libro-informes >/dev/null 2>&1; T0=$(date +%s)
  until [ "$(datos "wc -l < /datos/informes.jsonl")" -ge 2 ] || [ $(( $(date +%s) - T0 )) -ge 180 ]; do sleep 2; done
  inf=$(datos "tail -1 /datos/informes.jsonl")
  case "$inf" in *'"ok":true'*) bien "libro-informes (JDBC + Hikari) despierta la base: $(echo "$inf" | grep -oE '"conectar_ms":[0-9]+')";;
    *) mal "libro-informes tras dormir: ${inf:0:250}";; esac; } || mal "libro-informes: la base no se durmió"

echo "── C · otra vez carga, y la conciliación final"
lc start usuarios banco >/dev/null 2>&1
sleep 45
n_mal=$(datos "grep -cE '\"resultado\":\"(fallo|duplicado-mal|mala)\"' /datos/operaciones.jsonl")
[ "${n_mal:-0}" = 0 ] && bien "en todo el recorrido, ninguna operación fallida" || mal "$n_mal operaciones mal"
lc stop usuarios banco >/dev/null 2>&1
c=$(lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | tail -1)
case "$c" in *'"ok":true'*) bien "la conciliación final cuadra $(echo "$c" | grep -oE '"(transferencias|avisos|confirmadas|entregados|total)":[0-9]+' | tr '\n' ' ')";;
  *) mal "la conciliación final: ${c:0:300}";; esac
reint=$(datos "grep -c '\"intentos\":[2-9]' /datos/operaciones.jsonl")
echo "  · operaciones que necesitaron reintento (con la misma clave): ${reint:-0}"

[ -n "${GUARDA:-}" ] || { lc down -v >/dev/null 2>&1; hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null; rm -f "$LIBRO/.env"; }
echo
[ $fallos = 0 ] && echo "P7·1 (laboratorio) ✓ todo" || { echo "P7·1 (laboratorio) ✗ $fallos fallos"; exit 1; }
