#!/usr/bin/env bash
# UNA CARPETA DE GOOGLE DRIVE, DE PUNTA A PUNTA (ADR 0082 paso 5), sin cuenta de
# Google, contra el Drive de mentira (`drive-de-mentira.py`: no hay emulador de
# Drive; está escrito de lo que X0 midió contra el de verdad), en páginas de 2
# y con un `429` o un `403` de ritmo cada 7 peticiones.
#
# Drive fija un binario por su revisión (la actual, y sólo ella: un lector no ve
# las demás: X0 B2) y un nativo (Doc, Hoja) por su `version`, exportado y
# vigilado.
#
#   1  check: identidad, carpeta, listar, leer y versiones; una carpeta que la
#      cuenta no puede leer dice cómo compartirla; una que no está (o no se
#      compartió) lo dice así
#   2  el token es al portador: un `endpoint` que no es Google, rechazado fuera
#      del laboratorio (`ORE_GDRIVE_LABORATORIO`)
#   3  explorar y catálogo: las carpetas (y el acceso directo saltado); tablas
#      CSV, y los dos `a.csv` de la misma carpeta con su ID
#   4  versiones: cada fichero con su `<id>@<revisión>` y su `md5:`
#   5  bajar: el ítem, cotejado con su md5; con la huella cambiada, no se copia
#   6  el fichero CAMBIA entre `versiones` y `bajar`: la revisión fijada ya no
#      está, no se copia, y nunca los bytes nuevos
#   7  un nativo (un Doc): se exporta sin el BOM, se copia cotejado; si cambia
#      entre `versiones` y `bajar`, no se copia
#   8  el Origen de `ore-gdrive` (`tests/mentira.rs`): la lectura fijada, los
#      nativos, el feed de cambios y su respaldo con un token caducado
#   y al final: ninguna petición llegó sin su token
#
# Lo que el Drive de mentira no sabe —el `429` real, las unidades compartidas,
# la autosuplantación del metadata server, qué contesta Drive a un token de
# cambios caducado— queda para una celda (deuda temporal en el ADR).
#
#   bash pruebas-de-fuego/o6-gdrive.sh              (en Docker)
#   bash pruebas-de-fuego/o6-gdrive.sh --dentro     (aquí: hace falta cargo y python3)
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
RUST_IMG=rust:1-bookworm

# ── dentro: lo que se comprueba ───────────────────────────────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd "$RAIZ"
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  G=http://127.0.0.1:8792
  export ORE_GDRIVE_LABORATORIO=1 ORE_GCP_TOKEN=base ORE_GDRIVE_CUENTA=lab@p.iam.gserviceaccount.com ORE_GCP_IAM="$G/v1"
  PAGINA=2 CADA_429=7 python3 pruebas-de-fuego/drive-de-mentira.py 8792 > /tmp/drive.log 2>&1 &
  SERVIDOR=$!
  trap 'kill $SERVIDOR 2>/dev/null' EXIT
  cargo build --quiet -p ore-read-gdrive 2>&1 | tail -3
  for i in $(seq 1 50); do grep -q listo /tmp/drive.log && break; sleep 0.2; done
  grep -q listo /tmp/drive.log || { echo "el Drive de mentira no arranca: $(cat /tmp/drive.log)"; exit 2; }
  T="${CARGO_TARGET_DIR:-$RAIZ/target}/debug"; B=$T/ore-read-gdrive
  W=/tmp/o6; rm -rf $W; mkdir -p $W
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  sembrar() { # método qué ruta [más] [fichero]
    curl -s -o /dev/null -X "$1" ${5:+--data-binary @"$5"} "$G/_mentira$2?ruta=$3${4:-}"
  }
  subir() { sembrar PUT "" "$1" "" "$2"; }  # ruta fichero

  # la muestra
  M=$W/m; mkdir -p "$M"
  printf 'id,total\n1,10.5\n2,7\n' > "$M/ventas.csv"
  printf 'x,y\n1,2\n' > "$M/a1.csv"
  printf 'x,y\n9,9\n' > "$M/a2.csv"
  printf 'x,y\n7,7\n' > "$M/nueva.csv"
  printf 'Hola Drive\n' > "$M/doc1.txt"
  printf 'Adios Drive\n' > "$M/doc2.txt"
  subir ore/datos/ventas.csv "$M/ventas.csv"
  subir ore/docs/a.csv "$M/a1.csv"
  curl -s -o /dev/null -X PUT --data-binary @"$M/a2.csv" "$G/_mentira?ruta=ore/docs/a.csv&dup=1"
  sembrar POST /nativo "ore/docs/doc" "&clase=doc" "$M/doc1.txt"
  ID=$(curl -s "$G/_mentira/id?ruta=ore/datos/ventas.csv" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
  sembrar POST /atajo "ore/atajo" "&destino=$ID"
  U="gdrive://RAIZ/ore/?endpoint=$G"
  co() { printf '%s' "{\"url\":\"$1\",\"objeto\":\"\"}"; }

  # 1 check
  co "$U" | $B check > $W/check.json 2>/dev/null
  r=$(campo $W/check.json 'd["ok"], d["fija"], sorted(k for k, p in d["permisos"].items() if p.get("ok")), len(d["prefijos"])')
  [ "$r" = "(True, 'version', ['carpeta', 'identidad', 'leer', 'listar', 'versiones'], 2)" ] \
    && dice "1 check: los cinco pasos, fija por versión, 2 carpetas" || falla "1 check: $r $(cat $W/check.json)"
  co "${U/RAIZ/SINPERMISO}" | $B check > $W/c2.json 2>/dev/null
  grep -q '"ok":false' $W/c2.json && grep -q 'compártela como lector' $W/c2.json \
    && dice "1 check: una carpeta que la cuenta no puede leer dice cómo compartirla" || falla "1 check SINPERMISO: $(cat $W/c2.json)"
  co "${U/RAIZ/NOEXISTE}" | $B check > $W/c3.json 2>/dev/null
  grep -q 'o no existe, o no está compartida' $W/c3.json \
    && dice "1 check: una que no está dice que Drive no distingue 'no existe' de 'no compartida'" || falla "1 check NOEXISTE: $(cat $W/c3.json)"

  # 2 el token es al portador
  co "$U" | env -u ORE_GDRIVE_LABORATORIO $B check > /dev/null 2> $W/e
  grep -q 'never given to another server' $W/e && dice "2 fuera del laboratorio, un endpoint que no es Google no recibe el token" || falla "2 la guarda: $(cat $W/e)"

  # 3 explorar y catálogo
  co "$U" | $B explorar > $W/x.json 2>/dev/null
  r=$(campo $W/x.json 'sorted(c["nombre"] for c in d["contiene"]), "saltados" in d')
  [ "$r" = "(['datos/', 'docs/'], True)" ] || [ "$r" = "(['', 'datos/', 'docs/'], True)" ] || [ "$r" = "(['ore/datos/', 'ore/docs/'], True)" ] \
    && dice "3 explorar: las carpetas, y el acceso directo saltado" || falla "3 explorar: $r $(cat $W/x.json)"
  printf '%s' "$U" | $B catalogo f > $W/cat.json 2>/dev/null
  n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
  [ "$n" = "2 1" ] && dice "3 catálogo: 2 tablas (ventas, y los dos a.csv de una misma carpeta como una sola) y 1 conjunto de objetos (el Doc)" || falla "3 catálogo: $n"

  # 4 versiones, y el pedido de bajar
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"ore/datos/\",\"patrones\":[\"ventas.csv\"],\"conocidos\":[]}" | $B versiones > $W/v.json 2>$W/e
  r=$(campo $W/v.json 'd["items"][0]["version"].count("@") == 1, d["items"][0]["huella"].startswith("md5:")' 2>/dev/null)
  [ "$r" = "(True, True)" ] && dice "4 versiones: $(campo $W/v.json 'd["items"][0]["version"]'), $(campo $W/v.json 'd["items"][0]["huella"]')" || falla "4 versiones: $(cat $W/v.json $W/e)"
  python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v.json "$U" > $W/pedido.json

  # 5 bajar, cotejado; con la huella cambiada, no
  $B bajar < $W/pedido.json > $W/b.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b.bin && grep -aq '2,7' $W/b.bin && dice "5 bajar: el ítem, cotejado con su md5" || falla "5 bajar: $(head -c 300 $W/b.bin)"
  python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["items"][0]["huella"]="md5:AAAAAAAAAAAAAAAAAAAAAA=="; print(json.dumps(p))' $W/pedido.json | $B bajar > $W/b2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b2.bin && grep -aq 'md5:AAAA' $W/b2.bin && dice "5 con la huella cambiada, no se copia" || falla "5 huella: $(head -c 300 $W/b2.bin)"

  # 6 el fichero cambia entre `versiones` y `bajar`: se pidió la revisión fijada,
  #   que ya no está; no se copia, y nunca los bytes nuevos
  printf 'id,total\n1,10.5\n2,7\n3,NUEVA-FILA\n' > "$M/ventas2.csv"
  subir ore/datos/ventas.csv "$M/ventas2.csv"
  $B bajar < $W/pedido.json > $W/b3.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b3.bin && grep -aq 'no longer exists' $W/b3.bin && ! grep -aq 'NUEVA-FILA' $W/b3.bin \
    && dice "6 cambió por medio: la revisión fijada ya no está, no se copia, nunca los bytes nuevos" || falla "6: $(head -c 300 $W/b3.bin)"

  # 7 un nativo: exportado sin el BOM, cotejado; si cambia por medio, no
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"ore/docs/\",\"patrones\":[\"doc.txt\"],\"conocidos\":[]}" | $B versiones > $W/vn.json 2>$W/e
  r=$(campo $W/vn.json 'd["items"][0]["version"].count("#v") == 1, d["items"][0]["huella"].startswith("md5:")' 2>/dev/null)
  [ "$r" = "(True, True)" ] || falla "7 versiones del Doc: $(cat $W/vn.json $W/e)"
  python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/vn.json "$U" > $W/pedido-n.json
  $B bajar < $W/pedido-n.json > $W/n1.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/n1.bin && grep -aq 'Hola Drive' $W/n1.bin && ! grep -aq $'\xef\xbb\xbf' $W/n1.bin \
    && dice "7 el Doc: exportado sin el BOM y cotejado con su md5" || falla "7 bajar el Doc: $(head -c 300 $W/n1.bin)"
  sembrar POST /nativo "ore/docs/doc" "&clase=doc" "$M/doc2.txt"
  $B bajar < $W/pedido-n.json > $W/n2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/n2.bin && ! grep -aq 'Adios' $W/n2.bin \
    && dice "7 el Doc cambió por medio: no se copia, nunca el nuevo" || falla "7 el Doc cambiado: $(head -c 300 $W/n2.bin)"

  # 8 los tests de ore-gdrive contra el mismo Drive de mentira
  r=$(ORE_LAB_GDRIVE_MENTIRA="$G" cargo test --quiet -p ore-gdrive --test mentira 2>&1)
  echo "$r" | grep -q "test result: ok. 2 passed" && dice "8 el Origen de ore-gdrive: la lectura fijada, los nativos, el feed de cambios y su respaldo" \
    || falla "8 ore-gdrive: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"

  # y al final: ninguna petición sin su token, y el ritmo se respetó
  c=$(curl -s $G/_mentira/cuentas)
  echo "$c" | grep -q '"sin_token": 0' && dice "ninguna petición llegó sin su token ($c)" || falla "sin token: $c"
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker (o --dentro, con cargo y python3)"; exit 2; }
export MSYS_NO_PATHCONV=1  # los volúmenes de Docker, en Windows
echo "una carpeta de Google Drive (laboratorio: el Drive de mentira, páginas de 2, un 429 o 403 cada 7)"
docker run --rm -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o6-gdrive.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
