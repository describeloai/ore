#!/usr/bin/env bash
# P7·2 en el laboratorio (ADR 0058): Libro con día y noche, y el informe de SLOs.
#   · tres días comprimidos (DIA_S 240 s, con forma) y tres noches (NOCHE_S 150 s, sin clientes);
#     con dormir_tras 60 la base duerme cada noche, y el conciliador (un cron cada 200 s) a veces
#     la despierta de madrugada, como en la vida real;
#   · los usuarios se reinician a media corrida: el reloj sigue donde estaba (reanudable);
#   · el informe: disponibilidad, commit p50/p95/p99, despertar (la primera operación de cada pieza
#     tras cada noche), invariantes, cierres al anochecer, y los errores clasificados;
#   · el detector: un commit confirmado que se borra a mano tiene que salir como atribuible.
#
#   lab.sh arriba && p72.sh        (unos 25 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
LIBRO="$(cd "$LAB/../libro" && pwd -W 2>/dev/null || pwd)"
lc() { docker compose -f "$LIBRO/compose.yaml" --env-file "$LIBRO/.env" "$@"; }
datos() { docker run --rm -v libro-datos:/datos alpine:3.20 sh -c "$1"; }
informe() { lc run --rm --no-deps -T usuarios python -u /libro/informe/informe.py "$@"; }
DIA=240; NOCHE=150; DIAS=3

P="libro-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
observado() { campo "$(pide demo GET "$E")" estado observado; }

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
RELOJ=1
DIA_S=$DIA
NOCHE_S=$NOCHE
OPS=6
INTERVALO_CONCILIADOR=200
EOF
lc build -q 2>&1 | tail -3
docker volume rm libro-datos >/dev/null 2>&1

echo "── $DIAS días de $DIA s y noches de $NOCHE s"
lc up -d >/dev/null 2>&1
# El estado del endpoint cada 5 s: cuántas veces se durmió.
( while :; do echo "$(date +%s) $(observado)"; sleep 5; done ) > "$LIBRO/.estados" 2>/dev/null &
VIGIA=$!
until datos "test -s /datos/reloj.json" 2>/dev/null; do sleep 2; done
ORIGEN=$(datos "cat /datos/reloj.json" | grep -oE '"origen": *[0-9.]+' | grep -oE '[0-9.]+$')
FIN=$(( ${ORIGEN%.*} + DIAS * (DIA + NOCHE) + 60 ))   # y una hora del cuarto día: su primera operación
MITAD=$(( ${ORIGEN%.*} + (DIA + NOCHE) + DIA / 2 ))
until [ "$(date +%s)" -ge "$MITAD" ]; do sleep 5; done
lc restart usuarios >/dev/null 2>&1
sleep 20
O2=$(datos "cat /datos/reloj.json" | grep -oE '"origen": *[0-9.]+' | grep -oE '[0-9.]+$')
[ "$O2" = "$ORIGEN" ] && lc logs usuarios 2>&1 | grep -q "reloj · " \
  && bien "usuarios reiniciados a mediodía del segundo día: el reloj sigue donde estaba" || mal "el reloj cambió al reiniciar ($ORIGEN → $O2)"
echo "  · esperando al cuarto día ($(( (FIN - $(date +%s)) / 60 )) min)"
until [ "$(date +%s)" -ge "$FIN" ]; do sleep 10; done
lc stop usuarios banco >/dev/null 2>&1
kill $VIGIA 2>/dev/null

echo "── el informe"
informe | tee "$LIBRO/.informe"; ri=${PIPESTATUS[0]}
J=$(informe --json | tail -1)
v() { "$PY" -c 'import json,sys; d=json.loads(sys.argv[1])
for k in sys.argv[2].split("."): d=d[k] if not isinstance(d, list) else d[int(k)]
print(d)' "$J" "$1"; }
[ "$ri" = 0 ] && bien "ningún error atribuible perdido" || mal "el informe sale con $ri"
[ "$(v disponibilidad)" = 100.0 ] && bien "disponibilidad 100 % ($(v operaciones) operaciones en $(v dias) días)" || mal "disponibilidad $(v disponibilidad)"
[ "$(v despertar_ms.n)" -ge $(( 2 * DIAS )) ] && bien "despertares medidos: $(v despertar_ms.n) (las dos piezas, cada mañana); p95 $(v despertar_ms.p95) ms" \
  || mal "despertares: $(v despertar_ms.n)"
[ "$(v cierres.total)" -ge "$DIAS" ] && [ "$(v cierres.bien)" = "$(v cierres.total)" ] && [ "$(v cierres.exportadas_ultima)" -gt 0 ] \
  && bien "un cierre por anochecer: $(v cierres.bien)/$(v cierres.total), el último exportó $(v cierres.exportadas_ultima) movimientos" || mal "cierres $(v cierres.bien)/$(v cierres.total)"
[ "$(v conciliaciones.total)" -ge 5 ] && [ "$(v conciliaciones.cuadran)" = "$(v conciliaciones.total)" ] \
  && bien "todas las conciliaciones cuadran ($(v conciliaciones.total))" || mal "conciliaciones $(v conciliaciones.cuadran)/$(v conciliaciones.total)"
dormidas=$(awk '$2=="dormido" && p!="dormido"{n++} {p=$2} END{print n+0}' "$LIBRO/.estados")
[ "$dormidas" -ge "$DIAS" ] && bien "la base se durmió $dormidas veces (cada noche, y otra vez tras el cron de madrugada)" || mal "se durmió $dormidas veces"

echo "── el detector: un commit confirmado, borrado a mano"
c=$(lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | grep '^{' | tail -1)   # despierta la base
ID=$(datos "tail -1 /datos/transferencias.jsonl" | grep -oE '"id":"[^"]+"' | cut -d'"' -f4)
docker exec -i "$VM" gosu postgres psql -h /tmp -U cloud_admin -d "$P" -v ON_ERROR_STOP=1 -Atq >/dev/null <<SQL
begin;
update cuenta c set saldo = c.saldo - m.importe from movimiento m where m.transferencia = '$ID' and m.cuenta = c.id;
delete from movimiento where transferencia = '$ID';
delete from transferencia where id = '$ID';
commit;
SQL
c=$(lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py --una 2>&1 | grep '^{' | tail -1)
case "$c" in *'"ok":false'*'"faltan_transferencias":1'*) bien "el conciliador lo ve: falta 1 transferencia confirmada, y el dinero cuadra";;
  *) mal "el conciliador no lo vio: ${c:0:250}";; esac
out=$(informe 2>&1); ri=$?
[ "$ri" = 1 ] && grep -q "faltan_transferencias" <<<"$out" && bien "el informe lo da como atribuible perdido, y sale con 1" || mal "el informe no lo cuenta (sale con $ri): $(tail -3 <<<"$out")"

[ -n "${GUARDA:-}" ] || { lc down -v >/dev/null 2>&1; hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null; rm -f "$LIBRO/.env" "$LIBRO/.estados" "$LIBRO/.informe"; }
echo
[ $fallos = 0 ] && echo "P7·2 (laboratorio) ✓ todo" || { echo "P7·2 (laboratorio) ✗ $fallos fallos"; exit 1; }
