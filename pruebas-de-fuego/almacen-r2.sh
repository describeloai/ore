#!/usr/bin/env bash
# `ore-store-r2` contra un R2 de verdad.
#
# Lo que aquí se afirma no se puede afirmar con `cargo test`: que el artefacto
# llega, que **su nombre es su contenido**, y que un segundo intento con el mismo
# testigo **no sube ni un byte**. Las tres son del almacén, no del código.
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

CAB='{"plan":"sha256:aaaa","esquema":{"id":"Integer","pais":"String","total":"Decimal"},"conducto":"materialization.payload","bundle":"sha256:bbbb","testigo":{"modo":"log","valor":"__T__"}}'
entrada() {
  echo "${CAB/__T__/$1}"
  echo '{"id":"1","pais":"ES","total":"10.50"}'
  echo '{"id":"2","pais":"PT","total":"7.25"}'
}

campo() { echo "$1" | sed -n "s/.*\"$2\":\([^,}]*\).*/\1/p" | tr -d '"'; }

echo "── 1 · la copia existe, y se llama por su digest ───────────────"
a=$(entrada 4210 | "$BIN") || { echo "$a"; exit 1; }
clave=$(campo "$a" clave)
digest=$(campo "$a" digest)
[ "$(campo "$a" subido)" = "true" ] && ok "sube la primera vez" || mal "no subió: $a"
[ "ore/v1/${digest#sha256:}" = "$clave" ] && ok "el nombre ES el digest" || mal "$clave ≠ $digest"

echo "── 2 · el mismo testigo no sube ni un byte ─────────────────────"
b=$(entrada 4210 | "$BIN") || { echo "$b"; exit 1; }
[ "$(campo "$b" subido)" = "false" ] && ok "segunda vez: subido=false" || mal "volvió a subir: $b"
[ "$(campo "$b" clave)" = "$clave" ] && ok "y al mismo nombre" || mal "otro nombre: $b"

echo "── 3 · otro testigo es otra copia ──────────────────────────────"
c=$(entrada 4211 | "$BIN") || { echo "$c"; exit 1; }
otra=$(campo "$c" clave)
[ "$otra" != "$clave" ] && ok "otro nombre" || mal "el testigo no entra en el digest"
[ "$(campo "$c" subido)" = "true" ] && ok "y sube" || mal "no subió: $c"

echo "── 4 · lo que no es del tipo declarado se niega ─────────────────"
d=$({ echo "${CAB/__T__/4210}"; echo '{"id":"uno","pais":"ES","total":"1"}'; } | "$BIN" 2>&1)
case "$d" in
  *"no inventa una conversión"*) ok "se niega, y dice cuál columna" ;;
  *) mal "aceptó un Integer que no lo es: $d" ;;
esac

echo "── limpieza ────────────────────────────────────────────────────"
for k in "$clave" "$otra"; do
  python - "$k" <<'PY' 2>/dev/null || echo "  (borra a mano: $k)"
import os, sys, boto3
boto3.client("s3", endpoint_url=os.environ["ORE_R2_S3_ENDPOINT"],
             aws_access_key_id=os.environ["ORE_R2_ACCESS_KEY_ID"],
             aws_secret_access_key=os.environ["ORE_R2_SECRET_ACCESS_KEY"],
             region_name=os.environ.get("ORE_R2_REGION", "auto")
).delete_object(Bucket=os.environ["ORE_R2_BUCKET"], Key=sys.argv[1])
PY
done
echo "  borrados 2"

echo
[ "$fallos" -eq 0 ] && echo "todo verde" || echo "$fallos fallo(s)"
exit "$fallos"
