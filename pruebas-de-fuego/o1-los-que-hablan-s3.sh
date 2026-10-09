#!/usr/bin/env bash
# LOS QUE HABLAN S3, DE PUNTA A PUNTA (ADR 0061 O1·3), en Docker y sin cuentas:
#
#   versity  VersityGW (posix + --versioning-dir): CON versionado, como S3.
#   garage   Garage: SIN versionado, como Cloudflare R2 (ignora `versionId` y
#            respeta `If-Match`, medido en O1·0).
#
# (MinIO ya no publica imágenes: Docker Hub y quay, medido el 2026-10-09.)
#
#   1  check: cómo fija cada uno (`version` / `etag`), sin ARNs fuera de AWS
#   2  catálogo: las dos tablas (CSV, JSONL) y los tres conjuntos de objetos
#   3  versiones: con `versionId` en versity, `etag:…` en garage
#   4  bajar: el ítem de la colección mantenida, cotejado
#   5  la URL firmada: versity la da y se baja con curl (manipulada, 403);
#      garage no (sólo fija por ETag, D-O1)
#   6  la virtual por `ore-medios` (`laboratorio_s3.rs`): rango, entero, 412
#   7  reescrito entre `versiones` y `bajar`: garage NO copia (412, nunca otros
#      bytes); versity copia la versión que se listó, la de antes
#
#   bash pruebas-de-fuego/o1-los-que-hablan-s3.sh
#
# Necesita Docker. Las credenciales son de prueba, generadas en cada corrida.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
VERSITY_IMG=versity/versitygw@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499
GARAGE_IMG=dxflrs/garage@sha256:fdb8272fcbe643eef830ee17874d5d4ed623a86501f1acbac8012583113b1c26
AWS_IMG=amazon/aws-cli@sha256:e3e329e1d2894b7b4bbb0aacacd0a155262159b2e7a3b4275eb1f24046d3e06c
RUST_IMG=rust:1-bookworm
RED=ore-o1

# ── dentro: lo que se comprueba, en el contenedor de Rust ─────────────────────
if [ "${1:-}" = "--dentro" ]; then
  fase=$2
  cd /w
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  command -v python3 >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq python3 >/dev/null; }
  cargo build --quiet -p ore-read-s3 -p ore-firmar-s3 2>&1 | tail -3
  T="${CARGO_TARGET_DIR:-/w/target}/debug"
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  for p in versity garage; do
    if [ $p = versity ]; then U=$URL_VERSITY; FIJA=version; else U=$URL_GARAGE; FIJA=etag; fi
    W=/trabajo/$p
    mkdir -p $W
    if [ "$fase" = antes ]; then
      printf '%s' "{\"url\":\"$U\",\"objeto\":\"\"}" | $T/ore-read-s3 check > $W/check.json 2>/dev/null
      f=$(campo $W/check.json 'd["fija"]'); donde=$(campo $W/check.json 'd["permisos"]["listar"]["donde"]')
      [ "$f" = $FIJA ] && [ "$donde" = cubo ] && dice "$p · 1 check: fija por $f, sobre \`$donde\`" || falla "$p · 1 check: $(cat $W/check.json)"
      printf '%s' "$U" | $T/ore-read-s3 catalogo f > $W/cat.json 2>/dev/null
      n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
      [ "$n" = "2 3" ] && dice "$p · 2 catálogo: 2 tablas y 3 conjuntos de objetos" || falla "$p · 2 catálogo: $n"
      printf '%s' "{\"url\":\"$U\",\"objeto\":\"docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $T/ore-read-s3 versiones > $W/v.json 2>$W/e
      ver=$(campo $W/v.json 'd["items"][0]["version"]' 2>/dev/null)
      case "$p:$ver" in
        versity:etag:*|versity:|garage:) falla "$p · 3 versiones: '$ver' $(cat $W/e)";;
        versity:*) dice "$p · 3 versiones: versionId $ver";;
        garage:etag:*) dice "$p · 3 versiones: $ver (fijada por su ETag)";;
        *) falla "$p · 3 versiones: '$ver'";;
      esac
      # el pedido de bajar, para esta fase y para la de después
      python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v.json "$U" > $W/pedido.json
      $T/ore-read-s3 bajar < $W/pedido.json > $W/b.bin 2>$W/e
      grep -q '"fin":"ok"' $W/b.bin && grep -q '% uno' $W/b.bin && dice "$p · 4 bajar: el ítem, cotejado" || falla "$p · 4 bajar: $(grep -v aviso $W/e | head -2)"
      printf '%s' "{\"url\":\"$U\",\"segundos\":60,\"items\":[{\"clave\":\"docs/a.pdf\",\"version\":\"$ver\",\"tipo\":\"application/pdf\",\"disposicion\":\"inline\"}]}" | $T/ore-firmar-s3 > $W/f.json 2>$W/e
      if [ $p = versity ]; then
        u=$(campo $W/f.json 'd["firmadas"][0]["url"]' 2>/dev/null)
        c1=$(curl -s -o $W/u -w '%{http_code}' "$u"); c2=$(curl -s -o /dev/null -w '%{http_code}' "${u}x")
        [ "$c1" = 200 ] && grep -q '% uno' $W/u && [ "$c2" = 403 ] && dice "$p · 5 la URL firmada se baja con curl (manipulada: $c2)" || falla "$p · 5 URL: $c1/$c2"
      else
        grep -q 'sólo se fija por su ETag' $W/e && dice "$p · 5 la URL no se firma: sólo fija por su ETag (D-O1)" || falla "$p · 5 debía negarse: $(cat $W/f.json $W/e)"
      fi
    else
      # después: docs/a.pdf reescrito desde fuera; el pedido es el de antes
      $T/ore-read-s3 bajar < $W/pedido.json > $W/b2.bin 2>$W/e2
      if [ $p = garage ]; then
        ! grep -q '"fin":"ok"' $W/b2.bin && grep -q '412' $W/b2.bin $W/e2 && ! grep -q '% UNO' $W/b2.bin \
          && dice "$p · 7 reescrito: no se copia (412), nunca otros bytes" || falla "$p · 7: $(head -c 300 $W/b2.bin) $(grep -v aviso $W/e2 | head -2)"
      else
        grep -q '"fin":"ok"' $W/b2.bin && grep -q '% uno' $W/b2.bin \
          && dice "$p · 7 reescrito: copia la versión que se listó, la de antes" || falla "$p · 7: $(head -c 300 $W/b2.bin) $(grep -v aviso $W/e2 | head -2)"
      fi
    fi
  done
  if [ "$fase" = antes ]; then
    r=$(ORE_LAB_S3="versity=$URL_VERSITY|garage=$URL_GARAGE" cargo test --quiet -p ore-medios --test laboratorio_s3 -- --nocapture 2>&1)
    echo "$r" | grep -q "test result: ok. 1 passed" && dice "6 la virtual por ore-medios: $(echo "$r" | grep -c ' · rango, entero, 412') orígenes (rango, entero, 412, la URL)" \
      || falla "6 ore-medios: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"
  fi
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
TMP="$(mktemp -d)"
r() { head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
VK="ore$(r | head -c 12)"; VS="$(r)"; RPC="$(r)$(r | head -c 16)"
# Docker para Windows no entiende `/tmp/…` de Git Bash: la ruta, en la suya.
nativa() { cygpath -m "$1" 2>/dev/null || echo "$1"; }
limpiar() {
  docker rm -f o1-versity o1-garage >/dev/null 2>&1
  docker volume rm -f o1-versity-datos o1-versity-vers o1-garage o1-trabajo >/dev/null 2>&1
  rm -rf "$TMP"
}
trap limpiar EXIT
limpiar 2>/dev/null; TMP="$(mktemp -d)"
docker network inspect $RED >/dev/null 2>&1 || docker network create $RED >/dev/null

docker run -d --name o1-versity --network $RED -v o1-versity-datos:/datos -v o1-versity-vers:/versiones \
  "$VERSITY_IMG" --access "$VK" --secret "$VS" --port :7070 posix --versioning-dir /versiones /datos >/dev/null
cat > "$TMP/garage.toml" <<EOF
metadata_dir = "/var/lib/garage/meta"
data_dir = "/var/lib/garage/data"
db_engine = "sqlite"
replication_factor = 1
rpc_bind_addr = "[::]:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "$RPC"
[s3_api]
s3_region = "garage"
api_bind_addr = "[::]:3900"
root_domain = ".s3.garage.localhost"
EOF
docker create --name o1-garage --network $RED -v o1-garage:/var/lib/garage "$GARAGE_IMG" >/dev/null
docker cp "$(nativa "$TMP/garage.toml")" o1-garage:/etc/garage.toml >/dev/null
docker start o1-garage >/dev/null
sleep 4
G() { docker exec o1-garage /garage "$@" 2>/dev/null; }
G layout assign -z dc1 -c 1G "$(G node id -q | cut -d@ -f1)" >/dev/null
G layout apply --version 1 >/dev/null
CLAVE=$(G key create ore)
GK=$(echo "$CLAVE" | awk '/Key ID/ {print $NF}'); GS=$(echo "$CLAVE" | awk '/Secret key/ {print $NF}')
G bucket create cubo >/dev/null
G bucket allow --read --write --owner cubo --key ore >/dev/null

# la muestra
M="$TMP/m"; mkdir -p "$M/datos/ventas" "$M/datos/eventos" "$M/docs/Nueva carpeta" "$M/img"
printf 'id,total,fecha\n1,10.5,2026-01-01\n2,7,2026-01-02\n' > "$M/datos/ventas/ventas.csv"
printf '{"id":1,"tipo":"a"}\n{"id":2,"tipo":"b"}\n' > "$M/datos/eventos/e.jsonl"
printf '%%PDF-1.4\n%% uno\n%%%%EOF\n' > "$M/docs/a.pdf"
printf '%%PDF-1.4\n%% dos, con espacio\n%%%%EOF\n' > "$M/docs/Nueva carpeta/b.pdf"
printf '\x89PNG\r\n\x1a\n-una-imagen-' > "$M/img/c.png"
aws_() { # proveedor guion — el guion corre en la AWS CLI, con la muestra en /m
  local k s reg ep
  if [ "$1" = versity ]; then k=$VK; s=$VS; reg=us-east-1; ep=http://o1-versity:7070; else k=$GK; s=$GS; reg=garage; ep=http://o1-garage:3900; fi
  local c; c=$(docker create --network $RED -e AWS_ACCESS_KEY_ID="$k" -e AWS_SECRET_ACCESS_KEY="$s" -e AWS_DEFAULT_REGION=$reg \
    -e AWS_REQUEST_CHECKSUM_CALCULATION=when_required -e AWS_RESPONSE_CHECKSUM_VALIDATION=when_required \
    -e EP=$ep --entrypoint sh "$AWS_IMG" -c "$2")
  docker cp "$(nativa "$M")" "$c:/m" >/dev/null
  docker start -a "$c"; local x=$?; docker rm -f "$c" >/dev/null; return $x
}
aws_ versity 'aws --endpoint-url $EP s3api create-bucket --bucket cubo >/dev/null && aws --endpoint-url $EP s3api put-bucket-versioning --bucket cubo --versioning-configuration Status=Enabled' || exit 2
for p in versity garage; do aws_ $p 'aws --endpoint-url $EP s3 cp --recursive --only-show-errors /m s3://cubo/' || exit 2; done
echo "los que hablan S3 (laboratorio: versity con versionado, garage sin él)"

DENTRO() {
  docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v o1-trabajo:/trabajo \
    -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
    -e URL_VERSITY="s3://cubo/?region=us-east-1&access_key_id=$VK&secret_access_key=$VS&endpoint=http://o1-versity:7070" \
    -e URL_GARAGE="s3://cubo/?region=garage&access_key_id=$GK&secret_access_key=$GS&endpoint=http://o1-garage:3900" \
    -w /w "$RUST_IMG" bash pruebas-de-fuego/o1-los-que-hablan-s3.sh --dentro "$1" 2>&1 | grep -v '^info:\|^warn:'
  return "${PIPESTATUS[0]}"
}
DENTRO antes; f1=$?
# docs/a.pdf reescrito por fuera, entre `versiones` y `bajar`, con el MISMO
# tamaño: sólo el `If-Match` lo para (el tamaño del manifiesto casaría).
printf '%%PDF-1.4\n%% UNO\n%%%%EOF\n' > "$M/docs/a.pdf"
for p in versity garage; do aws_ $p 'aws --endpoint-url $EP s3 cp --only-show-errors /m/docs/a.pdf s3://cubo/docs/a.pdf' || exit 2; done
DENTRO despues; f2=$?
fallos=$((f1 + f2))
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
