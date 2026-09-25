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
# Y lo que E3 anadio (0027 ⑥, la ficha es una fila):
#
#   4e `GET /perfiles`            la lista tal como Bastion la publica
#   4f sin backend                `estado.fase: provisioning`, con motivo ·
#                                 `uso {hoy, mes}` a cero
#   4g un backend up sirve el id  `running` · `estado.backends` lo nombra
#      (el de verdad lo sondea: `--como-backend` contesta /v1/models)
#   4h el uso del mes             `uso.hoy` y `uso.mes` suman las filas del
#                                 gateway (solo el banco: al de verdad no se
#                                 le puede inyectar uso sin un token)
#   4i el autor                   `autor` = quien firmo el commit, `desde`
#   7b suscrito sin documento     una fila `retiring`, `declarado: false`
#   8b el gateway caido           `GET /modelos` contesta igual: `fase: error`
#                                 con el motivo, `gateway.contesta: false`
#
# Y lo que 0041 (v1alpha15) cambio: el modelo vive en un paquete y un schema.
#
#   4  el alta escribe `packages/ventas/modelos/v2-lite.yaml` (v1alpha15, con
#      `namespace`), y su referencia —la de `GET`/`DELETE`— es `ventas.v2-lite`
#   5b sin `paquete`              422: el modelo tiene que vivir en algun sitio
#   5c un schema que no se declara 422
#   5d el mismo nombre en `ventas.espana`  201: otro modelo · su fichero en la
#                                 carpeta del schema · el arbol compila
#   5e retirar uno de los dos     200 y la suscripcion SE QUEDA: el otro sirve
#                                 el mismo id
#   8b uno de antes en la raiz    sigue saliendo en la lista
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
SRV=""; GW=""; VLLM=""
PUERTO_VLLM="${PUERTO_VLLM:-8871}"
HOY=$(date -u +%F)
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
  [ -n "$VLLM" ] && { kill "$VLLM" 2>/dev/null; wait "$VLLM" 2>/dev/null; }
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
rm -rf "$REPO/modelos"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "el arbol de partida no compila"
# con historia: `autor` y `desde` salen del commit que trajo el fichero (4i)
( cd "$REPO" && git init -q && git -c user.name=banco -c user.email=banco@invalido add -A \
  && git -c user.name=banco -c user.email=banco@invalido commit -q -m "el arbol de partida" ) || falla "no se pudo dar historia al arbol"

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
  "$BASTION_GATEWAY" --listen "127.0.0.1:$((PUERTO_GW - 1000))" --admin "127.0.0.1:$PUERTO_GW" --db "$TMP/gw.db" --health-every 1 >"$TMP/gw.txt" 2>&1 &
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
COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "422" ] || falla "0 · sin --modelos el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q -- "--modelos" || falla "0 · el 422 no dice que falta --modelos: $(cuerpo)"
[ ! -f "$REPO/packages/ventas/modelos/v2-lite.yaml" ] || falla "0 · escribio el modelo sin gateway"
dice "0 · sin \`--modelos\`: 422, y nada en el arbol"
para

# ── 1–3 · el encaje de ⑦ ────────────────────────────────────────────────────
arranca --perfiles "$TMP/perfiles.json" --modelos "127.0.0.1:$PUERTO_GW"
COD=$(post '{"paquete":"ventas","name":"gpt","profile":"g1/gpt-5"}')
[ "$COD" = "422" ] || falla "1 · un perfil que no esta devolvio $COD: $(cuerpo)"
cuerpo | grep -q "g1/deepseek-v2-lite" || falla "1 · el 422 no dice que perfiles hay: $(cuerpo)"
[ ! -f "$REPO/packages/ventas/modelos/gpt.yaml" ] || falla "1 · escribio un modelo con perfil inexistente"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "gpt" && falla "1 · suscribio un perfil inexistente"
dice "1 · perfil que no esta: 422 con los que hay · nada escrito · nada suscrito"

COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite","tier":"dedicated"}')
[ "$COD" = "422" ] || falla "2 · dedicated devolvio $COD: $(cuerpo)"
cuerpo | grep -q "cuota" || falla "2 · el 422 no habla de cuota: $(cuerpo)"
dice "2 · \`tier: dedicated\`: 422, sin cuota de maquina (E5)"

COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite","digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}')
[ "$COD" = "422" ] || falla "3 · un digest que el perfil no publica devolvio $COD: $(cuerpo)"
cuerpo | grep -q "B4" || falla "3 · el 422 no dice por que: $(cuerpo)"
dice "3 · \`digest\` sin que el perfil publique el suyo: 422"

# ── 4 · el alta que cierra: documento + suscripcion en el mismo acto ────────
COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite","description":"DeepSeek-V2-Lite en un g1"}')
[ "$COD" = "201" ] || falla "4 · el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"model":"deepseek-ai/DeepSeek-V2-Lite"' || falla "4 · no resuelve el id servido: $(cuerpo)"
cuerpo | grep -q '"url":"http://127.0.0.1:8000/v1"' || falla "4 · no da la puerta: $(cuerpo)"
[ -f "$REPO/packages/ventas/modelos/v2-lite.yaml" ] || falla "4 · el fichero no esta en el arbol"
grep -q "kind: Model" "$REPO/packages/ventas/modelos/v2-lite.yaml" || falla "4 · el fichero no es un Model"
grep -q "apiVersion: oos.dev/v1alpha15" "$REPO/packages/ventas/modelos/v2-lite.yaml" || falla "4 · el fichero no declara v1alpha15"
grep -q "namespace: ventas" "$REPO/packages/ventas/modelos/v2-lite.yaml" || falla "4 · el fichero no dice su paquete"
cuerpo | grep -q '"ref":"ventas.v2-lite"' || falla "4 · el alta no da la referencia: $(cuerpo)"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "4 · el arbol no compila con el modelo escrito"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  || falla "4 · la celda no quedo suscrita en el gateway: $(curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants")"
dice "4 · 201 · packages/ventas/modelos/v2-lite.yaml (v1alpha15, ref ventas.v2-lite) · victor suscrita a deepseek-ai/DeepSeek-V2-Lite · el arbol compila"

curl -sf -H "$SUJ" "$BASE/modelos" > "$TMP/lista.json" || falla "4 · GET /modelos no contesta"
grep -q '"name":"v2-lite"' "$TMP/lista.json" || falla "4 · GET /modelos no lo lista: $(cat "$TMP/lista.json")"
grep -q '"certificado":true' "$TMP/lista.json" || falla "4 · GET /modelos no lo da por certificado"
curl -sf -H "$SUJ" "$BASE/modelos/ventas.v2-lite" | grep -q '"model":"deepseek-ai/DeepSeek-V2-Lite"' \
  || falla "4 · GET /modelos/ventas.v2-lite no resuelve"
COD=$(curl -s -o /dev/null -w '%{http_code}' -H "$SUJ" "$BASE/modelos/nadie")
[ "$COD" = "404" ] || falla "4 · un modelo que no existe devolvio $COD"
dice "4 · GET /modelos lo lista con model y url · GET /modelos/ventas.v2-lite resuelve · uno que no esta, 404"

# ── 4e–4i · la ficha es una fila (E3 ⑥) ─────────────────────────────────────
curl -sf -H "$SUJ" "$BASE/perfiles" > "$TMP/perfiles-vistos.json" || falla "4e · GET /perfiles no contesta"
grep -q '"g1/deepseek-v2-lite"' "$TMP/perfiles-vistos.json" || falla "4e · GET /perfiles no trae el g1: $(cat "$TMP/perfiles-vistos.json")"
grep -q '"g4/qwen3-235b-fp8"' "$TMP/perfiles-vistos.json" || falla "4e · GET /perfiles no trae el g4"
grep -q '"v":1' "$TMP/perfiles-vistos.json" || falla "4e · GET /perfiles no es la lista tal cual (falta v)"
dice "4e · GET /perfiles: la lista tal como Bastion la publica (g1, g4, v:1)"

grep -q '"fase":"provisioning"' "$TMP/lista.json" || falla "4f · sin backend la fase no es provisioning: $(cat "$TMP/lista.json")"
grep -q '"motivo":"ning' "$TMP/lista.json" || falla "4f · provisioning sin motivo: $(cat "$TMP/lista.json")"
grep -q '"uso":{"hoy":{"peticiones":0,"tokens":0,"usd":"0.0000"},"mes":{"peticiones":0' "$TMP/lista.json" || falla "4f · el uso no sale a cero: $(cat "$TMP/lista.json")"
grep -q '"gateway":{"backends_arriba":0,"contesta":true' "$TMP/lista.json" || falla "4f · la lista no dice que el gateway contesta: $(cat "$TMP/lista.json")"
dice "4f · sin backend: provisioning, con motivo · uso {hoy, mes} a cero · el gateway contesta"

"$PY" "$RAIZ/pruebas-de-fuego/gateway-de-banco.py" --port "$PUERTO_VLLM" --como-backend >"$TMP/vllm.txt" 2>&1 &
VLLM=$!
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PUERTO_VLLM/v1/models" && break; sleep 0.25; done
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST "http://127.0.0.1:$PUERTO_GW/admin/backends" -H 'Content-Type: application/json' \
  -d "{\"id\":\"banco-g1\",\"url\":\"http://127.0.0.1:$PUERTO_VLLM\",\"model\":\"deepseek-ai/DeepSeek-V2-Lite\",\"sovereignty\":\"eu-dc\"}")
[ "$COD" = "201" ] || falla "4g · el gateway no acepto el backend: $COD $(cuerpo)"
for _ in $(seq 1 40); do curl -s "http://127.0.0.1:$PUERTO_GW/admin/health" | grep -q '"up":true' && break; sleep 0.25; done
curl -sf -H "$SUJ" "$BASE/modelos/ventas.v2-lite" > "$TMP/ficha.json" || falla "4g · GET /modelos/ventas.v2-lite no contesta"
grep -q '"fase":"running"' "$TMP/ficha.json" || falla "4g · con un backend arriba la fase no es running: $(cat "$TMP/ficha.json")"
grep -q '"backends":\["banco-g1"\]' "$TMP/ficha.json" || falla "4g · estado.backends no nombra al que sirve: $(cat "$TMP/ficha.json")"
grep -q '"motivo":false' "$TMP/ficha.json" || falla "4g · running con motivo: $(cat "$TMP/ficha.json")"
dice "4g · un backend up sirve el id: running · estado.backends = [banco-g1]"

if [ -z "${BASTION_GATEWAY:-}" ]; then
  MES="${HOY%-*}-01"
  for fila in "{\"day\":\"$HOY\",\"requests\":3,\"prompt_tokens\":100,\"completion_tokens\":50,\"usd\":0.0012}" \
              "{\"day\":\"$MES\",\"requests\":2,\"prompt_tokens\":10,\"completion_tokens\":5,\"usd\":0.0003}" \
              "{\"day\":\"$HOY\",\"requests\":9,\"model\":\"otro/modelo\",\"usd\":9}"; do
    curl -s -o /dev/null -X POST "http://127.0.0.1:$PUERTO_GW/banco/uso" -H 'Content-Type: application/json' \
      -d "$(echo "$fila" | sed 's/^{/{"tenant":"victor","model":"deepseek-ai\/DeepSeek-V2-Lite",/')"
  done
  curl -sf -H "$SUJ" "$BASE/modelos/ventas.v2-lite" > "$TMP/ficha.json" || falla "4h · GET /modelos/ventas.v2-lite no contesta"
  grep -q '"hoy":{"peticiones":3,"tokens":150,"usd":"0.0012"}' "$TMP/ficha.json" || falla "4h · uso.hoy no suma las filas de hoy: $(cat "$TMP/ficha.json")"
  grep -q '"mes":{"peticiones":5,"tokens":165,"usd":"0.0015"}' "$TMP/ficha.json" || falla "4h · uso.mes no suma el mes: $(cat "$TMP/ficha.json")"
  dice "4h · uso.hoy 3 peticiones/150 tokens/0.0012 · uso.mes 5/165/0.0015 · el otro modelo no cuenta"
else
  dice "4h · (el uso no se inyecta en el de verdad sin un token: se mira la forma)"
  grep -q '"uso":{"hoy":{"peticiones":0' "$TMP/ficha.json" || falla "4h · la ficha no trae uso: $(cat "$TMP/ficha.json")"
fi

( cd "$REPO" && git add -A && GIT_AUTHOR_NAME=persona:ana GIT_AUTHOR_EMAIL=ana@invalido \
  git -c user.name=ore-serve -c user.email=ore-serve@invalido commit -q -m "alta de un modelo" ) || falla "4i · no se pudo firmar el commit"
curl -sf -H "$SUJ" "$BASE/modelos/ventas.v2-lite" > "$TMP/ficha.json" || falla "4i · GET /modelos/ventas.v2-lite no contesta"
grep -q '"autor":"persona:ana"' "$TMP/ficha.json" || falla "4i · la ficha no lleva al autor del commit: $(cat "$TMP/ficha.json")"
grep -q '"desde":"20' "$TMP/ficha.json" || falla "4i · la ficha no lleva la fecha: $(cat "$TMP/ficha.json")"
dice "4i · autor = persona:ana (quien firmo el commit) · desde = la fecha del commit"

COD=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE "http://127.0.0.1:$PUERTO_GW/admin/backends/banco-g1")
[ "$COD" = "204" ] || falla "4g · no se pudo retirar el backend: $COD"
kill "$VLLM" 2>/dev/null; wait "$VLLM" 2>/dev/null; VLLM=""
curl -sf -H "$SUJ" "$BASE/modelos/ventas.v2-lite" | grep -q '"fase":"provisioning"' || falla "4g · sin el backend la fase no vuelve a provisioning"
dice "4g · el backend se retira: provisioning otra vez"

# ── 5 · otra vez ────────────────────────────────────────────────────────────
COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "409" ] || falla "5 · el mismo nombre otra vez devolvio $COD: $(cuerpo)"
dice "5 · el mismo nombre otra vez: 409"

# ── 5b–5e · 0041: el sitio ──────────────────────────────────────────────────
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X POST -H "$SUJ" "$BASE/modelos" -d '{"name":"sinsitio","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "422" ] || falla "5b · sin paquete devolvio $COD: $(cuerpo)"
cuerpo | grep -q "paquete" || falla "5b · el 422 no dice que falta el paquete: $(cuerpo)"
dice "5b · sin \`paquete\`: 422"

COD=$(post '{"paquete":"ventas","schema":"espana","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "422" ] || falla "5c · un schema sin declarar devolvio $COD: $(cuerpo)"
cuerpo | grep -q "no declara el schema" || falla "5c · el 422 no dice por que: $(cuerpo)"
dice "5c · un schema que el paquete no declara: 422"

mkdir -p "$REPO/packages/ventas/espana"
printf '%s\n' 'apiVersion: oos.dev/v1alpha13' 'kind: Schema' 'metadata:' '  name: espana' '  namespace: ventas' > "$REPO/packages/ventas/espana/schema.yaml"
COD=$(post '{"paquete":"ventas","schema":"espana","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "201" ] || falla "5d · el mismo nombre en otro schema devolvio $COD: $(cuerpo)"
cuerpo | grep -q '"ref":"ventas.espana.v2-lite"' || falla "5d · la referencia no lleva el schema: $(cuerpo)"
[ -f "$REPO/packages/ventas/espana/modelos/v2-lite.yaml" ] || falla "5d · el fichero no esta en la carpeta del schema"
grep -q "schema: espana" "$REPO/packages/ventas/espana/modelos/v2-lite.yaml" || falla "5d · el fichero no dice su schema"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "5d · el arbol no compila con los dos: $( cd "$REPO" && "$ORE" validate . 2>&1 | tail -5)"
curl -sf -H "$SUJ" "$BASE/modelos/ventas.espana.v2-lite" | grep -q '"schema":"espana"' || falla "5d · GET /modelos/ventas.espana.v2-lite no lo da"
COD=$(post '{"paquete":"ventas","schema":"espana","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "409" ] || falla "5d · el mismo nombre en el mismo schema devolvio $COD"
dice "5d · el mismo nombre en ventas.espana: 201, otro modelo, en su carpeta · el arbol compila · otra vez ahi, 409"

COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/ventas.espana.v2-lite")
[ "$COD" = "200" ] || falla "5e · retirar el de espana devolvio $COD: $(cuerpo)"
cuerpo | grep -q "se queda" || falla "5e · no dice que la suscripcion se queda: $(cuerpo)"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  || falla "5e · retiro la suscripcion aunque ventas.v2-lite sirve el mismo id"
rm -rf "$REPO/packages/ventas/espana"
dice "5e · retirar uno de los dos: 200, y la suscripcion se queda (el otro sirve el mismo id)"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X PUT -H "$SUJ" "$BASE/documentos/Model/ventas/otro" -d '{"spec":{"profile":"g1/deepseek-v2-lite","tier":"shared","task":"chat"}}')
[ "$COD" = "405" ] || falla "5f · /documentos escribio un Model sin suscripcion: $COD $(cuerpo)"
curl -sf -H "$SUJ" "$BASE/documentos/Model" | grep -q '"name":"v2-lite"' || falla "5f · /documentos no lee el Model"
dice "5f · /documentos lee el Model pero no lo escribe (405): el documento y la suscripcion van juntos"

# ── 6 · una Function lo nombra: no se retira ────────────────────────────────
cp "$TMP/segmentar.yaml" "$FUNCION"
( cd "$REPO" && "$ORE" validate . >/dev/null 2>&1 ) || falla "6 · el arbol con la Function no compila"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/ventas.v2-lite")
[ "$COD" = "409" ] || falla "6 · retirar un modelo que una Function nombra devolvio $COD: $(cuerpo)"
cuerpo | grep -q "ventas.segmentar\` (model)" || falla "6 · el 409 no dice que funcion lo nombra: $(cuerpo)"
cuerpo | grep -q "no se retira" || falla "6 · el 409 no dice que alguien lo nombra: $(cuerpo)"
[ -f "$REPO/packages/ventas/modelos/v2-lite.yaml" ] || falla "6 · retiro el fichero aunque una Function lo nombra"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  || falla "6 · retiro la suscripcion aunque no retiro el modelo"
dice "6 · una Function lo nombra: DELETE 409 con quien (ventas.segmentar), el fichero y la suscripcion se quedan"

# ── 7 · sin nadie que lo nombre: fuera, y la suscripcion fuera ──────────────
rm -f "$FUNCION"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/ventas.v2-lite")
[ "$COD" = "200" ] || falla "7 · retirar devolvio $COD: $(cuerpo)"
[ ! -f "$REPO/packages/ventas/modelos/v2-lite.yaml" ] || falla "7 · el fichero sigue ahi"
curl -s "http://127.0.0.1:$PUERTO_GW/admin/tenants" | grep -q "deepseek-ai/DeepSeek-V2-Lite" \
  && falla "7 · la suscripcion sigue en el gateway"
COD=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/ventas.v2-lite")
[ "$COD" = "404" ] || falla "7 · retirar dos veces devolvio $COD"
dice "7 · DELETE 200 · el fichero fuera · la suscripcion fuera · otra vez, 404"

# ── 7b · suscrito en el gateway sin documento: deriva, fila retiring ────────
curl -s -o /dev/null -X POST "http://127.0.0.1:$PUERTO_GW/admin/tenants/victor/models" -H 'Content-Type: application/json' -d '{"model":"deepseek-ai/DeepSeek-V2-Lite"}'
curl -sf -H "$SUJ" "$BASE/modelos" > "$TMP/lista.json" || falla "7b · GET /modelos no contesta"
grep -q '"fase":"retiring"' "$TMP/lista.json" || falla "7b · la suscripcion sin documento no sale como retiring: $(cat "$TMP/lista.json")"
grep -q '"declarado":false' "$TMP/lista.json" || falla "7b · la fila de deriva se da por declarada: $(cat "$TMP/lista.json")"
grep -q '"name":false,"uso"' "$TMP/lista.json" || falla "7b · la fila de deriva lleva nombre: $(cat "$TMP/lista.json")"
curl -s -o /dev/null -X DELETE "http://127.0.0.1:$PUERTO_GW/admin/tenants/victor/models/deepseek-ai%2FDeepSeek-V2-Lite"
curl -sf -H "$SUJ" "$BASE/modelos" | grep -q '"modelos":\[\]' || falla "7b · retirada la suscripcion la lista no queda vacia"
dice "7b · suscrito sin documento: una fila retiring, declarado: false · retirada, la lista vacia"
# ── 7c · el arbol no compila por OTRA cosa: el modelo entra igual (no empeora) ──
# Medido en demo (0029 F4a I3): cinco bases foraneas con `owner: cambiame`
# (OOS2009, decisiones sin contestar) bloqueaban el alta de un modelo que no
# las toca. La regla es no EMPEORAR, la misma que la Forge y que retirar.
mkdir -p "$REPO/packages/roto"
cat > "$REPO/packages/roto/package.yaml" <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: roto, version: 0.1.0, status: active, domain: sales }
spec: { owner: cambiame }
Y
( cd "$REPO" && "$ORE" validate . 2>&1 | grep -q OOS2009 ) || falla "7c · la premisa no vale: el arbol con `roto` no da OOS2009"
COD=$(post '{"paquete":"ventas","name":"v2-lite","profile":"g1/deepseek-v2-lite"}')
[ "$COD" = "201" ] || falla "7c · con el arbol roto por otra base el alta devolvio $COD: $(cuerpo)"
[ -f "$REPO/packages/ventas/modelos/v2-lite.yaml" ] || falla "7c · no escribio el modelo"
COD=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -X DELETE -H "$SUJ" "$BASE/modelos/ventas.v2-lite")
[ "$COD" = "200" ] || falla "7c · retirar con el arbol roto por otra base devolvio $COD: $(cuerpo)"
rm -rf "$REPO/packages/roto"
dice "7c · el arbol roto por OTRA base (OOS2009): el modelo entra (201) y sale (200): no empeorar, no compilar entero"

# ── 8 · el gateway caido: nada se escribe ───────────────────────────────────
kill "$GW" 2>/dev/null; wait "$GW" 2>/dev/null; GW=""
COD=$(post '{"paquete":"ventas","name":"qwen","profile":"g4/qwen3-235b-fp8"}')
[ "$COD" = "502" ] || falla "8 · con el gateway caido el alta devolvio $COD: $(cuerpo)"
cuerpo | grep -q "no se escribi" || falla "8 · el 502 no dice que no escribio: $(cuerpo)"
[ ! -f "$REPO/packages/ventas/modelos/qwen.yaml" ] || falla "8 · escribio el modelo con el gateway caido"
dice "8 · el gateway caido: 502, y nada en el arbol"

# ── 8b · el gateway caido: la lista contesta igual, y dice error ────────────
mkdir -p "$REPO/modelos"
printf '%s\n' 'apiVersion: oos.dev/v1alpha9' 'kind: Model' 'metadata: { name: qwen }' 'spec: { profile: g4/qwen3-235b-fp8, tier: shared, task: chat }' > "$REPO/modelos/qwen.yaml"
T0=$(date +%s)
curl -sf -H "$SUJ" "$BASE/modelos" > "$TMP/lista.json" || falla "8b · con el gateway caido GET /modelos no contesta"
T1=$(date +%s)
grep -q '"name":"qwen"' "$TMP/lista.json" || falla "8b · la ficha no sale: $(cat "$TMP/lista.json")"
grep -q '"fase":"error"' "$TMP/lista.json" || falla "8b · con el gateway caido la fase no es error: $(cat "$TMP/lista.json")"
grep -q '"contesta":false' "$TMP/lista.json" || falla "8b · la lista no dice que el gateway no contesta: $(cat "$TMP/lista.json")"
grep -q '"motivo":"no se pudo conectar' "$TMP/lista.json" || falla "8b · el error no dice por que: $(cat "$TMP/lista.json")"
[ $((T1 - T0)) -le 5 ] || falla "8b · la lista tardo $((T1 - T0)) s con el gateway caido: la consola esperaria"
rm -rf "$REPO/modelos"
dice "8b · el gateway caido: la ficha sale igual, fase error con el motivo, gateway.contesta false, en $((T1 - T0)) s"
para

echo "✓ los tres verbos del modelo y la fila (E3): 0–8 ($QUE)"
