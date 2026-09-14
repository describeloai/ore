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
#   0  ⭐ el servidor entra con el usuario ACOTADO, y NO alcanza `cofre`
#   1  sin token                        401
#   2  con token                        solo SUS organizaciones
#   3  invitar                          el vale, UNA vez, y no vuelve a salir
#   4  invitar a `ORGADMIN`             SE NIEGA  ← un vale que nadie canjearia
#   5  ⭐ otorgar lo que NO SE TIENE       SE NIEGA  ← el rodeo, por contencion
#  5a  invitar SIN rol                   pertenecer no es un cargo
#  5b  los MIEMBROS · con su nombre del token · y una extraña NO los ve
#  5c  el CATALOGO · y `SECURITYADMIN` EMITE y no LEE, con su nota
#   6  conceder `lector` · revocar · revocar otra vez
#  6b  ⭐ sondear una concesion AJENA   mismo error que una inventada
#   7  conceder `owner`                 SE NIEGA  ← falta la travesia del arbol
#  7b  ⭐ conceder `usar` sobre un `secreto/`  ← la `018`, sin tocar `conceder`
#  7c  un recurso SIN CLASE             SE NIEGA  ← el asidero tiene forma
#   8  admitir con otro correo          SE NIEGA
#   9  y MIRAR tambien deja huella
#  10  ⭐ el APROVISIONADOR (0025 E5): registra un agente y da la celda por
#      aprovisionada, con huella; no lista nada; y una persona no hace lo suyo
#  11  ⭐ LOS DOS VERBOS DE LA 0025 E6: fundar por HTTP (quien pide es el dueño,
#      con celda de plataforma) y pedir una celda mas (ORGADMIN; nombre unico;
#      solo compartido); retirarla (no la de casa); y una USERADMIN no puede
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

# ── ⭐⭐ Y EL SERVIDOR SE CONECTA CON EL USUARIO ACOTADO, no con el de las
#      migraciones ────────────────────────────────────────────────────────────
#
# La `020` reparte los permisos entre dos papeles: `ore_iam` toca todo `iam` y
# **nada** de `cofre`; `ore_cofre` al reves, y de `iam` solo lo que necesita para
# autorizar. Pero eso solo esta EN VIGOR si quien se conecta no es superusuario.
#
# ⛔ Corriendo las pruebas como el usuario de las migraciones —que es
#   superusuario y se salta todos los `grant`— la separacion estaria escrita y no
#   probada: el dia que a `ore-iam` le faltara un permiso, se veria en produccion
#   y no aqui. Es la misma frase de la cabecera de este fichero, un piso mas
#   abajo: un control que solo se ejerce cuando alguien se acuerda no es un
#   control.
#
# ⚠️ Los papeles son del SERVIDOR y no de la base, asi que sobreviven al
#   `drop database` de arriba. De ahi el `do $$` en vez de un `create user` seco.
CLAVE_APP="prueba-no-secreta"
psql "$URL" -qtAc "do \$\$ begin
    if not exists (select 1 from pg_roles where rolname='iam_app_prueba') then
      create user iam_app_prueba login password '$CLAVE_APP';
    end if;
  end \$\$" >/dev/null 2>&1 || falla "no se pudo crear el usuario acotado"
psql "$URL" -qtAc "grant ore_iam to iam_app_prueba" >/dev/null 2>&1 \
  || falla "no se pudo dar el papel \`ore_iam\`"

# La misma URL, cambiando quien entra. `PG_URL` trae credencial y maquina.
SERVIDOR="${PG_URL#postgres://}"; SERVIDOR="${SERVIDOR#*@}"
URL_APP="postgres://iam_app_prueba:$CLAVE_APP@$SERVIDOR/iam_prueba"

# ⭐ Y la propiedad, comprobada aqui y no supuesta: quien autoriza NO alcanza el
#   material. Si algun dia alguien le diera `ore_iam` permiso sobre `cofre` —o
#   conectara el servidor como superusuario «para que funcione»— esto se pone
#   rojo, y es la unica guarda automatica que tiene esa frontera.
if psql "$URL_APP" -qtAc "select 1 from cofre.material" >/dev/null 2>&1; then
  falla "⛔ EL USUARIO DE \`ore-iam\` ALCANZA \`cofre\`. Quien dice quien puede no debe poder abrir nada"
fi
dice "el servidor entra como \`iam_app_prueba\`, y NO alcanza \`cofre\`"

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
tipo = sys.argv[6] if len(sys.argv) > 6 else None
cabeza = {"alg": "RS256", "typ": "JWT", "kid": "k1"}
# `name` es el claim estandar de OIDC. Va aqui porque sin el no se puede
# ejercitar el refresco del nombre, que corre en cada peticion.
cuerpo = {"iss": emisor, "aud": audiencia, "sub": sub, "email": correo,
          "name": sub.split(":")[-1].capitalize() + " (prueba)",
          "exp": int(ahora) + 300, "iat": int(ahora)}
# Y la clase, si se pide: es lo que el IdP estampa a un cliente de maquina.
if tipo:
    cuerpo["rubix_tipo"] = tipo
f = (b64(json.dumps(cabeza).encode()) + "." + b64(json.dumps(cuerpo).encode())).encode()
print(f.decode() + "." + b64(firmar(f)))
PYCODE

"$PY" "$TMP/acunar.py" jwks > "$TMP/jwks.json" || falla "no se pudo escribir el JWKS"
AHORA=$(date +%s)
acunar() { "$PY" "$TMP/acunar.py" "$1" "$2" "$EMISOR" "$AUDIENCIA" "$AHORA" "${3:-}"; }

# ── Dos organizaciones, y la segunda con un ADMINISTRADOR ───────────────────
#
# ⭐ La segunda existe sólo para que el rodeo se pueda medir: `fundar` deja
#   `ORGADMIN`, que tiene TODAS las potestades: nada queda fuera de lo suyo, asi
#   que la contencion siempre se cumple. Sin alguien con menos, la guarda es
#   codigo que nadie ha visto correr.
# ⛔ El SERVIDOR y `fundar` van con el usuario acotado; `psql` de esta prueba
#   sigue yendo con el de las migraciones, porque comprueba cosas —como que
#   una fila revocada siga en la tabla— que el servidor no expone.
export IAM_URL="$URL_APP"
# ⛔ Y su salida NO se tira. La `019` puso `kek not null` y `fundar` no la
#   escribia: esto reventaba con «✗ `fundar acme` fallo» y NADA mas, y hubo que
#   gastar una vuelta de CI para saber por que. Es la misma leccion que ya esta
#   arriba para el log del servidor, aplicada donde faltaba.
# ⭐ `acme` CON celda y `otra` SIN: las dos ramas de `fundar`, y de paso lo
#   que la 0024 ④ exige — que la celda de una organizacion no la vea otra.
"$IAM" fundar --organizacion acme --emisor "$EMISOR" --sub "persona:ada" \
  --correo "ada@paladio.io" \
  --celda ore-prueba --tier compartido --proveedor gcp --region europe-west1-b --puerta ore-prueba.ore.paladio.io \
  > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar acme\` fallo: $(tail -3 "$TMP/fundar.txt")"
grep -q '"celda": "cel_' "$TMP/fundar.txt" || falla "fundar acme no devolvio su celda: $(cat "$TMP/fundar.txt")"
"$IAM" fundar --organizacion otra --emisor "$EMISOR" --sub "persona:zoe" \
  --correo "zoe@paladio.io" > "$TMP/fundar.txt" 2>&1 \
  || falla "\`fundar otra\` fallo: $(tail -3 "$TMP/fundar.txt")"
grep -q 'ninguna' "$TMP/fundar.txt" || falla "fundar otra sin celda no lo DIJO"
# ⛔ Y una celda a medias no funda: parece entera y no lo es.
"$IAM" fundar --organizacion media --emisor "$EMISOR" --sub "persona:eva" \
  --celda x --tier compartido > "$TMP/fundar.txt" 2>&1 \
  && falla "fundar con celda a medias no fallo"
grep -q 'entera o no va' "$TMP/fundar.txt" || falla "la celda a medias no dijo por que"
dice "dos organizaciones fundadas: una con celda, otra sin, y la de a medias negada"

ORG=$(psql "$URL" -qtAc "select id from iam.organizacion where nombre='acme'")
[ -n "$ORG" ] || falla "no se encontro la organizacion"

# Ada funda; Bea entrara despues por la puerta de siempre: un vale.
ADA=$(acunar "persona:ada" "ada@paladio.io")

# ⭐ Con celda de plataforma (0025 E6): es lo que `POST /organizaciones` y
#   `POST …/celdas` dan a quien pide. Los mismos cinco que `fundar` lee.
ORE_CELDA=ore-prueba ORE_CELDA_TIER=compartido ORE_CELDA_PROVEEDOR=gcp \
ORE_CELDA_REGION=europe-west1-b ORE_CELDA_PUERTA=ore-prueba.ore.paladio.io \
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

# ⭐ Y CÓMO SE LLAMA SU ÁRBOL, desde la `017`. Sin esto la columna seria de solo
#   escritura: `fundar` la rellena y nadie comprueba que se pueda leer.
grep -q '"t-acme/ontologia"' "$TMP/r.json" \
  || falla "2 · no dice como se llama su arbol · $(cat "$TMP/r.json")"
# ⭐⭐ Y SU PUERTA, desde la `022`. Ésta es la que sustituyó a `ORE_SERVE_URL` en
#   la consola —la nota de aquí decía que era el árbol, y no lo era: el árbol
#   dice de dónde se lee, la entrada dice A QUIÉN se le pregunta—.
#
# ⛔ Y esta comprobación vale por tres, porque la cadena entera pasa por ella:
#   la migración crea la columna, `fundar` la escribe y esta ruta la devuelve.
#   Si cualquiera de las tres se cae, la consola manda su petición a un destino
#   que no existe — o peor, se queda con el de otro inquilino.
grep -q '"acme.ore.paladio.io"' "$TMP/r.json" \
  || falla "2 · no dice por donde se entra a su arbol · $(cat "$TMP/r.json")"
dice '2 · ve `acme` con su arbol y su puerta, y NO ve `otra`'

# ── 2b · la celda: DONDE corre, del plano de control ─────────────────────────
CODIGO=$(pide GET "/organizaciones/$ORG/celdas" "$ADA")
[ "$CODIGO" = "200" ] || falla "2b · celdas · http $CODIGO · $(cat "$TMP/r.json")"
grep -q '"cluster":"ore-prueba"' "$TMP/r.json" || falla "2b · no ve su celda: $(cat "$TMP/r.json")"
# ⭐ Y la celda dice su PUERTA (027): es lo que la consola coteja contra el DNS.
grep -q '"puerta":"ore-prueba.ore.paladio.io"' "$TMP/r.json" || falla "2b · la celda no dice su puerta: $(cat "$TMP/r.json")"
# ⭐ Y desde la 029: la celda se llama como su organizacion, el cluster va aparte,
#   y el arbol y la entrada son SUYOS (y, mientras dure la doble verdad, los mismos
#   que los de la organizacion — `iam.discrepancias` vacia lo cobra).
grep -q '"nombre":"acme"' "$TMP/r.json" || falla "2b · la celda no se llama como su organizacion: $(cat "$TMP/r.json")"
grep -q '"cluster":"ore-prueba"' "$TMP/r.json" || falla "2b · la celda no dice su cluster: $(cat "$TMP/r.json")"
grep -q '"arbol":"t-acme/ontologia"' "$TMP/r.json" || falla "2b · la celda no dice su arbol: $(cat "$TMP/r.json")"
grep -q '"entrada":"acme.ore.paladio.io"' "$TMP/r.json" || falla "2b · la celda no dice su entrada: $(cat "$TMP/r.json")"
# La vista existe solo mientras dura la doble verdad (029..031); despues no hay nada que cotejar.
if [ "$(psql "$URL" -qtAc "select to_regclass('iam.discrepancias') is not null")" = "t" ]; then
  [ "$(psql "$URL" -qtAc "select count(*) from iam.discrepancias")" = "0" ] || falla "2b · iam.discrepancias no esta vacia: $(psql "$URL" -qtAc "select * from iam.discrepancias")"
fi
grep -q '"tier":"compartido"' "$TMP/r.json" || falla "2b · sin tier"
grep -q '"estado":"activa"' "$TMP/r.json" || falla "2b · sin estado administrativo"
# ⭐ Y el tier DENTRO, desde la `026`: titulo, promesa y la cuota del compartido.
grep -q '"titulo":"Serverless"' "$TMP/r.json" || falla "2b · sin el titulo del tier: $(cat "$TMP/r.json")"
grep -q '"cuota":{' "$TMP/r.json" || falla "2b · el compartido no trae cuota"
grep -q '"cpu":"10"' "$TMP/r.json" || falla "2b · la cuota no es la de la plantilla"
# ⛔ Zoe no es de acme: cero celdas, y el MISMO 200 que «no tiene celda» —
#   decir cual revelaria que la organizacion existe a quien no es de ella.
ZOE=$(acunar "persona:zoe" "zoe@paladio.io")
CODIGO=$(pide GET "/organizaciones/$ORG/celdas" "$ZOE")
[ "$CODIGO" = "200" ] || falla "2b · zoe · http $CODIGO"
grep -q '"celdas":\[\]' "$TMP/r.json" || falla "2b · ⛔ ZOE VE LA CELDA DE ACME: $(cat "$TMP/r.json")"
dice "2b · la celda, solo a los suyos"

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
grep -q '"roles":\["ORGADMIN"\]' "$TMP/r.json" || falla "5b · falta el dueño"
grep -q '"roles":\["USERADMIN"\]' "$TMP/r.json" || falla "5b · falta la administradora"
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

# ── 5c · EL CATALOGO, y el rol que esta VACIO a proposito ──────────────────
#
# ⭐ La pantalla de roles pinta dos listas: lo que da pertenecer, y lo que AÑADE
#   cada cargo. Las dos salen de la base, no de una copia en la interfaz — que
#   es el motivo por el que su `admin/` las devolvia juntas.
[ "$(pide GET "/organizaciones/$ORG/roles" "$ADA")" = "200" ]   || falla "5c · no listo el catalogo: $(cat "$TMP/r.json")"
grep -q '"porDefecto"' "$TMP/r.json"   || falla "5c · sin estado por defecto"
grep -q '"ORGADMIN"' "$TMP/r.json"     || falla "5c · falta ORGADMIN en el catalogo"
grep -q '"SECURITYADMIN"' "$TMP/r.json"   || falla "5c · ⛔ SECURITYADMIN NO SALE. Un rol que existe y no se ve es peor que no tenerlo"

# ⛔⛔ Y sale con lo suyo Y CON SU NOTA. Es lo que lo hace honesto: un rol cuyas
#   potestades no se ejercen no significa nada todavia **y ademas parece que
#   si**, asi que la nota tiene que viajar con el.
"$PY" - "$TMP/r.json" <<'PYCODE' || falla "5c · el catalogo no describe bien a SECURITYADMIN"
import json, sys
d = json.load(open(sys.argv[1]))
cat = d["catalogo"]
s = cat["porRol"]["SECURITYADMIN"]

# ✏️ 2026-09-09 · la `018` le dio un motivo propio. Antes esto exigia que trajera
#   SOLO `actividad:leer-toda` y que su nota dijera «carcasa»: las dos cosas eran
#   ciertas y las dos dejaron de serlo el mismo dia.
#
# ⭐ Lo que se sigue exigiendo es lo que importa: que EMITA y NO LEA. Emitir es
#   una potestad de la organizacion; leer un secreto es una concesion sobre ESE
#   secreto. Si algun dia apareciera aqui una potestad que diera acceso a
#   valores, esta linea se pone roja — y es la unica guarda automatica que tiene
#   esa asimetria.
assert sorted(s["anade"]) == ["actividad:leer-toda", "secreto:emitir", "secreto:listar"], "SECURITYADMIN trae %r" % s["anade"]
assert not [p for p in s["anade"] if p.startswith("secreto") and "leer" in p], "⛔ SECURITYADMIN NO puede tener una potestad de LEER secretos: leer es una concesion"
assert cat["potestades"]["secreto:emitir"]["ejercida"] is False, "no hay verbo de emitir todavia, y el catalogo tiene que decirlo"
assert cat["potestades"]["actividad:leer-toda"]["ejercida"] is False, "esa potestad no tiene ruta todavia, y el catalogo tiene que decirlo"
assert "concesion" in s["nota"].lower(), "su nota tiene que decir por que emitir no da acceso: %r" % s["nota"]

# Y las que SI se ejercen, dichas como tales.
assert cat["potestades"]["invitacion:emitir"]["ejercida"] is True

o = cat["porRol"]["ORGADMIN"]
assert "org:traspasar" in o["anade"], "ORGADMIN sin `org:traspasar`: %r" % o["anade"]
PYCODE
dice '5c · el catalogo sale entero, y `SECURITYADMIN` dice que hoy es una carcasa'

# ⭐ Y las asignaciones distinguen quien lo dio. Ada es ORGADMIN por el
#   APROVISIONAMIENTO —no habia nadie dentro que pudiera concederselo— y eso es
#   `concedido_por` vacio. Bea lo tiene porque Ada la invito.
"$PY" - "$TMP/r.json" <<'PYCODE' || falla "5c · las asignaciones no distinguen el aprovisionamiento"
import json, sys
a = {x["rol"]: x for x in json.load(open(sys.argv[1]))["asignaciones"]}
assert a["ORGADMIN"]["concedido_por"] == "",     "el fundador NO lo recibio de nadie: %r" % a["ORGADMIN"]["concedido_por"]
assert a["USERADMIN"]["concedido_por"] != "",     "a Bea se lo dio alguien, y tiene que constar quien"
PYCODE
dice '5c · y `concedido_por` separa el aprovisionamiento de una concesion'

# ── 6 · conceder y revocar ──────────────────────────────────────────────────
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_x","recurso":"vista/ventas.Clientes","rol":"lector"}')" = "200" ] \
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
      '{"sujeto":"per_y","recurso":"vista/ventas.Pedidos","rol":"lector"}')" = "200" ] \
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
      '{"sujeto":"per_x","recurso":"vista/ventas.Clientes","rol":"owner"}')" = "422" ] \
  || falla '7 · ⛔ SE CONCEDIO `owner` SIN LA TRAVESIA. Nombrar owner exige ser owner del ambito'
grep -q "travesia" "$TMP/r.json" || falla "7 · se niega sin decir por que"
dice '7 · `owner` se niega mientras falte la travesia del arbol'

# ── 7b · ⭐ EL SECRETO EN EL MODELO (`018`) ──────────────────────────
#
# `usar` —resolver sin ver— es un rol de recurso como los otros dos, y se
# concede igual. Que esto pase sin tocar `conceder` es lo que se compro al no
# poner ordinal en `iam.rol_de_recurso`.
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_x","recurso":"secreto/pg-produccion","rol":"usar"}')" = "200" ] \
  || falla "7b · no se pudo conceder \`usar\` sobre un secreto: $(cat "$TMP/r.json")"
dice '7b · `usar` sobre un `secreto/` se concede como cualquier otro rol'

# ⛔ Y el asidero tiene FORMA. Sin clase no es un recurso: dos escrituras del
# mismo secreto dejarian de ser el mismo recurso, y eso no da error — da acceso
# donde no lo hay, o al reves.
[ "$(pide POST "/organizaciones/$ORG/concesiones" "$ADA" \
      '{"sujeto":"per_x","recurso":"ventas.Clientes","rol":"lector"}')" = "422" ] \
  || falla "7c · ⛔ SE CONCEDIO SOBRE UN RECURSO SIN CLASE"
grep -q "asidero" "$TMP/r.json" || falla "7c · se niega sin decir por que: $(cat "$TMP/r.json")"
dice '7c · un recurso sin clase se niega, y dice que es el asidero'

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

# ── 10 · el aprovisionador ──────────────────────────────────────────────────
# La 0025 E5: un sujeto de MAQUINA con `rubix_tipo=aprovisionador`. Dos verbos
# y ninguno mas; y ninguna persona hace esos dos.
APROV=$(acunar "servicio:aprovisionador" "" aprovisionador)
[ "$(pide GET /organizaciones "$APROV")" = "403" ] \
  || falla "10 · ⛔ EL APROVISIONADOR LISTO ORGANIZACIONES: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/acme/agentes" "$ADA" '{"sub":"maquina:x"}')" = "403" ] \
  || falla "10 · ⛔ UNA PERSONA REGISTRO UN AGENTE: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/acme/agentes" "$APROV" '{"sub":"maquina:agente-acme","nombre":"ore-agente-acme"}')" = "200" ] \
  || falla "10 · el aprovisionador no pudo registrar el agente: $(cat "$TMP/r.json")"
AGE=$(campo agente)
[ "$(campo ya)" = "False" ] || falla "10 · la primera vez dijo «ya»"
# El agente hereda `usar` sobre el secreto que 7b concedio: es el mismo nucleo
# que `ore-iam agente`, y esta es la prueba de que lo es.
[ "$(campo secretos_heredados)" -ge 1 ] || falla "10 · el agente no heredo el secreto de 7b: $(cat "$TMP/r.json")"
# Otra vez: idempotente, «ya», y sin huella nueva (no cambio nada).
H0=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion like 'agente:%'")
[ "$(pide POST "/organizaciones/acme/agentes" "$APROV" '{"sub":"maquina:agente-acme"}')" = "200" ] \
  || falla "10 · la segunda vez fallo: $(cat "$TMP/r.json")"
[ "$(campo ya)" = "True" ] && [ "$(campo agente)" = "$AGE" ] || falla "10 · la segunda vez no dijo «ya» con el mismo agente"
H1=$(psql "$URL" -qtAc "select count(*) from iam.huella where operacion like 'agente:%'")
[ "$H1" = "$H0" ] || falla "10 · «ya estaba» dejo huella ($H0 → $H1): un reconciliador que anota cada pasada es ruido"
# Y la huella del registro dice que fue EL APROVISIONADOR, no un operador.
QUIEN=$(psql "$URL" -qtAc "select quien from iam.huella where operacion = 'agente:registrar' and sobre = '$AGE'")
[ "$QUIEN" = "servicio:aprovisionador" ] || falla "10 · la huella del registro no nombra al aprovisionador: «$QUIEN»"
# La celda: nace sin `aprovisionada`, y el aprovisionador la da por hecha.
[ "$(pide GET "/organizaciones/$ORG/celdas" "$ADA")" = "200" ] || falla "10 · no se pudieron leer las celdas"
grep -q '"aprovisionada"' "$TMP/r.json" && falla "10 · la celda nacio ya aprovisionada, y nadie habia pasado"
# ⭐ La celda se llama como su organizacion (029): `acme`; `ore-prueba` es su CLUSTER.
[ "$(pide POST "/celdas/acme/aprovisionada" "$APROV")" = "200" ] \
  || falla "10 · el aprovisionador no pudo dar la celda por aprovisionada: $(cat "$TMP/r.json")"
[ "$(pide POST "/celdas/no-existe/aprovisionada" "$APROV")" = "422" ] || falla "10 · una celda inventada no dio 422"
[ "$(pide GET "/organizaciones/$ORG/celdas" "$ADA")" = "200" ] || falla "10 · no se pudieron releer las celdas"
grep -q '"aprovisionada"' "$TMP/r.json" || falla "10 · la celda no dice cuando la aprovisionaron: $(cat "$TMP/r.json")"
dice "10 · el aprovisionador: registra (idempotente, con su huella), da la celda por aprovisionada, y no lee nada"

# ── 11 · fundar por HTTP, pedir una celda, retirarla ────────────────────────
# Fundar: quien pide es el dueño, y nace con la celda de plataforma.
NOE=$(acunar "persona:noe" "noe@paladio.io")
[ "$(pide POST /organizaciones "$NOE" '{"nombre":"nova"}')" = "200" ] \
  || falla "11 · noe no pudo fundar \`nova\`: $(cat "$TMP/r.json")"
NOVA=$(campo organizacion)
case "$(campo celda)" in cel_*) ;; *) falla "11 · la organizacion fundada por HTTP no trae celda: $(cat "$TMP/r.json")" ;; esac
[ "$(pide POST /organizaciones "$ADA" '{"nombre":"nova"}')" = "422" ] \
  || falla "11 · ⛔ SE FUNDO DOS VECES \`nova\`"
[ "$(psql "$URL" -qtAc "select count(*) from iam.pertenencia_rol where organizacion='$NOVA' and rol='ORGADMIN'")" = "1" ] \
  || falla "11 · nova no tiene UN ORGADMIN"
[ "$(psql "$URL" -qtAc "select quien from iam.huella where operacion='organizacion:fundar' and sobre='$NOVA'")" = "persona:noe" ] \
  || falla "11 · la huella de fundar no nombra a quien pidio"
dice "11 · noe funda \`nova\` por HTTP: dueña, con celda \`nova\`, y el nombre no se repite"
# Pedir una celda mas: ORGADMIN si; USERADMIN (bea, en acme) no.
[ "$(pide POST "/organizaciones/acme/celdas" "$BEA" '{"nombre":"acme-eu"}')" = "422" ] \
  || falla "11 · ⛔ UNA USERADMIN PIDIO UNA CELDA"
grep -q "no puedes" "$TMP/r.json" || falla "11 · la negativa no dice «no puedes»: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/acme/celdas" "$ADA" '{"nombre":"acme-eu"}')" = "200" ] \
  || falla "11 · ada no pudo pedir \`acme-eu\`: $(cat "$TMP/r.json")"
[ "$(campo arbol)" = "t-acme-eu/ontologia" ] && [ "$(campo entrada)" = "acme-eu.ore.paladio.io" ] \
  || falla "11 · la celda nueva no deriva arbol y entrada de SU nombre: $(cat "$TMP/r.json")"
[ "$(pide POST "/organizaciones/acme/celdas" "$ADA" '{"nombre":"acme-eu"}')" = "422" ] \
  || falla "11 · ⛔ DOS CELDAS \`acme-eu\`"
[ "$(pide POST "/organizaciones/acme/celdas" "$ADA" '{"nombre":"nova"}')" = "422" ] \
  || falla "11 · ⛔ acme creo una celda con el nombre de OTRA organizacion (el nombre es global)"
[ "$(pide POST "/organizaciones/acme/celdas" "$ADA" '{"nombre":"acme-big","tier":"dedicado"}')" = "422" ] \
  || falla "11 · ⛔ se pidio un dedicado, y eso todavia no lo monta nadie"
[ "$(pide POST "/organizaciones/acme/celdas" "$ADA" '{"nombre":"Mal Nombre"}')" = "422" ] \
  || falla "11 · un nombre con espacios paso"
[ "$(pide GET "/organizaciones/$ORG/celdas" "$ADA")" = "200" ] || falla "11 · no se leen las celdas"
[ "$("$PY" -c 'import json;print(len(json.load(open("'"$TMP/r.json"'"))["celdas"]))')" = "2" ] \
  || falla "11 · acme no lista dos celdas: $(cat "$TMP/r.json")"
grep -q '"nombre":"acme-eu"' "$TMP/r.json" || falla "11 · la celda nueva no sale en la lista"
dice "11 · ada pide \`acme-eu\`: nace sin aprovisionar; el nombre es unico en toda la plataforma; solo compartido"
# Retirar: la de casa no; la otra si, y dos veces es «ya».
[ "$(pide POST "/celdas/acme/retirar" "$ADA")" = "422" ] || falla "11 · ⛔ SE RETIRO LA CELDA DE CASA"
[ "$(pide POST "/celdas/acme-eu/retirar" "$BEA")" = "422" ] || falla "11 · ⛔ UNA USERADMIN RETIRO UNA CELDA"
[ "$(pide POST "/celdas/acme-eu/retirar" "$ADA")" = "200" ] || falla "11 · ada no pudo retirar acme-eu: $(cat "$TMP/r.json")"
[ "$(campo ya)" = "False" ] || falla "11 · la primera retirada dijo «ya»"
[ "$(pide POST "/celdas/acme-eu/retirar" "$ADA")" = "200" ] && [ "$(campo ya)" = "True" ] \
  || falla "11 · retirar dos veces no es idempotente: $(cat "$TMP/r.json")"
[ "$(psql "$URL" -qtAc "select estado from iam.celda_de where celda='acme-eu'")" = "retirada" ] \
  || falla "11 · celda_de no enseña \`retirada\` al aprovisionador"
dice "11 · retirar: la de casa no, una USERADMIN no, y la segunda vez es «ya»; celda_de lo dice"

echo "✓ los cuatro verbos, sus dos negativas, el rodeo, los dos del aprovisionador, y los de la cuenta."
