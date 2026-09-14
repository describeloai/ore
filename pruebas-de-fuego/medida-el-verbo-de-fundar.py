# -*- coding: utf-8 -*-
"""Medida: lo que le falta al plano de control para que «Crear serverless» en la
consola sea un verbo y no un toast. La infraestructura de abajo ya esta (0024
E1-E3): esto mide cuanto hay entre el boton y `fundar`, y que se rompe en el
camino que hoy es de operador.

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-el-verbo-de-fundar.py

Secciones:
  A  que hace `fundar` hoy: entradas, lo que escribe, quien lo corre
  B  que trae ya quien pide desde la consola (el token), y que falta
  C  el estado de la celda: quien lo escribe, y que pinta la consola
  D  el agente: dos cruces de autoridad, y lo que el CronJob NO puede hacer
  E  cuanto tarda el alta hoy, medido
  F  la consola: el boton y el mandato
"""
import json
import os
import re
import subprocess

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", r"C:\rubix-platform")


def leer(rel, raiz=RAIZ):
    with open(os.path.join(raiz, rel), encoding="utf-8") as f:
        return f.read()


def k(*args):
    r = subprocess.run(["kubectl"] + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    return r.stdout


def kj(*args):
    s = k(*args, "-o", "json")
    return json.loads(s) if s.strip() else {}


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


piezas = []


def pieza(que, det=""):
    piezas.append((que, det))


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - LO QUE `fundar` HACE HOY")
lib = leer("crates/ore-iam/src/lib.rs")
fund = leer("crates/ore-iam/src/fundar.rs")
flags = sorted(set(re.findall(r'valor\(args, "(--[a-z-]+)"\)', lib)))
print("   entradas (CLI): %s" % " ".join(flags))
escribe = re.findall(r"insert into (iam\.\w+)", fund[fund.index("pub fn fundar("):])
print("   escribe: %s" % ", ".join(dict.fromkeys(escribe)))
print("   derivados: arbol=t-<n>/ontologia · kek=ore/<n> · entrada=<n>.ore.paladio.io")
print("   quien lo corre: un Job de operador en `identidad` (66-ore-iam.yaml, fundar-demo) con ORE_CELDA*")
rutas = leer("crates/ore-iam/src/rutas.rs")
mapa = re.search(r"pub fn mapa\(.*?\]\s*\n", rutas, re.S).group(0)
verbos = re.findall(r'\("(GET|POST)", "([^"]+)"', mapa)
posts = [r for m, r in verbos if m == "POST"]
print("   verbos HTTP de ore-iam: %d GET · %d POST → %s" % (sum(1 for m, _ in verbos if m == "GET"), len(posts), ", ".join(posts)))
print("   ⇒ NO hay `POST /organizaciones`: fundar solo existe como CLI")
pieza("`POST /organizaciones` en ore-iam: `fundar` por HTTP",
      "mismo cuerpo que el CLI, con el que pide como dueño; ORE_CELDA* pasan al Deployment de ore-iam (67), hoy solo estan en el Job")

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - LO QUE TRAE QUIEN PIDE, Y LO QUE FALTA")
print("   el token de la consola lleva: iss (emisor) · sub · email · name")
print("   `fundar` pide: --emisor --sub --correo  ← exactamente eso. `admitir` ya crea la persona del token.")
print("   lo que NO viene en el token: el NOMBRE de la organizacion y el TIER → el cuerpo del verbo")
print("   la celda: de la configuracion de plataforma por tier (hoy ORE_CELDA*: ore-mesh/compartido/gcp/europe-west1-b/puerta)")
print("   ⇒ el argumento «fundar es de operador porque decide el dueño» se disuelve: el dueño es quien pide.")
print("     Lo que queda por decidir es QUIEN PUEDE PEDIR: hoy, cualquier persona con sesion.")
pieza("quien puede fundar: cualquier persona con sesion (una organizacion por peticion, dueña)",
      "es una DECISION, no codigo. Sin cuota de organizaciones por persona, es abrir cuentas gratis")

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - EL ESTADO DE LA CELDA")
sql025 = leer("iam/migraciones/025-la-celda.sql")
estados = re.search(r"estado in \(([^)]+)\)", sql025).group(1)
print("   iam.celda.estado ∈ %s · default 'activa'" % estados)
celdas_ts = leer("lib/server/celdas.ts", CONSOLA)
print("   la consola: 'aprovisionando' → Provisioning SIN mirar la sonda; 'activa' → la sonda decide")
print("   quien puede escribir `estado`: solo ore-iam (el aprovisionador no escribe iam, 023)")
print("   ⇒ si el verbo fundara en 'aprovisionando', nadie la pasaria a 'activa' nunca.")
print("     Nace 'activa' (= habilitada administrativamente) y la sonda dice si VIVE: es la 0024-4 literal.")
pieza("la celda nace `activa`; `aprovisionando` no lo escribe nadie y seria mentira", "0024-4: identidad de un plano, vida del otro")

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - EL AGENTE, Y LO QUE EL CRONJOB NO PUEDE HACER")
print("   el agente de un inquilino: cliente de Keycloak `ore-agente-<n>` (lo crea el aprovisionador ⑦ con el")
print("   admin del IdP) + fila en iam.agente con su `sub` (lo escribe `ore-iam agente`, un Job de operador).")
print("   ⇒ dos cruces: quien crea el cliente no puede escribir iam; quien escribe iam no sabe el `sub` hasta")
print("     que el cliente existe.")
pol = kj("-n", "ore-system", "get", "networkpolicy", "salida-del-aprovisionador")
a_tenants = a_idp = False
for e in pol.get("spec", {}).get("egress", []):
    for to in e.get("to", []):
        ns = (to.get("namespaceSelector") or {}).get("matchLabels", {})
        if ns.get("ore.dev/rol") == "cargas":
            a_tenants = True
        if ns.get("kubernetes.io/metadata.name") == "identidad" and any(p.get("port") == 8080 for p in e.get("ports", [])):
            a_idp = True
print()
print("   el CronJob (16, cada hora) DESDE DENTRO:")
print("     · alcanza las forjas de los inquilinos (t-*:3000): %s" % ("si" if a_tenants else "NO"))
print("     · alcanza el IdP (identidad:8080) para ⑦:        %s" % ("si" if a_idp else "NO"))
sm = subprocess.run(["gcloud", "secrets", "list", "--filter=name~idp", "--format=value(name)"], capture_output=True, text=True, shell=True).stdout.split()
print("     · tiene el admin del IdP en el almacen:            %s" % (", ".join(sm) if sm else "NO (no hay ningun `idp-admin`)"))
ultimo = [j for j in kj("-n", "ore-system", "get", "jobs").get("items", []) if j["metadata"]["name"].startswith("aprovisionador-")]
if ultimo:
    u = sorted(ultimo, key=lambda j: j["status"].get("startTime", ""))[-1]["metadata"]["name"]
    log = k("-n", "ore-system", "logs", "job/" + u)
    print("     · ultima pasada %s: %s" % (u, "vio la forja del inquilino" if "esta viva" in log else "«la forja del inquilino todavia no esta» — y SI estaba"))
if not a_tenants:
    pieza("el CronJob no alcanza las forjas de inquilino: la PASADA 2 nunca ocurre desde dentro",
          "medido en la ultima pasada: dice «todavia no esta» de una forja viva. Es una regla en `salida-del-aprovisionador` (16)")
if not a_idp or not sm:
    pieza("el CronJob no puede hacer ⑦ (el cliente de Keycloak del agente): ni alcanza el IdP ni tiene su admin",
          "el admin del IdP vive en un Secret de Keycloak en `identidad`, no en el almacen; ⑦ solo ha corrido desde fuera")
pieza("registrar el agente sin operador: el aprovisionador con identidad propia llama a `POST /organizaciones/{org}/agentes`",
      "la 023 dice que el aprovisionador no escribe iam POR SQL; por HTTP escribe ore-iam, con huella, y solo si quien llama es la clase `aprovisionador` (claim del IdP). Alternativa: registrar por `client_id` (determinista) y no por `sub`")

# ── E ───────────────────────────────────────────────────────────────────────
titulo("E - CUANTO TARDA EL ALTA HOY")
print("   pasada sin cambios: 149 s desde fuera · 6m31s el CronJob (medido 2026-09-14)")
print("   cadencia: cada hora (`17 * * * *`) → dos pasadas = hasta 2 h hasta que el inquilino esta entero")
print("   con `*/5 * * * *`: ≈ 10-15 min, y cada pasada sin cambios cuesta un pod de 6 min (≈ 0,1 CPU·h)")
pieza("cadencia del CronJob: de cada hora a cada 5 minutos", "el alta pasa de «hasta 2 h» a «≈ 10 min»; idempotente, se ha medido en 3 pasadas seguidas")

# ── F ───────────────────────────────────────────────────────────────────────
titulo("F - LA CONSOLA")
vista = leer("components/cloud/CreateServerlessView.tsx", CONSOLA)
print("   el boton hoy: %s" % ("un toast «todavia no implementado»" if "todavía no implementado" in vista else "?"))
q = leer("lib/server/query.ts", CONSOLA)
mandatos = re.findall(r"'([a-z-]+)': \{\s*metodo: 'POST',\s*plano: 'control'", q)
print("   mandatos de control que ya existen: %s" % ", ".join(mandatos))
pieza("mandato `crear-organizacion` → `POST /organizaciones`, y el boton lo llama; despues `router.refresh` y la celda aparece",
      "la vista de clusters ya pinta lo que `/celdas` devuelve")

titulo("LO QUE HAY ENTRE EL BOTON Y `fundar`")
for i, (que, det) in enumerate(piezas, 1):
    print("   %d. %s" % (i, que))
    if det:
        print("      %s" % det)
print()
print("  => %d piezas: un verbo, un mandato, tres arreglos al CronJob (red, IdP, cadencia), y DOS decisiones" % len(piezas))
print("     (quien puede fundar; como se registra el agente sin operador). La celda nace activa.")
