#!/usr/bin/env bash
# EL CUSTODIO, contra un Postgres de verdad y con tokens de verdad.
#
# ── ⛔ Por qué el KMS es de mentira, y por qué eso NO debilita esto ──────────
#
# El cerrojo de la llave —que un inquilino no pueda usar la de otro— está
# probado contra Google en `malla/99-el-cerrojo-de-la-llave.yaml`, con IAM de
# verdad. Repetirlo aquí exigiría credenciales de nube en CI, que es justo lo que
# no se tiene ni se quiere.
#
# ⇒ Lo que esto prueba es lo OTRO, que es todo lo demás: quién puede emitir,
#   quién puede resolver, que «no existe» y «no es tuyo» den el mismo error, que
#   el valor no salga por ninguna otra puerta, y que abrir deje huella.
#
# ⭐ Y el cliente de mentira **distingue llaves**: descifrar con una distinta de
#   la que cerró falla. Sin eso, la prueba no notaría que el código pasa la llave
#   equivocada — que es exactamente el fallo que un KMS de verdad convertiría en
#   un `PERMISSION_DENIED` en producción.
#
# ── Lo que fija ────────────────────────────────────────────────────────────
#
#   1  sin token                              401
#   2  emitir sin `secreto:emitir`            SE NIEGA
#   3  emitir                                 y la respuesta NO trae el valor
#   4  listar                                 nombres, y NINGÚN valor
#   5  resolver                               el valor, y con qué rol
#   6  ⭐ resolver un secreto AJENO            mismo error que uno inventado
#   7  la huella                              emitir y resolver, SIN el valor
#   8  ⭐ se abre con la llave con la que se CERRÓ, no con la de ahora
#   9  ⭐⭐ el material NO está en la base central (0024-⑤): está en el almacén
#        de la celda, bajo el prefijo del inquilino — y lo viejo se MUDA
#  10  ⭐ la baja (037): owner o `secreto:retirar`; fila con retirado_en, concesiones
#      revocadas, material fuera del almacén, huella sin el valor; el agente no puede
#  11  ⭐ la baja de operador (038): `retirar-huerfanos --declaradas` retira las de fuentes
#      que ya no están, atribuidas al operador; `--seco`; sin lista se niega
#
# ── ⭐ Y el almacén también es de mentira, por lo mismo ────────────────────
#
#   El aislamiento por prefijo —que el cofre de `demo` no pueda tocar lo de
#   `prueba`— está probado contra Google en `medida-el-almacen-por-inquilino.py`,
#   desde dentro del pod y con la condición IAM de verdad. Lo que esto prueba es
#   que el cofre USA el almacén como debe: el nombre lleva el inquilino delante,
#   nace con la CMEK de la organización, la versión es la que el almacén
#   devuelve, y `cofre.material` se queda vacía.
#
#   Desde 0046 E9·3 el cofre habla con el almacén por su API REST (no por
#   `gcloud secrets`): el de mentira es un servidor HTTP con las MISMAS rutas y
#   los mismos códigos (409 al crear lo que hay, 404 al borrar lo que no), y el
#   cofre lo encuentra por `ORE_SECRETOS_API`.
#
#   PG_URL=postgres://postgres:x@localhost:5432 \
#   PGHOST=localhost PGUSER=postgres PGPASSWORD=x \
#     bash pruebas-de-fuego/el-cofre.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO_COFRE:-8905}"
BASE="http://127.0.0.1:$PUERTO"
PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432}"
TMP="$(mktemp -d)"
SRV=""
ALM=""

falla() {
  echo "✗ $*" >&2
  if [ -s "$TMP/arranque.txt" ]; then
    echo "── lo que dijo el custodio ──────────────────────────────" >&2
    tail -20 "$TMP/arranque.txt" >&2
  fi
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  [ -n "$ALM" ] && kill "$ALM" 2>/dev/null
  exit 1
}
dice() { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; [ -n "$ALM" ] && kill "$ALM" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/debug/$1"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
IAM="$(buscar ore-iam)"   || falla "no hay binario de \`ore-iam\`"
COFRE="$(buscar ore-cofre)" || falla "no hay binario de \`ore-cofre\`"
PY=$(command -v python3 || command -v python) || falla "hace falta python"
command -v psql >/dev/null || falla "hace falta psql"

EMISOR="https://login.paladio.io/realms/rubix"
AUDIENCIA="ore-serve"

# ── La base ─────────────────────────────────────────────────────────────────
psql "$PG_URL/postgres" -qtAc "drop database if exists cofre_prueba" >/dev/null 2>&1
psql "$PG_URL/postgres" -qtAc "create database cofre_prueba" >/dev/null 2>&1 \
  || falla "no se pudo crear la base de prueba"
URL="$PG_URL/cofre_prueba"

PGDATABASE=cofre_prueba bash "$RAIZ/iam/migrar.sh" > "$TMP/migrar.txt" 2>&1 \
  || falla "las migraciones fallaron: $(tail -5 "$TMP/migrar.txt")"
dice "$(grep -c '^·' "$TMP/migrar.txt" || echo 0) migraciones aplicadas"

# ── Los dos usuarios acotados, que es lo que la `020` reparte ────────────────
CLAVE="prueba-no-secreta"
for U in iam_app_c cofre_app_c; do
  psql "$URL" -qtAc "do \$\$ begin
      if not exists (select 1 from pg_roles where rolname='$U') then
        create user $U login password '$CLAVE';
      end if;
    end \$\$" >/dev/null 2>&1 || falla "no se pudo crear \`$U\`"
done
psql "$URL" -qtAc "grant ore_iam   to iam_app_c"   >/dev/null 2>&1 || falla "papel \`ore_iam\`"
psql "$URL" -qtAc "grant ore_cofre to cofre_app_c" >/dev/null 2>&1 || falla "papel \`ore_cofre\`"

SERVIDOR="${PG_URL#postgres://}"; SERVIDOR="${SERVIDOR#*@}"
URL_IAM="postgres://iam_app_c:$CLAVE@$SERVIDOR/cofre_prueba"
URL_COFRE="postgres://cofre_app_c:$CLAVE@$SERVIDOR/cofre_prueba"

# ⭐ Y la propiedad de la `020`, comprobada aqui tambien y desde el otro lado:
#   el custodio NO alcanza lo que no es suyo de `iam`.
if psql "$URL_COFRE" -qtAc "select 1 from iam.invitacion" >/dev/null 2>&1; then
  falla "⛔ EL CUSTODIO ALCANZA \`iam.invitacion\`. Solo debe leer lo justo para autorizar"
fi
dice "los dos papeles en vigor: el custodio no alcanza \`iam.invitacion\`"

# ── La casa de la moneda ────────────────────────────────────────────────────
cat > "$TMP/acunar.py" <<'PYCODE'
# -*- coding: utf-8 -*-
import base64, hashlib, json, sys

N = 98108802788390257451427544537569571344186445898290802556306202303563898746153694624427727358963513671154354914513738651625770608698713459103692963416659567693829394560290454767975140775489577446428399646018299491037316206274830556376268979724700474004190230381644330590836431547957441542425447960647896094871
D = 58486241606109815346595400118075370902635461720864906313568320457114881061285666040279031389708798321838495691825034032384259760307155288367215166817606418492512751565372832090822858982082061256407746822196621146620704044182961301819536667603341097088576106781350305670874531525425729267744540582701760709001
E = 65537
K = (N.bit_length() + 7) // 8
PREFIJO = bytes.fromhex("3031300d060960864801650304020105000420")


def b64(b):
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


def firmar(m):
    t = PREFIJO + hashlib.sha256(m).digest()
    em = b"\x00\x01" + b"\xff" * (K - len(t) - 3) + b"\x00" + t
    return pow(int.from_bytes(em, "big"), D, N).to_bytes(K, "big")


if sys.argv[1] == "jwks":
    print(json.dumps({"keys": [{
        "kty": "RSA", "use": "sig", "kid": "k1", "alg": "RS256",
        "n": b64(N.to_bytes(K, "big")), "e": b64(E.to_bytes(3, "big")),
    }]}))
    raise SystemExit(0)

sub, correo, emisor, audiencia, ahora = sys.argv[1:6]
cabeza = {"alg": "RS256", "typ": "JWT", "kid": "k1"}
cuerpo = {"iss": emisor, "aud": audiencia, "sub": sub, "email": correo,
          "name": sub.split(":")[-1].capitalize() + " (prueba)",
          "exp": int(ahora) + 300, "iat": int(ahora)}
f = (b64(json.dumps(cabeza).encode()) + "." + b64(json.dumps(cuerpo).encode())).encode()
print(f.decode() + "." + b64(firmar(f)))
PYCODE

"$PY" "$TMP/acunar.py" jwks > "$TMP/jwks.json" || falla "no se pudo escribir el JWKS"
AHORA=$(date +%s)
acunar() { "$PY" "$TMP/acunar.py" "$1" "$2" "$EMISOR" "$AUDIENCIA" "$AHORA"; }

# ── ⭐ EL CLIENTE DE MENTIRA, y distingue llaves ────────────────────────────
#
# Recibe los MISMOS argumentos que `gcloud kms` y habla por la entrada y la
# salida estandar, como el de verdad. Lo que cifra lleva DENTRO el nombre de la
# llave, asi que descifrar con otra falla — y falla diciendo `PERMISSION_DENIED`,
# que es lo que diria Google. (Solo lo usa `mudar`.)
cat > "$TMP/kms-de-mentira" <<'KMSCODE'
#!/usr/bin/env python3
import base64, os, sys

a = sys.argv[1:]
def opt(n):
    for i, x in enumerate(a):
        if x == n:
            return a[i + 1]
        if x.startswith(n + "="):
            return x.split("=", 1)[1]
    return None

# ── el KMS ──────────────────────────────────────────────────────────────────
verbo = a[1]
llave = "%s/%s" % (opt("--keyring"), opt("--key"))
dato = sys.stdin.buffer.read()

if verbo == "encrypt":
    sys.stdout.buffer.write(b"KMS:" + llave.encode() + b":" + base64.b64encode(dato))
elif verbo == "decrypt":
    if not dato.startswith(b"KMS:"):
        sys.stderr.write("ERROR: no parece cifrado por este KMS\n")
        raise SystemExit(1)
    _, cual, resto = dato.split(b":", 2)
    if cual.decode() != llave:
        sys.stderr.write(
            "ERROR: PERMISSION_DENIED: se cerro con `%s` y se pidio abrir con `%s`\n"
            % (cual.decode(), llave))
        raise SystemExit(1)
    sys.stdout.buffer.write(base64.b64decode(resto))
else:
    sys.stderr.write("ERROR: verbo desconocido %s\n" % verbo)
    raise SystemExit(2)
KMSCODE
chmod +x "$TMP/kms-de-mentira"

# ── ⭐ EL ALMACEN DE MENTIRA: Secret Manager por HTTP ──────────────────────
#
# Las rutas de la API v1 que el cofre usa, sobre un directorio: un secreto es
# una carpeta (con `cmek`, el cuerpo con que nacio), cada version un fichero
# numerado. Exige un `Bearer`, como Google.
cat > "$TMP/almacen-de-mentira" <<'ALMCODE'
#!/usr/bin/env python3
import base64, json, os, re, shutil, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse, parse_qs

RAIZ = os.environ["ALMACEN_DE_MENTIRA"]

class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def dar(self, c, cuerpo):
        b = json.dumps(cuerpo).encode()
        self.send_response(c)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def error(self, c, estado, m):
        self.dar(c, {"error": {"code": c, "message": m, "status": estado}})
    def cuerpo(self):
        n = int(self.headers.get("content-length") or 0)
        return self.rfile.read(n) if n else b""
    def atender(self, metodo):
        if not (self.headers.get("authorization") or "").startswith("Bearer "):
            return self.error(401, "UNAUTHENTICATED", "sin token")
        u = urlparse(self.path)
        m = re.fullmatch(r"/v1/projects/([^/]+)/secrets(?:/([A-Za-z0-9_-]+))?(/versions/latest:access|:addVersion)?", u.path)
        if not m:
            return self.error(404, "NOT_FOUND", "ruta desconocida %s" % u.path)
        proyecto, nombre, cola = m.groups()
        if metodo == "POST" and nombre is None:
            nombre = parse_qs(u.query).get("secretId", [""])[0]
            d = os.path.join(RAIZ, nombre)
            if os.path.isdir(d):
                return self.error(409, "ALREADY_EXISTS", "Secret [%s] already exists." % nombre)
            os.makedirs(d)
            open(os.path.join(d, "cmek"), "wb").write(self.cuerpo())
            return self.dar(200, {"name": "projects/%s/secrets/%s" % (proyecto, nombre)})
        d = os.path.join(RAIZ, nombre or "")
        if not nombre or not os.path.isdir(d):
            return self.error(404, "NOT_FOUND", "Secret [%s] not found." % nombre)
        if metodo == "DELETE" and cola is None:
            shutil.rmtree(d)
            return self.dar(200, {})
        vs = sorted(int(x) for x in os.listdir(d) if x.isdigit())
        if metodo == "POST" and cola == ":addVersion":
            dato = base64.b64decode(json.loads(self.cuerpo())["payload"]["data"])
            n = 1 + len(vs)
            open(os.path.join(d, str(n)), "wb").write(dato)
            return self.dar(200, {"name": "projects/%s/secrets/%s/versions/%d" % (proyecto, nombre, n)})
        if metodo == "GET" and cola == "/versions/latest:access":
            if not vs:
                return self.error(404, "NOT_FOUND", "Secret [%s] has no versions." % nombre)
            v = vs[-1]
            dato = open(os.path.join(d, str(v)), "rb").read()
            return self.dar(200, {"name": "projects/%s/secrets/%s/versions/%d" % (proyecto, nombre, v),
                                  "payload": {"data": base64.b64encode(dato).decode()}})
        return self.error(400, "INVALID_ARGUMENT", "%s %s" % (metodo, u.path))
    def do_GET(self):
        self.atender("GET")
    def do_POST(self):
        self.atender("POST")
    def do_DELETE(self):
        self.atender("DELETE")

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
ALMCODE

# ── Dos organizaciones: la de Ada, y la de Zoe para el 6 ────────────────────
export IAM_URL="$URL_IAM"
# ⭐ CON celda desde la 029: un secreto es de una celda (0025-4), y el cofre de
#   hoy no la dice — el disparador de la 029 pone la UNICA que tenga la
#   organizacion. Sin celda, emitir se negaria, y con razon.
"$IAM" fundar --organizacion acme --emisor "$EMISOR" --sub "persona:ada" \
  --correo "ada@paladio.io" \
  --celda ore-prueba --tier compartido --proveedor gcp --region europe-west1-b --puerta ore-prueba.ore.paladio.io \
  > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar acme\` fallo: $(tail -3 "$TMP/fundar.txt")"
"$IAM" fundar --organizacion otra --emisor "$EMISOR" --sub "persona:zoe" \
  --correo "zoe@paladio.io" > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar otra\` fallo: $(tail -3 "$TMP/fundar.txt")"
ORG=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='acme'")
[ -n "$ORG" ] || falla "no se encontro la organizacion"
dice "dos organizaciones, con su llave: $(psql "$URL" -qtAc "select string_agg(kek, ' ') from iam.organizacion")"

ADA=$(acunar "persona:ada" "ada@paladio.io")
ZOE=$(acunar "persona:zoe" "zoe@paladio.io")

# ── ⭐⭐ Cada custodio, el login de SU celda (0047 A7a, 040 y 041) ────────────
#
# Hasta la A7a los custodios entraban con un login compartido, y con el cada
# uno leia lo de todos. Desde la 041 un login de `ore_cofre` ve las filas de SU
# organizacion y ninguna mas, y la organizacion la dice el login: el papel de la
# celda, que da `iam.dar_papel_de_celda` (solo el aprovisionador). Un login sin
# celda —`cofre_app_c`, el de arriba— ya no veria nada, y el custodio de esta
# prueba no podria ni emitir: por eso corre con el de `acme`.
#
# `otra` tambien tiene celda, para que el bloque 12 pueda medir que un custodio
# no ve lo de la otra organizacion, y que no es solo que no haya nada que ver.
ORG_OTRA=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='otra'")
psql "$URL" -v ON_ERROR_STOP=1 -qtAc "insert into iam.celda
    (id, organizacion, nombre, tier, proveedor, region, cluster, puerta, arbol, entrada)
  values ('cel_otra', '$ORG_OTRA', 'otra', 'compartido', 'gcp', 'europe-west1-b',
          'ore-prueba', 'ore-prueba.ore.paladio.io', 'otra/arbol', 'otra.ore.paladio.io')" \
  >/dev/null 2>&1 || falla "no se pudo dar celda a \`otra\`"
dar_papel() { psql "$URL" -qtAc "set role ore_aprovisionador; select iam.dar_papel_de_celda('$1')" 2>/dev/null | tr -d ' \n'; }
PAPEL_ACME=$(dar_papel acme)
PAPEL_OTRA=$(dar_papel otra)
[ "${PAPEL_ACME%%:*}" = "cofre_acme" ] && [ "${PAPEL_OTRA%%:*}" = "cofre_otra" ] \
  || falla "no se dieron los papeles de las celdas"
URL_COFRE="postgres://$PAPEL_ACME@$SERVIDOR/cofre_prueba"
URL_OTRA="postgres://$PAPEL_OTRA@$SERVIDOR/cofre_prueba"
dice "cada celda, su login: el custodio corre como \`cofre_acme\`"

# ── El custodio ─────────────────────────────────────────────────────────────
mkdir -p "$TMP/almacen"
export ALMACEN_DE_MENTIRA="$TMP/almacen"
PUERTO_ALMACEN="${PUERTO_ALMACEN:-8906}"
python3 "$TMP/almacen-de-mentira" "$PUERTO_ALMACEN" > "$TMP/almacen.txt" 2>&1 &
ALM=$!
export ORE_SECRETOS_API="http://127.0.0.1:$PUERTO_ALMACEN/v1" ORE_GCP_TOKEN="de-mentira"
for _ in $(seq 1 50); do
  curl -s -o /dev/null "http://127.0.0.1:$PUERTO_ALMACEN/" && break
  sleep 0.1
done
COFRE_URL="$URL_COFRE" "$COFRE" servir --bind "127.0.0.1:$PUERTO" \
  --identidad oidc --emisor "$EMISOR" --audiencia "$AUDIENCIA" \
  --jwks "$TMP/jwks.json" --celda acme --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 \
  > "$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 60); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/salud" >/dev/null || falla "el custodio no arranco"

pide() {
  if [ $# -ge 4 ]; then
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" \
      -H "Authorization: Bearer $3" -H 'Content-Type: application/json' \
      -d "$4" "$BASE$2"
  else
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" \
      -H "Authorization: Bearer $3" "$BASE$2"
  fi
}
campo() { "$PY" -c "import json,sys;print(json.load(open(sys.argv[1])).get(sys.argv[2],''))" "$TMP/r.json" "$1"; }

# ── 1 · sin token ───────────────────────────────────────────────────────────
[ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/organizaciones/$ORG/secretos")" = "401" ] \
  || falla "1 · sin token no dio 401"
dice "1 · sin token · 401"

# ── 2 · emitir sin la potestad ──────────────────────────────────────────────
#
# Zoe es dueña de `otra` y no pinta nada en `acme`. Emitir es una POTESTAD de la
# organizacion, asi que aqui muerde `exige` — y su mensaje no distingue «no
# perteneces» de «no puedes».
[ "$(pide POST "/organizaciones/$ORG/secretos" "$ZOE" \
      '{"nombre":"colado","clase":"contrasena","valor":"x"}')" = "422" ] \
  || falla "2 · ⛔ UNA EXTRAÑA EMITIO UN SECRETO EN OTRA ORGANIZACION"
dice '2 · emitir sin `secreto:emitir` se niega'

# ── 3 · emitir ──────────────────────────────────────────────────────────────
CODIGO=$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
  '{"nombre":"pg-produccion","clase":"conexion","valor":"postgres://u:p@db/x"}')
[ "$CODIGO" = "200" ] || falla "3 · no se pudo emitir · http $CODIGO · $(cat "$TMP/r.json")"
# ⛔ Y la respuesta NO trae el valor. Quien lo acaba de escribir ya lo tiene;
#   devolverlo seria una segunda copia por un camino que se audita distinto.
grep -q "postgres://u:p@db/x" "$TMP/r.json" \
  && falla "3 · ⛔ EMITIR DEVUELVE EL VALOR. Sale por una puerta que no deja la misma huella"
[ -n "$(campo concesion)" ] || falla "3 · no nacio con su concesion de \`owner\`"
dice "3 · emitido · con su concesion $(campo concesion) · y sin devolver el valor"

# ── 4 · listar ──────────────────────────────────────────────────────────────
[ "$(pide GET "/organizaciones/$ORG/secretos" "$ADA")" = "200" ] || falla "4 · no listo: $(cat "$TMP/r.json")"
grep -q '"pg-produccion"' "$TMP/r.json" || falla "4 · no sale el que acaba de emitir: $(cat "$TMP/r.json")"
grep -q "postgres://u:p@db/x" "$TMP/r.json" \
  && falla "4 · ⛔ LISTAR DEVUELVE VALORES. Listar es ver que hay, no que dice"
dice "4 · listado · nombres y ni un valor"

# ── 5 · resolver ────────────────────────────────────────────────────────────
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] \
  || falla "5 · no resolvio: $(cat "$TMP/r.json")"
[ "$(campo valor)" = "postgres://u:p@db/x" ] \
  || falla "5 · lo que volvio NO es lo que entro: $(campo valor)"
[ "$(campo rol)" = "owner" ] || falla "5 · deberia resolver como \`owner\`: $(campo rol)"
dice "5 · resuelto · el mismo valor que entro · con rol \`$(campo rol)\`"

# ── 5c · ⭐⭐ UN AGENTE QUE LLEGA DESPUES HEREDA `usar` ──────────────────────
#
# El custodio concede `usar` a los agentes de la organizacion AL EMITIR. Un
# agente registrado despues —un cliente de Keycloak por inquilino, que es lo
# que la 024 dejo pedido— no estaba en esa lista, y su primer Job moriria con
# un 403 sobre `pg-produccion`, que ya existia. `ore-iam agente` copia ahora las
# concesiones vivas de sus hermanos al registrarse.
#
# `maquina:uno` llega DESPUES del secreto; `maquina:dos` llega ANTES del
# segundo. Los dos tienen que poder resolver los dos.
"$IAM" agente --organizacion acme --emisor "$EMISOR" --sub "maquina:uno" --nombre "uno" \
  > "$TMP/agente.txt" 2>&1 || falla "5c · no se registro el agente: $(cat "$TMP/agente.txt")"
grep -q '"secretos_heredados": 1' "$TMP/agente.txt" \
  || falla "5c · el agente no heredo el secreto que ya habia: $(cat "$TMP/agente.txt")"
UNO=$(acunar "maquina:uno" "")
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$UNO")" = "200" ] \
  || falla "5c · el agente tardio NO resuelve lo emitido antes: $(cat "$TMP/r.json")"
[ "$(campo rol)" = "usar" ] || falla "5c · deberia resolver como \`usar\`: $(campo rol)"

"$IAM" agente --organizacion acme --emisor "$EMISOR" --sub "maquina:dos" --nombre "dos" \
  > "$TMP/agente.txt" 2>&1 || falla "5c · no se registro el segundo agente"
CODIGO=$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
  '{"nombre":"pg-lectura","clase":"conexion","valor":"postgres://r:o@db/x"}')
[ "$CODIGO" = "200" ] || falla "5c · no se pudo emitir el segundo · $CODIGO"
DOS=$(acunar "maquina:dos" "")
for A in "$UNO" "$DOS"; do
  [ "$(pide GET "/organizaciones/$ORG/secretos/pg-lectura" "$A")" = "200" ] \
    || falla "5c · un agente no resuelve el secreto emitido con los dos registrados: $(cat "$TMP/r.json")"
done
# ⛔ Y de otra organizacion, nada: el agente de `acme` no toca lo de `otra`.
OTRA=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='otra'")
[ "$(pide GET "/organizaciones/$OTRA/secretos/pg-produccion" "$UNO")" != "200" ] \
  || falla "5c · ⛔ EL AGENTE DE ACME RESUELVE EN OTRA"
dice "5c · el agente tardio hereda \`usar\`, el temprano lo recibe al emitir, y ninguno cruza"

# ── 5b · ⭐⭐ Y POR NOMBRE, QUE ES COMO LLAMAN LOS CLIENTES ──────────────────
#
# ⛔ Esta comprobacion faltaba, y su ausencia costo el primer secreto que este
#   custodio tenia que guardar de verdad. Todo el guion resolvia el id con SQL
#   —`select id from iam.organizacion where nombre='acme'`— asi que el camino
#   del NOMBRE no se ejercitaba nunca.
#
#   Y por ahi llaman los clientes: `ore-serve` arranca con `--organizacion demo`
#   y pedia `/organizaciones/demo/secretos`. El custodio comparaba `demo` contra
#   una columna que guarda `org_b7b98fdd…`, no encontraba nada, y contestaba
#   «no puedes hacer eso en esa organizacion» — un problema de UNIDADES
#   disfrazado de problema de permisos.
#
# ⇒ Las dos formas tienen que dar exactamente lo mismo. Si algun dia dejan de
#   darlo, se entera aqui y no un Job de catalogo tres horas despues.
[ "$(pide GET "/organizaciones/acme/secretos/pg-produccion" "$ADA")" = "200" ]   || falla "5b · por NOMBRE no resolvio: $(cat "$TMP/r.json")"
[ "$(campo valor)" = "postgres://u:p@db/x" ]   || falla "5b · por nombre volvio otra cosa: $(campo valor)"
[ "$(pide GET "/organizaciones/acme/secretos" "$ADA")" = "200" ]   || falla "5b · por NOMBRE no listo: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/acme/secretos" "$ADA"      '{"nombre":"por-nombre","clase":"contrasena","valor":"x"}')" = "200" ]   || falla "5b · por NOMBRE no emitio: $(cat "$TMP/r.json")"
# Y una que no existe sigue siendo un no, no un si por descuido.
[ "$(pide GET "/organizaciones/no-existe/secretos" "$ADA")" != "200" ]   || falla "5b · ⛔ una organizacion inventada contesto 200"
dice "5b · el nombre y el id llevan al mismo sitio, y lo inventado a ninguno"

# ── 6 · ⭐ EL DIRECTORIO DE LO AJENO ────────────────────────────────────────
#
# Si un secreto ajeno diera «no tienes acceso» y uno inventado «no existe»,
# cualquiera con una cuenta tendria un directorio de los secretos de los demas,
# consultable nombre a nombre. Es la misma sonda que la `0021` cazo en `revocar`.
#
# ✏️ 0047 A7a.4 · La sonda es para QUIEN PREGUNTA: lo que no puede distinguir
#   es un secreto que existe de uno que no. Esto comparaba a Zoe pidiendo uno
#   que existe con ADA pidiendo uno inventado, y valia mientras el custodio
#   veia a todas las personas. Desde la 041 no ve a quien no es de su
#   organizacion, y a Zoe le contesta «no eres de aqui» —lo mismo para uno que
#   existe que para uno inventado: no hay directorio—. Se mide eso.
pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ZOE" >/dev/null
AJENO=$(campo error)
pide GET "/organizaciones/$ORG/secretos/no-existe-nada" "$ZOE" >/dev/null
INVENTADO=$(campo error)
[ -n "$AJENO" ] && [ "$AJENO" = "$INVENTADO" ] \
  || falla "6 · ⛔ SONDA: a Zoe, uno ajeno da «$AJENO» y uno inventado «$INVENTADO»"
dice "6 · a quien no pertenece, un secreto que existe y uno inventado le dan el MISMO error"

# ── 7 · la huella ───────────────────────────────────────────────────────────
EMITIR=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion='secreto:emitir'")
ABRIR=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion='secreto:resolver'")
[ "$EMITIR" -ge 1 ] && [ "$ABRIR" -ge 1 ] \
  || falla "7 · falta huella (emitir=$EMITIR resolver=$ABRIR)"
# ⛔⛔ Y NI UN VALOR DENTRO. Una huella con el secreto dentro es el secreto en
#   una tabla que todo el mundo lee por otro nombre.
FUGA=$(psql "$URL" -qtAc "select count(*) from iam.huella where detalle::text like '%postgres://u:p@db%'")
[ "$FUGA" = "0" ] || falla "7 · ⛔⛔ EL VALOR ESTA EN LA HUELLA"
ROL=$(psql "$URL" -qtAc "select detalle->>'rol' from iam.huella where operacion='secreto:resolver' limit 1")
[ "$ROL" = "owner" ] || falla "7 · la huella no dice con que rol se abrio: «$ROL»"
dice "7 · huella de emitir y de resolver · con el rol · y sin el valor"

# ── 8 · ⭐ se abre con la llave con la que se CERRÓ ─────────────────────────
#
# La organizacion cambia de llave maestra. Lo que ya estaba cerrado tiene que
# seguir abriendose: cada version del almacen quedo cifrada con la CMEK que
# habia, y rotar la de la organizacion no la toca. Aqui el cliente de mentira
# no cifra el almacen, asi que lo que esto fija es que el codigo NO necesita la
# llave de ahora para leer — que es la propiedad.
psql "$URL" -qtAc "update iam.organizacion set kek = 'ore/acme-nueva' where id = '$ORG'" >/dev/null
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] \
  || falla "8 · ⛔ tras cambiar la llave de la organizacion, lo viejo dejo de abrirse: $(cat "$TMP/r.json")"
[ "$(campo valor)" = "postgres://u:p@db/x" ] || falla "8 · abrio, pero devolvio otra cosa"
dice "8 · la llave cambio y lo cerrado antes sigue abriendose"

# ── 9 · ⭐⭐ EL MATERIAL NO ESTA EN LA BASE CENTRAL, Y LO VIEJO SE MUDA ─────
#
# La 0024-⑤: el metadato (`cofre.secreto`, `iam.concesion`) se queda en el plano
# de control; el material va al almacen de la celda, bajo el prefijo del
# inquilino. Lo que se cobra:
#   · `cofre.material` esta VACIA tras emitir
#   · el almacen tiene `t-acme-cofre-pg-produccion` con la CMEK de la llave
#   · la huella de emitir dice donde quedo y que version
#   · y un secreto viejo —cifrado a mano en `cofre.material`— lo lleva `mudar`
#     al almacen, abriendolo con la llave con la que se cerro, y borra la fila
# En una base recien migrada la 028 ya borro `cofre.material`: no existir ES la
# prueba de que el material no esta aqui.
[ "$(psql "$URL" -qtAc "select to_regclass('cofre.material') is null")" = "t" ] \
  || falla "9 · ⛔ \`cofre.material\` sigue existiendo tras la 028"
[ -f "$TMP/almacen/t-acme-cofre-pg-produccion/1" ] \
  || falla "9 · el almacen no tiene \`t-acme-cofre-pg-produccion\` v1: $(ls "$TMP/almacen")"
grep -q "keyRings/ore/cryptoKeys/acme" "$TMP/almacen/t-acme-cofre-pg-produccion/cmek" \
  || falla "9 · el secreto no nacio con la CMEK de la organizacion: $(cat "$TMP/almacen/t-acme-cofre-pg-produccion/cmek")"
DONDE=$(psql "$URL" -qtAc "select detalle->>'almacen' from iam.huella where operacion='secreto:emitir' and detalle->>'nombre'='pg-produccion'")
[ "$DONDE" = "t-acme-cofre-pg-produccion" ] || falla "9 · la huella de emitir no dice donde quedo: «$DONDE»"

# Sin tabla, `mudar` no muere: dice que no hay nada que mudar.
COFRE_URL="$URL_COFRE" "$COFRE" mudar --organizacion acme \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/mudar.txt" 2>&1 \
  || falla "9 · \`mudar\` sin tabla deberia terminar bien: $(cat "$TMP/mudar.txt")"
grep -q "0 secretos mudados" "$TMP/mudar.txt" || falla "9 · mudar sin tabla no dijo 0: $(cat "$TMP/mudar.txt")"

# ⭐ Y UNA BASE COMO LA DE ANTES: se recrea `cofre.material` y `cofre.vigente` tal
#   como las dejaron la 020 y la 021, para probar la mudanza de verdad — es la
#   situacion de todo inquilino que tenia secretos el 2026-09-14.
psql "$URL" -qtAc "create table cofre.material (
  secreto text not null references cofre.secreto(id) on delete cascade,
  version integer not null check (version > 0), cifrado bytea not null,
  kek text not null, en timestamptz not null default now(), primary key (secreto, version));
create view cofre.vigente as select m.* from cofre.material m
  join (select secreto, max(version) as version from cofre.material group by secreto) u
    on u.secreto = m.secreto and u.version = m.version;
grant select, insert, update, delete on cofre.material, cofre.vigente to ore_cofre;" >/dev/null \
  || falla "9 · no se pudo recrear la tabla vieja"

# Un secreto VIEJO: como los guardaba el cofre hasta la 0024-⑤, cifrado a mano
# con la llave de entonces (`ore/acme`, no la de ahora), en `cofre.material`.
VIEJO=$(printf 'postgres://viejo:v@db/y' | "$TMP/kms-de-mentira" kms encrypt --keyring ore --key acme --plaintext-file=- --ciphertext-file=- | base64 -w0)
# Con su celda (030: `celda not null`). Un secreto viejo de verdad la tendria desde
# la 029, que rellenó todos con la unica celda de su organizacion.
psql "$URL" -qtAc "insert into cofre.secreto (id, organizacion, nombre, clase, emitio, celda)
  values ('sec_viejo', '$ORG', 'pg-viejo', 'conexion', (select id from iam.persona where sub='persona:ada'),
          (select id from iam.celda where organizacion = '$ORG'))" >/dev/null
psql "$URL" -qtAc "insert into cofre.material (secreto, version, cifrado, kek)
  values ('sec_viejo', 1, decode('$VIEJO', 'base64'), 'ore/acme')" >/dev/null
psql "$URL" -qtAc "select iam.conceder_de_secreto('con_viejo', (select id from iam.persona where sub='persona:ada'), 'secreto/pg-viejo', 'owner', (select id from iam.persona where sub='persona:ada'), '$ORG')" >/dev/null
# Antes de mudar, resolver falla: el almacen no lo tiene, y el cofre NO mira la base.
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-viejo" "$ADA")" = "422" ] \
  || falla "9 · un secreto sin mudar deberia fallar al resolver, no contestar: $(cat "$TMP/r.json")"
COFRE_URL="$URL_COFRE" "$COFRE" mudar --organizacion acme \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/mudar.txt" 2>&1 \
  || falla "9 · \`mudar\` fallo: $(cat "$TMP/mudar.txt")"
grep -q "1 secretos mudados" "$TMP/mudar.txt" || falla "9 · mudar no dijo cuantos: $(cat "$TMP/mudar.txt")"
[ "$(psql "$URL" -qtAc "select count(*) from cofre.material")" = "0" ] || falla "9 · mudar no borro la fila de cofre.material"
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-viejo" "$ADA")" = "200" ] \
  || falla "9 · tras mudar no resuelve: $(cat "$TMP/r.json")"
[ "$(campo valor)" = "postgres://viejo:v@db/y" ] || falla "9 · mudo otra cosa: $(campo valor)"
# Y repetir es seguro: sin material, no hay nada que mudar y no se confirma nada.
COFRE_URL="$URL_COFRE" "$COFRE" mudar --organizacion acme \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/mudar.txt" 2>&1 \
  || falla "9 · la segunda pasada de \`mudar\` fallo: $(cat "$TMP/mudar.txt")"
grep -q "0 secretos mudados" "$TMP/mudar.txt" || falla "9 · la segunda pasada deberia mudar 0: $(cat "$TMP/mudar.txt")"
dice "9 · el material esta en el almacen y no en la base · con la CMEK de la organizacion · y lo viejo se muda una vez"

# ── 10 · la baja (037): retirar un secreto ───────────────────────────────────
#
# Medido el 2026-09-18: 19 fuentes retiradas en `demo` dejaron 19 credenciales
# vivas en el custodio, porque no habia verbo. Ahora lo hay, y lo que fija:
#   · Zoe (de `otra`, sin potestad ni concesion) NO puede, y el error es el
#     mismo que «no existe» — no se destapa lo ajeno
#   · Ada, `owner` por haberlo emitido, retira: 200, la fila queda con
#     `retirado_en` y quien (no un delete), las concesiones revocadas con fecha,
#     el material FUERA del almacen, y la huella `secreto:retirar` SIN el valor
#   · despues, resolver dice «no existe o no es tuyo», y retirar otra vez igual
#   · el agente (`usar`) no puede retirar: un agente no decide que algo deje
#     de existir
[ "$(pide DELETE "/organizaciones/$ORG/secretos/pg-produccion" "$ZOE")" = "422" ] \
  || falla "10 · Zoe retiro un secreto ajeno: $(cat "$TMP/r.json")"
# ✏️ 0047 A7a.4 · Como en el 6: desde la 041 el custodio de `acme` no ve a Zoe,
#   y le dice que no la conoce. Lo que no puede destaparse es si el secreto
#   existe: el mismo error para uno que existe y para uno inventado.
ZOE_EXISTE=$(campo error)
[ "$(pide DELETE "/organizaciones/$ORG/secretos/no-existe-nada" "$ZOE")" = "422" ] \
  || falla "10 · retirar uno inventado no dio 422: $(cat "$TMP/r.json")"
[ -n "$ZOE_EXISTE" ] && [ "$ZOE_EXISTE" = "$(campo error)" ] \
  || falla "10 · el error de Zoe destapa algo: «$ZOE_EXISTE» frente a «$(campo error)»"
[ "$(pide DELETE "/organizaciones/$ORG/secretos/pg-produccion" "$UNO")" = "422" ] \
  || falla "10 · el agente retiro un secreto: $(cat "$TMP/r.json")"
[ -d "$TMP/almacen/t-acme-cofre-pg-produccion" ] || falla "10 · algo borro el material antes de tiempo"
[ "$(pide DELETE "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] \
  || falla "10 · Ada no pudo retirar el suyo: $(cat "$TMP/r.json")"
[ "$(campo retirado)" = "True" ] || falla "10 · la respuesta no dice retirado: $(cat "$TMP/r.json")"
[ "$(campo concesiones_revocadas)" -ge 2 ] || falla "10 · no revoco owner y usar: $(cat "$TMP/r.json")"
[ ! -d "$TMP/almacen/t-acme-cofre-pg-produccion" ] || falla "10 · el material sigue en el almacen: $(ls "$TMP/almacen")"
FILA=$(psql "$URL" -qtAc "select (retirado_en is not null) and (retiro is not null) from cofre.secreto where nombre='pg-produccion' and organizacion='$ORG'")
[ "$FILA" = "t" ] || falla "10 · la fila no quedo con retirado_en y retiro: $FILA"
VIVAS=$(psql "$URL" -qtAc "select count(*) from iam.concesion_viva where recurso='secreto/pg-produccion' and organizacion='$ORG'")
[ "$VIVAS" = "0" ] || falla "10 · quedan concesiones vivas sobre el secreto retirado: $VIVAS"
REVOCADAS=$(psql "$URL" -qtAc "select count(*) from iam.concesion where recurso='secreto/pg-produccion' and organizacion='$ORG' and revocada_en is not null and revoco is not null")
[ "$REVOCADAS" -ge 2 ] || falla "10 · las concesiones no quedaron revocadas con fecha y quien: $REVOCADAS"
HUELLA=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion='secreto:retirar' and detalle->>'nombre'='pg-produccion' and detalle->>'como'='owner'")
[ "$HUELLA" = "1" ] || falla "10 · falta la huella secreto:retirar como owner: $HUELLA"
FUGA=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion='secreto:retirar' and detalle::text like '%postgres://%'")
[ "$FUGA" = "0" ] || falla "10 · la huella de la baja lleva el valor dentro"
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "422" ] \
  || falla "10 · retirado y se sigue resolviendo: $(cat "$TMP/r.json")"
[ "$(pide DELETE "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "422" ] \
  || falla "10 · retirar dos veces no dio el mismo error: $(cat "$TMP/r.json")"
# Y con la POTESTAD, sin ser owner: Ada emite otro, y quien tiene
# `secreto:retirar` en `acme` sin haberlo emitido... es Ada misma (ORGADMIN);
# se comprueba la potestad en la tabla, que es lo que la 037 añade.
POT=$(psql "$URL" -qtAc "select string_agg(rol, ' ' order by rol) from iam.rol_potestad where potestad='secreto:retirar'")
[ "$POT" = "ACCOUNTADMIN ORGADMIN SECURITYADMIN" ] || falla "10 · la potestad secreto:retirar no esta en los roles que emiten: $POT"
dice "10 · la baja: Zoe y el agente no pueden (mismo error que «no existe») · Ada, owner, retira: fila con retirado_en, concesiones revocadas con fecha, material fuera del almacen, huella sin el valor · despues no resuelve ni se retira dos veces"

# ── 10b · ⭐⭐ EL NOMBRE LO OCUPA EL VIVO, NO EL RETIRADO (043) ──────────────
#
# Hasta la `043` el índice único era `(celda, nombre)` a secas, así que un nombre
# retirado quedaba ocupado para siempre: dar de baja un origen y volver a darlo
# de alta con el mismo nombre fallaba aquí con el texto de Postgres, un 422, y
# `ore-serve` lo pintaba como un 502 que además mentía.
[ "$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
  '{"nombre":"pg-produccion","clase":"conexion","valor":"postgres://u:p@db/segunda"}')" = "200" ] \
  || falla "10b · no se pudo volver a emitir un nombre retirado: $(cat "$TMP/r.json")"
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] \
  || falla "10b · el nuevo no resuelve: $(cat "$TMP/r.json")"
grep -q "postgres://u:p@db/segunda" "$TMP/r.json" || falla "10b · resolvio otro valor que el nuevo"
FILAS=$(psql "$URL" -qtAc "select count(*) filter (where retirado_en is null) || '/' || count(*) from cofre.secreto where nombre='pg-produccion' and organizacion='$ORG'")
[ "$FILAS" = "1/2" ] || falla "10b · tenia que haber el retirado y el nuevo (vivos/total = 1/2): $FILAS"
[ "$(psql "$URL" -qtAc "select count(*) from iam.concesion_viva where recurso='secreto/pg-produccion' and organizacion='$ORG' and rol='owner'")" = "1" ] \
  || falla "10b · el nuevo no nacio con su owner (y solo el suyo)"
# Y sobre uno VIVO, 409 con una frase: es lo que `ore-serve` lee como «nombre ocupado».
[ "$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
  '{"nombre":"pg-produccion","clase":"conexion","valor":"postgres://u:p@db/tercera"}')" = "409" ] \
  || falla "10b · emitir sobre un vivo no dio 409: $(cat "$TMP/r.json")"
grep -q "ya hay un secreto vivo" "$TMP/r.json" || falla "10b · el 409 no lo dice con una frase: $(cat "$TMP/r.json")"
grep -q "duplicate key" "$TMP/r.json" && falla "10b · el 409 lleva el texto de Postgres"
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] && grep -q "db/segunda" "$TMP/r.json" \
  || falla "10b · el 409 toco el vivo: $(cat "$TMP/r.json")"
dice "10b · un nombre retirado se vuelve a emitir (el retirado se queda, 1 vivo de 2) · sobre uno vivo, 409 con una frase, y el vivo intacto"

# ── 11 · la baja de operador (038): las credenciales de fuentes que ya no estan ──
#
# Lo que quedo de antes de la 037 no lo alcanza `DELETE /fuentes`: su fuente ya
# no esta en ningun manifiesto. `ore-cofre retirar-huerfanos` corre EN el
# inquilino con la lista de fuentes declaradas HOY, y retira el resto — con su
# propia atribucion (`retiro_agente`, `revoco_agente`), no con una persona que
# no lo hizo. `--seco` dice que se iria sin tocar nada; sin `--declaradas` se niega.
[ "$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
      '{"nombre":"fuente-viva","clase":"conexion","valor":"postgres://v:v@db/viva"}')" = "200" ] \
  || falla "11 · no se pudo emitir fuente-viva: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/$ORG/secretos" "$ADA" \
      '{"nombre":"fuente-muerta","clase":"conexion","valor":"postgres://m:m@db/muerta"}')" = "200" ] \
  || falla "11 · no se pudo emitir fuente-muerta: $(cat "$TMP/r.json")"
COFRE_URL="$URL_COFRE" "$COFRE" retirar-huerfanos --organizacion acme \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/huerf.txt" 2>&1 \
  && falla "11 · sin --declaradas tenia que negarse (todo seria huerfano)"
grep -q "falta \`--declaradas" "$TMP/huerf.txt" || falla "11 · no dijo que falta --declaradas: $(cat "$TMP/huerf.txt")"
COFRE_URL="$URL_COFRE" "$COFRE" retirar-huerfanos --organizacion acme --declaradas viva --seco \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/huerf.txt" 2>&1 \
  || falla "11 · --seco fallo: $(cat "$TMP/huerf.txt")"
grep -q "fuente-muerta se iría" "$TMP/huerf.txt" || falla "11 · --seco no dijo que fuente-muerta se iria: $(cat "$TMP/huerf.txt")"
grep -q "fuente-viva se iría" "$TMP/huerf.txt" && falla "11 · --seco tomo la viva por huerfana"
[ -d "$TMP/almacen/t-acme-cofre-fuente-muerta" ] || falla "11 · --seco toco el almacen"
COFRE_URL="$URL_COFRE" "$COFRE" retirar-huerfanos --organizacion acme --declaradas viva \
  --kms "$TMP/kms-de-mentira" --proyecto proyecto-de-mentira --lugar europe-west1 > "$TMP/huerf.txt" 2>&1 \
  || falla "11 · retirar-huerfanos fallo: $(cat "$TMP/huerf.txt")"
grep -q "ok · 1 credencial(es) huérfana(s) retiradas" "$TMP/huerf.txt" || falla "11 · tenia que retirar exactamente 1: $(cat "$TMP/huerf.txt")"
[ ! -d "$TMP/almacen/t-acme-cofre-fuente-muerta" ] || falla "11 · el material de la muerta sigue en el almacen"
[ -d "$TMP/almacen/t-acme-cofre-fuente-viva" ] || falla "11 · se llevo el material de la viva"
FILA=$(psql "$URL" -qtAc "select (retirado_en is not null) and retiro is null and retiro_agente = 'ore-cofre retirar-huerfanos' from cofre.secreto where nombre='fuente-muerta' and organizacion='$ORG'")
[ "$FILA" = "t" ] || falla "11 · la fila no quedo atribuida al operador (retiro_agente) y sin persona: $FILA"
REV=$(psql "$URL" -qtAc "select count(*) from iam.concesion where recurso='secreto/fuente-muerta' and organizacion='$ORG' and revocada_en is not null and revoco is null and revoco_agente = 'ore-cofre retirar-huerfanos'")
[ "$REV" -ge 1 ] || falla "11 · las concesiones no quedaron revocadas por el operador: $REV"
HUELLA=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion='secreto:retirar' and detalle->>'nombre'='fuente-muerta' and detalle->>'como'='operador' and quien='operador'")
[ "$HUELLA" = "1" ] || falla "11 · falta la huella del operador: $HUELLA"
FUGA=$(psql "$URL" -qtAc "select count(*) from iam.huella where detalle::text like '%postgres://m:m@db%'")
[ "$FUGA" = "0" ] || falla "11 · la huella lleva el valor de la muerta"
[ "$(pide GET "/organizaciones/$ORG/secretos/fuente-viva" "$ADA")" = "200" ] || falla "11 · la viva dejo de resolver: $(cat "$TMP/r.json")"
[ "$(pide GET "/organizaciones/$ORG/secretos/fuente-muerta" "$ADA")" = "422" ] || falla "11 · la muerta sigue resolviendo"
dice "11 · retirar-huerfanos: sin --declaradas se niega · --seco dice que se iria · retira la de la fuente que no esta (fila y concesiones atribuidas al operador, material fuera, huella sin valor) y deja la viva"

# ── 12 · ⭐⭐ CADA CUSTODIO, SU ORGANIZACION (041, 0047 A7a.4) ────────────────
#
# Lo que la 041 existe para cortar, medido de los dos lados: `cofre_acme` no ve
# NADA de `otra`, y `cofre_otra` si ve lo suyo —si no, «no ve nada» podria ser
# solo que no habia nada—. Y los que no son custodios siguen como estaban.
#
# `otra` necesita algo que ver: un secreto y una concesion suyos, sembrados como
# superusuario (el custodio de esta prueba es el de `acme` y no podria).
ZID=$(psql "$URL" -qtAc "select id from iam.persona where sub='persona:zoe'")
psql "$URL" -v ON_ERROR_STOP=1 -qtAc "
  insert into cofre.secreto (id, organizacion, nombre, clase, emitio, celda)
    values ('sec_otra', '$ORG_OTRA', 'de-otra', 'conexion', '$ZID', 'cel_otra');
  insert into iam.concesion (id, sujeto, recurso, rol, concedio, organizacion)
    values ('con_otra', '$ZID', 'secreto/de-otra', 'owner', '$ZID', '$ORG_OTRA');" \
  >"$TMP/semilla-otra.txt" 2>&1 || falla "12 · no se sembro \`otra\`: $(tail -2 "$TMP/semilla-otra.txt")"

cuenta() { psql "$1" -qtAc "$2" 2>/dev/null | tr -d ' '; }
for Q in "select count(*) from cofre.secreto          where organizacion = '$ORG_OTRA'" \
         "select count(*) from iam.concesion_viva     where organizacion = '$ORG_OTRA'" \
         "select count(*) from iam.potestades_de_persona where organizacion = '$ORG_OTRA'" \
         "select count(*) from iam.celda              where organizacion = '$ORG_OTRA'" \
         "select count(*) from iam.organizacion       where id = '$ORG_OTRA'" \
         "select count(*) from iam.persona            where sub = 'persona:zoe'"; do
  [ "$(cuenta "$URL_COFRE" "$Q")" = "0" ] || falla "12 · ⛔ EL CUSTODIO DE \`acme\` VE LO DE \`otra\`: $Q"
  [ "$(cuenta "$URL_OTRA" "$Q")" -ge 1 ] 2>/dev/null || falla "12 · el custodio de \`otra\` no ve lo suyo: $Q"
done
# Y lo de `acme` lo sigue viendo el suyo (los bloques 3–11 lo usaron entero).
[ "$(cuenta "$URL_COFRE" "select count(*) from cofre.secreto where organizacion = '$ORG'")" -ge 1 ] \
  || falla "12 · el custodio de \`acme\` no ve sus secretos"

# Escribir por encima de las politicas: conceder y revocar en otra organizacion.
if psql "$URL_COFRE" -qtAc "select iam.conceder_de_secreto('con_x', '$ZID', 'secreto/de-otra', 'usar', '$ZID', '$ORG_OTRA')" >/dev/null 2>&1; then
  falla "12 · ⛔ EL CUSTODIO DE \`acme\` CONCEDIO EN \`otra\`"
fi
if psql "$URL_COFRE" -qtAc "select iam.revocar_de_secreto('secreto/de-otra', '$ORG_OTRA', '$ZID')" >/dev/null 2>&1; then
  falla "12 · ⛔ EL CUSTODIO DE \`acme\` REVOCO EN \`otra\`"
fi
if psql "$URL_COFRE" -qtAc "insert into cofre.secreto (id, organizacion, nombre, clase, emitio, celda) values ('sec_x', '$ORG_OTRA', 'x', 'conexion', '$ZID', 'cel_otra')" >/dev/null 2>&1; then
  falla "12 · ⛔ EL CUSTODIO DE \`acme\` ESCRIBIO UN SECRETO DE \`otra\`"
fi
[ "$(cuenta "$URL" "select count(*) from iam.concesion_viva where recurso = 'secreto/de-otra'")" = "1" ] \
  || falla "12 · la concesion de \`otra\` cambio"

# Por la puerta: Zoe, con el custodio de `acme`, no alcanza nada de `otra`.
[ "$(pide GET "/organizaciones/$ORG_OTRA/secretos/de-otra" "$ZOE")" != "200" ] \
  || falla "12 · ⛔ EL CUSTODIO DE \`acme\` RESOLVIO UN SECRETO DE \`otra\`"

# Los que no son custodios, como estaban.
[ "$(cuenta "$URL_IAM" "select count(*) from iam.organizacion where id in ('$ORG', '$ORG_OTRA')")" = "2" ] \
  || falla "12 · ⛔ \`ore-iam\` DEJO DE VER UNA ORGANIZACION"
[ "$(cuenta "$URL" "set role ore_aprovisionador; select count(*) from iam.organizacion where kek is not null")" -ge 2 ] \
  || falla "12 · ⛔ EL APROVISIONADOR DEJO DE LEER LAS LLAVES: la convergencia de todas las celdas caeria"
[ "$(cuenta "$URL" "set role ore_aprovisionador; select count(*) from iam.celda_de")" -ge 2 ] \
  || falla "12 · el aprovisionador dejo de leer las celdas"
# Y un login de `ore_cofre` sin celda no ve nada: es lo que habria pasado con
# el compartido si la 041 hubiera entrado antes que la A7a.3.
[ "$(cuenta "postgres://cofre_app_c:$CLAVE@$SERVIDOR/cofre_prueba" "select count(*) from iam.organizacion")" = "0" ] \
  || falla "12 · ⛔ UN CUSTODIO SIN CELDA VE ORGANIZACIONES"
dice "12 · cada custodio, su organizacion: no ve, no concede, no revoca y no escribe lo de otra; ore-iam y el aprovisionador, como estaban"

echo
# ── 13 · lo del custodio, en la actividad de su organizacion (0047 A6.1) ─────
[ "$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion like 'secreto:%' and quien <> 'operador' and (organizacion is null or celda is null or celda not like 'cel_%')")" = "0" ]   || falla "13 · ⛔ HAY SECRETOS SIN ORGANIZACION O SIN EL ID DE SU CELDA: $(psql "$URL" -qtAc "select operacion, organizacion, celda from iam.huella where operacion like 'secreto:%' and (organizacion is null or celda is null)")"
[ "$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion = 'secreto:resolver' and organizacion is not null")" -ge 1 ]   || falla "13 · resolver no quedo con su organizacion"
dice "13 · emitir, resolver y retirar llevan su organizacion y el id de su celda en columna: salen en la actividad"

echo "✓ el custodio guarda, abre a quien puede, y no deja el valor en ningun otro sitio"
