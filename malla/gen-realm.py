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

✏️ 2026-09-15 · Y eso dejó de ser así el 2026-09-14, cuando `rubix-dev` se
renombró a `rubix` y `ore-iam` pasó a aceptar SÓLO ese emisor (034): un token
de `rubix-dev` entra en la consola y `ore-iam` lo rechaza. La consola en local
entra por `rubix`, y por eso `rubix-consola` lleva AHÍ el `localhost:3000`
(`con_consola_local`). El mismo 500 volvió a costar una mañana: el
`.env.local` seguía diciendo `rubix-dev`.

    uso:  python malla/gen-realm.py
"""
import glob
import json
import pathlib

PLANTILLAS = sorted(glob.glob(r"C:\Rubix\deploy\identidad\salida\*.json"))
SALIDA = pathlib.Path(r"C:\ORE\malla\61-realms.yaml")

# ── Lo que ORE añade, y sólo al realm de producción ─────────────────────────
#
# `ore-serve` estaba configurado con `--emisor …/realms/rubix`. Meter sus clientes
# en los otros dos sería declarar una audiencia que nadie va a pedir y que nadie
# va a aceptar — ruido con forma de configuración.
# ✏️ 2026-09-09 · y desde hoy tampoco eso es cierto: `ore-serve` valida contra
#   `rubix-dev`, como `ore-iam`. La frase se deja porque lo que sigue la refuta.
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
            # 0027 ② (E2): el gateway de modelos es una AUDIENCIA mas del
            # agente, como cadena y no como cliente — el gateway no inicia
            # sesion de nadie ni es cliente del realm; verifica con el JWKS de
            # fichero y solo mira que `modelos` este en `aud`. Los clientes por
            # celda (`ore-agente-<celda>`, ⑦ del aprovisionador) llevan este y
            # ademas `rubix_celda`; este generico no es de ninguna celda.
            "name": "audiencia-modelos",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-audience-mapper",
            "consentRequired": False,
            "config": {
                "included.custom.audience": "modelos",
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
#   rubix          produccion. `redirectUris` -> https://app.paladio.io. Y el
#                  UNICO con gente: el 2026-09-14 `rubix-dev` (donde vivian
#                  demo, prueba y sus agentes) se RENOMBRO a `rubix`, y el
#                  `rubix` vacio de antes se borro. Registro abierto (abajo).
#   rubix-dev      desarrollo. `redirectUris` -> http://localhost:3000. Vacio:
#                  el import lo vuelve a crear cuando haga falta
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


# ── ⛔⛔ EL AMBITO `basic`, Y COSTO UN TOKEN ANONIMO ────────────────────────
#
#   Desde Keycloak 24 el `sub` **dejo de estar cableado** en el constructor del
#   access token y vive en el ambito `basic`. Su plantilla le fija a
#   `rubix-consola` la lista entera de ambitos a mano —
#
#       ['organization', 'profile', 'email', 'roles', 'web-origins']
#
#   — y `basic` no esta. Fijar una lista completa es exactamente lo que hace que
#   un valor por defecto NUEVO no llegue nunca.
#
#   ⭐ El sintoma, medido el 2026-09-08: la consola entraba bien, el token venia
#     bien firmado, con el emisor correcto y con nuestra audiencia dentro… y
#     `ore-iam` lo rechazaba con **«el token no dice de quien es»**. Un token
#     valido y ANONIMO, que es de las cosas que mas cuestan de creer.
#
#   ⚠️ Y afecta solo a `rubix-consola`: los demas clientes no fijan la lista, asi
#     que heredan el defecto del realm y `basic` les llega solo.
AMBITO_DEL_SUJETO = "basic"


def con_sujeto(realm):
    """Se asegura de que el token lleve `sub`. Idempotente."""
    for c in realm.get("clients", []):
        ambitos = c.get("defaultClientScopes")
        # ⛔ Solo si la lista esta FIJADA. Si el cliente no la declara, hereda la
        #   del realm y meter mano seria empezar a fijarla nosotros.
        if ambitos is not None and AMBITO_DEL_SUJETO not in ambitos:
            ambitos.append(AMBITO_DEL_SUJETO)
    return realm


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


# ── ⭐⭐ EL REGISTRO ABIERTO EN `rubix` (2026-09-14) ─────────────────────────
#
# La cuenta nace en la interfaz de registro de Keycloak y la organizacion en
# `POST /organizaciones` (0025 E6). El generador de la plataforma trae el
# registro cerrado; aqui se abre SOLO en produccion, y con `verifyEmail` apagado
# porque el realm no tiene servidor de correo (`smtpServer: {}`): con los dos
# encendidos, quien se registra queda encerrado esperando un correo que no llega.
# El dia que haya correo, `verifyEmail` vuelve a `true` aqui y no en otro sitio.
#
# ⚠️ Un realm que YA existe no lo toca el import (se salta y dice Done): estos
#   dos valores se aplicaron en vivo con `kcadm update realms/rubix` el mismo
#   dia, y esto es para que el proximo import los lleve.
REGISTRO_EN_PRODUCCION = {"realm": "rubix", "registrationAllowed": True, "verifyEmail": False}


# ⭐ Y el registro PREGUNTA POR LA ORGANIZACION (035): un atributo del perfil de
#   usuario, obligatorio, que la consola lee del token (`rubix_organizacion`) al
#   entrar por primera vez para fundar la cuenta con ese TITULO. El identificador
#   se deriva. El perfil de usuario viaja en el import como un `component`
#   `UserProfileProvider` con `kc.user.profile.config`; si el JSON de la
#   plataforma no lo trae, se construye con los cuatro de siempre mas este.
ATRIBUTO_ORGANIZACION = {
    "name": "organizacion",
    "displayName": "Organización",
    "validations": {"length": {"min": 2, "max": 80}},
    "annotations": {"inputHelperTextBefore": "El nombre de tu empresa o equipo. Sera tu cuenta en Rubix; el identificador (minusculas y guiones) se deriva de el."},
    "required": {"roles": ["user"]},
    "permissions": {"view": ["admin", "user"], "edit": ["admin", "user"]},
    "multivalued": False,
}
MAPEADOR_ORGANIZACION = {
    "name": "rubix-organizacion",
    "protocol": "openid-connect",
    "protocolMapper": "oidc-usermodel-attribute-mapper",
    "consentRequired": False,
    "config": {
        "user.attribute": "organizacion",
        "claim.name": "rubix_organizacion",
        "jsonType.label": "String",
        "id.token.claim": "true",
        "access.token.claim": "true",
        "userinfo.token.claim": "true",
    },
}
PERFIL_BASE = [
    {"name": "username", "displayName": "${username}", "validations": {"length": {"min": 3, "max": 255}, "username-prohibited-characters": {}, "up-username-not-idn-homograph": {}}, "permissions": {"view": ["admin", "user"], "edit": ["admin", "user"]}, "multivalued": False},
    {"name": "email", "displayName": "${email}", "validations": {"email": {}, "length": {"max": 255}}, "required": {"roles": ["user"]}, "permissions": {"view": ["admin", "user"], "edit": ["admin", "user"]}, "multivalued": False},
    {"name": "firstName", "displayName": "${firstName}", "validations": {"length": {"max": 255}, "person-name-prohibited-characters": {}}, "required": {"roles": ["user"]}, "permissions": {"view": ["admin", "user"], "edit": ["admin", "user"]}, "multivalued": False},
    {"name": "lastName", "displayName": "${lastName}", "validations": {"length": {"max": 255}, "person-name-prohibited-characters": {}}, "required": {"roles": ["user"]}, "permissions": {"view": ["admin", "user"], "edit": ["admin", "user"]}, "multivalued": False},
]


def con_organizacion_en_el_registro(realm):
    comps = realm.setdefault("components", {})
    lista = comps.setdefault("org.keycloak.userprofile.UserProfileProvider", [])
    if not lista:
        lista.append({"name": "declarative-user-profile", "providerId": "declarative-user-profile", "subComponents": {}, "config": {}})
    cfg = lista[0].setdefault("config", {})
    perfil = json.loads(cfg["kc.user.profile.config"][0]) if cfg.get("kc.user.profile.config") else {"attributes": list(PERFIL_BASE), "groups": [{"name": "user-metadata", "displayHeader": "User metadata", "displayDescription": "Attributes, which refer to user metadata"}]}
    if not any(a["name"] == "organizacion" for a in perfil["attributes"]):
        perfil["attributes"].append(ATRIBUTO_ORGANIZACION)
    cfg["kc.user.profile.config"] = [json.dumps(perfil, ensure_ascii=False)]
    for c in realm.get("clients", []):
        if c.get("clientId") == "rubix-consola":
            m = c.setdefault("protocolMappers", [])
            if not any(x.get("name") == "rubix-organizacion" for x in m):
                m.append(MAPEADOR_ORGANIZACION)
    return realm


def con_registro(realm):
    if realm["realm"] == REGISTRO_EN_PRODUCCION["realm"]:
        realm["registrationAllowed"] = REGISTRO_EN_PRODUCCION["registrationAllowed"]
        realm["verifyEmail"] = REGISTRO_EN_PRODUCCION["verifyEmail"]
        realm = con_organizacion_en_el_registro(realm)
    return realm


# ⭐ LA CONSOLA EN LOCAL ENTRA POR `rubix` (ver la cabecera): `rubix-consola` admite
#   tambien `http://localhost:3000`. Es un cliente publico con PKCE (S256), asi que
#   una URI de vuelta a localhost no entrega nada a nadie que no tenga ya el
#   navegador y el verificador: el codigo solo lo canjea quien lo pidio.
CONSOLA_LOCAL = "http://localhost:3000"


def con_consola_local(realm):
    if realm["realm"] != REGISTRO_EN_PRODUCCION["realm"]:
        return realm
    for c in realm.get("clients", []):
        if c.get("clientId") != "rubix-consola":
            continue
        uris = c.setdefault("redirectUris", [])
        if CONSOLA_LOCAL + "/auth/callback" not in uris:
            uris.append(CONSOLA_LOCAL + "/auth/callback")
        origenes = c.setdefault("webOrigins", [])
        if CONSOLA_LOCAL not in origenes:
            origenes.append(CONSOLA_LOCAL)
        attrs = c.setdefault("attributes", {})
        salidas = [s for s in attrs.get("post.logout.redirect.uris", "").split("##") if s]
        if CONSOLA_LOCAL + "/" not in salidas:
            salidas.append(CONSOLA_LOCAL + "/")
        attrs["post.logout.redirect.uris"] = "##".join(salidas)
    return realm


documentos = []
for f in PLANTILLAS:
    realm = json.loads(pathlib.Path(f).read_text(encoding="utf-8"))
    nombre = realm["realm"]
    # ⭐ A los tres y sin condicion: un token sin `sub` no sirve para nada, sea
    #   cual sea el realm.
    realm = con_sujeto(realm)
    if nombre in REALMS_CON_ORE:
        realm = con_ore(realm)
    realm = con_registro(realm)
    realm = con_consola_local(realm)
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
