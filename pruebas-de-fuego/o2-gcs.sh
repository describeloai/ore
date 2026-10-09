#!/usr/bin/env bash
# UN BUCKET DE GCS, DE PUNTA A PUNTA (ADR 0061 O2·4), en Docker y sin cuentas,
# contra `fake-gcs-server` (en memoria: sin `-backend memory` contesta 500):
#
#   cubo   con Object Versioning: las generaciones de antes se siguen leyendo.
#   llano  sin él: reescribir un objeto borra la generación de antes.
#
#   1  check: lo que el emulador no sabe (`testIamPermissions`) se prueba
#      listando y leyendo (`como: probando`); un bucket que no existe se dice así
#   2  el token es al portador: un `endpoint` que no es de Google, rechazado
#      fuera del laboratorio (`ORE_GCS_LABORATORIO`)
#   3  explorar y catálogo: las carpetas; dos tablas (CSV, JSONL) y tres
#      conjuntos de objetos
#   4  versiones: cada ítem fijado por su generación, con su `crc32c`
#   5  bajar: el ítem de la colección mantenida, cotejado; con la huella
#      cambiada, no se copia
#   6  reescrito entre `versiones` y `bajar`: en `cubo` se copia la generación
#      que se listó, la de antes; en `llano` ya no está y NO se copia (nunca
#      otros bytes)
#   7  el Origen de `ore-gcs` (`tests/laboratorio.rs`): dos generaciones, la de
#      antes con su huella, rango, generación que no está → cambiado
#   8  la virtual por `ore-medios` (`tests/laboratorio_gcs.rs`)
#
# Lo que el emulador no sabe —el token, `testIamPermissions`, la firma por
# `signBlob`, la paginación— queda para GCS de verdad (deuda temporal en el
# ADR); la firma V4 se coteja con los vectores de Google en `ore-gcs`.
#
#   bash pruebas-de-fuego/o2-gcs.sh
#
# Necesita Docker.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
GCS_IMG=fsouza/fake-gcs-server@sha256:d47b4cf8b87006cab8fbbecfa5f06a2a3c5722e464abddc0d107729663d40ec4
RUST_IMG=rust:1-bookworm
RED=ore-o2
EP=http://o2-gcs:4443

# ── dentro: lo que se comprueba, en el contenedor de Rust ─────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd /w
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  command -v python3 >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq python3 >/dev/null; }
  cargo build --quiet -p ore-read-gcs 2>&1 | tail -3
  B="${CARGO_TARGET_DIR:-/w/target}/debug/ore-read-gcs"
  W=/tmp/o2; rm -rf $W; mkdir -p $W
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  q() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1"; }
  subir() { # bucket clave fichero
    curl -sf -o /dev/null -H 'authorization: Bearer x' -H 'content-type: application/octet-stream' \
      --data-binary "@$3" "$EP/upload/storage/v1/b/$1/o?uploadType=media&name=$(q "$2")"
  }

  # la muestra, en los dos buckets
  M=$W/m; mkdir -p "$M/datos/ventas" "$M/datos/eventos" "$M/docs/Nueva carpeta" "$M/img"
  printf 'id,total,fecha\n1,10.5,2026-01-01\n2,7,2026-01-02\n' > "$M/datos/ventas/ventas.csv"
  printf '{"id":1,"tipo":"a"}\n{"id":2,"tipo":"b"}\n' > "$M/datos/eventos/e.jsonl"
  printf '%%PDF-1.4\n%% uno\n%%%%EOF\n' > "$M/docs/a.pdf"
  printf '%%PDF-1.4\n%% dos, con espacio\n%%%%EOF\n' > "$M/docs/Nueva carpeta/b.pdf"
  printf '\x89PNG\r\n\x1a\n-una-imagen-' > "$M/img/c.png"
  for b in cubo llano; do
    v=false; [ $b = cubo ] && v=true
    curl -sf -o /dev/null -H 'authorization: Bearer x' -H 'content-type: application/json' \
      -d "{\"name\":\"$b\",\"versioning\":{\"enabled\":$v}}" "$EP/storage/v1/b?project=p" || { echo "el bucket $b no se crea"; exit 2; }
    (cd "$M" && find . -type f | sed 's|^\./||') | while read -r k; do subir $b "$k" "$M/$k" || echo "  ✗ $b/$k no se sube"; done
  done
  U="gs://cubo/?endpoint=$EP"; L="gs://llano/?endpoint=$EP"

  # 1 check
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | $B check > $W/check.json 2>/dev/null
  r=$(campo $W/check.json 'd["ok"], d["como"], d["fija"], d["permisos"]["listar"]["rol"], len(d["prefijos"])')
  [ "$r" = "(True, 'probando', 'version', 'roles/storage.objectViewer', 3)" ] \
    && dice "1 check: probando (el emulador no tiene testIamPermissions), fija por generación, 3 carpetas" || falla "1 check: $r $(cat $W/check.json)"
  printf '%s' "{\"url\":\"gs://otro/?endpoint=$EP\",\"objeto\":\"\"}" | $B check > $W/c2.json 2>/dev/null
  grep -q '"porque":"el bucket no existe' $W/c2.json && dice "1 check: un bucket que no existe se dice así, no como un rol que falta" || falla "1 check otro: $(cat $W/c2.json)"

  # 2 el token es al portador
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | env -u ORE_GCS_LABORATORIO $B check > /dev/null 2> $W/e
  grep -q 'no es de Google' $W/e && dice "2 fuera del laboratorio, un endpoint que no es de Google no recibe el token" || falla "2 la guarda: $(cat $W/e)"

  # 3 explorar y catálogo
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | $B explorar > $W/x.json 2>/dev/null
  n=$(campo $W/x.json 'len(d["contiene"])'); [ "$n" = 3 ] && dice "3 explorar: 3 carpetas, cada una con su URL" || falla "3 explorar: $(cat $W/x.json)"
  printf '%s' "$U" | $B catalogo f > $W/cat.json 2>/dev/null
  n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
  [ "$n" = "2 3" ] && dice "3 catálogo: 2 tablas y 3 conjuntos de objetos" || falla "3 catálogo: $n"

  # 4 versiones, y el pedido de bajar de cada bucket
  for b in cubo llano; do
    url=$U; [ $b = llano ] && url=$L
    printf '%s' "{\"url\":\"$url\",\"objeto\":\"docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $B versiones > $W/v-$b.json 2>$W/e
    r=$(campo $W/v-$b.json 'd["items"][0]["version"].isdigit(), d["items"][0]["huella"].startswith("crc32c:")' 2>/dev/null)
    [ "$r" = "(True, True)" ] && dice "4 versiones ($b): generación $(campo $W/v-$b.json 'd["items"][0]["version"]'), $(campo $W/v-$b.json 'd["items"][0]["huella"]')" || falla "4 versiones $b: $(cat $W/v-$b.json $W/e)"
    python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v-$b.json "$url" > $W/pedido-$b.json
  done

  # 5 bajar, cotejado; con la huella cambiada, no
  $B bajar < $W/pedido-cubo.json > $W/b.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b.bin && grep -aq '% uno' $W/b.bin && dice "5 bajar: el ítem, cotejado con su crc32c" || falla "5 bajar: $(head -c 300 $W/b.bin)"
  python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["items"][0]["huella"]="crc32c:AAAAAA=="; print(json.dumps(p))' $W/pedido-cubo.json | $B bajar > $W/b2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b2.bin && grep -aq 'y el manifiesto crc32c:AAAAAA==' $W/b2.bin && dice "5 con la huella cambiada, no se copia (la que GCS da al abrirla no casa)" || falla "5 huella: $(head -c 300 $W/b2.bin)"

  # 6 reescrito por fuera entre `versiones` y `bajar`, con el MISMO tamaño
  printf '%%PDF-1.4\n%% UNO\n%%%%EOF\n' > "$M/docs/a.pdf"
  for b in cubo llano; do subir $b docs/a.pdf "$M/docs/a.pdf" || falla "6 no se reescribe en $b"; done
  $B bajar < $W/pedido-cubo.json > $W/b3.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b3.bin && grep -aq '% uno' $W/b3.bin && ! grep -aq '% UNO' $W/b3.bin \
    && dice "6 cubo (con versiones): copia la generación que se listó, la de antes" || falla "6 cubo: $(head -c 300 $W/b3.bin)"
  $B bajar < $W/pedido-llano.json > $W/b4.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b4.bin && ! grep -aq '% UNO' $W/b4.bin && grep -aq 'ya no está' $W/b4.bin \
    && dice "6 llano (sin versiones): la generación ya no está y NO se copia, nunca otros bytes" || falla "6 llano: $(head -c 300 $W/b4.bin)"

  # 7 y 8: los tests de laboratorio, sobre `cubo` ya con sus dos generaciones
  r=$(ORE_LAB_GCS="$U" cargo test --quiet -p ore-gcs --test laboratorio -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "7 el Origen de ore-gcs: $(echo "$r" | grep '^gcs:' | cut -c6-)" \
    || falla "7 ore-gcs: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  r=$(ORE_LAB_GCS="$U" cargo test --quiet -p ore-medios --test laboratorio_gcs -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "8 la virtual por ore-medios: rango, entero con crc32c, cambiado, la firma dicha como fallo" \
    || falla "8 ore-medios: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f o2-gcs >/dev/null 2>&1; }
trap limpiar EXIT
limpiar
docker network inspect $RED >/dev/null 2>&1 || docker network create $RED >/dev/null
docker run -d --name o2-gcs --network $RED "$GCS_IMG" \
  -scheme http -port 4443 -public-host o2-gcs:4443 -backend memory >/dev/null
sleep 3
echo "un bucket de GCS (laboratorio: fake-gcs-server, cubo con versiones y llano sin ellas)"
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -e ORE_GCP_TOKEN=de-laboratorio -e ORE_GCS_LABORATORIO=1 -e EP=$EP \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o2-gcs.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
