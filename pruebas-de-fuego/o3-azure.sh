#!/usr/bin/env bash
# UN CONTENEDOR DE AZURE BLOB, DE PUNTA A PUNTA (ADR 0061 O3·4), en Docker y sin
# cuentas, contra Azurite por HTTPS con `--oauth basic` (un Bearer: Azurite mira
# emisor, audiencia y fechas, no la firma), con un certificado de un día.
#
# Azurite no versiona (O3·0): todo se fija por ETag, como una cuenta sin
# versionado o ADLS Gen2 (D-O1).
#
#   1  check: identidad, listar, leer, versiones (fija por `etag`) y la clave
#      de delegación; un contenedor que no existe se dice así
#   2  el token es al portador: un `endpoint` que no es de Azure, rechazado
#      fuera del laboratorio (`ORE_AZURE_LABORATORIO`)
#   3  explorar y catálogo: las carpetas; dos tablas (CSV, JSONL) y los
#      conjuntos de objetos
#   4  versiones: cada ítem fijado por su ETag, con su `md5`
#   5  bajar: el ítem de la colección mantenida, cotejado; con la huella
#      cambiada, no se copia
#   6  reescrito entre `versiones` y `bajar`, con el MISMO tamaño: NO se copia
#      (412), nunca otros bytes
#   7  el Origen de `ore-azure` (`tests/laboratorio.rs`): ETag, `md5` (y un blob
#      por bloques sin él), entero, rango y por el final, 412, la guarda del
#      `versionid` ignorado, la SAS de delegación bajada y manipulada
#   8  la virtual por `ore-medios` (`tests/laboratorio_azure.rs`)
#
# Lo que Azurite no sabe —el versionado, `sr=bv`, ADLS Gen2, el canje con
# Entra— queda para Azure de verdad (deuda temporal en el ADR).
#
#   bash pruebas-de-fuego/o3-azure.sh
#
# Necesita Docker.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
AZ_IMG=mcr.microsoft.com/azure-storage/azurite@sha256:830430c1da1a2d537e08f3e6764dd1f5ae00cf0346bcaf625b968ec3f0971fd5
PY_IMG=python@sha256:05cda9777409a9c3ffddd94a4c476b79f0769a0b4857f0c7ed9226b6800b0d6f
RUST_IMG=rust:1-bookworm
RED=ore-o3
EP=https://o3-az:10000
CUENTA=devstoreaccount1

# ── dentro: lo que se comprueba, en el contenedor de Rust ─────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd /w
  cp /c/c.pem /usr/local/share/ca-certificates/o3.crt && update-ca-certificates >/dev/null 2>&1
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  command -v python3 >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq python3 >/dev/null; }
  cargo build --quiet -p ore-read-azure 2>&1 | tail -3
  B="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-azure"
  W=/tmp/o3; rm -rf $W; mkdir -p $W
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  q() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe="/"))' "$1"; }
  az_() { # método ruta [curl…]: con el Bearer, sobre la cuenta
    local m=$1 r=$2; shift 2
    curl -sf -o /dev/null -X "$m" -H "authorization: Bearer $ORE_AZURE_TOKEN" -H 'x-ms-version: 2023-11-03' "$@" "$EP/$CUENTA/$r"
  }
  subir() { az_ PUT "cubo/$(q "$1")" -H 'x-ms-blob-type: BlockBlob' --data-binary "@$2"; }

  # la muestra: Put Blob (con MD5), y un blob por bloques (sin él)
  M=$W/m; mkdir -p "$M/datos/ventas" "$M/datos/eventos" "$M/docs/Nueva carpeta" "$M/img"
  printf 'id,total,fecha\n1,10.5,2026-01-01\n2,7,2026-01-02\n' > "$M/datos/ventas/ventas.csv"
  printf '{"id":1,"tipo":"a"}\n{"id":2,"tipo":"b"}\n' > "$M/datos/eventos/e.jsonl"
  printf '%%PDF-1.4\n%% uno\n%%%%EOF\n' > "$M/docs/a.pdf"
  printf '%%PDF-1.4\n%% dos, con espacio\n%%%%EOF\n' > "$M/docs/Nueva carpeta/b.pdf"
  printf '\x89PNG\r\n\x1a\n-una-imagen-' > "$M/img/c.png"
  az_ PUT 'cubo?restype=container' || { echo "el contenedor no se crea"; exit 2; }
  (cd "$M" && find . -type f | sed 's|^\./||') | while read -r k; do subir "$k" "$M/$k" || echo "  ✗ $k no se sube"; done
  BLK=$(printf 'bloque-0' | base64)
  az_ PUT "cubo/grande.bin?comp=block&blockid=$(q "$BLK" | sed 's/=/%3D/g')" --data-binary 'yyyyyyyyyy' \
    && az_ PUT 'cubo/grande.bin?comp=blocklist' -H 'content-type: application/xml' \
         --data-binary "<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList><Latest>$BLK</Latest></BlockList>" \
    || echo "  ✗ grande.bin no se sube por bloques"
  U="az://$CUENTA/cubo/?tenant=t&cliente=c&endpoint=$EP"

  # 1 check
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | $B check > $W/check.json 2>/dev/null
  r=$(campo $W/check.json 'd["ok"], d["fija"], sorted(k for k, p in d["permisos"].items() if p.get("ok")), len(d["prefijos"])')
  [ "$r" = "(True, 'etag', ['firmar', 'identidad', 'leer', 'listar', 'versiones'], 3)" ] \
    && dice "1 check: los cinco pasos, fija por ETag (sin versionado), 3 carpetas" || falla "1 check: $r $(cat $W/check.json)"
  printf '%s' "{\"url\":\"${U/cubo/otro}\",\"objeto\":\"\"}" | $B check > $W/c2.json 2>/dev/null
  grep -q '"porque":"el contenedor `otro` no existe' $W/c2.json && dice "1 check: un contenedor que no existe se dice así, no como un rol que falta" || falla "1 check otro: $(cat $W/c2.json)"

  # 2 el token es al portador
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | env -u ORE_AZURE_LABORATORIO $B check > /dev/null 2> $W/e
  grep -q 'no se le da a otro servidor' $W/e && dice "2 fuera del laboratorio, un endpoint que no es de Azure no recibe el token" || falla "2 la guarda: $(cat $W/e)"

  # 3 explorar y catálogo
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | $B explorar > $W/x.json 2>/dev/null
  n=$(campo $W/x.json 'len([c for c in d["contiene"] if c["nombre"]])'); [ "$n" = 3 ] && dice "3 explorar: 3 carpetas, cada una con su URL" || falla "3 explorar: $(cat $W/x.json)"
  printf '%s' "$U" | $B catalogo f > $W/cat.json 2>/dev/null
  n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
  [ "$n" = "2 4" ] && dice "3 catálogo: 2 tablas y 4 conjuntos de objetos (la raíz con grande.bin)" || falla "3 catálogo: $n"

  # 4 versiones, y el pedido de bajar
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $B versiones > $W/v.json 2>$W/e
  r=$(campo $W/v.json 'd["items"][0]["version"].startswith("etag:"), d["items"][0]["huella"].startswith("md5:")' 2>/dev/null)
  [ "$r" = "(True, True)" ] && dice "4 versiones: $(campo $W/v.json 'd["items"][0]["version"]'), $(campo $W/v.json 'd["items"][0]["huella"]')" || falla "4 versiones: $(cat $W/v.json $W/e)"
  python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v.json "$U" > $W/pedido.json

  # 5 bajar, cotejado; con la huella cambiada, no
  $B bajar < $W/pedido.json > $W/b.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b.bin && grep -aq '% uno' $W/b.bin && dice "5 bajar: el ítem, cotejado con su md5" || falla "5 bajar: $(head -c 300 $W/b.bin)"
  python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["items"][0]["huella"]="md5:AAAAAAAAAAAAAAAAAAAAAA=="; print(json.dumps(p))' $W/pedido.json | $B bajar > $W/b2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b2.bin && grep -aq 'y el manifiesto md5:AAAAAAAAAAAAAAAAAAAAAA==' $W/b2.bin && dice "5 con la huella cambiada, no se copia" || falla "5 huella: $(head -c 300 $W/b2.bin)"

  # 6 reescrito por fuera entre `versiones` y `bajar`, con el MISMO tamaño
  printf '%%PDF-1.4\n%% UNO\n%%%%EOF\n' > "$M/docs/a.pdf"
  subir docs/a.pdf "$M/docs/a.pdf" || falla "6 no se reescribe"
  $B bajar < $W/pedido.json > $W/b3.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b3.bin && ! grep -aq '% UNO' $W/b3.bin && grep -aq '412' $W/b3.bin \
    && dice "6 reescrito: NO se copia (412), nunca otros bytes" || falla "6: $(head -c 300 $W/b3.bin)"

  # 7 y 8: los tests de laboratorio, con docs/a.pdf ya reescrito
  r=$(ORE_LAB_AZURE="$U" cargo test --quiet -p ore-azure --test laboratorio -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "7 el Origen de ore-azure: $(echo "$r" | grep '^azure:' | cut -c8-)" \
    || falla "7 ore-azure: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  r=$(ORE_LAB_AZURE="$U" cargo test --quiet -p ore-medios --test laboratorio_azure -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "8 la virtual por ore-medios: rango, por el final, entero con md5, cambiado, sin URL (D-O1)" \
    || falla "8 ore-medios: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f o3-az >/dev/null 2>&1; docker volume rm -f o3-cert >/dev/null 2>&1; }
trap limpiar EXIT
limpiar
docker network inspect $RED >/dev/null 2>&1 || docker network create $RED >/dev/null
docker run --rm -v o3-cert:/c "$PY_IMG" python -c "
import subprocess; subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout','/c/k.pem','-out','/c/c.pem','-days','1','-subj','/CN=o3-az','-addext','subjectAltName=DNS:o3-az'],check=True,capture_output=True)" \
  || { echo "el certificado no se crea"; exit 2; }
docker run -d --name o3-az --network $RED -v o3-cert:/c "$AZ_IMG" \
  azurite-blob --blobHost 0.0.0.0 --loose --oauth basic --cert /c/c.pem --key /c/k.pem >/dev/null
sleep 3
# Un Bearer de laboratorio: Azurite no mira la firma.
b64() { base64 -w0 | tr '+/' '-_' | tr -d '='; }
N=$(date +%s)
TOKEN="$(printf '{"alg":"RS256","typ":"JWT"}' | b64).$(printf '{"aud":"https://storage.azure.com","iss":"https://sts.windows.net/ab1f708d-50f6-404c-a006-d71b2ac7a606/","iat":%d,"nbf":%d,"exp":%d,"oid":"00000000-0000-0000-0000-000000000001","tid":"ab1f708d-50f6-404c-a006-d71b2ac7a606"}' $((N-60)) $((N-60)) $((N+3600)) | b64).laboratorio"
echo "un contenedor de Azure Blob (laboratorio: Azurite por HTTPS, sin versionado)"
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v o3-cert:/c \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -e ORE_AZURE_TOKEN="$TOKEN" -e ORE_AZURE_LABORATORIO=1 -e EP=$EP -e CUENTA=$CUENTA \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o3-azure.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
