#!/usr/bin/env bash
# UN SERVIDOR SFTP, DE PUNTA A PUNTA (ADR 0061 O4·4), en Docker y sin cuentas,
# contra `atmoz/sftp` (OpenSSH de verdad), con una clave Ed25519 de la "celda"
# generada en cada corrida y autorizada en el servidor, y otra que no.
#
# Un SFTP no versiona (D-O1): sólo colecciones mantenidas, y cada lectura se
# vigila por tamaño y `mtime` (D-O4).
#
#   1  check: conexión, huella, identidad, listar y leer; sin huella fijada,
#      dice la del servidor para confirmarla
#   2  la huella cambiada: se niega, y la clave NO se envía (identidad sin probar)
#   3  una clave no autorizada: se dice qué hacer
#   4  explorar y catálogo: las carpetas; dos tablas y los conjuntos de objetos
#   5  versiones: cada fichero con su validador; con una edad mínima larga, ninguno
#   6  bajar: el fichero, copiado; reescrito por fuera (mismo tamaño) entre
#      `versiones` y `bajar`: NO se copia, nunca otros bytes
#   7  el Origen de `ore-sftp` (`tests/laboratorio.rs`): huellas, enlaces,
#      edad, rangos, permiso, D-O1, y un fichero reescrito a mitad de lectura
#   8  `ore` de punta a punta: `discover` de una foránea sobre el SFTP da
#      colecciones MANTENIDAS, y una forzada a virtual se niega (D-O1)
#
#   bash pruebas-de-fuego/o4-sftp.sh
#
# Necesita Docker.
set -u
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
export MSYS_NO_PATHCONV=1
SFTP_IMG=atmoz/sftp@sha256:0960390462a4441dbb63698d7c185b76a41ffcee7b78ff4adf275f3e66f9c475
RUST_IMG=rust:1-bookworm
RED=ore-o4

# ── dentro: lo que se comprueba, en el contenedor de Rust ─────────────────────
if [ "${1:-}" = "--dentro" ]; then
  cd /w
  fallos=0
  falla() { echo "  ✗ $*"; fallos=$((fallos + 1)); }
  dice() { echo "  ✓ $*"; }
  command -v python3 >/dev/null || { apt-get update -qq >/dev/null && apt-get install -y -qq python3 >/dev/null; }
  cargo build --quiet -p ore-read-sftp -p ore-cli 2>&1 | tail -3
  T="${CARGO_TARGET_DIR:-/w/target}/debug"; B=$T/ore-read-sftp
  W=/tmp/o4; rm -rf $W; mkdir -p $W
  campo() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$@"; }
  co() { printf '%s' "{\"url\":\"$1\",\"objeto\":\"\"}"; }
  U="sftp://ore@o4-sftp/datos/?huella=$HUELLA&edad=0"

  # 1 check
  co "$U" | $B check > $W/c.json 2>/dev/null
  r=$(campo $W/c.json 'd["ok"], d["fija"], sorted(k for k, p in d["permisos"].items() if p.get("ok")), len(d["prefijos"]), "cerrado.pdf" in d["permisos"]["leer"].get("porque", "")')
  [ "$r" = "(True, 'ninguna', ['conexion', 'huella', 'identidad', 'leer', 'listar'], 4, True)" ] \
    && dice "1 check: conexión, huella, identidad, listar y leer (y dice el fichero que no se deja); fija: ninguna" || falla "1 check: $r $(cat $W/c.json)"
  co "${U%%\?*}" | $B check > $W/c.json 2>/dev/null
  r=$(campo $W/c.json 'd["ok"], d["huella"] == "'"$HUELLA"'", d["permisos"]["identidad"].get("probado")')
  [ "$r" = "(False, True, False)" ] && dice "1 sin huella fijada: dice la del servidor ($HUELLA) y no entra" || falla "1 sin huella: $r $(cat $W/c.json)"

  # 2 la huella cambiada
  co "${U/$HUELLA/SHA256:otraotraotraotraotraotraotra}" | $B check > $W/c.json 2>/dev/null
  r=$(campo $W/c.json 'd["ok"], d["permisos"]["identidad"].get("probado"), "otro" in d["porque"]')
  [ "$r" = "(False, False, True)" ] && dice "2 la huella cambiada: se niega, y la clave no se envía" || falla "2: $r $(cat $W/c.json)"

  # 3 una clave no autorizada
  co "$U" | ORE_SFTP_CLAVE=/k/otra $B check > $W/c.json 2>/dev/null
  grep -q 'authorized_keys' $W/c.json && [ "$(campo $W/c.json 'd["ok"]')" = False ] \
    && dice "3 una clave no autorizada: «la clave pública de la celda tiene que estar en su authorized_keys»" || falla "3: $(cat $W/c.json)"

  # 4 explorar y catálogo
  co "$U" | $B explorar > $W/x.json 2>/dev/null
  [ "$(campo $W/x.json 'len([c for c in d["contiene"] if c["nombre"].endswith("/") and "url" in c])')" -ge 3 ] \
    && dice "4 explorar: las carpetas, cada una con su URL" || falla "4 explorar: $(cat $W/x.json)"
  printf '%s' "$U" | $B catalogo f > $W/cat.json 2>/dev/null
  n=$(campo $W/cat.json 'str(len(d["tables"])) + " " + str(len(d["objects"]))')
  [ "${n%% *}" = 2 ] && [ "${n##* }" -ge 3 ] && dice "4 catálogo: 2 tablas y ${n##* } conjuntos de objetos" || falla "4 catálogo: $n"

  # 5 versiones, y el pedido de bajar
  printf '%s' "{\"url\":\"$U\",\"objeto\":\"datos/docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $B versiones > $W/v.json 2>/dev/null
  [ "$(campo $W/v.json 'd["items"][0]["version"].startswith("etag:")')" = True ] \
    && dice "5 versiones: $(campo $W/v.json 'd["items"][0]["version"]') (tamaño y mtime)" || falla "5 versiones: $(cat $W/v.json)"
  printf '%s' "{\"url\":\"${U/edad=0/edad=3600}\",\"objeto\":\"datos/docs/\",\"patrones\":[\"a.pdf\"],\"conocidos\":[]}" | $B versiones > $W/v2.json 2>/dev/null
  [ "$(campo $W/v2.json 'len(d["items"])')" = 0 ] && dice "5 con una edad mínima de una hora, nada se copia todavía" || falla "5 edad: $(cat $W/v2.json)"
  python3 -c 'import json,sys; i=json.load(open(sys.argv[1]))["items"][0]; print(json.dumps({"url":sys.argv[2],"hilos":1,"items":[{k:i[k] for k in ("clave","version","huella","tamano")}]}))' $W/v.json "$U" > $W/pedido.json

  # 6 bajar
  $T/ore-read-sftp bajar < $W/pedido.json > $W/b.bin 2>/dev/null
  grep -aq '"fin":"ok"' $W/b.bin && grep -aq '% uno' $W/b.bin && dice "6 bajar: el fichero, copiado" || falla "6 bajar: $(head -c 300 $W/b.bin)"

  # 7 el test de laboratorio (antes de reescribir a.pdf: lo espera como era)
  r=$(ORE_LAB_SFTP="$U" cargo test --quiet -p ore-sftp --test laboratorio un_servidor_sftp_como_origen -- --nocapture 2>&1)
  echo "$r" | grep -q "test result: ok. 1 passed" && dice "7 el Origen de ore-sftp: $(echo "$r" | grep '^sftp:' | cut -c7-)" \
    || falla "7 ore-sftp: $(echo "$r" | grep -E 'panicked|FAILED|error' | head -4)"

  # 6′ reescrito por fuera entre `versiones` y `bajar`, con el MISMO tamaño
  #    (un segundo después: el `mtime` de SFTP v3 son segundos)
  sleep 1.1
  ORE_LAB_SFTP="$U" cargo test --quiet -p ore-sftp --test laboratorio reescribir_a_pdf -- --ignored >/dev/null 2>&1 \
    || falla "6′ no se pudo reescribir docs/a.pdf"
  $T/ore-read-sftp bajar < $W/pedido.json > $W/b2.bin 2>/dev/null
  ! grep -aq '"fin":"ok"' $W/b2.bin && ! grep -aq '% UNO' $W/b2.bin && grep -aq 'como se listó' $W/b2.bin \
    && dice "6′ reescrito con el mismo tamaño: NO se copia, nunca otros bytes" || falla "6′ reescrito: $(head -c 300 $W/b2.bin)"

  # 8 `ore` de punta a punta
  export PATH="$T:$PATH" ORE_SFTP_URL="$U"
  A=$W/arbol; mkdir -p $A; cd $A
  ore init . --name ficheros >/dev/null 2>&1 || falla "8 ore init"
  printf 'datasources:\n  - { name: sftp_demo, type: sftp, connectionEnv: ORE_SFTP_URL }\n' >> ontology.config.yaml
  ore source catalog sftp_demo --out $W/catalogo.json > $W/cat.txt 2>&1 || falla "8 catalog: $(tail -2 $W/cat.txt)"
  ore discover --source sftp_demo --type foreign --only docs.docs --no-model --owner team:datos \
    --out packages/docs --name docs > $W/disc.txt 2>&1 || falla "8 discover: $(tail -3 $W/disc.txt)"
  C=$(find packages/docs -path "*collections*" -name "*.yaml" | head -1)
  [ -n "$C" ] && ! grep -q "virtual: true" "$C" && dice "8 discover de una foránea sobre el SFTP: la colección sale mantenida" \
    || falla "8 discover: $(cat "$C" 2>/dev/null | head -20) $(tail -3 $W/disc.txt)"
  if [ -n "$C" ]; then
    printf '  virtual: true\n' >> "$C"
    ore materialize . > $W/m.txt 2>&1
    grep -q "D-O1" $W/m.txt && grep -q "sólo puede ser mantenida" $W/m.txt \
      && dice "8 forzada a virtual: \`ore\` la niega (D-O1)" || falla "8 virtual: $(grep -v '^ *$' $W/m.txt | tail -4)"
  fi
  exit $fallos
fi

# ── fuera: el laboratorio ─────────────────────────────────────────────────────
command -v docker >/dev/null || { echo "hace falta Docker"; exit 2; }
limpiar() { docker rm -f o4-sftp >/dev/null 2>&1; docker volume rm -f o4-k o4-pub >/dev/null 2>&1; }
trap limpiar EXIT
limpiar
docker network inspect $RED >/dev/null 2>&1 || docker network create $RED >/dev/null
# la clave de la "celda" (autorizada) y otra (no), de esta corrida
docker run --rm -v o4-k:/k -v o4-pub:/p --entrypoint sh "$SFTP_IMG" -c \
  'ssh-keygen -q -t ed25519 -N "" -f /k/id && ssh-keygen -q -t ed25519 -N "" -f /k/otra && chmod 600 /k/id /k/otra && cp /k/id.pub /p/'
docker run -d --name o4-sftp --network $RED -v o4-pub:/home/ore/.ssh/keys:ro "$SFTP_IMG" ore::1001 >/dev/null
sleep 3
docker exec o4-sftp sh -c '
  D=/home/ore/datos; mkdir -p "$D/docs/Nueva carpeta" "$D/ventas" "$D/eventos" "$D/img"
  printf "%%PDF-1.4\n%% uno\n%%%%EOF\n" > $D/docs/a.pdf
  printf "%%PDF-1.4\n%% dos, con espacio\n%%%%EOF\n" > "$D/docs/Nueva carpeta/b.pdf"
  printf "id,total,fecha\n1,10.5,2026-01-01\n2,7,2026-01-02\n" > $D/ventas/ventas.csv
  printf "{\"id\":1,\"tipo\":\"a\"}\n{\"id\":2,\"tipo\":\"b\"}\n" > $D/eventos/e.jsonl
  printf "\211PNG\r\n\032\n-una-imagen-" > $D/img/c.png
  head -c 8388608 /dev/zero | tr "\0" "A" > $D/grande.bin
  ln -s docs/a.pdf $D/enlace.pdf
  printf "secreto" > $D/cerrado.pdf && chmod 000 $D/cerrado.pdf
  chown -R 1001 $D'
HUELLA=$(docker exec o4-sftp sh -c 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub' | awk '{print $2}')
echo "un servidor SFTP (laboratorio: atmoz/sftp, OpenSSH; la clave de la celda autorizada)"
docker run --rm --network $RED -v "$(cd "$RAIZ" && (pwd -W 2>/dev/null || pwd)):/w" -v o4-k:/k \
  -v ore-cargo-registry:/usr/local/cargo/registry -v ore-pruebas-t:/tt -e CARGO_TARGET_DIR=/tt/main \
  -e ORE_SFTP_CLAVE=/k/id -e HUELLA="$HUELLA" \
  -w /w "$RUST_IMG" bash pruebas-de-fuego/o4-sftp.sh --dentro 2>&1 | grep -v '^info:\|^warn:'
fallos=${PIPESTATUS[0]}
[ "$fallos" = 0 ] && echo "todo bien" || { echo "$fallos fallos"; exit 1; }
