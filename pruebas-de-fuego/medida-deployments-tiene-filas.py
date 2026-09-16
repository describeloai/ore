# -*- coding: utf-8 -*-
"""MEDIDA · Deployments tiene filas, antes de E3 (0027).

E3 es la consola cruzando dos verdades —lo que el árbol declara (`GET /modelos`)
y lo que el gateway cuenta (⑥)— y dejando de fingir: el Hub pinta la matriz de
certificación (⑦) y no un catálogo inventado; *Crear* llama a `POST /modelos`.
Esto mide dónde cae cada pieza con lo que hay hoy, y qué forma tiene cada dato:

    A  la consola hoy      qué pinta Models → Hub y → Deployments, y con qué habla
    B  lo que el árbol da  la ficha de `GET /modelos` (E1), y qué le falta para una fila
    C  lo que el gateway   `/admin/health` y `/admin/usage`: qué cuenta, y quién lo alcanza
       cuenta
    D  el cruce (⑥)        cómo sale `estado` de las dos verdades, y qué columna de la
                           consola no tiene dato detrás
    E  la identidad        con qué token escribe la consola, y quién queda de autor
    F  los pasos           lo que E3 construye, por orden, y qué acepta cada paso

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-deployments-tiene-filas.py [celda]

Es un METRO: lee la consola (`C:\\rubix-platform`), el gateway (`C:\\bastion`), este
árbol y la celda; acuña un token de agente para mirar `GET /modelos`, y lo tira.
"""
import json
import os
import re
import subprocess
import sys

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", "C:/rubix-platform")
BASTION = os.environ.get("BASTION", "C:/bastion")
PROYECTO = "project-8853a180-450d-47be-b83"
IDP = "https://login.paladio.io/realms/rubix"


def leer(*partes):
    p = os.path.join(*partes)
    try:
        with open(p, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def fila(k, v):
    print("     %-46s %s" % (k, v))


print()
print("  ═══ DEPLOYMENTS TIENE FILAS · antes de E3 · %s ═══" % CELDA)

# ── A · la consola hoy ───────────────────────────────────────────────────────
print("\n  A · LA CONSOLA HOY (%s)" % CONSOLA)
hub = leer(CONSOLA, "components/models/HubView.tsx")
dep = leer(CONSOLA, "components/models/DeploymentsView.tsx")
cat = leer(CONSOLA, "lib/models/catalogo.ts")
query = leer(CONSOLA, "lib/server/query.ts")
pagina = leer(CONSOLA, "app/(workspace)/clusters/[celda]/models/deployments/page.tsx")
if not hub:
    fila("consola", "✗ no está en %s (CONSOLA=…)" % CONSOLA)
else:
    fila("Models → Hub pinta", "`CATALOGO` estático: %d modelos, píldora %s — lo que 0027 retiró (mide vCPU, no perfiles)" % (
        cat.count("{ id: '"), re.findall(r"cabe: \{ texto: '([^']+)'", hub)[0] if re.search(r"cabe: \{ texto: '([^']+)'", hub) else "?"))
    fila("Models → Deployments pinta", "`despliegues: Despliegue[] = []` — «los dirá el snapshot v2 del informador»: vacía y dicha" if "const despliegues: Despliegue[] = []" in pagina else "?")
    campos = re.search(r"export interface Despliegue \{(.*?)\n\}", dep, re.S)
    campos = re.findall(r"^\s+(\w+)[?]?:", campos.group(1), re.M) if campos else []
    fila("la fila que espera (`Despliegue`)", ", ".join(campos))
    estados = re.search(r"estado: ('[^;]+);", dep)
    fila("sus estados", estados.group(1) if estados else "?")
    fila("Crear hoy", "formulario prerrellenado desde el Hub y botón deshabilitado con el motivo (`ore-serve POST /modelos` «por construir» — ya no)" if "deshabilitado" in dep else "?")
    rutas = re.findall(r"^\s+'?([\w-]+)'?: \{ metodo: '(\w+)', plano: '(\w+)', ruta: [^}]*?'([^']+)'", query, re.M)
    conoce = sorted({r[3] for r in rutas})
    fila("con qué habla (`lib/server/query.ts`)", "%d consultas: %s" % (len(rutas), ", ".join(conoce)))
    fila("conoce `/modelos`", "sí" if "/modelos" in query else "NO: E3 añade las consultas `modelos`, `modelo`, `perfiles` y los mandatos `crear-modelo`, `retirar-modelo`")
    fila("cómo llega al árbol", "por `entradaActual()`: la entrada pública de la celda (`https://<celda>.ore.paladio.io`), con el token de la sesión (persona)")

# ── B · lo que el árbol da ───────────────────────────────────────────────────
print("\n  B · LO QUE EL ÁRBOL DA (`GET /modelos`, E1)")
cli = sh("gcloud secrets versions access latest --secret=t-%s-agente-cliente --project=%s" % (CELDA, PROYECTO))
sec = sh("gcloud secrets versions access latest --secret=t-%s-agente-secreto --project=%s" % (CELDA, PROYECTO))
ficha = {}
if cli and sec:
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials", "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    try:
        tok = json.loads(r.stdout)["access_token"]
        r = subprocess.run(["curl", "-s", "-m", "15", "-H", "authorization: Bearer " + tok, "https://%s.ore.paladio.io/modelos" % CELDA], capture_output=True, text=True, encoding="utf-8")
        lista = json.loads(r.stdout).get("modelos", [])
        ficha = lista[0] if lista else {}
        fila("`GET /modelos` en %s" % CELDA, "%d modelo(s)" % len(lista))
    except (ValueError, KeyError):
        fila("`GET /modelos` en %s" % CELDA, "✗ " + (r.stdout or "")[:100])
if ficha:
    fila("la ficha (claves)", ", ".join(sorted(ficha)))
    fila("la ficha (valores)", json.dumps({k: ficha[k] for k in ("name", "profile", "model", "tier", "task", "url", "certificado") if k in ficha}, ensure_ascii=False))
fila("lo que ya cubre de la fila", "nombre ← name · modelo ← model · tarea ← task · recursos ← profile (máquina) · endpoint ← url (interno; público es E4)")
fila("lo que NO tiene y la fila pide", "**estado** (¿está sirviendo?), **listas/deseadas** (backends `up` que sirven ese id), **creadoPor/modificado** (el autor y la fecha del commit de `modelos/<n>.yaml`), **uso** (tokens, $)")
fila("de dónde sale el autor", "`escribiendo(sujeto, …)` ya firma el commit con quien pidió: `git log -1 --format=%an|%aI -- modelos/<n>.yaml` en el clon")

# ── C · lo que el gateway cuenta ─────────────────────────────────────────────
print("\n  C · LO QUE EL GATEWAY CUENTA (Bastion B3, plano de control)")
admin = leer(BASTION, "crates/bastion-gateway/src/admin.rs")
db = leer(BASTION, "crates/bastion-gateway/src/db.rs")
rutas_gw = re.findall(r'\.route\("([^"]+)", (\w+)\(', admin)
fila("rutas", " · ".join("%s %s" % (m.upper(), r) for r, m in rutas_gw))
fila("`/admin/health`", "backends[{id, model, sovereignty, inflight, up}] · tenants{<celda>: {inflight, spend_usd_month, tokens_last_minute}}")
cols = re.search(r"UsageAgg \{ (.*?) \}", db)
fila("`/admin/usage?tenant=&from=&to=`", "filas por día × tenant × modelo: " + (", ".join(re.findall(r"(\w+): r\.get", cols.group(1))) if cols else "?"))
fila("lo que NO cuenta por celda", "latencia/TTFT (sólo en `/metrics`, agregado); `estado` por modelo es `backends[].up` del id servido")
fila("quién lo alcanza", "sólo la VPC: `ore-serve` (rol `control` → MODELOS:9000, E2 I1); la consola NO — el cruce lo hace `ore-serve`")

# ── D · el cruce ─────────────────────────────────────────────────────────────
print("\n  D · EL CRUCE (⑥): las dos verdades y una fila")
for k, v in [
    ("declarado y un backend `up` sirve su `model`", "**running** — listas = nº de backends up con ese id, deseadas = 1 (un pod por modelo, de la plataforma)"),
    ("declarado y ningún backend up", "**provisioning** (la máquina no está, o carga) — es lo que pasa hoy con `modelos-e0` apagada"),
    ("declarado y el gateway no contesta", "**error** con el motivo (timeout 15 s): `estado: {fase: error, motivo}`"),
    ("suscrito en el gateway y no declarado", "**retiring** — sólo por deriva: los verbos retiran las dos cosas a la vez"),
    ("uso", "`/admin/usage?tenant=<celda>` filtrado por `model` = el id del perfil: requests, tokens, usd; hoy vs 30 días"),
]:
    fila(k, v)
fila("dónde se hace", "en `ore-serve GET /modelos` y `GET /modelos/{n}`: pregunta al gateway con `http::pedir` (plazo 15 s) y devuelve `estado` y `uso` dentro de la ficha; si el gateway no contesta, la ficha sale igual con `estado: error` — la consola nunca espera al gateway")
fila("columnas de `Despliegue` sin dato detrás", "`recursos` (la consola lo pinta desde el perfil: máquina y GPUs, de `perfiles.json`) · `endpoint.publico/claves` (E4: hoy `{publico: false, claves: 0}` y la url interna)")

# ── E · la identidad ─────────────────────────────────────────────────────────
print("\n  E · LA IDENTIDAD")
fila("con qué escribe la consola", "el token de la PERSONA (aud `ore-serve`, `rubix_tipo: persona`): `POST /modelos` firma el commit con ella — «el Model está en el árbol con el autor de la sesión» sale gratis")
fila("la suscripción", "la hace `ore-serve` en el gateway (E1 I3), no la consola: la consola no sabe de `10.10.0.100`")
fila("quién puede crear", "hoy, quien tenga sesión en la celda (como `POST /fuentes`); un permiso propio (`modelo:crear`) es de `iam` y no de E3")

# ── F · los pasos ────────────────────────────────────────────────────────────
print("\n  F · LOS PASOS DE E3")
for n, que, acepta in [
    ("I1", "`ore-serve`: `GET /perfiles` (la lista de la cola, tal cual) · `GET /modelos` y `/modelos/{n}` ganan `estado {fase, backends, motivo?}`, `uso {hoy, mes: {requests, tokens, usd}}`, `autor`, `desde`",
     "`los-modelos.sh`: con el gateway de banco sirviendo el id → `running`; sin backend → `provisioning`; gateway caído → `error` y la ficha entera; el autor es el sujeto"),
    ("I2", "la consola: consultas `perfiles`, `modelos`, `modelo` y mandatos `crear-modelo`, `retirar-modelo` en `query.ts`; el Hub pinta `perfiles.json` (Certified · máquina · tok/s · $/M · Por certificar) y retira `CATALOGO`",
     "el Hub enseña 3 perfiles y ninguna píldora en vCPU; el banco (`BANCO`) tiene sus datos"),
    ("I3", "Deployments con filas: `GET /modelos` → `Despliegue` (estado del cruce, listas/deseadas, recursos del perfil, endpoint interno, autor, fecha); *Use in this cluster* / *Create* → `POST /modelos`; los 422 tal cual",
     "desde el Hub, *Use in this cluster* → fila *provisioning* con la hora → *running* con la máquina encendida; el `Model` en el árbol con el autor de la sesión"),
    ("I4", "el detalle: *Overview* (el documento y su commit) · *Endpoint* (url interna, id servido; lo público es E4) · *Usage* (hoy / 30 días); *Retirar* → `DELETE`, y el 409 «alguien lo nombra» en pantalla",
     "retirar con una Function nombrándolo enseña el 409 y no toca nada; sin nadie, la fila desaparece y el Job recibe 401"),
    ("I5", "la aceptación en `victor` con `modelos-e0` encendida: crear desde la consola, un Job de la celda consume, *Usage* enseña los tokens contados",
     "la tabla de la ADR, y ninguna píldora inventada en ninguna pantalla"),
]:
    fila(n + " · " + que.split(":")[0], que.split(":", 1)[1].strip() if ":" in que else que)
    fila("    acepta", acepta)
print()
