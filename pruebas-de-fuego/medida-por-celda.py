# -*- coding: utf-8 -*-
"""LA PUERTA DE LA E4 (0025): el aprovisionador y el renderizador toman la CELDA, y
con `demo` y `prueba` —celdas que se llaman como su organizacion— **lo rendido es
byte a byte lo de antes**.

  A · el renderizador VIEJO (un commit de git) y el NUEVO, para cada celda viva:
      mismos ficheros, mismos bytes. Y el nuevo con `--organizacion otra` cambia
      SOLO lo que es de la cuenta (ore-serve --organizacion, ore init --name).
  B · el aprovisionador viejo y el nuevo EN SECO, para cada celda: la misma salida
      salvo la linea `organizacion: <org>` que el nuevo añade. (Lento: ~2 min por
      celda y version. `--sin-seco` lo salta.)
  C · el CronJob (16): cada 5 min; salida a las forjas de las celdas (t-*:3000) y
      al IdP (identidad:8080); trae `idp-admin`; y el IdP lo deja entrar (60).
  D · el almacen: `idp-admin` existe, con version, y solo lo lee el aprovisionador.
  E · en vivo: la ultima pasada del CronJob vio la forja de cada celda y pudo
      hacer ⑦ (o dice por que no).

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-por-celda.py [--viejo REF] [--sin-seco] [--json]
"""
import difflib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COMO_JSON = "--json" in sys.argv
SIN_SECO = "--sin-seco" in sys.argv
VIEJO = sys.argv[sys.argv.index("--viejo") + 1] if "--viejo" in sys.argv else "20017d4"
PY = sys.executable
# ⛔ `bash` a secas desde Python en Windows es el de WSL (System32), que no existe
#   como distro: hay que nombrar el de Git. Fuera de Windows, el del PATH.
BASH = next((b for b in (r"C:\Program Files\Git\bin\bash.exe", r"C:\Program Files\Git\usr\bin\bash.exe") if os.path.exists(b)), "bash")
FALLOS = []


def correr(*args, **kw):
    # ⛔ Con `shell=True` en Windows la orden va como UNA cadena: en lista, Python la
    #   entrecomilla entera y cmd no la encuentra.
    orden = " ".join(args) if kw.get("shell") else list(args)
    r = subprocess.run(orden, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       cwd=kw.get("cwd", RAIZ), timeout=kw.get("timeout", 600), shell=kw.get("shell", False))
    return r.stdout, r.stderr, r.returncode


def sql(q):
    return correr("kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q)[0].strip()


def kj(*args):
    s = correr("kubectl", *args, "-o", "json")[0]
    return json.loads(s) if s.strip() else {}


def titulo(t):
    if not COMO_JSON:
        print("\n" + "═" * 78 + "\n " + t + "\n" + "═" * 78)


def di(s):
    if not COMO_JSON:
        print(s)


def falla(que):
    FALLOS.append(que)
    di("   ✗ " + que)


# ── el arbol viejo, de git ────────────────────────────────────────────────────
TMP = tempfile.mkdtemp(prefix="e4-")
viejo_tar = os.path.join(TMP, "viejo.tar")
with open(viejo_tar, "wb") as f:
    subprocess.run(["git", "archive", VIEJO, "malla"], stdout=f, cwd=RAIZ, check=True)
with tarfile.open(viejo_tar) as t:
    t.extractall(os.path.join(TMP, "viejo"))
GEN_VIEJO = os.path.join(TMP, "viejo", "malla", "gen-inquilino.py")
GEN_NUEVO = os.path.join(RAIZ, "malla", "gen-inquilino.py")
APROV_VIEJO = os.path.join(TMP, "viejo", "malla", "aprovisionar-inquilino.sh")
APROV_NUEVO = os.path.join(RAIZ, "malla", "aprovisionar-inquilino.sh")

celdas = [l.split("|") for l in sql("select celda, organizacion, arbol from iam.celda_de order by celda").splitlines() if l]
resultado = {"viejo": VIEJO, "celdas": [c[0] for c in celdas]}

# ── A ─────────────────────────────────────────────────────────────────────────
titulo("A · EL RENDERIZADOR — viejo %s contra el de este arbol, por celda" % VIEJO)


def rendir(gen, celda, extra=()):
    d = os.path.join(TMP, "r-%s-%s-%d" % (os.path.basename(os.path.dirname(os.path.dirname(gen))), celda, len(extra)))
    out, err, rc = correr(PY, gen, celda, *extra, "--a", d)
    if rc != 0:
        return None, err
    return {f: open(os.path.join(d, f), encoding="utf-8").read() for f in sorted(os.listdir(d))}, ""


a_ok = True
for celda, org, arbol in celdas:
    v, ev = rendir(GEN_VIEJO, celda, ("--arbol", arbol))
    n, en = rendir(GEN_NUEVO, celda, ("--organizacion", org, "--arbol", arbol))
    if v is None or n is None:
        falla("%s: no rinde (%s%s)" % (celda, ev, en)); a_ok = False; continue
    if set(v) != set(n):
        falla("%s: ficheros distintos %s / %s" % (celda, sorted(set(v) - set(n)), sorted(set(n) - set(v)))); a_ok = False; continue
    dist = [f for f in v if v[f] != n[f]]
    if dist:
        a_ok = False
        for f in dist:
            falla("%s/%s difiere:" % (celda, f))
            for l in list(difflib.unified_diff(v[f].splitlines(), n[f].splitlines(), lineterm="", n=0))[:12]:
                di("       " + l)
    else:
        di("   %-8s %d ficheros · %d bytes · byte a byte los mismos" % (celda, len(v), sum(len(x) for x in v.values())))

# y con OTRA organizacion: solo cambia lo de la cuenta
n1, _ = rendir(GEN_NUEVO, "prueba", ("--arbol", "t-prueba/ontologia"))
n2, _ = rendir(GEN_NUEVO, "prueba", ("--organizacion", "acme", "--arbol", "t-prueba/ontologia"))
lineas = []
if n1 and n2:
    for f in n1:
        for l in difflib.unified_diff(n1[f].splitlines(), n2[f].splitlines(), lineterm="", n=0):
            if l.startswith(("+", "-")) and not l.startswith(("+++", "---")):
                lineas.append("%s: %s" % (f, l.strip()))
di("   con --organizacion acme para la celda prueba cambian %d lineas:" % len(lineas))
for l in lineas:
    di("       " + l)
solo_cuenta = all(("--organizacion" in l or "- acme" in l or "- prueba" in l or "--name" in l or "organizacion" in l) for l in lineas)
if not solo_cuenta or not lineas:
    falla("con otra organizacion cambia algo que no es de la cuenta (o no cambia nada)")
resultado["A"] = {"byte_a_byte": a_ok, "lineas_de_la_cuenta": lineas}

# ── B ─────────────────────────────────────────────────────────────────────────
titulo("B · EL APROVISIONADOR EN SECO — la misma salida, mas `organizacion:`")
resultado["B"] = {}
if SIN_SECO:
    di("   (saltado: --sin-seco)")
else:
    for celda, org, arbol in celdas:
        sv, _, rv = correr(BASH, APROV_VIEJO, celda, "--seco", timeout=900)
        sn, _, rn = correr(BASH, APROV_NUEVO, celda, "--seco", timeout=900)
        def quita(s):  # sin `organizacion:` (nueva) y sin el bloque ⑧ (la E4 escribe el CNAME a proposito)
            fuera, out = False, []
            for l in s.splitlines():
                if "⑧" in l: fuera = True
                if "⑨" in l: fuera = False
                if fuera or "organizacion: " in l: continue
                out.append(re.sub(r"/tmp/tmp\.\w+|\S+e4-\w+\S*", "<tmp>", l))
            return out
        d = [l for l in difflib.unified_diff(quita(sv), quita(sn), lineterm="", n=0) if l.startswith(("+", "-")) and not l.startswith(("+++", "---"))]
        # lineas que cambian de texto a proposito (sin efecto): se listan para leerlas
        di("   %-8s viejo rc=%d %d lineas · nuevo rc=%d %d lineas · difieren %d" % (celda, rv, len(sv.splitlines()), rn, len(sn.splitlines()), len(d)))
        for l in d[:10]:
            di("       " + l)
        ok = rv == rn and len(d) == 0 and ("organizacion: %s" % org) in sn
        if not ok:
            falla("%s: la salida en seco no es la de antes (mas `organizacion: %s`)" % (celda, org))
        resultado["B"][celda] = {"rc_viejo": rv, "rc_nuevo": rn, "difieren": d}

# ── C ─────────────────────────────────────────────────────────────────────────
titulo("C · EL CRONJOB (16) Y LA ENTRADA DEL IDP (60) — en git y en vivo")
cj = kj("-n", "ore-system", "get", "cronjob", "aprovisionador")
sched = cj.get("spec", {}).get("schedule")
inits = cj.get("spec", {}).get("jobTemplate", {}).get("spec", {}).get("template", {}).get("spec", {}).get("initContainers", [])
trae_idp = any("idp-admin" in " ".join(i.get("args", [])) for i in inits)
env = {e["name"]: e.get("value") for c in cj.get("spec", {}).get("jobTemplate", {}).get("spec", {}).get("template", {}).get("spec", {}).get("containers", []) for e in c.get("env", [])}
pol = kj("-n", "ore-system", "get", "networkpolicy", "salida-del-aprovisionador")
a_tenants = a_idp = False
for e in pol.get("spec", {}).get("egress", []):
    for to in e.get("to", []):
        ns = (to.get("namespaceSelector") or {}).get("matchLabels", {})
        if ns.get("ore.dev/rol") == "cargas" and any(p.get("port") == 3000 for p in e.get("ports", [])):
            a_tenants = True
        if ns.get("kubernetes.io/metadata.name") == "identidad" and any(p.get("port") == 8080 for p in e.get("ports", [])):
            a_idp = True
ent = kj("-n", "identidad", "get", "networkpolicy", "entrada-al-idp")
deja_entrar = any((f.get("namespaceSelector") or {}).get("matchLabels", {}).get("kubernetes.io/metadata.name") == "ore-system"
                  for i in ent.get("spec", {}).get("ingress", []) for f in i.get("from", []))
git16 = open(os.path.join(RAIZ, "malla", "16-el-aprovisionador.yaml"), encoding="utf-8").read()
en_git = ('schedule: "*/5 * * * *"' in git16, "--secret=idp-admin" in git16, "ore.dev/rol: cargas" in git16)
di("   en git (16):   cada 5 min: %s · trae idp-admin: %s · salida a t-*: %s" % en_git)
di("   en vivo:       schedule %s · trae idp-admin: %s · IDP_ADMIN_USER=%s" % (sched, trae_idp, env.get("IDP_ADMIN_USER")))
di("                  salida a t-*:3000: %s · salida a identidad:8080: %s · el IdP deja entrar a ore-system: %s" % (a_tenants, a_idp, deja_entrar))
resultado["C"] = {"git": en_git, "schedule": sched, "trae_idp": trae_idp, "usuario": env.get("IDP_ADMIN_USER"),
                  "a_tenants": a_tenants, "a_idp": a_idp, "deja_entrar": deja_entrar}
if not all(en_git):
    falla("el 16 de git no lleva los tres arreglos")
if sched != "*/5 * * * *" or not (trae_idp and a_tenants and a_idp and deja_entrar):
    di("   ~ en vivo todavia no (Flux tarda hasta 5 min tras el push): se cuenta como pendiente, no como fallo")
    resultado["C"]["vivo"] = False
else:
    resultado["C"]["vivo"] = True

# ── D ─────────────────────────────────────────────────────────────────────────
titulo("D · EL ALMACEN — `idp-admin`")
desc, _, rc = correr("gcloud secrets describe idp-admin --format=json", shell=True)
if rc != 0:  # gcloud en Windows a veces falla por el cerrojo de su base de credenciales: una vez mas
    desc, _, rc = correr("gcloud secrets describe idp-admin --format=json", shell=True)
if rc != 0:
    falla("no hay secreto `idp-admin` en el almacen (68-el-admin-del-aprovisionador.sh)")
    resultado["D"] = {"existe": False}
else:
    vers = correr('gcloud secrets versions list idp-admin --filter=state=enabled --format=value(name)', shell=True)[0].split()
    pol, _, _ = correr("gcloud secrets get-iam-policy idp-admin --format=json", shell=True)
    miembros = sorted(m for b in json.loads(pol or "{}").get("bindings", []) for m in b.get("members", []))
    di("   existe · %d version(es) activa(s) · lectores: %s" % (len(vers), ", ".join(miembros) or "nadie"))
    if not vers:
        falla("`idp-admin` no tiene ninguna version")
    if miembros != ["serviceAccount:ore-aprovisionador@project-8853a180-450d-47be-b83.iam.gserviceaccount.com"]:
        falla("`idp-admin` lo lee alguien mas (o nadie): %s" % miembros)
    resultado["D"] = {"existe": True, "versiones": len(vers), "lectores": miembros}

# ── E ─────────────────────────────────────────────────────────────────────────
titulo("E · LA ULTIMA PASADA DEL CRONJOB")
jobs = [j for j in kj("-n", "ore-system", "get", "jobs").get("items", []) if j["metadata"]["name"].startswith("aprovisionador-")]
resultado["E"] = {}
if not jobs:
    di("   (ninguna pasada registrada)")
else:
    u = sorted(jobs, key=lambda j: j["status"].get("startTime", ""))[-1]
    nombre = u["metadata"]["name"]
    log = correr("kubectl", "-n", "ore-system", "logs", "job/" + nombre, "--all-containers", timeout=120)[0]
    vio = log.count("esta viva")
    sin_idp = "sin `IDP_ADMIN_PASS`" in log
    agente = log.count("agente `ore-agente-")
    org = re.findall(r"organizacion: (\S+)", log)
    ok = u["status"].get("succeeded", 0) >= 1
    estado = "OK" if ok else ("EN CURSO" if u["status"].get("active") else "FALLO")
    errores = len(re.findall(r"^ERROR:", log, re.M))
    di("   %s · %s · inicio %s · lineas ERROR de gcloud: %d" % (nombre, estado, u["status"].get("startTime"), errores))
    for l in re.findall(r"^ERROR:.*", log, re.M)[:6]:
        di("       " + l[:150])
    di("   vio la forja de la celda: %d veces · organizacion leida: %s · ⑦ sin admin del IdP: %s · agentes resueltos: %d"
       % (vio, org, sin_idp, agente))
    resultado["E"] = {"job": nombre, "estado": estado, "vio_forjas": vio, "orgs": org, "sin_idp": sin_idp, "agentes": agente, "errores_gcloud": errores}

shutil.rmtree(TMP, ignore_errors=True)
resultado["fallos"] = FALLOS
if COMO_JSON:
    print(json.dumps(resultado, ensure_ascii=False, indent=1))
else:
    print("\n" + ("✓ la E4 pasa su puerta" if not FALLOS else "✗ %d fallo(s):\n   · %s" % (len(FALLOS), "\n   · ".join(FALLOS))))
sys.exit(1 if FALLOS else 0)
