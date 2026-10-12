#!/usr/bin/env bash
# P8·3 en producción (ADR 0058): lo que tarda Tables. El SQL es el de la consola (p83-sql.mts lo saca de
# su catalogo.ts) y corre como Tables: `POST …/ramas/{r}/sql`, en solo lectura y como el rol. Los ms son
# los del plano (del plano al cómputo y vuelta, con el hijo); la consola añade su ore-serve y la red.
#   · la lista de una rama con 301 tablas en un schema;
#   · una tabla pequeña (conteo exacto);
#   · una tabla de un millón de filas (115 MB, sobre el tope de 50 MB: conteo estimado): página 1 y página
#     del offset 900.000; y lo que costaría contarla.
#
#   p83.sh     (unos 5 min; `pg` despierto; CONSOLA = la raíz de rubix-platform, por defecto C:/rubix-platform)
#   P83_GUARDAR=1 p83.sh             no borra la instancia al acabar (para depurar)
#   P83_PROYECTO=p83-NNNN p83.sh     usa esa instancia, ya preparada (su rol es `prueba`)
PRUEBA=p83
source "$(dirname "$0")/comun.sh"
A=demo
export CATALOGO="${CONSOLA:-C:/rubix-platform}/components/postgres/catalogo.ts"   # ruta de Windows: la lee Node
GEN="$(dirname "$0")/p83-sql.mts"
genera() { npx -y tsx "$GEN" "$@"; }

# Un paso `pide` de SQL: el cuerpo, en JSON con las comillas simples como \u0027 (el paso viaja entre ''
# por sh -c, y Kubernetes cambia `$$` por `$` en el command de un pod: las dos, escapadas; sin barras en el
# Python, que Git Bash se come una en Windows). $1 = SQL; $2 = "true" para solo lectura.
sql_pide() {
  local cuerpo
  cuerpo=$(python -c 'import json,sys
d={"base":sys.argv[1],"rol":sys.argv[2],"sql":sys.argv[3]}
if sys.argv[4]=="true": d["solo_lectura"]=True
print(json.dumps(d).replace(chr(39), chr(92) + "u0027").replace(chr(36), chr(92) + "u0024"))' "$P" "$ROL" "$1" "${2:-false}")
  echo "pide POST /v1/postgres/proyectos/$P/ramas/main/sql '$cuerpo'"
}
de() {   # de "CODIGO JSON" EXPRESIÓN → la expresión sobre el JSON (r). La línea entra por stdin: la lista
  # de 301 tablas pasa de 32 KB, el tope de un argumento en Windows.
  printf '%s' "$1" | python -c 'import json,sys
c, _, t = sys.stdin.read().partition(" ")
try: r = json.loads(t)
except Exception: r = {}
print(eval(sys.argv[1], {"json": json}, {"r": r, "c": c}))' "$2" 2>/dev/null
}
# La relación `nombre` de p83 en lo que contestó la lista (para generar su SQL como la vista).
relacion_de() { de "$1" 'json.dumps(next(x for s in json.loads(r["sentencias"][0]["filas"][0][0]) if s["nombre"]=="p83" for x in s["relaciones"] if x["nombre"]=="'"$2"'"))'; }
ms3() {   # ms3 CLAVE SQL → corre tres veces en solo lectura, apunta el p50 de ms; deja la última línea en $ULTIMA
  local l v=() i
  mapfile -t l < <(en "$A" "$(sql_pide "$2" true)" "$(sql_pide "$2" true)" "$(sql_pide "$2" true)" | grep -E '^[0-9]{3} ')
  for i in 0 1 2; do v+=("$(de "${l[$i]}" 'r["ms"] if r.get("ok") else "x"')"); done
  ULTIMA=${l[2]}
  if [[ " ${v[*]} " == *" x "* || ${#l[@]} -lt 3 ]]; then mal "$1 (${#l[@]} líneas): ${l[0]:0:300} | ${l[2]:0:300}"; return 1; fi
  numero "$1" "$(percentil 0.5 "${v[@]}")"; echo "    ($1: ${v[*]} ms)"
}

genera catalogo >/dev/null || { echo "✗ p83-sql.mts no saca el SQL de $CATALOGO"; exit 1; }

echo "── la instancia"
if [ -n "${P83_PROYECTO:-}" ]; then
  P=$P83_PROYECTO; ROL=prueba; echo "  · $P ($ROL), la de antes"
else
p="p83-$(date +%s | tail -c 5)"
r=$(crea "$A" "$p") || exit 1
read -r ROL _ <<<"$r"; P=$p
echo "  · $P ($ROL), $(computo_de "$P")"

echo "── los datos: 300 tablas y una de un millón de filas"
l=$(en "$A" "$(sql_pide "create schema p83;
do \$\$ begin for i in 1..300 loop execute format('create table p83.t%s (id bigint primary key, nombre text, creado timestamptz default now())', i); end loop; end \$\$;
insert into p83.t1 select g, 'n' || g from generate_series(1, 20) g;
create table p83.grande (id bigint primary key, cliente int, importe numeric(10,2), nota text, creado timestamptz default now());
insert into p83.grande (id, cliente, importe, nota) select g, g % 1000, (g % 10000) / 100.0, md5(g::text) from generate_series(1, 1000000) g")" | grep -E '^[0-9]{3} ')
[ "$(de "$l" 'r.get("ok")')" = True ] && numero p83_preparar_ms "$(de "$l" 'r["ms"]')" || { mal "preparar: ${l:0:300}"; borra "$A" "$P"; exit 1; }
fi

echo "── la lista de la rama (301 tablas en p83)"
ms3 p83_lista_ms "$(genera catalogo)"
n=$(de "$ULTIMA" 'len(next(s for s in json.loads(r["sentencias"][0]["filas"][0][0]) if s["nombre"]=="p83")["relaciones"])')
[ "$n" = 301 ] && bien "301 relaciones en p83" || mal "relaciones en p83: $n"
numero p83_lista_bytes "$(de "$ULTIMA" 'len(r["sentencias"][0]["filas"][0][0])')"
G=$(relacion_de "$ULTIMA" grande); T1=$(relacion_de "$ULTIMA" t1)
echo "    grande: $G"

echo "── una tabla pequeña (t1, 20 filas)"
ms3 p83_pequena_ms "$(genera relacion p83 "$T1" 0 100)"
[ "$(de "$ULTIMA" 'r["sentencias"][1]["filas"][0][0]')" = 20 ] && bien "t1: conteo exacto 20" || mal "t1: ${ULTIMA:0:300}"

echo "── grande (un millón de filas, más de 50 MB): conteo estimado"
ms3 p83_grande_p1_ms "$(genera relacion p83 "$G" 0 100)"
[ "$(de "$ULTIMA" '(r["sentencias"][1]["filas"][0][0], r["sentencias"][2]["filas"][0][0])')" = "(None, '1')" ]   && bien "sin count(*) (el case no lo pide) y la página empieza en 1" || mal "grande: ${ULTIMA:0:300}"
ms3 p83_grande_p9000_ms "$(genera relacion p83 "$G" 900000 100)"
[ "$(de "$ULTIMA" 'r["sentencias"][2]["filas"][0][0]')" = 900001 ] && bien "la página del offset 900.000 empieza en 900001" || mal "offset: ${ULTIMA:0:300}"
echo "── lo que costaría contarla (sin el tope)"
ms3 p83_grande_count_ms "select count(*) from p83.grande"

if [ -n "${P83_GUARDAR:-}${P83_PROYECTO:-}" ]; then echo "  · $P se queda (P83_PROYECTO=$P)"; else borra "$A" "$P"; echo "  · $P borrado"; fi
echo
[ $fallos = 0 ] && echo "P8·3 (producción) ✓" || { echo "P8·3 (producción) ✗ $fallos fallos"; exit 1; }
