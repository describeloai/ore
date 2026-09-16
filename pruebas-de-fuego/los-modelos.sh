#!/usr/bin/env bash
# LOS TRES VERBOS DEL MODELO (0027 E1), contra un `ore-serve` de verdad.
#
# Lo que fija, y es la aceptacion de E1 tal como la escribio la ADR (⑦):
#
#   0  sin `--modelos`            POST 422 y NADA en el arbol: un modelo sin
#                                 suscripcion es una promesa
#   1  perfil que no esta         422 con los que si estan · nada escrito ·
#                                 nada suscrito
#   2  `tier: dedicated`          422 (E5: sin cuota de maquina)
#   3  `digest` sin que el perfil publique el suyo   422
#   4  perfil certificado         201 · `modelos/<n>.yaml` en el arbol · la
#                                 celda suscrita en el gateway al id servido ·
#                                 `GET /modelos` lo lista con `model` y `url`
#                                 (la resolucion de `modelo/<n>`) · `GET
#                                 /modelos/<n>` · el arbol compila
#   5  el mismo nombre otra vez   409
#   6  una Function lo nombra     DELETE 409 y el fichero SE QUEDA (OOS2005)
#   7  sin nadie que lo nombre    DELETE 200 · el fichero fuera · la
#                                 suscripcion fuera
#   8  el gateway caido           POST 502 y NADA en el arbol
#
# El gateway es el de banco (`gateway-de-banco.py`: el contrato del plano de
# control de Bastion B3) o, con `BASTION_GATEWAY=<binario>`, el de verdad.
# La lista de perfiles es un fichero con la forma que Bastion publica.
#
# Uso:  bash pruebas-de-fuego/los-modelos.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8907}"
PUERTO_GW="${PUERTO_GW:-9871}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""; GW=""
PY=$(command -v python3 || command -v python)

falla() {
  echo "✗ $*" >&2
  [ -s "$TMP/arranque.txt" ] && { echo "── lo que dijo el servidor ──" >&2; tail -20 "$TMP/arranque.txt" >&2; }
  limpiar; exit 1
}
dice()  { echo "  · $*"; }
limpiar() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  [ -n "$GW" ] && { kill "$GW" 2>/dev/null; wait "$GW" 2>/dev/null; }
  sleep 0.5   # que el gateway suelte su base antes de borrarla (Windows)
  rm -rf "$TMP"
}
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\` — cargo build -p ore-cli"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\` — cargo build -p ore-serve"

# ── el arbol: el de la conformidad de v1alpha9, sin el modelo (lo pone el verbo) ──
REPO="$TMP/repo"
cp -r "$RAIZ/vendor/oos/conformance/v1alpha9/valid/a-function-invokes-a-model/input" "$REPO"
FUNCION="$REPO/functions/segmentar.yaml"
mv "$FUNCION" "$TMP/segmentar.yaml"
rm -f "$REPO/modelos/v2-lite.yaml"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila"

# ── la lista de perfiles, con la forma que Bastion publica ──────────────────
cat > "$TMP/perfiles.json" <<'JSON'
{"v": 1, "image": "bastion/env@sha256:0", "generated": "2026-09-16T17:10:33Z", "profiles": [
  {"profile": "g1/deepseek-v2-lite", "model": "deepseek-ai/DeepSeek-V2-Lite", "machine": "g1", "gpus": 1,
   "status": "validated", "tok_s": {"1": 209.0, "32": 1507.0}, "usd_h": 1.5, "usd_per_mtok": 0.2765, "digest": null},
  {"profile": "g4/qwen3-235b-fp8", "model": "Qwen/Qwen3-235B-A22B-Instruct-2507-FP8", "machine": "g4", "gpus": 4,
   "status": "validated", "tok_s": {"32": 587.0}, "usd_h": 5.87, "usd_per_mtok": 2.7778, "digest": null}
]}
JSON

# ── el gateway: el de banco, o el de verdad ─────────────────────────────────
if [ -n "${BASTION_GATEWAY:-}" ]; then
  "$BASTION_GATEWAY" --listen "127.0.0.1:$((PUERTO_GW - 1000))" --admin "127.0.0.1:$PUERTO_GW" --db "$TMP/gw.db" >"$TMP/gw.txt" 2>&1 &
  GW=$!; QUE="bastion-gateway de verdad"
else
  "$PY" "$RAIZ/pruebas-de-fuego/gateway-de-banco.py" --port "$PUERTO_GW" >"$TMP/gw.txt" 2>&1 &
  GW=$!; QUE="gateway de banco"
fi
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PUERTO_GW/admin/health" && break; sleep 0.25; done
curl -sf -o /dev/null "http://127.0.0.1:$PUERTO_GW/admin/health" || falla "el gateway no arranco: $(cat "$TMP/gw.txt")"
dice "gateway: $QUE en :$PUERTO_GW"

arranca() { # [opciones extra] → SRV
  "$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
           --identidad cabecera --no-es-produccion --organizacion victor "$@" >"$TMP/arranque.txt" 2>&1 &
  SRV=$!
  for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
}
para() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null; SRV=""; }
SUJ='x-ore-sujeto: persona:ana'
post() { curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/modelos" -d "$1"; }
cuerpo() { cat "$TMP/r.json"; }

# ── 0 · sin gateway configurado ─────────────────────────────────────────────
arranca --perfiles "$TMP/perfiles.json"
COD=$(post '{"name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "422" ] || falla "0 · sin --modelos el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q -- "--modelos" || falla "0 · el 422 no dice que falta --modelos: $(cuerpo)"
[ ! -f "$REPO/modelos/v2-lite.yaml" ] || falla "0 · escribio el modelo sin gateway"
dice "0 · sin \`--modelos\`: 422, y nada en el arbol"
para

# ── 1–3 · el encaje de ⑦ ────────────────────────────────────────────────────
arranca --perfiles "$TMP/perfiles.json" --modelos "127.0.0.1:$PUERTO_GW"
COD=$(post '{"name":"gpt","profile":"g1/gpt-5"}')
[ "$COD" = "422" ] || falla "1 · un perfil que no esta devolvio $COD: $(cuerpo)"
cuerpo | grep -q "g1/deepseek-v2-lite" || falla "1 · el 422 no dice que perfiles hay: $(cuerpo)"
[ ! -f "$REPO/modelos/gpt.yaml" ] || falla "1 · escribio un modelo con perfil inexistente"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "gpt" && falla "1 · suscribio un perfil inexistente"
dice "1 · perfil que no esta: 422 con los que hay · nada escrito · nada suscrito"

COD=$(post '{"name":"v2-lite","profile":"g1/deepseek-v2-lite","tier":"dedicated"}')
[ "$COD" = "422" ] || falla "2 · dedicated devolvio $COD: $(cuerpo)"
cuerpo | grep -q "cuota" || falla "2 · el 422 no habla de cuota: $(cuerpo)"
dice "2 · \`tier: dedicated\`: 422, sin cuota de maquina (E5)"

COD=$(post '{"name":"v2-lite","profile":"g1/deepseek-v2-lite","digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}')
[ "$COD" = "422" ] || falla "3 · un digest que el perfil no publica devolvio $COD: $(cuerpo)"
cuerpo | grep -q "B4" || falla "3 · el 422 no dice por que: $(cuerpo)"
dice "3 · \`digest\` sin que el perfil publique el suyo: 422"

# ── 4 · el alta que cierra: documento + suscripcion en el mismo acto ────────
COD=$(post '{"name":"v2-lite","profile":"g1/deepseek-v2-lite","description":"DeepSeek-V2-Lite en un g1"}')
[ "$COD" = "201" ] || falla "4 · el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"model":"deepseek-ai/DeepSeek-V2-Lite"' || falla "4 · no resuelve el id servido: $(cuerpo)"
cuerpo | grep -q '"url":"http://127.0.0.1:8000/v1"' || falla "4 · no da la puerta: $(cuerpo)"
[ -f "$REPO/modelos/v2-lite.yaml" ] || falla "4 · el fichero no esta en el arbol"
grep -q "kind: Model" "$REPO/modelos/v2-lite.yaml" || falla "4 · el fichero no es un Model"
grep -q "apiVersion: oos.dev/v1alpha9" "$REPO/modelos/v2-lite.yaml" || falla "4 · el fichero no declara v1alpha9"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "4 · el arbol no compila con el modelo escrito"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  || falla "4 · la celda no quedo suscrita en el gateway: $(curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants")"
dice "4 · 201 · modelos/v2-lite.yaml (v1alpha9) · victor suscrita a deepseek-ai/DeepSeek-V2-Lite · el arbol compila"

curl -sf -H "$SUJ" "$BASE/modelos" > "$TMP/lista.json" || falla "4 · GET /modelos no contesta"
grep -q '"name":"v2-lite"' "$TMP/lista.json" || falla "4 · GET /modelos no lo lista: $(cat "$TMP/lista.json")"
grep -q '"certificado":true' "$TMP/lista.json" || falla "4 · GET /modelos no lo da por certificado"
curl -sf -H "$SUJ" "$BASE/modelos/v2-lite" | grep -q '"model":"deepseek-ai/DeepSeek-V2-Lite"' \
  || falla "4 · GET /modelos/v2-lite no resuelve"
COD=$(curl -s -o /dev/null -w '%{http_code}' -H "$SUJ" "$BASE/modelos/nadie")
[ "$COD" = "404" ] || falla "4 · un modelo que no existe devolvio $COD"
dice "4 · GET /modelos lo lista con model y url · GET /modelos/v2-lite resuelve · uno que no esta, 404"

# ── 5 · otra vez ────────────────────────────────────────────────────────────
COD=$(post '{"name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "409" ] || falla "5 · el mismo nombre otra vez devolvio $COD: $(cuerpo)"
dice "5 · el mismo nombre otra vez: 409"

# ── 6 · una Function lo nombra: no se retira ────────────────────────────────
cp "$TMP/segmentar.yaml" "$FUNCION"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "6 · el arbol con la Function no compila"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/v2-lite")
[ "$COD" = "409" ] || falla "6 · retirar un modelo que una Function nombra devolvio $COD: $(cuerpo)"
cuerpo | grep -q "OOS2005" || falla "6 · el 409 no dice que no resuelve: $(cuerpo)"
cuerpo | grep -q "no se retira" || falla "6 · el 409 no dice que alguien lo nombra: $(cuerpo)"
[ -f "$REPO/modelos/v2-lite.yaml" ] || falla "6 · retiro el fichero aunque una Function lo nombra"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  || falla "6 · retiro la suscripcion aunque no retiro el modelo"
dice "6 · una Function lo nombra: DELETE 409 (OOS2005), el fichero y la suscripcion se quedan"

# ── 7 · sin nadie que lo nombre: fuera, y la suscripcion fuera ──────────────
rm -f "$FUNCION"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/v2-lite")
[ "$COD" = "200" ] || falla "7 · retirar devolvio $COD: $(cuerpo)"
[ ! -f "$REPO/modelos/v2-lite.yaml" ] || falla "7 · el fichero sigue ahi"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  && falla "7 · la suscripcion sigue en el gateway"
COD=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/v2-lite")
[ "$COD" = "404" ] || falla "7 · retirar dos veces devolvio $COD"
dice "7 · DELETE 200 · el fichero fuera · la suscripcion fuera · otra vez, 404"

# ── 8 · el gateway caido: nada se escribe ───────────────────────────────────
kill "$GW" 2>/dev/null; wait "$GW" 2>/dev/null; GW=""
COD=$(post '{"name":"qwen","profile":"g4/qwen3-235b-fp8"}')
[ "$COD" = "502" ] || falla "8 · con el gateway caido el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q "no se escribi" || falla "8 · el 502 no dice que no escribio: $(cuerpo)"
[ ! -f "$REPO/modelos/qwen.yaml" ] || falla "8 · escribio el modelo con el gateway caido"
dice "8 · el gateway caido: 502, y nada en el arbol"
para

echo "✓ los tres verbos del modelo: 0–8 ($QUE)"
