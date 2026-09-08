#!/usr/bin/env bash
# LOS CUATRO VERBOS, contra un Postgres de verdad y con tokens de verdad.
#
# ── ⛔ Por qué esto existe y no bastaba el Job del clúster ──────────────────
#
# `malla/98-los-cuatro-verbos.yaml` fija lo mismo, y muy bien — pero se corre A
# MANO, contra un clúster que hay que tener en pie. ⇒ el día que alguien rompa
# la guarda del rodeo, **nada se pone rojo**. Un control que sólo se ejerce
# cuando alguien se acuerda no es un control: es una costumbre.
#
# Aquí no hay clúster, ni Keycloak, ni red: un Postgres, el binario, y tokens
# acuñados en el sitio con la misma llave fija que `servidor-oidc.sh`.
#
# ── Lo que fija ────────────────────────────────────────────────────────────
#
#   1  sin token                        401
#   2  con token                        solo SUS organizaciones
#   3  invitar                          el vale, UNA vez, y no vuelve a salir
#   4  invitar a `ORGADMIN`             SE NIEGA  ← un vale que nadie canjearia
#   5  ⭐ otorgar lo que NO SE TIENE       SE NIEGA  ← el rodeo, por contencion
#  5a  invitar SIN rol                   pertenecer no es un cargo
#  5b  los MIEMBROS · con su nombre del token · y una extraña NO los ve
#   6  conceder `lector` · revocar · revocar otra vez
#  6b  ⭐ sondear una concesion AJENA   mismo error que una inventada
#   7  conceder `owner`                 SE NIEGA  ← falta la travesia del arbol
#   8  admitir con otro correo          SE NIEGA
#   9  y MIRAR tambien deja huella
#
# El 5 es el que ninguna otra prueba cubre: en el clúster el sujeto es
# `ORGADMIN`, que lo tiene TODO, asi que la guarda del rodeo **nunca llega a
# morder** — nada queda fuera de lo suyo. Aqui entra una USERADMIN de verdad,
# por su vale, para que muerda.
#
# ⚠️ Dos formas de nombrar la misma base, y hacen falta las dos: `PG_URL` la usa
#   `psql` aqui, y `iam/migrar.sh` lee las `PG*` de siempre. Unificarlas seria
#   parsear una URL en bash, que es mas facil de romper que de escribir.
#
#   PG_URL=postgres://postgres:x@localhost:5432 #   PGHOST=localhost PGUSER=postgres PGPASSWORD=x #     bash pruebas-de-fuego/los-verbos.sh
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8903}"
BASE="http://127.0.0.1:$PUERTO"
PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432}"
TMP="$(mktemp -d)"
SRV=""

# ⛔ Y al fallar, EL LOG DEL SERVIDOR. Un `http 000` significa que no contesto
#   —se murio, o nunca escucho— y esa respuesta no esta en curl: esta en su
#   salida de error, que llevabamos capturando y tirando.
falla() {
  echo "✗ $*" >&2
  if [ -s "$TMP/arranque.txt" ]; then
    echo "── lo que dijo el servidor ──────────────────────────────" >&2
    tail -20 "$TMP/arranque.txt" >&2
  fi
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  exit 1
}
dice()  { echo "  · $*"; }
limpiar() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null; rm -rf "$TMP"; }
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/debug/$1"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
IAM="$(buscar ore-iam)" || falla "no hay binario de \`ore-iam\`"
PY=$(command -v python3 || command -v python) || falla "hace falta python"
command -v psql >/dev/null || falla "hace falta psql"

EMISOR="https://login.paladio.io/realms/rubix"
AUDIENCIA="ore-serve"

# ── La base ─────────────────────────────────────────────────────────────────
# Se tira y se rehace: una prueba que depende de lo que dejo la anterior no fija
# nada, fija el orden en que se corrieron.
psql "$PG_URL/postgres" -qtAc "drop database if exists iam_prueba" >/dev/null 2>&1
psql "$PG_URL/postgres" -qtAc "create database iam_prueba" >/dev/null 2>&1 \
  || falla "no se pudo crear la base de prueba"
URL="$PG_URL/iam_prueba"

PGDATABASE=iam_prueba bash "$RAIZ/iam/migrar.sh" > "$TMP/migrar.txt" 2>&1 \
  || falla "las migraciones fallaron: $(tail -5 "$TMP/migrar.txt")"
dice "$(grep -c '^·' "$TMP/migrar.txt" || echo 0) migraciones aplicadas"

# ── La casa de la moneda ────────────────────────────────────────────────────
# La misma llave y el mismo argumento que `servidor-oidc.sh`: firmar
# RSA-PKCS1v15 es rellenar un bloque y elevar a `d`, y no traer una biblioteca
# es lo que hace que esto corra donde corra el runner.
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
# `name` es el claim estandar de OIDC. Va aqui porque sin el no se puede
# ejercitar el refresco del nombre, que corre en cada peticion.
cuerpo = {"iss": emisor, "aud": audiencia, "sub": sub, "email": correo,
          "name": sub.split(":")[-1].capitalize() + " (prueba)",
          "exp": int(ahora) + 300, "iat": int(ahora)}
f = (b64(json.dumps(cabeza).encode()) + "." + b64(json.dumps(cuerpo).encode())).encode()
print(f.decode() + "." + b64(firmar(f)))
PYCODE

"$PY" "$TMP/acunar.py" jwks > "$TMP/jwks.json" || falla "no se pudo escribir el JWKS"
AHORA=$(date +%s)
acunar() { "$PY" "$TMP/acunar.py" "$1" "$2" "$EMISOR" "$AUDIENCIA" "$AHORA"; }

# ── Dos organizaciones, y la segunda con un ADMINISTRADOR ───────────────────
#
# ⭐ La segunda existe sólo para que el rodeo se pueda medir: `fundar` deja
#   `ORGADMIN`, que tiene TODAS las potestades: nada queda fuera de lo suyo, asi
#   que la contencion siempre se cumple. Sin alguien con menos, la guarda es
#   codigo que nadie ha visto correr.
export IAM_URL="$URL"
"$IAM" fundar --organizacion acme --emisor "$EMISOR" --sub "persona:ada" \
  --correo "ada@paladio.io" >/dev/null 2>&1 || falla "\`fundar acme\` fallo"
"$IAM" fundar --organizacion otra --emisor "$EMISOR" --sub "persona:zoe" \
  --correo "zoe@paladio.io" >/dev/null 2>&1 || falla "\`fundar otra\` fallo"
dice "dos organizaciones fundadas"

ORG=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='acme'")
[ -n "$ORG" ] || falla "no se encontro la organizacion"

# Ada funda; Bea entrara despues por la puerta de siempre: un vale.
ADA=$(acunar "persona:ada" "ada@paladio.io")

"$IAM" servir --bind "127.0.0.1:$PUERTO" --identidad oidc \
  --emisor "$EMISOR" --audiencia "$AUDIENCIA" --jwks "$TMP/jwks.json" \
  > "$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 60); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/salud" >/dev/null || falla "el servidor no arranco: $(cat "$TMP/arranque.txt")"

pide() { # metodo ruta token [cuerpo]
  if [ $# -ge 4 ]; then
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" \
      -H "Authorization: Bearer $3" -H 'Content-Type: application/json' \
      -d "$4" "$BASE$2"
  else
    curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$1" \
      -H "Authorization: Bearer $3" "$BASE$2"
  fi
}
sin_token() { curl -s -o /dev/null -w '%{http_code}' "$BASE$1"; }
campo() { "$PY" -c "import json,sys;print(json.load(open(sys.argv[1])).get(sys.argv[2],''))" "$TMP/r.json" "$1"; }

# ── 1 · sin token ───────────────────────────────────────────────────────────
[ "$(sin_token /organizaciones)" = "401" ] || falla "1 · sin token no dio 401"
dice "1 · sin token · 401"

# ── 2 · sólo SUS organizaciones ─────────────────────────────────────────────
# ⚠️ El codigo Y el cuerpo. Un `falla` que solo dice «no pudo» obliga a otra
#   vuelta de CI para averiguar que contesto, y esa vuelta la paga quien depura.
CODIGO=$(pide GET /organizaciones "$ADA")
[ "$CODIGO" = "200" ] || falla "2 · no pudo listar · http $CODIGO · $(cat "$TMP/r.json")"
grep -q '"acme"' "$TMP/r.json" || falla "2 · no ve la suya"
grep -q '"otra"' "$TMP/r.json" && falla "2 · ⛔ VE LA DE OTRO. Eso es una fuga con forma de comodidad"
dice '2 · ve `acme` y NO ve `otra`'

# ── 3 · invitar, y el vale sale UNA vez ─────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$ADA" \
      '{"correo":"Bea@Paladio.IO","rol":"USERADMIN"}')" = "200" ] \
  || falla "3 · invitar fallo: $(cat "$TMP/r.json")"
VALE_BEA=$(campo vale)
[ -n "$VALE_BEA" ] || falla "3 · no devolvio vale"
dice "3 · invitada, y el vale salio una vez"

[ "$(pide GET "/organizaciones/$ORG/invitaciones" "$ADA")" = "200" ] || falla "3 · no listo"
grep -q "$VALE_BEA" "$TMP/r.json" && falla "3 · ⛔ EL LISTADO DEVUELVE EL VALE. Listar seria una forma de conseguirlos"
grep -q '"correo":"bea@paladio.io"' "$TMP/r.json" || falla "3 · el correo no se plego"
dice "3 · el listado no lleva el vale, y el correo se guardo plegado"

# ── 4 · invitar a `ORGADMIN` ────────────────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$ADA" \
      '{"correo":"c@paladio.io","rol":"ORGADMIN"}')" = "422" ] \
  || falla "4 · ⛔ SE PUEDE INVITAR A UN ORGADMIN. Ese vale no lo podria canjear nadie"
grep -q "traspasarla" "$TMP/r.json" || falla "4 · se niega sin decir por que"
dice '4 · a `ORGADMIN` no se invita, y lo dice'

# ── 5 · ⭐ EL RODEO ─────────────────────────────────────────────────────────
# Bea redime su vale y queda como `USERADMIN`. Desde ahi intenta lo que la
# plataforma escribio con la cicatriz al lado.
BEA=$(acunar "persona:bea" "bea@paladio.io")
[ "$(pide POST /invitaciones/admitir "$BEA" "{\"vale\":\"$VALE_BEA\"}")" = "200" ] \
  || falla "5 · Bea no pudo entrar: $(cat "$TMP/r.json")"
[ "$(campo rol)" = "USERADMIN" ] || falla "5 · entro con otro rol"

[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$BEA" \
      '{"correo":"complice@paladio.io","rol":"ACCOUNTADMIN"}')" = "422" ] \
  || falla "5 · ⛔ UN USERADMIN OTORGA ACCOUNTADMIN. Eso es escalada de privilegio con forma de cortesia"
dice '5 · el rodeo muerde: un USERADMIN no otorga `ACCOUNTADMIN`'

# ⭐⭐ Y LO QUE UN ORDINAL NO SABIA DECIR: dos roles INCOMPARABLES.
#
#   `SECURITYADMIN` no es «mas» ni «menos» que `USERADMIN` — uno corta y el otro
#   da de alta—. Con una escalera habia que inventar cual va encima; con
#   contencion la respuesta sale sola: ninguno contiene al otro, asi que Bea
#   tampoco puede otorgar ESE, y por un motivo distinto al de arriba.
[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$BEA" \
      '{"correo":"vigilante@paladio.io","rol":"SECURITYADMIN"}')" = "422" ] \
  || falla "5 · ⛔ UN USERADMIN OTORGA SECURITYADMIN, que no contiene"
dice '5 · y tampoco `SECURITYADMIN`, que no es mas alto: es incomparable'

# ── 5a · invitar SIN rol: pertenecer y nada mas ────────────────────────────
#
# ⭐ Antes esto no se podia decir —la columna era `not null`— y hubo que
#   inventar un rol `miembro` para taparlo. Desde la `014`, `null` significa
#   pertenecer, que es lo que siempre quiso decir.
[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$ADA" \
      '{"correo":"solo@paladio.io"}')" = "200" ] \
  || falla "5a · no se pudo invitar sin rol: $(cat "$TMP/r.json")"
dice '5a · se invita sin rol: pertenecer no es un cargo'

# ── 5b · los MIEMBROS, y el nombre que se refresca solo ────────────────────
#
# ⭐ Basta con pertenecer para verlos: `76` §2 — esconder quien manda es
#   seguridad por oscuridad. Y el nombre NO se pidio a nadie: llego en el token
#   y se guardo al entrar, que es lo que evita necesitar `manage-realm`.
[ "$(pide GET "/organizaciones/$ORG/miembros" "$ADA")" = "200" ] || falla "5b · no listo miembros"
CUANTOS=$("$PY" -c "import json;print(len(json.load(open('$TMP/r.json'))['miembros']))")
[ "$CUANTOS" = "2" ] || falla "5b · esperaba 2 miembros y hay $CUANTOS"
grep -q '"rol":"ORGADMIN"' "$TMP/r.json"      || falla "5b · falta el dueño"
grep -q '"rol":"USERADMIN"' "$TMP/r.json"     || falla "5b · falta la administradora"
grep -q '"nombre":"Ada (prueba)"' "$TMP/r.json" \
  || falla "5b · ⛔ EL NOMBRE NO SE REFRESCO. Venia en el token y la pantalla enseñaria un id opaco"
grep -q '"conocido":true' "$TMP/r.json" || falla "5b · `conocido` no distingue"
dice "5b · $CUANTOS miembros, con su rol y su nombre del token"

# ⛔ Y una extraña no los ve. Es la misma acotacion de 6b, en una ruta de leer:
#   la lista de quien trabaja en un cliente es de ese cliente.
ZOE0=$(acunar "persona:zoe" "zoe@paladio.io")
[ "$(pide GET "/organizaciones/$ORG/miembros" "$ZOE0")" = "422" ] \
  || falla "5b · ⛔ UNA EXTRAÑA VE LOS MIEMBROS DE OTRA ORGANIZACION"
dice "5b · y una extraña no los ve"

# ── 6 · conceder y revocar ──────────────────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_x","recurso":"ventas.Clientes","rol":"lector"}')" = "200" ] \
  || falla "6 · conceder fallo: $(cat "$TMP/r.json")"
CON=$(campo concesion)
[ "$(pide POST "/concesiones/$CON/revocar" "$ADA")" = "200" ] || falla "6 · revocar fallo"
[ "$(pide POST "/concesiones/$CON/revocar" "$ADA")" = "422" ] || falla "6 · se revoco dos veces"
VIVAS=$(psql "$URL" -qtAc "select count(*) from iam.concesion_viva")
TODAS=$(psql "$URL" -qtAc "select count(*) from iam.concesion")
[ "$VIVAS" = "0" ] && [ "$TODAS" = "1" ] \
  || falla "6 · revocar BORRO la fila ($TODAS en la tabla). Sin ella, «nunca tuvo permiso» y «se lo quitamos» son indistinguibles"
dice "6 · concedida, revocada, y la fila sigue: $TODAS en la tabla · $VIVAS vivas"

# ── 6b · ⭐ LA SONDA ENTRE INQUILINOS ───────────────────────────────────────
#
# Zoe es dueña de `otra` y no pinta nada en `acme`. Si al intentar revocar una
# concesion de `acme` recibiera un mensaje DISTINTO del que da un id inventado,
# tendria un directorio de lo ajeno consultable id a id: existe / no existe /
# ya revocada. Es la primera consecuencia de `0021` y solo aparece cuando una
# persona puede estar en varias organizaciones.
ZOE=$(acunar "persona:zoe" "zoe@paladio.io")
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_y","recurso":"ventas.Pedidos","rol":"lector"}')" = "200" ] \
  || falla "6b · no se pudo conceder la segunda"
VIVA=$(campo concesion)

pide POST "/concesiones/$VIVA/revocar" "$ZOE" >/dev/null
AJENA=$(campo error)
pide POST "/concesiones/con_no_existe_00000000000000000000/revocar" "$ZOE" >/dev/null
INVENTADA=$(campo error)
[ -n "$AJENA" ] && [ "$AJENA" = "$INVENTADA" ] \
  || falla "6b · ⛔ SONDA: una concesion ajena da «$AJENA» y una inventada «$INVENTADA». Distinguirlas es un directorio de lo ajeno"

# Y no la ha tocado: sigue viva.
SIGUE=$(psql "$URL" -qtAc "select count(*) from iam.concesion_viva where id = '$VIVA'")
[ "$SIGUE" = "1" ] || falla "6b · ⛔ UNA EXTRAÑA REVOCO UNA CONCESION AJENA"
dice "6b · la sonda entre inquilinos no distingue, y la concesion sigue viva"

# ── 7 · conceder `owner` ────────────────────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_x","recurso":"ventas.Clientes","rol":"owner"}')" = "422" ] \
  || falla '7 · ⛔ SE CONCEDIO `owner` SIN LA TRAVESIA. Nombrar owner exige ser owner del ambito'
grep -q "travesia" "$TMP/r.json" || falla "7 · se niega sin decir por que"
dice '7 · `owner` se niega mientras falte la travesia del arbol'

# ── 8 · admitir con otro correo ─────────────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/invitaciones" "$ADA" \
      '{"correo":"dani@paladio.io"}')" = "200" ] || falla "8 · no se pudo invitar"
VALE_D=$(campo vale)
[ "$(pide POST /invitaciones/admitir "$BEA" "{\"vale\":\"$VALE_D\"}")" = "422" ] \
  || falla "8 · ⛔ UN VALE AJENO SE REDIMIO. Eso es un traspaso que nadie autorizo"
dice "8 · un vale de otro no sirve"

# ── 9 · y mirar deja huella ─────────────────────────────────────────────────
# La idea de su `022`: la potestad mas barata del catalogo es tambien la mas
# intima. Un verbo de lectura sin rastro es un agujero con forma de optimizacion.
# ⚠️ La columna es `operacion`. Lo primero que escribi fue `accion`, psql erro,
#   la cuenta salio vacia y el paso fallo — CERRADO, que es como tiene que
#   fallar una guarda que no puede leer lo que vigila.
MIRO=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion = 'organizacion:listar'")   || falla "9 · no se pudo leer la huella"
[ "${MIRO:-0}" -ge 1 ] || falla "9 · ⛔ LISTAR NO DEJO HUELLA"
TOTAL=$(psql "$URL" -qtAc "select count(*) from iam.huella")
[ "${TOTAL:-0}" -ge "$MIRO" ] || falla "9 · la cuenta de huellas no cuadra"
dice "9 · $TOTAL huellas, $MIRO de ellas por MIRAR"

echo "✓ los cuatro verbos, sus dos negativas y el rodeo."
