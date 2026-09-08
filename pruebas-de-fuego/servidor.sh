#!/usr/bin/env bash
# El plano de control, de punta a punta y contra un servidor de verdad.
#
# No es una prueba unitaria con un manejador simulado: levanta `ore-serve` en un
# puerto, le habla por HTTP con `curl` y comprueba lo que contesta. Lo que se
# quiere saber no es si las funciones devuelven lo que devuelven — eso ya lo
# dicen 14 pruebas dentro del crate— sino **si la puerta es una puerta**.
#
# Los cuatro hechos que esto fija, y ninguno se puede afirmar sin un socket:
#
#   1. sin `--identidad`, las rutas de datos NO ESTAN
#   2. con identidad y sin sujeto, 401 — y con sujeto, pasa
#   3. una URL con credencial dentro se NIEGA, con el motivo escrito
#   4. el flujo `alta -> inducir -> cola -> responder` cierra las decisiones
#
# Uso:  bash pruebas-de-fuego/servidor.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8899}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""

falla() { echo "✗ $*" >&2; limpiar; exit 1; }
dice()  { echo "  · $*"; }

limpiar() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$TMP"
}
trap limpiar EXIT

# Los binarios. Se busca en `release` y luego en `debug`, y con `.exe` y sin él:
# esto corre igual en un portátil con Windows que en el runner de CI, y hacerlo
# depender del perfil sería una prueba que sólo se pasa en un sitio.
buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"     || falla "no hay binario de \`ore\` — cargo build -p ore-cli"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\` — cargo build -p ore-serve"

REPO="$TMP/repo"
mkdir -p "$REPO"
( cd "$REPO" && "$ORE" init . >/dev/null 2>&1 ) || falla "\`ore init\` fallo"

# Un catalogo escrito a mano: `discover --from` acepta uno venga de donde venga,
# y asi esta prueba no necesita ningun driver ni ningun origen.
cat > "$REPO/catalogo.json" <<'JSON'
{
  "source": "demo",
  "tables": [
    { "name": "public.clientes",
      "columns": [ { "name": "id", "type": "Integer" },
                   { "name": "email", "type": "String" } ] },
    { "name": "public.pedidos",
      "columns": [ { "name": "id", "type": "Integer" },
                   { "name": "cliente_id", "type": "Integer" } ] }
  ]
}
JSON

# ── 1 · Sin identidad, las rutas de datos no estan ──────────────────────────
"$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" >/dev/null 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done

curl -sf "$BASE/salud" | grep -q '"ok":true' || falla "1 · /salud no contesta"
dice "1 · /salud contesta sin sujeto, que es lo unico que debe hacer"

COD=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/fuentes")
[ "$COD" = "404" ] || falla "1 · sin proveedor, /fuentes devolvio $COD y no 404"
dice "1 · sin proveedor de identidad, /fuentes NO ESTA (404, no 401)"

kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null; SRV=""

# ── El modo de banco exige los dos interruptores ────────────────────────────
if "$SERVE" --repo "$REPO" --ore "$ORE" --identidad cabecera >/dev/null 2>&1; then
  falla "arranco en modo de banco sin \`--no-es-produccion\`"
fi
dice "· el modo de banco se niega a arrancar con un solo interruptor"

# ── 2 en adelante · con identidad ───────────────────────────────────────────
"$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
         --identidad cabecera --no-es-produccion >/dev/null 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done

SUJ='x-ore-sujeto: persona:ana'

COD=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/fuentes")
[ "$COD" = "401" ] || falla "2 · sin sujeto, /fuentes devolvio $COD y no 401"
COD=$(curl -s -o /dev/null -w '%{http_code}' -H "x-ore-sujeto: con espacio" "$BASE/fuentes")
[ "$COD" = "401" ] || falla "2 · un sujeto invalido devolvio $COD"
curl -sf -H "$SUJ" "$BASE/fuentes" | grep -q '"datasources"' \
  || falla "2 · con sujeto, /fuentes no contesta"
dice "2 · sin sujeto 401 · sujeto invalido 401 · con sujeto, pasa"

# ── 3 · La credencial no entra ──────────────────────────────────────────────
R=$(curl -s -X POST -H "$SUJ" "$BASE/fuentes" \
     -d '{"name":"pg","url":"postgres://ana:clave@host/db"}')
echo "$R" | grep -q "credencial" || falla "3 · acepto una URL con la clave dentro: $R"
dice "3 · una URL con credencial se niega, y dice por que"

# ── 4 · El flujo entero ─────────────────────────────────────────────────────
COD=$(curl -s -o "$TMP/alta.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/fuentes" \
       -d '{"name":"demo","url":"bigquery://un-proyecto/ventas"}')
[ "$COD" = "201" ] || falla "4 · el alta devolvio $COD: $(cat "$TMP/alta.json")"
grep -q "plano de control" "$TMP/alta.json" \
  || falla "4 · el alta no dice que leer el origen no corre aqui"
dice "4 · POST /fuentes da de alta, y ANUNCIA que leer el origen es de un Job"

curl -sf -H "$SUJ" "$BASE/fuentes" | grep -q '"name":"demo"' \
  || falla "4 · la fuente no aparece en la lista"
curl -sf -H "$SUJ" "$BASE/fuentes" | grep -q "clave" \
  && falla "4 · la lista de fuentes filtro algo que parece un secreto"
dice "4 · GET /fuentes la ve, y no devuelve ningun secreto"

( cd "$REPO" && "$ORE" discover --from catalogo.json --out packages/ventas --name ventas >/dev/null 2>&1 ) \
  || falla "4 · \`ore discover\` fallo"

curl -sf -H "$SUJ" "$BASE/paquetes" | grep -qE '"decisionesPendientes":[1-9]' \
  || falla "4 · /paquetes no dice que hay decisiones abiertas"
dice "4 · GET /paquetes ve el paquete y dice que tiene decisiones abiertas"

curl -sf -H "$SUJ" "$BASE/paquetes/ventas/decisiones" -o "$TMP/cola.json" \
  || falla "4 · /decisiones no contesta"
grep -q '"options"' "$TMP/cola.json" || falla "4 · la cola no trae \`options\`"
grep -q '"because"' "$TMP/cola.json" || falla "4 · la cola no trae \`because\`"
dice "4 · la cola sale con \`options\` y \`because\`: es un formulario en JSON"

# Un camino que no puede existir. El alfabeto de un nombre ya no lo admite.
COD=$(curl -s -o /dev/null -w '%{http_code}' -H "$SUJ" "$BASE/paquetes/..%2f..%2fetc/decisiones")
[ "$COD" = "404" ] || [ "$COD" = "422" ] || falla "4 · un nombre con \`..\` devolvio $COD"
dice "4 · un nombre de paquete no puede salir de su directorio"

curl -s -X POST -H "$SUJ" "$BASE/paquetes/ventas/decisiones" -o "$TMP/resp.json" \
  -d '{"answers":{"clave/public.clientes":["id"],"clave/public.pedidos":["id"],"dueno/ventas":"team:datos","relacion/public.pedidos.cliente_id":"si"}}'
grep -q '"informe"' "$TMP/resp.json" || falla "4 · responder no devolvio informe: $(cat "$TMP/resp.json")"
grep -q "cerrada" "$TMP/resp.json" || dice "  (el informe no dijo «cerradas»; se mira abajo si quedan)"
dice "4 · POST /decisiones aplica las respuestas: $(head -c 200 "$TMP/resp.json")"

# El fallo que destapo la forja: `ore review` deja la cola escrita CON LA LISTA
# VACIA cuando las cierra todas. Mirar si el fichero esta decia «hay decisiones
# pendientes» para siempre, y una consola con una alarma que nadie puede apagar
# es peor que una consola sin alarma.
grep -q '"quedan":0' "$TMP/resp.json"   || falla "5 · tras cerrarlas todas, \`quedan\` no es 0: $(cat "$TMP/resp.json")"
curl -sf -H "$SUJ" "$BASE/paquetes" | grep -q '"decisionesPendientes":0'   || falla "5 · /paquetes sigue diciendo que hay decisiones abiertas"
dice "5 · y vuelven a 0: se CUENTAN las decisiones, no se mira si el fichero esta"

echo
echo "ok · el plano de control atiende, y lo que toca el mundo sigue fuera"
