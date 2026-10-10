#!/usr/bin/env bash
# UNA BIBLIOTECA DE SHAREPOINT, DE PUNTA A PUNTA (ADR 0061 O5·4), en Docker y
# sin tenant, contra el Graph de mentira (`graph-de-mentira.py`: no hay
# emulador de Graph; está escrito de la documentación), en páginas de 2 y con
# un `429` cada 7 peticiones, dentro del mismo contenedor de Rust.
#
# SharePoint versiona (D-O5): cada fichero se fija por `<id>@<versión>`, la
# actual se lee vigilada por su `cTag` y una vieja por su id.
#
#   1  check: identidad, sitio, biblioteca, listar, leer y versiones; un sitio
#      sin la concesión dice cómo darla (PnP); una biblioteca que no está dice
#      cuáles hay
#   2  el token es al portador: un `endpoint` que no es Graph, rechazado fuera
#      del laboratorio (`ORE_SHAREPOINT_LABORATORIO`)
#   3  explorar y catálogo: las carpetas y las bibliotecas del sitio (y lo
#      saltado); dos tablas (CSV, JSONL) y los conjuntos de objetos
#   4  versiones: cada fichero con su `<id>@<versión>` y su `quickxor:`
#   5  bajar: el ítem, cotejado; con la huella cambiada, no se copia
#   6  una versión NUEVA entre `versiones` y `bajar`: se copia la fijada (por
#      su id), nunca la nueva
#   7  la fijada, recortada por la biblioteca: no se copia
#   8  el Origen de `ore-graph` (`tests/laboratorio.rs`): páginas, cuadernos y
#      accesos, la actual vigilada y la vieja, rangos, eTag≠cTag, una versión
#      nueva a mitad de lectura, 403, el ritmo, sin token en la descarga
#   9  la virtual por `ore-medios` (`tests/laboratorio_sharepoint.rs`)
#  10  `ore` de punta a punta: `discover` de una foránea sobre la biblioteca da
#      una colección VIRTUAL (SharePoint versiona), el árbol valida y
#      `materialize` no la niega (sin lago aquí, se para al ir a escribir)
#   y al final: ninguna descarga recibió el token de Graph
#
# Lo que el Graph de mentira no sabe —`Sites.Selected` de verdad, el canje con
# Entra, el ritmo y los errores reales, `delta`— queda para un tenant (deuda
# temporal en el ADR).
#
#   bash pruebas-de-fuego/o5-sharepoint.sh
#
# Necesita Docker.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
RUST_IMG=rust:1-bookworm

# ── dentro: lo que se comprueba, en el contenedor de Rust ─────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd /w
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  G=http://127.0.0.1:8790
  PAGINA=2 CADA_429=7 GRAPH_TOKEN="$ORE_GRAPH_TOKEN" python3 pruebas-de-fuego/graph-de-mentira.py 8790 8791 > /tmp/graph.log 2>&1 &
  cargo build --quiet -p ore-read-sharepoint -p ore-cli 2>&1 | tail -3
  grep -q listo /tmp/graph.log || { echo "el Graph de mentira no arranca: $(cat /tmp/graph.log)"; exit 2; }
  T="${CARGO_TARGET_DIR:-/w/target}/debug"; B=$T/ore-read-sharepoint
  W=/tmp/o5; rm -rf $W; mkdir -p $W
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  siembra() { # método qué ruta fichero|- [más]: en «Activos del sitio» de sites/Finanzas
    python3 - "$G" "$1" "$2" "$3" "$4" "${5:-}" <<'PY'
import sys, urllib.request, urllib.parse
g, metodo, que, ruta, fichero, mas = sys.argv[1:7]
q = urllib.parse.urlencode({"sitio": "sites/Finanzas", "biblioteca": "Activos del sitio", "ruta": ruta}) + mas
datos = open(fichero, "rb").read() if fichero != "-" else None
urllib.request.urlopen(urllib.request.Request(f"{g}/_mentira{que}?{q}", data=datos, method=metodo)).read()
PY
  }
  subir() { siembra PUT "" "$1" "$2"; }

  # la muestra
  M=$W/m; mkdir -p "$M"
  printf 'id,total,fecha\n1,10.5,2026-01-01\n2,7,2026-01-02\n' > "$M/ventas.csv"
  printf '{"id":1,"tipo":"a"}\n{"id":2,"tipo":"b"}\n' > "$M/e.jsonl"
  printf '%%PDF-1.4\n%% uno\n%%%%EOF\n' > "$M/a1.pdf"
  printf '%%PDF-1.4\n%% UNO\n%%%%EOF\n' > "$M/a2.pdf"
  printf '%%PDF-1.4\n%% TRES, otra\n%%%%EOF\n' > "$M/a3.pdf"
  printf '%%PDF-1.4\n%% dos, con espacio\n%%%%EOF\n' > "$M/b.pdf"
  printf '\x89PNG\r\n\x1a\n-una-imagen-' > "$M/c.png"
  subir datos/ventas/ventas.csv "$M/ventas.csv"; subir datos/eventos/e.jsonl "$M/e.jsonl"
  subir docs/a.pdf "$M/a1.pdf"; subir docs/a.pdf "$M/a2.pdf"
  subir "docs/Nueva carpeta/b.pdf" "$M/b.pdf"; subir img/c.png "$M/c.png"
  siembra PUT "" Cuaderno "$M/c.png" "&tipo=package"
  U="sharepoint://contoso.sharepoint.com/sites/Finanzas/Activos%20del%20sitio/?tenant=t&cliente=app&endpoint=$G"
  co() { printf '%s' "{\"url\":\"$1\",\"objeto\":\"\"}"; }

  # 1 check
  co "$U" | $B check > $W/check.json 2>/dev/null
  r=$(campo $W/check.json 'd["ok"], d["fija"], sorted(k for k, p in d["permisos"].items() if p.get("ok")), len(d["prefijos"])')
  [ "$r" = "(True, 'version', ['biblioteca', 'identidad', 'leer', 'listar', 'sitio', 'versiones'], 3)" ] \
    && dice "1 check: los seis pasos, fija por versión, 3 carpetas" || falla "1 check: $r $(cat $W/check.json)"
  co "${U/Finanzas/RRHH}" | $B check > $W/c2.json 2>/dev/null
  grep -q 'Grant-PnPEntraIDAppSitePermission -AppId app' $W/c2.json && grep -q '"ok":false' $W/c2.json \
    && dice "1 check: un sitio sin la concesión dice cómo darla (Sites.Selected y PnP)" || falla "1 check RRHH: $(cat $W/c2.json)"
  co "${U/Activos%20del%20sitio/Facturas}" | $B check > $W/c3.json 2>/dev/null
  grep -q 'no hay una biblioteca `Facturas` en el sitio; hay: `Documentos`, `Activos del sitio`' $W/c3.json \
    && dice "1 check: una biblioteca que no está dice cuáles hay" || falla "1 check biblioteca: $(cat $W/c3.json)"

  # 2 el token es al portador
  co "$U" | env -u ORE_SHAREPOINT_LABORATORIO $B check > /dev/null 2> $W/e
  grep -q 'no se le da a otro servidor' $W/e && dice "2 fuera del laboratorio, un endpoint que no es Graph no recibe el token" || falla "2 la guarda: $(cat $W/e)"

  # 3 explorar y catálogo
  co "$U" | $B explorar > $W/x.json 2>/dev/null
  r=$(campo $W/x.json 'len([c for c in d["contiene"] if c["nombre"]]), sorted(b["nombre"] for b in d["bibliotecas"]), "saltados" in d')
  [ "$r" = "(3, ['Activos del sitio', 'Documentos'], True)" ] \
    && dice "3 explorar: 3 carpetas, las 2 bibliotecas del sitio con su URL, y el cuaderno saltado" || falla "3 explorar: $r $(cat $W/x.json)"
  printf '%s' "$U" | $B catalogo f > $W/cat.json 2>/dev/null
  n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
  [ "$n" = "2 3" ] && dice "3 catálogo: 2 tablas y 3 conjuntos de objetos (sin el cuaderno)" || falla "3 catálogo: $n"

  # 4 versiones, y el pedido de bajar
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $B versiones > $W/v.json 2>$W/e
  r=$(campo $W/v.json 'd["items"][0]["version"].endswith("@2.0"), d["items"][0]["huella"].startswith("quickxor:")' 2>/dev/null)
  [ "$r" = "(True, True)" ] && dice "4 versiones: $(campo $W/v.json 'd["items"][0]["version"]'), $(campo $W/v.json 'd["items"][0]["huella"]')" || falla "4 versiones: $(cat $W/v.json $W/e)"
  python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v.json "$U" > $W/pedido.json

  # 5 bajar, cotejado; con la huella cambiada, no
  $B bajar < $W/pedido.json > $W/b.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b.bin && grep -aq '% UNO' $W/b.bin && dice "5 bajar: el ítem, cotejado con su quickXorHash" || falla "5 bajar: $(head -c 300 $W/b.bin)"
  python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); p["items"][0]["huella"]="quickxor:AAAAAAAAAAAAAAAAAAAAAAAAAAA="; print(json.dumps(p))' $W/pedido.json | $B bajar > $W/b2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b2.bin && grep -aq 'y el manifiesto quickxor:AAAA' $W/b2.bin && dice "5 con la huella cambiada, no se copia" || falla "5 huella: $(head -c 300 $W/b2.bin)"

  # 6 una versión nueva entre `versiones` y `bajar`: se copia la fijada
  subir docs/a.pdf "$M/a3.pdf"
  $B bajar < $W/pedido.json > $W/b3.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b3.bin && grep -aq '% UNO' $W/b3.bin && ! grep -aq 'TRES' $W/b3.bin \
    && dice "6 una versión nueva por medio: se copia la fijada (2.0, ya por su id), nunca la nueva" || falla "6: $(head -c 300 $W/b3.bin)"

  # 7 la fijada, recortada
  siembra DELETE /version docs/a.pdf - "&version=2.0"
  $B bajar < $W/pedido.json > $W/b4.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b4.bin && grep -aq 'ya no está' $W/b4.bin && ! grep -aq 'TRES' $W/b4.bin \
    && dice "7 la fijada, recortada por la biblioteca: no se copia, nunca otra" || falla "7: $(head -c 300 $W/b4.bin)"

  # 8 y 9: los tests de laboratorio (el de ore-graph siembra en «Documentos»)
  r=$(ORE_LAB_SHAREPOINT="${U/Activos%20del%20sitio/Documentos}" cargo test --quiet -p ore-graph --test laboratorio -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "8 el Origen de ore-graph: $(echo "$r" | grep '^sharepoint:' | cut -c13-)" \
    || falla "8 ore-graph: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  r=$(ORE_LAB_SHAREPOINT="$U" cargo test --quiet -p ore-medios --test laboratorio_sharepoint -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "9 la virtual por ore-medios: rango, por el final, entero con quickxor, una versión nueva no cambia lo fijado, sin URL, recortada" \
    || falla "9 ore-medios: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"

  # 10 `ore` de punta a punta
  export PATH="$T:$PATH" ORE_SHAREPOINT_URL="$U"
  A=$W/arbol; mkdir -p $A; cd $A
  ore init . --name ficheros >/dev/null 2>&1 || falla "10 ore init"
  printf 'datasources:\n  - { name: sp_demo, type: sharepoint, connectionEnv: ORE_SHAREPOINT_URL }\n' >> ontology.config.yaml
  ore source catalog sp_demo --out $W/catalogo.json > $W/cat.txt 2>&1 || falla "10 catalog: $(tail -2 $W/cat.txt)"
  ore discover --source sp_demo --type foreign --only docs.docs --no-model --owner team:datos \
    --out packages/docs --name docs > $W/disc.txt 2>&1 || falla "10 discover: $(tail -3 $W/disc.txt)"
  C=$(find packages/docs -path "*collections*" -name "*.yaml" | head -1)
  [ -n "$C" ] && grep -q "virtual: true" "$C" && dice "10 discover de una foránea sobre la biblioteca: la colección sale virtual" \
    || falla "10 discover: $(cat "$C" 2>/dev/null | head -20) $(tail -3 $W/disc.txt)"
  ore validate . > $W/val.txt 2>&1 && dice "10 ore validate: el árbol con la virtual vale" || falla "10 validate: $(tail -4 $W/val.txt)"
  # Sin lago en este laboratorio, `materialize` no puede escribir el manifiesto:
  # lo que se prueba es que llega hasta ahí —la virtual no se niega, como sí
  # sobre un SFTP (D-O1)—.
  ore materialize . > $W/m.txt 2>&1
  ! grep -q "D-O1" $W/m.txt && grep -q "ORE_R2_S3_ENDPOINT" $W/m.txt \
    && dice "10 materialize no niega la virtual (sólo le falta el lago de este laboratorio)" || falla "10 materialize: $(grep -v '^ *$' $W/m.txt | tail -4)"
  cd /w

  # y al final: el token de Graph no llegó a ninguna descarga
  c=$(curl -s $G/_mentira/cuentas)
  echo "$c" | grep -q '"con_token": 0' && dice "el token de Graph no llegó a ninguna descarga ($c)" || falla "con token: $c"
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
echo "una biblioteca de SharePoint (laboratorio: el Graph de mentira, páginas de 2, un 429 cada 7)"
TOKEN="lab-$(date +%s)-$RANDOM"
docker run --rm -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -e ORE_GRAPH_TOKEN="$TOKEN" -e ORE_SHAREPOINT_LABORATORIO=1 \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o5-sharepoint.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
