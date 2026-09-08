# -*- coding: utf-8 -*-
"""Genera `malla/61-realm.yaml` desde la plantilla de la plataforma.

⛔ El realm NO se escribe a mano. `deploy/identidad/realm.mjs` lo emite, y su
propia cabecera dice por qué: «un fichero copiado es un fichero que diverge; al
tercer cliente, uno tendrá PKCE y otro no, y eso no dará error nunca».

Aquí se parte de su salida y se añade **sólo lo que ORE necesita**, que es una
audiencia. Todo lo demás —AAL2, la política de clave, los cinco flujos, las
organizaciones— viaja tal cual.
"""
import json
import pathlib

PLANTILLA = pathlib.Path(r"C:\Rubix\deploy\identidad\salida\rubix.json")
SALIDA = pathlib.Path(r"C:\ORE\malla\61-realm.yaml")

realm = json.loads(PLANTILLA.read_text(encoding="utf-8"))

clientes = realm.setdefault("clients", [])
ya = {c.get("clientId") for c in clientes}

# ── La audiencia de ORE ─────────────────────────────────────────────────────
#
# Todos los flujos apagados: este cliente **no inicia sesión de nadie**. Existe
# para ser un nombre en el `aud`, exactamente igual que `rubix-api`. Un token
# emitido para otro servicio del mismo realm llega aquí perfectamente firmado y
# no vale — y eso es lo que este cliente hace posible decir.
if "ore-serve" not in ya:
    clientes.append({
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
    })

# ── Y quien pide tokens PARA esa audiencia ──────────────────────────────────
#
# Un cliente de cuenta de servicio. Es el sujeto AGENTE —una maquina— y es lo
# que permite ejercitar la cadena entera sin que exista todavia la entrada del
# navegador: la mitad de `F3` que su propio reconocimiento ya habia separado.
if "ore-agente" not in ya:
    clientes.append({
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
            {
                # Sin esto el token sale sin `aud: ore-serve` y `ore-serve` lo
                # rechaza — con razon. La audiencia no se hereda: se declara.
                "name": "audiencia-ore-serve",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-audience-mapper",
                "consentRequired": False,
                "config": {
                    "included.client.audience": "ore-serve",
                    "id.token.claim": "false",
                    "access.token.claim": "true",
                },
            },
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
    })

# La consola tiene que poder pedir un token PARA `ore-serve`.
for c in clientes:
    if c.get("clientId") == "rubix-consola":
        mapeadores = c.setdefault("protocolMappers", [])
        if not any(m.get("name") == "audiencia-ore-serve" for m in mapeadores):
            mapeadores.append({
                "name": "audiencia-ore-serve",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-audience-mapper",
                "consentRequired": False,
                "config": {
                    "included.client.audience": "ore-serve",
                    "id.token.claim": "false",
                    "access.token.claim": "true",
                },
            })

cabecera = """# EL REALM — GENERADO. No se edita aqui.
#
#   python <scratchpad>/gen_realm.py
#
# Sale de `C:\\Rubix\\deploy\\identidad\\salida\\rubix.json`, que a su vez lo
# emite `deploy/identidad/realm.mjs`. Su cabecera dice por que es un generador
# y no un JSON de ejemplo: «un fichero copiado es un fichero que diverge; al
# tercer cliente, uno tendra PKCE y otro no, y eso no dara error nunca».
#
# ── Lo que ORE anade, y es SOLO esto ────────────────────────────────────────
#
#   ore-serve    una AUDIENCIA. Todos los flujos apagados: no inicia sesion de
#                nadie. Existe para poder decir que un token es PARA nosotros
#   ore-agente   una cuenta de servicio que pide tokens para esa audiencia. Es
#                el sujeto AGENTE, y es lo que permite ejercitar la cadena sin
#                que exista todavia la entrada del navegador
#   y en `rubix-consola`, el mapeador que mete `ore-serve` en el `aud`
#
# Todo lo demas viaja tal cual: AAL2 contra NIST SP 800-63B-4, la politica de
# clave, los cinco flujos propios, las passkeys y las organizaciones.
#
# ── ⚠️ Lo que un `KeycloakRealmImport` NO hace ──────────────────────────────
#
# Mantener. Medido por ellos el 2026-08-26: **se salta un realm que ya existe y
# se declara `Done: True` igualmente**. Esto sirve para CREAR el realm, no para
# cambiarlo. Cambiarlo es de la Admin API — su `aplicar-entrada.mjs`.
"""

cr = {
    "apiVersion": "k8s.keycloak.org/v2alpha1",
    "kind": "KeycloakRealmImport",
    "metadata": {
        "name": "rubix",
        "namespace": "identidad",
        "labels": {"ore.dev/tenant": "system", "ore.dev/rol": "identidad"},
    },
    "spec": {"keycloakCRName": "idp", "realm": realm},
}

# JSON es un subconjunto de YAML, asi que el CR se emite como JSON con sangria
# y es YAML valido. Sin analizador de YAML y sin una dependencia mas.
SALIDA.write_text(cabecera + json.dumps(cr, indent=2, ensure_ascii=False) + "\n",
                  encoding="utf-8", newline="\n")
print("escrito %s · %d clientes: %s" % (
    SALIDA, len(clientes), ", ".join(c["clientId"] for c in clientes)))
