#!/usr/bin/env bash
# LA MEDIA POR SU PUERTA (0049 B2·2): `ore-serve` decide y `ore-medios` sirve.
#
# Un `ore-serve` de verdad sobre un árbol con una colección (virtual) y su
# puntero, y un `ore-medios` de mentira que devuelve lo que le llegó. Se
# comprueba lo que la puerta hace —encontrar la colección y su clase en el
# árbol, leer el puntero, reenviar con el `metadata_location` y la consulta
# decodificada— y lo que dice cuando no puede:
#
#   1  list   GET /media/legal/archivo/contratos/items?prefix=Nueva%20carpeta%2F → reenvía
#   2  stat   GET …/item?path=… → reenvía path y version
#   3  url    POST …/urls → reenvía items y ttl_s
#   4  lo que no existe → 404 media/no-existe, sin preguntar a ore-medios
#   5  sin ore-medios desplegado → 503 media/no-desplegado
#   6  un puesto entra (está en su lista)
#   7  content de una virtual: la fuente sale del árbol (objectTable →
#      datasource → connectionEnv) y su credencial, del custodio; sin custodio,
#      503 y nada llega a ore-medios (0049 B3·3)
#   8  content de una mantenida → 307 con Location a donde ore-medios dijo
#   9  list con as_of de antes (H1): el metadata_location de esa transacción,
#      de la historia del puntero, y el de hoy en `actual`
#  10  as_of que la colección no tuvo → 404 sin preguntar a ore-medios; el de
#      hoy, sin `actual`; uno que no es una transacción → 422
#  11  el cursor lleva la transacción (H2): la página 2, tras una transacción
#      nueva, sigue en la de la página 1; un cursor y un as_of que no casan → 422
#
# Lo de verdad —el índice sobre el lago y la firma— se mide en vivo (B2·4).
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8931}"
FALSO="${FALSO:-8932}"
TMP="$(mktemp -d)"
BASE="http://127.0.0.1:$PUERTO"
fallos=0
falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
dice() { echo "  ✓ $*"; }
buscar() {
  for d in "${CARGO_TARGET_DIR:-$RAIZ/target}/debug" "$RAIZ/target/debug"; do
    for x in "$d/$1" "$d/$1.exe"; do [ -x "$x" ] && { echo "$x"; return 0; }; done
  done
  return 1
}
ORE="$(buscar ore)" || { echo "no hay binario de ore"; exit 2; }
SERVE="$(buscar ore-serve)" || { echo "no hay binario de ore-serve"; exit 2; }
PY="$(command -v python3 || command -v python)"

# ── el árbol: la colección virtual de la conformidad de v1alpha17, y su puntero
cp -r "$RAIZ/vendor/oos/conformance/v1alpha17/valid/a-query-over-a-collection/input/." "$TMP/arbol/" 2>/dev/null \
  || { mkdir -p "$TMP/arbol"; cp -r "$RAIZ/vendor/oos/conformance/v1alpha17/valid/a-query-over-a-collection/input/." "$TMP/arbol/"; }
mkdir -p "$TMP/arbol/datasets/legal/archivo"
# una mantenida al lado, con su puntero (para `content`, 0049 B3·3)
cat > "$TMP/arbol/packages/legal/archivo/collections/fotos.yaml" <<'EOF'
apiVersion: oos.dev/v1alpha17
kind: MediaCollection
metadata: { name: fotos, namespace: legal, schema: archivo }
spec: { owner: team:legal, media: image, formats: [jpg], from: { objectTable: s3_ventas.docs.fotos } }
EOF
cat > "$TMP/arbol/datasets/legal/archivo/fotos.json" <<'EOF'
{"dataset":"colecciones/legal/archivo/fotos","metadata_location":"gs://lago/ore/v2/colecciones/legal/archivo/fotos/metadata/00001-y.metadata.json","transaccion":"1"}
EOF
# el puntero de contratos con historia (H1): la transacción 2 y luego la 3
G() { git -C "$TMP/arbol" -c user.name=t -c user.email=t@t -c commit.gpgsign=false "$@" >/dev/null; }
G init --quiet
cat > "$TMP/arbol/datasets/legal/archivo/contratos.json" <<'EOF'
{"dataset":"colecciones/legal/archivo/contratos","metadata_location":"gs://lago/ore/v2/colecciones/legal/archivo/contratos/metadata/00002-w.metadata.json","transaccion":"2"}
EOF
G add -A; G commit --quiet -m "tx 2"
cat > "$TMP/arbol/datasets/legal/archivo/contratos.json" <<'EOF'
{"dataset":"colecciones/legal/archivo/contratos","metadata_location":"gs://lago/ore/v2/colecciones/legal/archivo/contratos/metadata/00003-x.metadata.json","transaccion":"3"}
EOF
G add -A; G commit --quiet -m "tx 3"

# ── el ore-medios de mentira: guarda la petición y contesta lo suyo
cat > "$TMP/falso.py" <<'EOF'
import http.server, json, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        cuerpo = self.rfile.read(n).decode()
        open(sys.argv[2], "a").write(json.dumps({"ruta": self.path, "cuerpo": json.loads(cuerpo)}) + "\n")
        r = {"/indice/items": {"as_of": "3", "cursor": "c0ffee", "items": []},
             "/indice/item": {"path": "x", "current": True},
             "/indice/urls": {"urls": [{"item": {"checksum": "crc64nvme:A", "digest": None, "path": "a.pdf", "version": "v1"}, "url": "https://firmada", "ttl_s": 300}]},
             "/indice/abrir": {"url": "https://lago/firmada", "desde": "lago", "ttl_s": 300, "version": "v1", "item": {"path": "a.jpg", "digest": None}}}[self.path]
        b = json.dumps(r).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
EOF
"$PY" "$TMP/falso.py" "$FALSO" "$TMP/pedidas.jsonl" &
FAL=$!
# que conteste antes de empezar (un POST que no es una ruta suya no lo tumba)
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$FALSO/" && break; sleep 0.25; done

arrancar() { # [ORE_MEDIOS_DIRECCION]
  ORE_MEDIOS_DIRECCION="${1:-}" "$SERVE" --repo "$TMP/arbol" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
    --identidad cabecera --no-es-produccion > "$TMP/arranque.txt" 2>&1 &
  SRV=$!
  for _ in $(seq 1 40); do curl -s -o /dev/null "$BASE/salud" && break; sleep 0.25; done
}
limpiar() { kill "$SRV" "$FAL" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT
SUJ='x-ore-sujeto: persona:ana'
pide() { # metodo ruta [cuerpo]
  if [ -n "${3:-}" ]; then
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H "$SUJ" -H 'Content-Type: application/json' -d "$3" "$BASE$2"
  else
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" -H "$SUJ" "$BASE$2"
  fi
}
ultima() { tail -1 "$TMP/pedidas.jsonl" 2>/dev/null; }

arrancar "127.0.0.1:$FALSO"
echo "la media por su puerta"

c=$(pide GET "/media/legal/archivo/contratos/items?prefix=Nueva%20carpeta%2F&limit=50")
u=$(ultima)
if [ "$c" = 200 ] && echo "$u" | grep -q '"ruta": "/indice/items"' && echo "$u" | grep -q '"prefix": "Nueva carpeta/"' \
   && echo "$u" | grep -q '"virtual": "true"' && echo "$u" | grep -q '00003-x.metadata.json' && echo "$u" | grep -q '"transaccion": "3"' \
   && echo "$u" | grep -q '"coleccion": "legal.archivo.contratos"'; then
  dice "1 · list: reenvía la colección, su clase, el metadata_location y la consulta decodificada"
else falla "1 · list ($c): $(cat "$TMP/r.json") · $u"; fi

c=$(pide GET "/media/legal/archivo/contratos/item?path=docs%2Fa.pdf&version=v1")
u=$(ultima)
if [ "$c" = 200 ] && echo "$u" | grep -q '"ruta": "/indice/item"' && echo "$u" | grep -q '"path": "docs/a.pdf"' && echo "$u" | grep -q '"version": "v1"'; then
  dice "2 · stat: reenvía path y version"
else falla "2 · stat ($c): $(cat "$TMP/r.json") · $u"; fi

c=$(pide POST "/media/legal/archivo/contratos/urls" '{"items":[{"path":"a.pdf"}],"ttl_s":120}')
u=$(ultima)
if [ "$c" = 200 ] && echo "$u" | grep -q '"ruta": "/indice/urls"' && echo "$u" | grep -q '"ttl_s": 120' && grep -q 'https://firmada' "$TMP/r.json" && grep -Eq '"digest": ?null' "$TMP/r.json"; then
  dice "3 · url: reenvía items y ttl_s, y devuelve las URLs tal cual (null sigue siendo null)"
else falla "3 · url ($c): $(cat "$TMP/r.json") · $u"; fi

antes=$(wc -l < "$TMP/pedidas.jsonl")
c=$(pide GET "/media/legal/archivo/nada/items")
despues=$(wc -l < "$TMP/pedidas.jsonl")
if [ "$c" = 404 ] && grep -q 'media/no-existe' "$TMP/r.json" && [ "$antes" = "$despues" ]; then
  dice "4 · lo que no existe: 404 media/no-existe, sin preguntar a ore-medios"
else falla "4 · no existe ($c): $(cat "$TMP/r.json")"; fi

c=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$SUJ" -H 'x-ore-puesto: p1' "$BASE/media/legal/archivo/contratos/items")
if ! grep -q 'desde un puesto sólo entran' "$TMP/r.json"; then
  dice "6 · un puesto entra por la media ($c)"
else falla "6 · al puesto se le cierra la media: $(cat "$TMP/r.json")"; fi

antes=$(wc -l < "$TMP/pedidas.jsonl")
c=$(pide GET "/media/legal/archivo/contratos/content?path=docs%2Fa.pdf")
despues=$(wc -l < "$TMP/pedidas.jsonl")
if [ "$c" = 503 ] && grep -q 'credencial de una fuente' "$TMP/r.json" && [ "$antes" = "$despues" ]; then
  dice "7 · content de una virtual: su fuente sale del árbol y, sin custodio, 503 sin llegar a ore-medios"
else falla "7 · content virtual ($c): $(cat "$TMP/r.json")"; fi

c=$(curl -s -D "$TMP/h.txt" -o "$TMP/r.json" -w '%{http_code}' -H "$SUJ" "$BASE/media/legal/archivo/fotos/content?path=a.jpg")
u=$(ultima)
if [ "$c" = 307 ] && grep -qi '^location: https://lago/firmada' "$TMP/h.txt" && echo "$u" | grep -q '"ruta": "/indice/abrir"' \
   && echo "$u" | grep -q '"path": "a.jpg"' && echo "$u" | grep -q '"ttl_s": 300' && grep -Eq '"digest": ?null' "$TMP/r.json"; then
  dice "8 · content de una mantenida: 307 a donde ore-medios dijo, con el cuerpo tal cual"
else falla "8 · content mantenida ($c): $(cat "$TMP/h.txt" "$TMP/r.json") · $u"; fi

c=$(pide GET "/media/legal/archivo/contratos/items?as_of=2&limit=5")
u=$(ultima)
if [ "$c" = 200 ] && echo "$u" | grep -q '"ruta": "/indice/items"' && echo "$u" | grep -q '00002-w.metadata.json' \
   && echo "$u" | grep -q '"transaccion": "2"' && echo "$u" | grep -q '"actual": "gs://lago/.*00003-x.metadata.json"'; then
  dice "9 · list con as_of=2: el metadata_location de la 2, de la historia del puntero, y el de hoy en actual"
else falla "9 · as_of ($c): $(cat "$TMP/r.json") · $u"; fi

antes=$(wc -l < "$TMP/pedidas.jsonl")
c=$(pide GET "/media/legal/archivo/contratos/items?as_of=7")
despues=$(wc -l < "$TMP/pedidas.jsonl")
c2=$(pide GET "/media/legal/archivo/contratos/items?as_of=3")
u=$(ultima)
c3=$(pide GET "/media/legal/archivo/contratos/items?as_of=a%2Fb")
if [ "$c" = 404 ] && [ "$antes" = "$despues" ] && [ "$c2" = 200 ] && echo "$u" | grep -q '00003-x' \
   && ! echo "$u" | grep -q '"actual"' && [ "$c3" = 422 ]; then
  dice "10 · as_of que no tuvo: 404 sin preguntar; el de hoy, sin actual; uno que no es transacción, 422"
else falla "10 · as_of ($c, $c2, $c3): $(cat "$TMP/r.json") · $u"; fi

c=$(pide GET "/media/legal/archivo/contratos/items?limit=1")
cur=$("$PY" -c 'import json,sys; print(json.load(open(sys.argv[1]))["cursor"])' "$TMP/r.json")
cat > "$TMP/arbol/datasets/legal/archivo/contratos.json" <<'EOF'
{"dataset":"colecciones/legal/archivo/contratos","metadata_location":"gs://lago/ore/v2/colecciones/legal/archivo/contratos/metadata/00004-z.metadata.json","transaccion":"4"}
EOF
G add -A; G commit --quiet -m "tx 4"
c2=$(pide GET "/media/legal/archivo/contratos/items?limit=1&cursor=$cur")
u=$(ultima)
c3=$(pide GET "/media/legal/archivo/contratos/items?as_of=4&cursor=$cur")
if [ "$c" = 200 ] && [ "$cur" = "3.c0ffee" ] && [ "$c2" = 200 ] && echo "$u" | grep -q '00003-x.metadata.json' \
   && echo "$u" | grep -q '"transaccion": "3"' && echo "$u" | grep -q '"cursor": "c0ffee"' \
   && echo "$u" | grep -q '"actual": "gs://lago/.*00004-z' && [ "$c3" = 422 ]; then
  dice "11 · el cursor lleva la transacción: la página 2, tras la 4, sigue en la 3; cursor y as_of que no casan, 422"
else falla "11 · cursor ($c $cur, $c2, $c3): $(cat "$TMP/r.json") · $u"; fi

kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
arrancar ""
c=$(pide GET "/media/legal/archivo/contratos/items")
if [ "$c" = 503 ] && grep -q 'media/no-desplegado' "$TMP/r.json"; then
  dice "5 · sin ore-medios: 503 media/no-desplegado"
else falla "5 · sin ore-medios ($c): $(cat "$TMP/r.json")"; fi

[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
