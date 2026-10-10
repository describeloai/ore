#!/usr/bin/env bash
# P7·4 en el laboratorio (ADR 0058): el conciliador de Libro con días de datos, en minutos.
#   · referencia: Libro con carga sobre una base pequeña; la completa y tres incrementales;
#   · de golpe, N transferencias «de hace una hora» (SQL en el cómputo: sus movimientos, los
#     saldos y su línea de confirmada en transferencias.jsonl), como si el soak llevara días;
#   · la completa sobre todo eso, y otra vez carga y tres incrementales: tienen que costar lo
#     mismo que con la base pequeña (miran lo nuevo, no lo acumulado);
#   · el detector: una transferencia reciente y otra antigua, borradas a mano: la incremental ve
#     la reciente; la completa, las dos;
#   · al borrar el proyecto no queda nada en el almacén.
#
#   lab.sh arriba && N=500000 p74.sh        (N por defecto 500 000; unos 10 min)
source "$(dirname "$0")/lab.sh"
fallos=0
bien() { echo "  ✓ $*"; }
mal() { echo "  ✗ $*"; fallos=$((fallos+1)); }
LIBRO="$(cd "$LAB/../libro" && pwd -W 2>/dev/null || pwd)"
lc() { docker compose -f "$LIBRO/compose.yaml" --env-file "$LIBRO/.env" "$@"; }
datos() { docker run --rm -v libro-datos:/datos alpine:3.20 sh -c "$1"; }
con() { lc run --rm --no-deps -T libro-conciliador python -u /app/conciliador.py "$@" 2>&1 | grep '^{' | tail -1; }
ms_de() { grep -oE '"ms":[0-9]+' <<<"$1" | tail -1 | cut -d: -f2; }
mediana() { printf '%s\n' "$@" | sort -n | sed -n "$(( ($# + 1) / 2 ))p"; }
N=${N:-500000}

P="libro-$(date +%s | tail -c 6)"
E="/v1/postgres/proyectos/$P/ramas/main/endpoints/principal"
echo "── el proyecto $P (demo), sin dormir"
r=$(pide demo POST /v1/postgres/proyectos "{\"id\":\"$P\",\"dueno\":\"user:libro\"}")
ROL=$(campo "$r" rol nombre); CLAVE=$(campo "$r" rol contrasena)
VM=$(listo demo "$P"); [ -n "$VM" ] || { echo "✗ no queda listo"; exit 1; }
[ "$(hecha demo "$(pide demo POST "$E/ajustes" '{"dormir_tras":"0"}')")" = hecha ] || { echo "✗ ajustes"; exit 1; }
TENANT=$(campo "$(pide demo GET "/v1/postgres/proyectos/$P")" tenant)
ep=$(pide demo GET "$E")
cat > "$LIBRO/.env" <<EOF
EP_HOST=$(campo "$ep" host)
EP_POOL=$(campo "$ep" host_pool)
BASE=$P
ROL=$ROL
CLAVE=$CLAVE
PUERTO_PG=4432
OPS=6
INTERVALO_CONCILIADOR=3600
INTERVALO_INFORMES=3600
EOF
sqlvm() { docker exec -i "$VM" gosu postgres psql -h /tmp -U cloud_admin -d "$P" -v ON_ERROR_STOP=1 -Atq "$@"; }
lc build -q 2>&1 | tail -3
docker volume rm libro-datos >/dev/null 2>&1
lc up -d >/dev/null 2>&1
sleep 40

incrementales() {   # tres, con carga, separadas 10 s → sus ms
  local v=() s
  for _ in 1 2 3; do sleep 10; s=$(con --incremental)
    case "$s" in *'"ok":true'*'"modo":"incremental"'*) v+=("$(ms_de "$s")");; *) mal "incremental: ${s:0:250}";; esac; done
  echo "${v[@]}"
}

echo "── referencia: la base pequeña"
s=$(con --una); case "$s" in *'"ok":true'*) bien "completa: $(grep -oE '"transferencias":[0-9]+' <<<"$s"), $(ms_de "$s") ms";; *) mal "completa: ${s:0:250}"; exit 1;; esac
read -ra REF <<<"$(incrementales)"
bien "incrementales: ${REF[*]} ms"

echo "── de golpe, $N transferencias de hace una hora"
lc stop usuarios banco >/dev/null 2>&1
T0=$(date +%s)
sqlvm <<SQL
begin;
-- Dinero para todo: un aviso grande por cuenta.
insert into aviso (clave, cuenta, importe, recibido)
  select 'sint-aviso-' || id, id, 100000000, now() - interval '1 hour' from cuenta;
insert into movimiento (cuenta, importe, aviso, cuando)
  select cuenta, importe, clave, recibido from aviso where clave like 'sint-aviso-%';
-- N transferencias entre dos cuentas distintas, con sus dos movimientos.
create temp table t as
  select 'sint-' || g as id, o, (o + 1 + g % 49) % 50 as d, 1 + g % 7 as importe
    from (select g, g % 50 as o from generate_series(1, $N) g) x;
insert into transferencia (id, origen, destino, importe, hecha)
  select id, 'c' || lpad((o + 1)::text, 4, '0'), 'c' || lpad((d + 1)::text, 4, '0'), importe, now() - interval '1 hour' from t;
insert into movimiento (cuenta, importe, transferencia, cuando)
  select 'c' || lpad((o + 1)::text, 4, '0'), -importe, id, now() - interval '1 hour' from t
  union all
  select 'c' || lpad((d + 1)::text, 4, '0'), importe, id, now() - interval '1 hour' from t;
update cuenta c set saldo = s.s from (select cuenta, sum(importe) s from movimiento group by cuenta) s where s.cuenta = c.id;
commit;
analyze;
SQL
[ $? = 0 ] && bien "metidas en $(( $(date +%s) - T0 )) s: $(sqlvm -c "select count(*) from movimiento") movimientos, $(sqlvm -c "select pg_size_pretty(pg_total_relation_size('movimiento'))") en movimiento" || { mal "el SQL"; exit 1; }
# Y su rastro de confirmadas y entregados, como si los clientes las hubieran visto.
lc run --rm --no-deps -T -e N="$N" usuarios python -c '
import json, os
n = int(os.environ["N"])
with open("/datos/transferencias.jsonl", "a") as f:
    for g in range(1, n + 1):
        f.write(json.dumps({"t": "2026-01-01T00:00:00Z", "id": f"sint-{g}", "importe": 1 + g % 7}, separators=(",", ":")) + "\n")
ids = json.load(open("/datos/cuentas.json"))
with open("/datos/avisos.jsonl", "a") as f:
    for c in ids:
        f.write(json.dumps({"t": "2026-01-01T00:00:00Z", "clave": f"sint-aviso-{c}", "cuenta": c, "importe": 100000000}, separators=(",", ":")) + "\n")
' && bien "transferencias.jsonl: $(datos "wc -l < /datos/transferencias.jsonl") confirmadas, $(datos "du -h /datos/transferencias.jsonl | cut -f1")"

echo "── la completa, sobre todo eso"
s=$(con --una); case "$s" in *'"ok":true'*) bien "completa: $(grep -oE '"(transferencias|confirmadas)":[0-9]+' <<<"$s" | tr '\n' ' ')en $(ms_de "$s") ms";;
  *) mal "completa: ${s:0:300}";; esac
lc start usuarios banco >/dev/null 2>&1
sleep 20
read -ra GRANDE <<<"$(incrementales)"
mr=$(mediana "${REF[@]}"); mg=$(mediana "${GRANDE[@]}")
[ "${#GRANDE[@]}" = 3 ] && [ "$mg" -le $(( mr * 3 > 300 ? mr * 3 : 300 )) ] \
  && bien "incrementales con $N más: ${GRANDE[*]} ms (mediana $mg, frente a $mr con la base pequeña): miran lo nuevo, no lo acumulado" \
  || mal "incrementales con $N más: ${GRANDE[*]} ms (mediana $mg frente a $mr)"

echo "── el detector: una reciente y una antigua, borradas a mano"
lc stop usuarios banco >/dev/null 2>&1
s=$(con --incremental)
lc start usuarios >/dev/null 2>&1; sleep 6; lc stop usuarios >/dev/null 2>&1
RECIENTE=$(datos "tail -1 /datos/transferencias.jsonl" | grep -oE '"id":"[^"]+"' | cut -d'"' -f4)
borra() { sqlvm >/dev/null <<SQL
begin;
update cuenta c set saldo = c.saldo - m.importe from movimiento m where m.transferencia = '$1' and m.cuenta = c.id;
delete from movimiento where transferencia = '$1';
delete from transferencia where id = '$1';
commit;
SQL
}
borra "$RECIENTE"; borra "sint-$(( N / 2 ))"
s=$(con --incremental)
case "$s" in *'"ok":false'*'"faltan_transferencias":1'*) bien "la incremental ve la reciente (faltan 1), en $(ms_de "$s") ms; $(grep -oE "\"descuadres\":[0-9]+" <<<"$s" || echo "sin descuadres")";; *) mal "incremental: ${s:0:300}";; esac
s=$(con --una)
case "$s" in *'"ok":false'*'"faltan_transferencias":2'*) bien "la completa ve las dos (faltan 2), en $(ms_de "$s") ms; el dinero cuadra";; *) mal "completa: ${s:0:300}";; esac

echo "── limpiar"
lc down -v >/dev/null 2>&1
hecha demo "$(pide demo DELETE "/v1/postgres/proyectos/$P")" >/dev/null
rm -f "$LIBRO/.env"
docker run --rm -v p5lab-almacen:/a alpine:3.20 test -e "/a/$TENANT" && mal "quedan datos en el almacén (/almacen/$TENANT)" || bien "borrado el proyecto, no queda nada en el almacén ni en libro-datos"
echo
[ $fallos = 0 ] && echo "P7·4 (laboratorio) ✓ todo" || { echo "P7·4 (laboratorio) ✗ $fallos fallos"; exit 1; }
