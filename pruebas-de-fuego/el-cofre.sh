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

falla() {
  echo "✗ $*" >&2
  if [ -s "$TMP/arranque.txt" ]; then
    echo "── lo que dijo el custodio ──────────────────────────────" >&2
    tail -20 "$TMP/arranque.txt" >&2
  fi
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  exit 1
}
dice() { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$TMP"; }
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
# que es lo que diria Google.
cat > "$TMP/kms-de-mentira" <<'KMSCODE'
#!/usr/bin/env python3
import base64, sys

a = sys.argv[1:]
verbo = a[1]
def opt(n):
    for i, x in enumerate(a):
        if x == n:
            return a[i + 1]
        if x.startswith(n + "="):
            return x.split("=", 1)[1]
    return None

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

# ── Dos organizaciones: la de Ada, y la de Zoe para el 6 ────────────────────
export IAM_URL="$URL_IAM"
"$IAM" fundar --organizacion acme --emisor "$EMISOR" --sub "persona:ada" \
  --correo "ada@paladio.io" > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar acme\` fallo: $(tail -3 "$TMP/fundar.txt")"
"$IAM" fundar --organizacion otra --emisor "$EMISOR" --sub "persona:zoe" \
  --correo "zoe@paladio.io" > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar otra\` fallo: $(tail -3 "$TMP/fundar.txt")"
ORG=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='acme'")
[ -n "$ORG" ] || falla "no se encontro la organizacion"
dice "dos organizaciones, con su llave: $(psql "$URL" -qtAc "select string_agg(kek, ' ') from iam.organizacion")"

ADA=$(acunar "persona:ada" "ada@paladio.io")
ZOE=$(acunar "persona:zoe" "zoe@paladio.io")

# ── El custodio ─────────────────────────────────────────────────────────────
COFRE_URL="$URL_COFRE" "$COFRE" servir --bind "127.0.0.1:$PUERTO" \
  --identidad oidc --emisor "$EMISOR" --audiencia "$AUDIENCIA" \
  --jwks "$TMP/jwks.json" --kms "$TMP/kms-de-mentira" --lugar europe-west1 \
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
pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ZOE" >/dev/null
AJENO=$(campo error)
pide GET "/organizaciones/$ORG/secretos/no-existe-nada" "$ADA" >/dev/null
INVENTADO=$(campo error)
[ -n "$AJENO" ] && [ "$AJENO" = "$INVENTADO" ] \
  || falla "6 · ⛔ SONDA: uno ajeno da «$AJENO» y uno inventado «$INVENTADO»"
dice "6 · un secreto ajeno y uno inventado dan el MISMO error"

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
# seguir abriendose — `cofre.material.kek` guarda con cual se cerro, y por eso
# rotar no es una caida. Si el codigo usara la llave de AHORA, el cliente de
# mentira lo cazaria: descifrar con otra falla.
psql "$URL" -qtAc "update iam.organizacion set kek = 'ore/acme-nueva' where id = '$ORG'" >/dev/null
[ "$(pide GET "/organizaciones/$ORG/secretos/pg-produccion" "$ADA")" = "200" ] \
  || falla "8 · ⛔ tras cambiar la llave de la organizacion, lo viejo dejo de abrirse: $(cat "$TMP/r.json")"
[ "$(campo valor)" = "postgres://u:p@db/x" ] || falla "8 · abrio, pero devolvio otra cosa"
dice "8 · la llave cambio y lo cerrado antes sigue abriendose"

echo
echo "✓ el custodio guarda, abre a quien puede, y no deja el valor en ningun otro sitio"
