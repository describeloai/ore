# -*- coding: utf-8 -*-
"""Genera `malla/61-realms.yaml` desde las plantillas de la plataforma.

⛔ Los realms NO se escriben a mano. `deploy/identidad/realm.mjs` los emite, y su
propia cabecera dice por qué: «un fichero copiado es un fichero que diverge; al
tercer cliente, uno tendrá PKCE y otro no, y **eso no dará error nunca**».

Aquí se parte de su salida —los TRES— y se añade **sólo lo que ORE necesita**,
que es una audiencia, y **sólo en `rubix`**.

── Por qué los tres, y no sólo el de producción ────────────────────────────

Costó un `HTTP 500`. La consola en local apunta a `RUBIX_IDP_EMISOR=…/realms/
rubix-dev`, y con `rubix-dev` sin importar el descubrimiento daba `404` — que
`lib/auth/oidc.ts` convierte en una excepción, y Next en un 500. El realm de
desarrollo no es un duplicado del de producción: es el que lleva
`http://localhost:3000/auth/callback` en sus `redirectUris`, y por eso existe.

    uso:  python malla/gen-realm.py
"""
import glob
import json
import pathlib

PLANTILLAS = sorted(glob.glob(r"C:\Rubix\deploy\identidad\salida\*.json"))
SALIDA = pathlib.Path(r"C:\ORE\malla\61-realms.yaml")

# ── Lo que ORE añade, y sólo al realm de producción ─────────────────────────
#
# `ore-serve` está configurado con `--emisor …/realms/rubix`. Meter sus clientes
# en los otros dos sería declarar una audiencia que nadie va a pedir y que nadie
# va a aceptar — ruido con forma de configuración.
# ⛔⛔ ESTO DECIA `SOLO_EN = "rubix"`, Y COSTABA UN 401 QUE NO SE ENTIENDE.
#
#   El argumento era bueno: *«meter sus clientes en los otros dos seria declarar
#   una audiencia que nadie va a pedir y que nadie va a aceptar»*. Dejo de ser
#   cierto el dia que la consola local —que entra por `rubix-dev`— tuvo que
#   hablar con `ore-iam`: su token salia SIN `ore-serve` en el `aud` y se
#   rechazaba con «no es para nosotros».
#
#   ⭐ Y el sintoma mandaba al sitio equivocado: se mira la ruta, que esta bien.
#     Una audiencia de mas es ruido; una de menos es una hora buscando.
#
#   ⚠️ `rubix-interno` tambien la lleva, y a proposito: el dia que algo interno
#     hable con este plano, que no vuelva a pasar lo mismo. Si un realm no la
#     necesita, la audiencia sobrante no autoriza nada — sin token no hay nada,
#     y con token de otro sujeto tampoco.
REALMS_CON_ORE = ("rubix", "rubix-dev", "rubix-interno")

AUDIENCIA = {
    "clientId": "ore-serve",
    "name": "ORE · el plano de control",
    "description": "Audiencia. No inicia sesion: existe para poder decir que un token es PARA nosotros.",
    "enabled": True,
    "protocol": "openid-connect",
    "publicClient": False,
    "standardFlowEnabled": False,
    "directAccessGrantsEnabled": False,
    "serviceAccountsEnabled": False,
    "implicitFlowEnabled": False,
}

MAPEADOR_AUDIENCIA = {
    "name": "audiencia-ore-serve",
    "protocol": "openid-connect",
    "protocolMapper": "oidc-audience-mapper",
    "consentRequired": False,
    "config": {
        "included.client.audience": "ore-serve",
        "id.token.claim": "false",
        "access.token.claim": "true",
    },
}

AGENTE = {
    "clientId": "ore-agente",
    "name": "ORE · agente",
    "description": "Cuenta de servicio: pide tokens para `ore-serve`. Es un agente, no una persona.",
    "enabled": True,
    "protocol": "openid-connect",
    "publicClient": False,
    "standardFlowEnabled": False,
    "directAccessGrantsEnabled": False,
    "serviceAccountsEnabled": True,
    "implicitFlowEnabled": False,
    "attributes": {"access.token.lifespan": "300"},
    "protocolMappers": [
        MAPEADOR_AUDIENCIA,
        {
            # El tipo de sujeto, igual que `rubix-consola` marca `persona`.
            "name": "rubix-tipo-agente",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-hardcoded-claim-mapper",
            "consentRequired": False,
            "config": {
                "claim.name": "rubix_tipo",
                "claim.value": "agente",
                "jsonType.label": "String",
                "access.token.claim": "true",
            },
        },
    ],
}

CABECERA = """# LOS REALMS — GENERADOS. No se editan aqui.
#
#   python malla/gen-realm.py
#
# Salen de `C:\\\\Rubix\\\\deploy\\\\identidad\\\\salida\\\\*.json`, que a su vez emite
# `deploy/identidad/realm.mjs`. Su cabecera dice por que es un generador y no un
# JSON de ejemplo: «un fichero copiado es un fichero que diverge; al tercer
# cliente, uno tendra PKCE y otro no, y eso no dara error nunca».
#
# ── Los tres, y ninguno sobra ───────────────────────────────────────────────
#
#   rubix          produccion. `redirectUris` -> https://app.paladio.io
#   rubix-dev      desarrollo. `redirectUris` -> http://localhost:3000
#   rubix-interno  sin clientes propios
#
# ⭐ El de desarrollo NO es un duplicado, y no importarlo costo un `HTTP 500`:
# la consola en local declara `RUBIX_IDP_EMISOR=.../realms/rubix-dev`, y con ese
# realm ausente el descubrimiento daba `404`, que `lib/auth/oidc.ts` convierte en
# una excepcion y Next en un 500 sin mas texto.
#
# ── Lo que ORE anade, en LOS TRES ───────────────────────────────────────────
#
#   ore-serve    una AUDIENCIA. Todos los flujos apagados: no inicia sesion de
#                nadie. Existe para poder decir que un token es PARA nosotros
#   ore-agente   una cuenta de servicio que pide tokens para esa audiencia
#   y en `rubix-consola`, el mapeador que mete `ore-serve` en el `aud`
#
# Todo lo demas viaja tal cual: la politica de clave, los cinco flujos propios,
# las passkeys y las organizaciones.
#
# ── ⛔⛔ Y LO QUE VIAJA RELAJADO, QUE ANTES DECIA AQUI LO CONTRARIO ──────────
#
# Esta cabecera afirmaba «AAL2 contra NIST SP 800-63B-4». **No es cierto hoy.**
# El generador de la plataforma lleva `EXIGIR_SEGUNDO_FACTOR = false` desde el
# 2026-08-26 —para poder levantar el primer login del prototipo— con fecha de
# muerte `2026-09-30`, y su propia nota dice la consecuencia entera:
#
#   > CONDITIONAL significa exactamente esto: la entrada vuelve a ser de UN
#   > factor. No es «MFA opcional»: es que el realm ya no opera a AAL2, aunque
#   > sus atributos lo sigan diciendo.
#
# Su repositorio pone `check-entrada` en rojo mientras eso valga `false`. Esa
# alarma NO viajaba con el artefacto — la deuda cruzo de cluster y perdio su
# despertador. Ahora la lleva `pruebas-de-fuego/el-segundo-factor.sh`, que avisa
# mientras quede plazo y se pone ROJO el 2026-09-30.
#
# ── ⚠️ Lo que un `KeycloakRealmImport` NO hace ──────────────────────────────
#
# Mantener. Medido por ellos: **se salta un realm que ya existe y se declara
# `Done: True` igualmente**. Esto sirve para CREAR un realm, no para cambiarlo.
"""


def con_ore(realm):
    """Anade la audiencia de ORE. Idempotente: si ya esta, no duplica."""
    clientes = realm.setdefault("clients", [])
    ya = {c.get("clientId") for c in clientes}
    if "ore-serve" not in ya:
        clientes.append(AUDIENCIA)
    if "ore-agente" not in ya:
        clientes.append(AGENTE)
    for c in clientes:
        if c.get("clientId") == "rubix-consola":
            m = c.setdefault("protocolMappers", [])
            if not any(x.get("name") == "audiencia-ore-serve" for x in m):
                m.append(MAPEADOR_AUDIENCIA)
    return realm


documentos = []
for f in PLANTILLAS:
    realm = json.loads(pathlib.Path(f).read_text(encoding="utf-8"))
    nombre = realm["realm"]
    if nombre in REALMS_CON_ORE:
        realm = con_ore(realm)
    documentos.append({
        "apiVersion": "k8s.keycloak.org/v2alpha1",
        "kind": "KeycloakRealmImport",
        "metadata": {
            "name": nombre,
            "namespace": "identidad",
            "labels": {"ore.dev/tenant": "system", "ore.dev/rol": "identidad"},
        },
        "spec": {"keycloakCRName": "idp", "realm": realm},
    })

# JSON es un subconjunto de YAML, asi que cada CR se emite como JSON con sangria
# y el conjunto es un YAML multidocumento valido. Sin dependencias.
cuerpo = "\n---\n".join(json.dumps(d, indent=2, ensure_ascii=False) for d in documentos)
SALIDA.write_text(CABECERA + cuerpo + "\n", encoding="utf-8", newline="\n")

for d in documentos:
    r = d["spec"]["realm"]
    print("%-14s %d clientes: %s" % (
        r["realm"], len(r.get("clients", [])),
        ", ".join(c["clientId"] for c in r.get("clients", [])) or "—"))
print("escrito %s" % SALIDA)
