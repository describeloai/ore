#!/usr/bin/env bash
# EL SEGUNDO FACTOR — la deuda con fecha, y su despertador.
#
# ── ⛔⛔ QUÉ ES ESTO Y QUÉ NO ES ────────────────────────────────────────────
#
# No es un fallo. Es una relajación **deliberada y datada**, escrita en el
# generador de la plataforma:
#
#     export const EXIGIR_SEGUNDO_FACTOR = false;
#     export const MFA_RELAJADA_HASTA = '2026-09-30';
#
# Con su motivo: *«el 2026-08-26 se baja a CONDITIONAL para levantar el primer
# login del prototipo: con REQUIRED, el primer usuario tiene que enrolar passkey
# o TOTP antes de poder entrar»*. Y con su consecuencia dicha entera:
#
#   > CONDITIONAL significa exactamente esto: la entrada vuelve a ser de UN
#   > factor. No es «MFA opcional»: es que el realm ya no opera a AAL2, aunque
#   > sus atributos lo sigan diciendo.
#
# ── ⭐ POR QUÉ EXISTE ESTE FICHERO ─────────────────────────────────────────
#
# Porque su repositorio pone `check-entrada` en ROJO mientras eso valga `false`
# —*«una deuda que no se ve no es una deuda: es un olvido con buena letra»*— y
# `gen-realm.py` se trajo la relajación **sin traerse la alarma**. La deuda
# cruzó de clúster y perdió su despertador.
#
# ⇒ Esto es el despertador, aquí. Mientras quede plazo avisa y pasa; **el
#   2026-09-30 se pone rojo** y no hay forma de no verlo.
#
# ── ⛔ Y MIRA LAS DOS PUERTAS, NO UNA ──────────────────────────────────────
#
# Su propia acta cuenta que `medir-entrada.mjs` recorría `realm.browserFlow` y
# nada más, así que **ninguna guarda miraba la reposición**. Y ahí pesa más:
# con el correo como único paso, el buzón pasa a ser un factor equivalente a la
# contraseña. NIST SP 800-63B-4 §6.1.2.3 es explícito — la recuperación **no
# puede rebajar el AAL**.
#
# ── Cómo se salda ──────────────────────────────────────────────────────────
#
#   1. enrolar TOTP o passkey en una persona de verdad   ← lo hace una persona
#   2. `EXIGIR_SEGUNDO_FACTOR = true` y regenerar
#   3. ⚠️ y APLICARLO: un `KeycloakRealmImport` **se salta un realm que ya
#      existe** y se declara `Done: True` igualmente. El realm vivo se cambia
#      con `kcadm`, no volviendo a aplicar el manifiesto.
set -u

RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
ARTEFACTO="$RAIZ/malla/61-realms.yaml"
PLAZO="2026-09-30"

PY=$(command -v python3 || command -v python) || { echo "✗ hace falta python" >&2; exit 1; }
[ -f "$ARTEFACTO" ] || { echo "✗ no existe $ARTEFACTO" >&2; exit 1; }

"$PY" - "$ARTEFACTO" "$PLAZO" <<'PYCODE'
# -*- coding: utf-8 -*-
"""Lee el artefacto de realms y contesta si la entrada opera a AAL2.

⭐ Sin biblioteca de YAML: `gen-realm.py` emite cada CR como JSON con sangría y
los une con `---`, así que JSON es todo lo que hace falta. La prueba corre donde
corra el runner, que es la misma razón por la que `servidor-oidc.sh` firma RSA a
mano.
"""
import datetime
import json
import sys

# La consola de Windows por defecto es cp1252 y se ahoga con un `⚠`. Reconfigurar
# la salida es una linea; escribir la prueba en ASCII para que quepa en la
# terminal mas pobre seria dejar que la terminal decida como se lee el codigo.
try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

ARTEFACTO, PLAZO = sys.argv[1], sys.argv[2]

# (flujo que contiene el paso, subflujo del segundo factor, qué puerta es)
PUERTAS = [
    ("browser-rubix-formularios", "browser-rubix-segundo-factor", "la entrada"),
    ("reposicion-rubix", "reposicion-rubix-segundo-factor", "la reposicion"),
]

crudo = open(ARTEFACTO, encoding="utf-8").read()
# La cabecera son comentarios; el primer documento empieza en su `{`.
trozos = [t for t in crudo.split("\n---\n")]
trozos[0] = trozos[0][trozos[0].index("{"):]
documentos = [json.loads(t) for t in trozos if t.strip()]

hoy = datetime.date.today()
vencido = hoy.isoformat() >= PLAZO
relajadas, ausentes = [], []

for d in documentos:
    realm = d["spec"]["realm"]
    flujos = {f["alias"]: f for f in realm.get("authenticationFlows", [])}
    for contenedor, subflujo, puerta in PUERTAS:
        f = flujos.get(contenedor)
        if f is None:
            ausentes.append("%s · %s: no existe el flujo `%s`" % (realm["realm"], puerta, contenedor))
            continue
        pasos = [e for e in f.get("authenticationExecutions", [])
                 if e.get("flowAlias") == subflujo]
        if not pasos:
            ausentes.append("%s · %s: `%s` no cuelga de `%s`"
                            % (realm["realm"], puerta, subflujo, contenedor))
            continue
        req = pasos[0].get("requirement")
        marca = "✓" if req == "REQUIRED" else "⚠"
        print("  %s %-14s %-14s %s" % (marca, realm["realm"], puerta, req))
        if req != "REQUIRED":
            relajadas.append("%s · %s" % (realm["realm"], puerta))

if ausentes:
    print()
    for a in ausentes:
        print("  ✗ " + a)
    print("\n✗ el artefacto no tiene la forma que esta guarda sabe leer.")
    print("  ⛔ Y eso NO se aprueba por omision: un flujo que no esta es un flujo")
    print("     que no exige nada.")
    raise SystemExit(1)

if not relajadas:
    print("\n✓ las dos puertas exigen segundo factor en los tres realms: AAL2.")
    raise SystemExit(0)

quedan = (datetime.date.fromisoformat(PLAZO) - hoy).days
print()
print("  ⛔ %d puerta(s) operan a UN FACTOR: %s" % (len(relajadas), ", ".join(relajadas)))
print("     No es «MFA opcional»: es que el realm no opera a AAL2, aunque sus")
print("     atributos lo sigan diciendo.")
print()
if vencido:
    print("✗ el plazo era %s y hoy es %s." % (PLAZO, hoy.isoformat()))
    print("  La deuda se acepto con fecha; la fecha paso. `EXIGIR_SEGUNDO_FACTOR = true`,")
    print("  regenerar, y aplicarlo con `kcadm` — el import NO cambia un realm que existe.")
    raise SystemExit(1)

print("⚠ deuda VIVA y aceptada: quedan %d dias (plazo %s)." % (quedan, PLAZO))
print("  Se salda enrolando TOTP o passkey en una persona de verdad y poniendo")
print("  `EXIGIR_SEGUNDO_FACTOR = true`. Este paso se pone ROJO solo el %s." % PLAZO)
raise SystemExit(0)
PYCODE
