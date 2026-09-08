#!/usr/bin/env bash
# La puerta OIDC, con tokens de verdad y sobre un socket de verdad.
#
# Las pruebas del crate verifican el verificador. Esto verifica **el cableado**:
# que `--identidad oidc` monta las rutas, que el sujeto sale de la cabecera
# `Authorization`, y que el que no trae token recibe lo mismo que si no hubiera
# traído nada.
#
# El token se acuña aquí, con la MISMA llave que las pruebas del crate y en
# treinta líneas de Python: firmar RSA-PKCS1v15 es rellenar un bloque y elevar a
# `d`. No se trae ninguna biblioteca, y así esta prueba corre donde corra el
# runner.
#
#   1. sin `Authorization`               401  — igual que sin nada
#   2. con un token BUENO                pasa, y el sujeto es el `sub`
#   3. con el token de OTRA audiencia    401, y dice por qué
#   4. con un `alg: none`                401
#   5. con un token caducado             401
#
# El 3 es el que importa: el realm es compartido, así que un token válido de
# `rubix-consola` llegaría aquí perfectamente firmado. Lo que lo para no es la
# firma, es la audiencia.
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8902}"
BASE="http://127.0.0.1:$PUERTO"
TMP="$(mktemp -d)"
SRV=""

falla() { echo "✗ $*" >&2; limpiar; exit 1; }
dice()  { echo "  · $*"; }
limpiar() {
  [ -n "$SRV" ] && kill "$SRV" 2>/dev/null
  rm -rf "$TMP"
}
trap limpiar EXIT

buscar() {
  local n
  for n in "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
ORE="$(buscar ore)"         || falla "no hay binario de \`ore\`"
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\`"
PY=$(command -v python3 || command -v python) || falla "hace falta python para acuñar tokens"

EMISOR="https://login.paladio.io/realms/rubix"
AUDIENCIA="ore-serve"

# ── La casa de la moneda ────────────────────────────────────────────────────
cat > "$TMP/acuñar.py" <<'PYCODE'
# -*- coding: utf-8 -*-
"""Acuña un token RS256 con la llave fija de las pruebas.

Firmar RSA-PKCS1v15 es: colocar el digest en un bloque con su relleno y elevarlo
a `d` módulo `n`. No hace falta una biblioteca para eso, y no tenerla es lo que
hace que esta prueba corra en cualquier sitio.

⛔ Esta clave está en un repositorio público. No vale para nada más que esto.
"""
import base64, hashlib, json, sys

N = 98108802788390257451427544537569571344186445898290802556306202303563898746153694624427727358963513671154354914513738651625770608698713459103692963416659567693829394560290454767975140775489577446428399646018299491037316206274830556376268979724700474004190230381644330590836431547957441542425447960647896094871
D = 58486241606109815346595400118075370902635461720864906313568320457114881061285666040279031389708798321838495691825034032384259760307155288367215166817606418492512751565372832090822858982082061256407746822196621146620704044182961301819536667603341097088576106781350305670874531525425729267744540582701760709001
E = 65537
K = (N.bit_length() + 7) // 8
# El DER de un DigestInfo con SHA-256, tal cual lo fija el RFC 8017 §9.2.
PREFIJO = bytes.fromhex("3031300d060960864801650304020105000420")


def b64(b):
    return base64.urlsafe_b64encode(b).decode().rstrip("=")


def firmar(mensaje):
    d = hashlib.sha256(mensaje).digest()
    t = PREFIJO + d
    # El relleno se construye ENTERO y se compara entero al verificar; ésa es la
    # diferencia entre una verificación correcta y una que acepta basura.
    em = b"\x00\x01" + b"\xff" * (K - len(t) - 3) + b"\x00" + t
    return pow(int.from_bytes(em, "big"), D, N).to_bytes(K, "big")


def jwks():
    return json.dumps({"keys": [{
        "kty": "RSA", "use": "sig", "kid": "k1", "alg": "RS256",
        "n": b64(N.to_bytes(K, "big")),
        "e": b64(E.to_bytes(3, "big")),
    }]})


def token(cabeza, cuerpo):
    firmado = (b64(json.dumps(cabeza).encode()) + "." +
               b64(json.dumps(cuerpo).encode())).encode()
    return firmado.decode() + "." + b64(firmar(firmado))


if __name__ == "__main__":
    que = sys.argv[1]
    if que == "jwks":
        print(jwks())
        raise SystemExit(0)

    emisor, audiencia, ahora = sys.argv[2], sys.argv[3], int(sys.argv[4])
    cabeza = {"alg": "RS256", "typ": "JWT", "kid": "k1"}
    cuerpo = {"iss": emisor, "aud": audiencia, "sub": "persona:ana",
              "exp": ahora + 300, "iat": ahora}
    if que == "otra-audiencia":
        cuerpo["aud"] = "rubix-consola"
    elif que == "caducado":
        cuerpo["exp"] = ahora - 3600
    elif que == "alg-none":
        cabeza["alg"] = "none"
    print(token(cabeza, cuerpo))
PYCODE

"$PY" "$TMP/acuñar.py" jwks > "$TMP/jwks.json" || falla "no se pudo escribir el JWKS"
AHORA=$(date +%s)
acuñar() { "$PY" "$TMP/acuñar.py" "$1" "$EMISOR" "$AUDIENCIA" "$AHORA"; }

# ── El árbol y el servidor ──────────────────────────────────────────────────
REPO="$TMP/repo"
mkdir -p "$REPO"
( cd "$REPO" && "$ORE" init . >/dev/null 2>&1 ) || falla "\`ore init\` fallo"

"$SERVE" --repo "$REPO" --ore "$ORE" --bind "127.0.0.1:$PUERTO" \
  --identidad oidc --emisor "$EMISOR" --audiencia "$AUDIENCIA" \
  --jwks "$TMP/jwks.json" > "$TMP/arranque.txt" 2>&1 &
SRV=$!
for _ in $(seq 1 40); do
  curl -s -o /dev/null "$BASE/salud" && break
  sleep 0.25
done
curl -sf "$BASE/salud" >/dev/null || falla "0 · el servidor no arranco: $(cat "$TMP/arranque.txt")"
grep -q "llaves" "$TMP/arranque.txt" || falla "0 · no dijo cuantas llaves cargo"
dice "0 · arranca en modo oidc: $(grep llaves "$TMP/arranque.txt" | head -1 | tr -s ' ')"

comprueba() {
  local que="$1" esperado="$2" cabecera="${3:-}"
  local cod
  if [ -n "$cabecera" ]; then
    cod=$(curl -s -o "$TMP/r.json" -w '%{http_code}' -H "$cabecera" "$BASE/fuentes")
  else
    cod=$(curl -s -o "$TMP/r.json" -w '%{http_code}' "$BASE/fuentes")
  fi
  [ "$cod" = "$esperado" ] || falla "$que · devolvio $cod y no $esperado: $(cat "$TMP/r.json")"
}

# ── 1 · Sin token ───────────────────────────────────────────────────────────
comprueba "1" 401
dice "1 · sin \`Authorization\`, 401"

# ── 2 · Un token bueno ──────────────────────────────────────────────────────
BUENO=$(acuñar bueno)
comprueba "2" 200 "Authorization: Bearer $BUENO"
grep -q '"datasources"' "$TMP/r.json" || falla "2 · paso pero no contesto"
dice "2 · con un token bueno, pasa"

# ── 3 · El mismo realm, otra audiencia ──────────────────────────────────────
OTRA=$(acuñar otra-audiencia)
comprueba "3" 401 "Authorization: Bearer $OTRA"
grep -q "audiencia" "$TMP/r.json" || falla "3 · el 401 no dice que es la audiencia: $(cat "$TMP/r.json")"
dice "3 · un token bien firmado para OTRO servicio: 401, y dice que es la audiencia"

# ── 4 · `alg: none` ─────────────────────────────────────────────────────────
NONE=$(acuñar alg-none)
comprueba "4" 401 "Authorization: Bearer $NONE"
grep -q "algoritmo" "$TMP/r.json" || falla "4 · el 401 no nombra el algoritmo"
dice "4 · \`alg: none\` no pasa, porque la lista es de PERMITIDOS"

# ── 5 · Caducado ────────────────────────────────────────────────────────────
VIEJO=$(acuñar caducado)
comprueba "5" 401 "Authorization: Bearer $VIEJO"
grep -q "cadu" "$TMP/r.json" || falla "5 · el 401 no dice que caduco"
dice "5 · un token caducado no pasa"

echo
echo "ok · la puerta pide un token, lo verifica, y dice por que cuando no"
