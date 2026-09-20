#!/usr/bin/env bash
# `ore-store-r2` contra un R2 de verdad.
#
# Lo que aquí se afirma no se puede afirmar con `cargo test`: que el dataset
# llega al bucket como una tabla Iceberg (W3.6a, 0031 §10), que un segundo
# sellado sobre él es **un snapshot más y no una tabla más**, y que `leer`
# devuelve por el puntero lo que se selló. Las tres son del almacén, no del
# código.
#
#   set -a; . ./.env.local; set +a
#   bash pruebas-de-fuego/almacen-r2.sh
#
# Deja el bucket como lo encontró.
set -u

BIN="${BIN:-./target/debug/ore-store-r2}"
[ -x "$BIN" ] || BIN="./target/debug/ore-store-r2.exe"

for v in ORE_R2_S3_ENDPOINT ORE_R2_BUCKET ORE_R2_ACCESS_KEY_ID ORE_R2_SECRET_ACCESS_KEY; do
  if [ -z "${!v:-}" ]; then
    echo "falta \$$v — carga el entorno primero:"
    echo "  set -a; . ./.env.local; set +a"
    exit 2
  fi
done

fallos=0
ok()   { echo "  ✓ $1"; }
mal()  { echo "  ✗ $1"; fallos=$((fallos + 1)); }

DS="copias/prueba_almacen_$$"
# En su forma canónica (claves ordenadas): es la que `leer` devuelve tal cual.
CAB='{"clave":["id"],"conducto":"materialization.payload","esquema":{"id":"Integer","pais":"String","total":"Decimal"},"plan":"sha256:aaaa","testigo":{"modo":"log","valor":"__T__"}}'
entrada() { # testigo [base] [fundir]
  local extra="\"dataset\":\"$DS\",\"fundir\":${3:-false},"
  [ -n "${2:-}" ] && extra="$extra\"base\":\"$2\","
  local cab="${CAB/__T__/$1}"
  echo "{$extra${cab#\{}"
  echo '{"id":"1","pais":"ES","total":"10.50"}'
  echo '{"id":"2","pais":"PT","total":"7.25"}'
}

campo() { python -c 'import json,sys; v=json.loads(sys.argv[1]).get(sys.argv[2],""); print(v if not isinstance(v,bool) else str(v).lower())' "$1" "$2"; }

echo "── 1 · el dataset nace: UN metadata.json, y el puntero lo nombra ──"
a=$(entrada 4210 | "$BIN" sellar) || { echo "$a"; exit 1; }
ml1=$(campo "$a" metadata_location)
[ "$(campo "$a" operacion)" = "creada" ] && ok "creada" || mal "no nació: $a"
case "$ml1" in "s3://$ORE_R2_BUCKET/ore/v2/$DS/metadata/00000-"*.metadata.json) ok "el puntero: $ml1" ;; *) mal "el puntero no es el metadata.json de la tabla: $ml1" ;; esac
[ "$(campo "$a" filas)" = "2" ] && ok "2 filas" || mal "filas: $a"

echo "── 2 · buscar: el puntero sigue en el bucket ───────────────────"
b=$(printf '{"metadata_location":"%s"}\n' "$ml1" | "$BIN" buscar) || { echo "$b"; exit 1; }
[ "$(campo "$b" existe)" = "true" ] && ok "existe" || mal "buscar no lo ve: $b"
b=$(printf '{"metadata_location":"%s"}\n' "${ml1%.metadata.json}-no.metadata.json" | "$BIN" buscar) || { echo "$b"; exit 1; }
[ "$(campo "$b" existe)" = "false" ] && ok "y lo que no está, no" || mal "buscar ve lo que no está: $b"

echo "── 3 · otro testigo sobre la base: un snapshot más, la misma tabla ─"
c=$(entrada 4211 "$ml1" true | "$BIN" sellar) || { echo "$c"; exit 1; }
ml2=$(campo "$c" metadata_location)
[ "$(campo "$c" operacion)" = "refrescada" ] && ok "refrescada (fundida sobre la base)" || mal "no refrescó: $c"
case "$ml2" in "s3://$ORE_R2_BUCKET/ore/v2/$DS/metadata/00001-"*) ok "el siguiente metadata.json de la MISMA tabla" ;; *) mal "otra tabla: $ml2" ;; esac
[ "$(campo "$c" retirados)" = "1" ] && ok "retira el fichero de datos anterior" || mal "retirados: $c"

echo "── 4 · leer devuelve la cabecera sellada y las filas ───────────"
l=$(printf '{"metadata_location":"%s"}\n' "$ml2" | "$BIN" leer) || { echo "$l"; exit 1; }
[ "$(echo "$l" | head -1)" = "${CAB/__T__/4211}" ] && ok "la cabecera, tal cual (sin dataset/base/fundir)" || mal "la cabecera no es la sellada: $(echo "$l" | head -1)"
[ "$(echo "$l" | grep -c '^{"id"')" = "2" ] && ok "2 filas" || mal "filas: $l"
echo "$l" | grep -q '"total":"10.5"' && ok "el decimal vuelve canónico" || mal "el decimal: $l"
l1=$(printf '{"metadata_location":"%s"}\n' "$ml1" | "$BIN" leer) || { echo "$l1"; exit 1; }
[ "$(echo "$l1" | head -1)" = "${CAB/__T__/4210}" ] && ok "y el snapshot anterior sigue leyéndose con SU cabecera" || mal "el snapshot anterior: $(echo "$l1" | head -1)"

echo "── 5 · lo que no es del tipo declarado se queda texto, y se dice ─"
# 0032: un valor que no analiza no se inventa ni rompe la copia. La columna
# entera queda como texto y `sin_estrechar` dice cuántos valores y cuál.
cab="${CAB/__T__/4212}"
d=$({ echo "{\"dataset\":\"$DS\",\"base\":\"$ml2\",\"fundir\":false,${cab#\{}"; echo '{"id":"uno","pais":"ES","total":"1"}'; } | "$BIN" sellar 2>&1) || { echo "$d"; exit 1; }
ml3=$(campo "$d" metadata_location)
case "$d" in
  *'"sin_estrechar":{"id":"1 de 1 valores no son Integer'*'`uno`'*) ok "sella, y dice qué columna se quedó texto y por qué" ;;
  *) mal "no dijo que \`id\` se quedó texto: $d" ;;
esac
[ "$(campo "$d" esquema_cambiado)" = "true" ] && ok "y la tabla adoptó el esquema (id pasa a texto)" || mal "el esquema no cambió: $d"

echo "── 6 · recoger: expiran los snapshots superados y se van sus ficheros ─"
r=$(printf '{"dataset":"%s","metadata_location":"%s"}\n' "$DS" "$ml3" | "$BIN" recoger-seco) || { echo "$r"; exit 1; }
[ "$(campo "$r" expirados)" = "2" ] && ok "en seco: 2 por expirar" || mal "seco: $r"
r=$(printf '{"dataset":"%s","metadata_location":"%s"}\n' "$DS" "$ml3" | "$BIN" recoger) || { echo "$r"; exit 1; }
ml4=$(campo "$r" metadata_location)
[ "$(campo "$r" expirados)" = "2" ] && [ "$ml4" != "$ml3" ] && ok "2 expirados, puntero nuevo: $ml4" || mal "recoger: $r"
[ "$(campo "$r" ficheros)" -ge 5 ] && ok "$(campo "$r" ficheros) ficheros que nadie nombraba, fuera" || mal "ficheros: $r"
l=$(printf '{"metadata_location":"%s"}\n' "$ml4" | "$BIN" leer) || { echo "$l"; exit 1; }
[ "$(echo "$l" | grep -c '^{"id"')" = "1" ] && ok "la vigente sigue entera" || mal "tras recoger: $l"

echo "── limpieza ────────────────────────────────────────────────────"
# Por fuera del delegado y sólo el prefijo de ESTE dataset: `recoger-huerfanas`
# retira todo lo que la lista no reclama, y en un bucket compartido eso es más
# de lo que esta prueba dejó. Lo prueba `la-pregunta-se-contesta.sh` (caso 9)
# contra el S3 de mentira.
python - "ore/v2/$DS/" <<'PY' 2>/dev/null || echo "  (borra a mano: ore/v2/$DS/)"
import os, sys, boto3
c = boto3.client("s3", endpoint_url=os.environ["ORE_R2_S3_ENDPOINT"],
                 aws_access_key_id=os.environ["ORE_R2_ACCESS_KEY_ID"],
                 aws_secret_access_key=os.environ["ORE_R2_SECRET_ACCESS_KEY"],
                 region_name=os.environ.get("ORE_R2_REGION", "auto"))
B = os.environ["ORE_R2_BUCKET"]
ll = [{"Key": o["Key"]} for o in c.list_objects_v2(Bucket=B, Prefix=sys.argv[1]).get("Contents", [])]
if ll:
    c.delete_objects(Bucket=B, Delete={"Objects": ll})
print("  borrados %d" % len(ll))
PY
r=$(printf '{"metadata_location":"%s"}\n' "$ml4" | "$BIN" buscar)
[ "$(campo "$r" existe)" = "false" ] && ok "y buscar ya no lo ve" || mal "sigue ahí: $r"

echo
[ "$fallos" -eq 0 ] && echo "todo verde" || echo "$fallos fallo(s)"
exit "$fallos"
