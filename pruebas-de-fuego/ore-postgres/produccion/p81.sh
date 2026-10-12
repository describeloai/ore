#!/usr/bin/env bash
# P8·1 en producción (ADR 0058): el SQL de la consola contra una VM de verdad. El plano, como una
# celda (un pod de usar y tirar con la cuenta de su ore-serve, ../celdas.sh); el SQL no pasa por el
# proxy: lo ejecuta ore-postgres contra el cómputo (P8·1).
#   · la contraseña guardada es la de crearla;
#   · crear, insertar y leer como el rol; el error con su posición; el tope; solo lectura; en caliente;
#   · otra organización no entra;
#   · dormido, una consulta lo despierta: despertar_ms y ms en frío (una VM, no un contenedor).
#
#   p81.sh            (unos 5 min; `pg` despierto y la malla 86 con --clave-contrasenas)
PRUEBA=p81
source "$(dirname "$0")/comun.sh"
A=demo

proyecto_sql() {   # P ROL CLAVE HOST, con dormir_tras 60
  local p="p81-$(date +%s | tail -c 5)" r
  r=$(crea "$A" "$p") || exit 1
  case "$(ajusta "$A" "$p" '{"dormir_tras":"60"}')" in 2*) ;; *) echo "✗ ajustes" >&2; exit 1;; esac
  read -r ROL CLAVE HOST _ <<<"$r"; P=$p
}
cuerpo() { printf '{"base":"%s","rol":"%s",%s}' "$P" "$ROL" "$1"; }
sql_pide() { echo "pide POST /v1/postgres/proyectos/$P/ramas/main/sql '$(cuerpo "$1")'"; }
de() {   # de "CODIGO JSON" EXPRESIÓN → lo que dice la expresión sobre el JSON (r)
  local l=$1; python -c 'import json,sys
c, _, t = sys.argv[1].partition(" ")
try: r = json.loads(t)
except Exception: r = {}
print(eval(sys.argv[2], {}, {"r": r, "c": c}))' "$l" "$2" 2>/dev/null
}

echo "── la instancia"
proyecto_sql
echo "  · $P ($ROL), $(computo_de "$P")"

echo "── SQL como el rol, en caliente"
mapfile -t L < <(en "$A" \
  "pide GET /v1/postgres/proyectos/$P/ramas/main/roles/$ROL/contrasena" \
  "$(sql_pide '"sql":"create table cuenta (id int primary key, n text); insert into cuenta select g, null from generate_series(1, 3) g; select count(*), current_user from cuenta"')" \
  "$(sql_pide '"sql":"select 1 frm x"')" \
  "$(sql_pide '"sql":"select generate_series(1, 5000)","limite_filas":100')" \
  "$(sql_pide '"sql":"insert into cuenta values (9, null)","solo_lectura":true')" \
  "$(sql_pide '"sql":"select 1"')" "$(sql_pide '"sql":"select 1"')" "$(sql_pide '"sql":"select 1"')" | grep -E '^[0-9]{3} ')
[ "$(de "${L[0]}" 'r.get("contrasena")')" = "$CLAVE" ] && bien "la contraseña guardada es la de crearla" || mal "la contraseña: ${L[0]:0:200}"
[ "$(de "${L[1]}" 'r["sentencias"][2]["filas"]')" = "[['3', '$ROL']]" ] \
  && bien "crear, insertar y leer como $ROL: $(de "${L[1]}" 'r["ms"]') ms" || mal "las tres sentencias: ${L[1]:0:300}"
[ "$(de "${L[2]}" '(r["error"]["sqlstate"], r["error"]["posicion"])')" = "('42601', 14)" ] \
  && bien "el error, con su posición" || mal "el error: ${L[2]:0:200}"
[ "$(de "${L[3]}" '(len(r["sentencias"][0]["filas"]), r["sentencias"][0]["truncada"])')" = "(100, True)" ] \
  && bien "el tope: 100 de 5000" || mal "el tope: ${L[3]:0:200}"
[ "$(de "${L[4]}" 'r["error"]["sqlstate"]')" = 25006 ] && bien "solo lectura: 25006" || mal "solo lectura: ${L[4]:0:200}"
v=(); for i in 5 6 7; do v+=("$(de "${L[$i]}" 'r["ms"]')"); done
numero p81_sql_caliente_ms_p50 "$(percentil 0.5 "${v[@]}")"
echo "    (select 1 en caliente, tres veces: ${v[*]} ms, del plano al cómputo y vuelta, con el hijo)"

echo "── otra organización"
l=$(en victor "$(sql_pide '"sql":"select 1"')" | grep -E '^[0-9]{3} ')
[ "${l%% *}" = 404 ] && bien "victor no ve el proyecto de demo" || mal "otra organización: ${l:0:200}"

echo "── dormido, una consulta lo despierta"
dormido_ep "$P" 300 && bien "dormido" || mal "no se durmió ($(observado "$P"))"
l=$(en "$A" "$(sql_pide '"sql":"select count(*) from cuenta"')" | grep -E '^[0-9]{3} ')
if [ "$(de "$l" 'r["sentencias"][0]["filas"]')" = "[['3']]" ]; then
  bien "despertó y contestó con los datos de antes"
  numero p81_sql_frio_despertar_ms "$(de "$l" 'r["despertar_ms"]')"
  numero p81_sql_frio_ms "$(de "$l" 'r["ms"]')"
else
  mal "dormido: ${l:0:300}"
fi
echo "  · auditoría: $(plano_sql "select count(*) || ' ejecutadas, ' || count(error) || ' con error' from plano.sql_ejecutado where proyecto = '$P'")"

borra "$A" "$P"; echo "  · $P borrado"
echo
[ $fallos = 0 ] && echo "P8·1 (producción) ✓" || { echo "P8·1 (producción) ✗ $fallos fallos"; exit 1; }
