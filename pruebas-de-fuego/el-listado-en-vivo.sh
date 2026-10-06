#!/usr/bin/env bash
# 0053 F9·1 · EL LISTADO DE UN `ObjectTable`, EN VIVO, POR LA PASARELA.
#
# `ore-federation` con `ore-read-s3` delante del S3 de mentira: `POST /v1/read`
# con `listado` devuelve una fila por objeto (sus metadatos, nunca sus bytes),
# con su `match`, sus particiones y el `limit`. Y `ore explain` reparte un
# `SELECT … FROM <ObjectTable>` como una lectura de su listado.
#
#   FED=target/debug/ore-federation ORE=target/debug/ore bash pruebas-de-fuego/el-listado-en-vivo.sh
#
# Python con pyarrow. Los conectores se buscan junto a `ore-federation`.
set -u
abs() { case "$1" in /*) echo "$1" ;; *) echo "$PWD/$1" ;; esac; }
FED=$(abs "${FED:-target/debug/ore-federation}"); ORE=$(abs "${ORE:-target/debug/ore}")
PY=$(command -v python3 || command -v python)
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"; PIDS=""
trap 'kill $PIDS 2>/dev/null; rm -rf "$TMP"' EXIT
MAL=0
falla() { echo "  ✗ $*"; MAL=1; }
dice()  { echo "  ✓ $*"; }
libre() { "$PY" -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }

# ── el S3 de mentira, con un bucket y unos objetos ───────────────────────────
"$PY" "$RAIZ/pruebas-de-fuego/de-mentira.py" s3 0 > "$TMP/s3.log" 2>&1 & PIDS="$PIDS $!"
for _ in $(seq 1 40); do [ -s "$TMP/s3.log" ] && break; sleep 0.25; done
S3="http://127.0.0.1:$(awk '{print $2}' "$TMP/s3.log")"
curl -s -X PUT "$S3/docs" >/dev/null
for k in "contratos/anio=2026/a.pdf:aaaa" "contratos/anio=2025/b.pdf:bb" "contratos/anio=2026/c.txt:c" "otros/d.pdf:d"; do
  curl -s -X PUT --data-binary "${k#*:}" "$S3/docs/${k%%:*}" >/dev/null
done
URL="s3://docs?region=us-east-1&endpoint=$S3&access_key_id=de&secret_access_key=mentira"

# ── la pasarela ──────────────────────────────────────────────────────────────
PF=$(libre)
"$FED" --escucha "127.0.0.1:$PF" --conectores "$(dirname "$FED")" --tipos s3 >"$TMP/fed.log" 2>&1 & PIDS="$PIDS $!"
for _ in $(seq 1 40); do curl -s -o /dev/null "http://127.0.0.1:$PF/v1/health" && break; sleep 0.25; done

leer() { # id cuerpo-de-peticion → filas en $TMP/<id>.arrow
  printf '{"id":"%s","origen":"docs","tipo":"s3","url":"%s","peticion":%s}' "$1" "$URL" "$2" > "$TMP/c.json"
  curl -s -o "$TMP/$1.arrow" -w '%{http_code}' -X POST -H 'content-type: application/json' \
    --data-binary @"$TMP/c.json" "http://127.0.0.1:$PF/v1/read"
}
filas() { "$PY" - "$TMP/$1.arrow" <<'PYX'
import pyarrow.ipc as i, sys
t = i.open_stream(open(sys.argv[1], "rb")).read_all()
print(t.num_rows, "|", ",".join(t.column_names), "|", ";".join(str(r) for r in t.to_pylist()))
PYX
}

C=$(leer l1 '{"objeto":"contratos/","proyeccion":{"key":"key","size":"size","anio":"anio","contentType":"contentType"},"listado":{"match":"**/*.pdf"}}')
R=$(filas l1 2>&1)
case "$C|$R" in
  "200|2 |"*"'anio': '2025', 'contentType': None, 'key': 'contratos/anio=2025/b.pdf', 'size': 2"*"contratos/anio=2026/a.pdf"*)
    dice "el listado: 2 PDF bajo el prefijo, con tamaño y partición, sin abrir ninguno";;
  *) falla "listado: $C · $R · $(tail -3 "$TMP/fed.log")";;
esac
C=$(leer l2 '{"objeto":"contratos/","proyeccion":{"key":"key"},"listado":{"match":""},"limit":1}')
R=$(filas l2 2>&1)
case "$C|$R" in "200|1 |"*) dice "con limit 1, una fila";; *) falla "limit: $C · $R";; esac

# ── y el reparto lo da por lectura del listado (`ore explain`) ──────────────
A="$TMP/arbol"; mkdir -p "$A/packages/docs/archivo/objects"
cat > "$A/ontology.config.yaml" <<YAML
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: listado, version: 0.1.0 }
datasources:
  - { name: docs, type: s3, connectionEnv: DOCS_URL, federation: true }
YAML
cat > "$A/conduits.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: listado }
spec:
  owner: team:listado
  conduits:
    federation.read: { oos.maturity: DRAFT }
YAML
cat > "$A/packages/docs/package.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: docs, version: 0.1.0, status: draft, domain: docs }
spec: { owner: "team:listado" }
YAML
cat > "$A/packages/docs/archivo/schema.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: archivo, namespace: docs }
spec: { owner: team:listado }
YAML
cat > "$A/packages/docs/archivo/objects/contratos.yaml" <<'YAML'
apiVersion: oos.dev/v1alpha16
kind: ObjectTable
metadata: { name: contratos, namespace: docs, schema: archivo }
spec:
  datasource: docs
  prefix: "contratos/"
  match: "**/*.pdf"
  partitions: [anio]
  media: document
  reads: { fullScan: cheap }
  changes: { mode: retract, witness: listing }
YAML
"$ORE" explain "SELECT key, size FROM docs.archivo.contratos WHERE anio = '2026' LIMIT 10" --json --path "$A" > "$TMP/plan.json" 2>&1
"$PY" -c '
import json,sys
p=json.load(open(sys.argv[1]))
l=[x for x in p["lecturas"] if x["tabla"]=="docs.archivo.contratos"][0]
assert l["objeto"]=="contratos/" and l["listado"]=={"match":"**/*.pdf"}, l
assert "anio" in l["columnas"] and "key" in l["columnas"], l
' "$TMP/plan.json" && dice "ore explain: un SELECT sobre el ObjectTable es la lectura de su listado" \
  || falla "explain: $(head -c 600 "$TMP/plan.json")"

echo
[ "$MAL" = 0 ] && echo "✓ el listado en vivo (0053 F9·1)" || { echo "✗ el listado en vivo"; exit 1; }
