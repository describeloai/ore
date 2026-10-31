# shellcheck shell=bash
# ═══════════════════════════════════════════════════════════════════════════
# EL CATÁLOGO DE UNA PRUEBA DE FUEGO (0079 ·5g·3 R1b)
#
#   . "$RAIZ/pruebas-de-fuego/catalogo-de-prueba.sh"
#   catalogo_arrancar los-schemas          # un esquema propio, y lo que hace falta para usarlo
#   "$SERVE" … $ARGS_DEL_CATALOGO          # ore-serve, con sus punteros en ese esquema
#   poner_puntero ventas.clientes '{…}'    # lo que antes era escribir datasets/…json en el árbol
#   puntero ventas.clientes [rama]         # lo que antes era leerlo de la forja (vacío: no hay)
#
# Los punteros viven en el catálogo transaccional, y sólo ahí: como en producción, una prueba no
# los escribe ni los lee como ficheros. Necesita `ORE_CATALOGO_PRUEBAS` (la URL de un Postgres
# donde se puedan crear esquemas: en el CI, el del job) y `psql`; sin ellos, falla diciéndolo.
# El esquema se borra al salir (`catalogo_soltar`, en el `trap` de la prueba).
#
# `ore` hereda la puerta: `CATALOGO_URL` y `ORE_PUNTEROS` quedan exportados, y un `ore` que la
# prueba lance a mano lee y escribe en el mismo esquema que su `ore-serve`.
# ═══════════════════════════════════════════════════════════════════════════

catalogo_arrancar() { # <nombre de la prueba>
  [ -n "${ORE_CATALOGO_PRUEBAS:-}" ] || {
    echo "⛔ sin ORE_CATALOGO_PRUEBAS: los punteros viven en el catálogo, y esta prueba necesita uno" >&2
    exit 1
  }
  command -v psql >/dev/null || { echo "⛔ sin psql: hace falta para crear el esquema de la prueba" >&2; exit 1; }
  CATALOGO_BIN="${CATALOGO_BIN:-$(command -v ore-catalogo || true)}"
  if [ -z "$CATALOGO_BIN" ]; then
    for d in target/debug target/release; do
      [ -x "${RAIZ:-.}/$d/ore-catalogo" ] && { CATALOGO_BIN="${RAIZ:-.}/$d/ore-catalogo"; break; }
    done
  fi
  [ -n "$CATALOGO_BIN" ] || { echo "⛔ no hay binario de \`ore-catalogo\` (cargo build -p ore-catalogo)" >&2; exit 1; }
  CATALOGO_ESQUEMA="t_f_$(printf '%s' "$1" | tr -c 'a-z0-9' '_')_$$"
  psql "$ORE_CATALOGO_PRUEBAS" -qX -v ON_ERROR_STOP=1 -c "CREATE SCHEMA $CATALOGO_ESQUEMA" >/dev/null \
    || { echo "⛔ no se pudo crear el esquema $CATALOGO_ESQUEMA en ORE_CATALOGO_PRUEBAS" >&2; exit 1; }
  CATALOGO_URL_FICHERO="$(mktemp)"
  printf '%s' "$ORE_CATALOGO_PRUEBAS" > "$CATALOGO_URL_FICHERO"
  export CATALOGO_URL="$ORE_CATALOGO_PRUEBAS"
  export ORE_PUNTEROS="$CATALOGO_BIN punteros --esquema $CATALOGO_ESQUEMA"
  ARGS_DEL_CATALOGO="--punteros catalogo --catalogo-esquema $CATALOGO_ESQUEMA --catalogo-url-fichero $CATALOGO_URL_FICHERO"
}

catalogo_soltar() {
  [ -n "${CATALOGO_ESQUEMA:-}" ] || return 0
  psql "$ORE_CATALOGO_PRUEBAS" -qX -c "DROP SCHEMA IF EXISTS $CATALOGO_ESQUEMA CASCADE" >/dev/null 2>&1 || true
  rm -f "${CATALOGO_URL_FICHERO:-}"
}

# El protocolo de la puerta (ore_core::punteros::protocolo), una petición tras abrirla en `ref`.
_catalogo_pedir() { # <ref o vacío> <petición JSON> → la respuesta
  local ref=null
  [ -n "$1" ] && ref="\"$1\""
  printf '%s\n%s\n' "{\"op\":\"abrir\",\"raiz\":\".\",\"carpeta\":null,\"ajenos\":null,\"ref\":$ref}" "$2" \
    | CATALOGO_URL="$ORE_CATALOGO_PRUEBAS" "$CATALOGO_BIN" punteros --esquema "$CATALOGO_ESQUEMA" \
    | tail -1
}

puntero() { # <nombre> [rama] → el puntero tal como está guardado, o nada si no lo hay
  _catalogo_pedir "${2:-}" "{\"op\":\"uno\",\"nombre\":\"$1\"}" | python3 -c '
import json, sys
r = json.loads(sys.stdin.read() or "{}")
if "ok" not in r:
    sys.exit("la puerta del catálogo contestó: %s" % r)
if r["ok"] is not None:
    print(r["ok"]["texto"])'
}

quitar_puntero() { # <nombre> [rama]
  _catalogo_pedir "${2:-}" "{\"op\":\"escribir\",\"cambios\":[{\"nombre\":\"$1\",\"que\":\"retirar\",\"esperado\":\"cualquiera\"}],\"contexto\":{\"autor\":\"prueba\",\"operacion\":\"prueba\",\"idempotencia\":null}}" \
    | grep -q '"ok":true' || { echo "⛔ no se pudo quitar el puntero de $1" >&2; return 1; }
}

poner_puntero() { # <nombre> <puntero JSON> [rama] — `PUNTERO_AUTOR` es quien lo escribe (la historia lo guarda)
  local c
  c=$(python3 -c '
import json, sys
print(json.dumps({"op": "escribir",
                  "cambios": [{"nombre": sys.argv[1], "que": {"sustituir": json.loads(sys.argv[2])}, "esperado": "cualquiera"}],
                  "contexto": {"autor": sys.argv[3], "operacion": "prueba", "idempotencia": None}}))' "$1" "$2" "${PUNTERO_AUTOR:-prueba}")
  _catalogo_pedir "${3:-}" "$c" | grep -q '"ok":true' || {
    echo "⛔ no se pudo poner el puntero de $1" >&2
    return 1
  }
}
