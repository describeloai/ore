# -*- coding: utf-8 -*-
"""MEDIDA · el modelo en el árbol, antes de escribir E1 (0027).

E0 dejó la forma; E1 la escribe: `kind: Model` en `oos`, los tres verbos de
`ore-serve` (`POST /modelos` · `GET /modelos` · `DELETE /modelos/{n}`) que escriben
el árbol y provisionan la suscripción en el gateway EN EL MISMO ACTO, la lista de
perfiles como fichero que deja el aprovisionador, y `modelo/<n>` resuelto por
`ore-serve`. Esto mide el terreno donde cada pieza cae, con lo que hay hoy:

    A  la gramática     cuántos `kind` hay, dónde vive uno, qué dijo `ore` a `kind: Model`
    B  los verbos       la figura de `POST /fuentes` (clonar → escribir → empujar → derivar), y qué le falta
    C  los perfiles     lo que Bastion mide hoy y la forma del fichero; dónde deja ficheros el aprovisionador
    D  la red           si `ore-serve` (rol `control`) alcanza HOY el plano de control del gateway
    E  la resolución    de `modelo/v2-lite` a `(url, id)`: qué lo dio en E0 y qué lo dará
    F  la aceptación    los casos de ⑦ contra la forma de `los-verbos.sh`

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-el-modelo-en-el-arbol.py [celda]

Es un METRO: lee código, la malla y la celda; no escribe nada.
"""
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
NS = "t-" + CELDA
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASTION = os.environ.get("BASTION", "C:/bastion" if os.name == "nt" else os.path.expanduser("~/bastion"))
MODELOS = "10.10.0.100"


def leer(rel):
    with open(os.path.join(RAIZ, rel), encoding="utf-8") as f:
        return f.read()


def correr(*args, entrada=None, cwd=None):
    r = subprocess.run(list(args), input=entrada, capture_output=True, text=True, encoding="utf-8", cwd=cwd,
                       env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return r.returncode, (r.stdout or ""), (r.stderr or "")


def fila(k, v):
    print("     %-46s %s" % (k, v))


print()
print("  ═══ EL MODELO EN EL ÁRBOL · antes de E1 · celda %s ═══" % CELDA)

# ── A · la gramática ─────────────────────────────────────────────────────────
print("\n  A · LA GRAMÁTICA")
doc = leer("crates/ore-core/src/document.rs")
kinds = re.findall(r'Kind::(\w+) => "\1"', doc)
fila("`kind` que el núcleo conoce", "%d: %s" % (len(kinds), ", ".join(kinds)))
claves_fn = re.search(r"Kind::Function => &\[(.*?)\]", doc, re.S)
claves_fn = re.findall(r'"([a-zA-Z]+)"', claves_fn.group(1)) if claves_fn else []
fila("claves de `Function.spec` (cerradas)", ", ".join(claves_fn))
fila("`runtime` se comprueba", "no — ningún literal `wasm` en el núcleo (E0 pasó `runtime: model`)")
fila("extensiones admitidas", "`x-<proveedor>-<clave>` (E0 metió el prompt en `x-ore-prompt`)")
# qué costó el último kind (Table, v1alpha8) en el submódulo
rc, out, _ = correr("git", "-C", os.path.join(RAIZ, "vendor/oos"), "log", "--format=%h", "--diff-filter=A", "--", "spec/v1alpha8/01-table.md")
if rc == 0 and out.strip():
    h = out.strip().splitlines()[-1]
    rc, stat, _ = correr("git", "-C", os.path.join(RAIZ, "vendor/oos"), "show", "--stat", "--format=", h)
    resumen = [l for l in stat.splitlines() if "files changed" in l]
    fila("lo que costó `Table` en `oos` (v1alpha8, %s)" % h, resumen[0].strip() if resumen else "?")
    fila("  piezas de un `kind` en el submódulo", "spec/<v>/NN-<kind>.md · schemas/<v>/<kind>.schema.json · conformance/<v>/{valid,invalid,diff}")
fila("  piezas de un `kind` en el núcleo", "`Kind::Model` + su `apiVersion` + claves de `spec` + reglas de forma + códigos `OOSxxxx` + `diff`")
# qué dice `ore` hoy a un kind: Model
ore = shutil.which("ore") or os.path.join(RAIZ, "target/release/ore.exe")
if os.path.exists(ore):
    d = tempfile.mkdtemp(prefix="ore-e1-")
    rc, out, err = correr(ore, "init", "--name", "e1", d, cwd=d)
    os.makedirs(os.path.join(d, "modelos"), exist_ok=True)
    with open(os.path.join(d, "modelos", "v2-lite.yaml"), "w", encoding="utf-8") as f:
        f.write("apiVersion: oos.dev/v1alpha8\nkind: Model\nmetadata: { name: v2-lite }\nspec:\n  profile: g1/deepseek-v2-lite\n  tier: shared\n  task: chat\n")
    rc, out, err = correr(ore, "validate", d)
    fila("`ore validate` con `kind: Model` hoy", "rc %d · %s" % (rc, (err or out).strip().splitlines()[0][:110] if (err or out).strip() else "(nada)"))
    shutil.rmtree(d, ignore_errors=True)

# ── B · los verbos ───────────────────────────────────────────────────────────
print("\n  B · LOS VERBOS DE `ore-serve`")
rutas = leer("crates/ore-serve/src/rutas.rs")
tabla = re.findall(r'\("(GET|POST|DELETE|PUT)", "(/[^"]*)"', rutas)
fila("rutas hoy", "%d: %s" % (len(tabla), " · ".join("%s %s" % t for t in tabla)))
fila("la figura que escribe", "`escribiendo(sujeto, mensaje, f)`: clona la forja → f(raíz) → si hay cambios, commit con quién pidió y push (409 si alguien se adelantó)")
fila("`POST /fuentes` deriva EN EL MISMO ACTO", "el documento (`ore source add`) + la credencial al cofre (`http::pedir`) + el Job a la cola; si la credencial falla: 502 «quedó declarada, pero…»")
rc, out, _ = correr(ore, "--help") if os.path.exists(ore) else (1, "", "")
verbos = [l.split()[0] for l in out.splitlines() if l.startswith("  ") and len(l.split()) > 1 and l.split()[0].islower()]
fila("`ore <verbo>` que escriben un documento", ", ".join(v for v in verbos if v in ("init", "source", "package", "discover", "promote")) + " — **no hay `ore model`**")
http = leer("crates/ore-entrada/src/http.rs")
plazo = re.search(r"pub fn pedir.*?from_secs\((\d+)\)", http, re.S)
fila("el cliente saliente que tiene", "`ore_entrada::http::pedir(método, host:puerto, camino, testigo, json)`: HTTP/1.1 llano, sin https, plazo %s s" % (plazo.group(1) if plazo else "?"))
fila("lo que E1 llama con él", "`POST %s:9000/admin/tenants/%s/models {model}` al crear · `DELETE …/models/{id}` al retirar" % (MODELOS, CELDA))

# ── C · los perfiles ─────────────────────────────────────────────────────────
print("\n  C · LA LISTA DE PERFILES (lo que Bastion mide; ⑦ rechaza lo que no está)")
perfiles = []
for p in sorted(glob.glob(os.path.join(BASTION, "env/profiles/*/*.env"))):
    maquina = os.path.basename(os.path.dirname(p))
    if os.path.basename(p) == "machine.env":
        continue
    def env(ruta):
        return {k: (a or b) for k, a, b in re.findall(r'^([A-Z_]+)=(?:"([^"]*)"|(\S*))', open(ruta, encoding="utf-8").read(), re.M)}
    kv = env(p)
    m = env(os.path.join(os.path.dirname(p), "machine.env"))
    toks = kv.get("EXPECT_TOKS", "").split()
    usd_h = float(m.get("MACHINE_USD_H", "0") or 0)
    t32 = float(toks[-1]) if toks else 0.0
    perfiles.append({
        "profile": "%s/%s" % (maquina, os.path.basename(p)[:-4]), "model": kv.get("MODEL"), "machine": m.get("MACHINE_DESC"),
        "gpus": int(kv.get("GPUS", "1") or 1), "status": kv.get("STATUS", "?"), "tok_s_32": t32, "usd_h": usd_h,
        "usd_per_mtok": round(usd_h / (t32 * 3600) * 1e6, 2) if t32 else None, "digest": None,
    })
if perfiles:
    for q in perfiles:
        fila(q["profile"], "%s · %s · %s tok/s@32 · %s $/M · %s · digest %s" % (q["model"], q["machine"], q["tok_s_32"], q["usd_per_mtok"], q["status"], q["digest"]))
    fila("la forma del fichero (`perfiles.json`)", json.dumps(perfiles[0], ensure_ascii=False)[:150] + "…")
    fila("`digest` del perfil", "NINGUNO tiene: B4 no existe. ⑦ «digest que no es el del perfil → 422» no se puede exigir en E1; se acepta ausente y se dice")
else:
    fila("perfiles", "✗ no encuentro %s/env/profiles (BASTION=…)" % BASTION)
gen = leer("malla/gen-inquilino.py")
fila("dónde deja ficheros el aprovisionador", "`plantilla-catalogo.txt` rendida por celda, en la COLA (`trabajo.git`), y `ore-serve` la lee al encolar (`cola.rs`)")
fila("la misma figura para los perfiles", "`perfiles.json` en la cola, publicado por Bastion (B2) y copiado por el aprovisionador; `ore-serve` lo lee en `POST /modelos`")

# ── D · la red ───────────────────────────────────────────────────────────────
print("\n  D · LA RED (desde el pod de `ore-serve`, rol `control`)")
rc, out, err = correr("kubectl", "exec", "-n", NS, "deploy/ore-serve", "--", "sh", "-c",
                      "wget -q -T 6 -O - http://%s:9000/admin/health 2>&1 | head -c 120; echo; echo rc=$?" % MODELOS)
fila("`%s:9000/admin/health` hoy" % MODELOS, ("✗ " + out.strip().replace("\n", " · ")) if "timed out" in out or "rc=1" in out else out.strip()[:120])
rc, pol, _ = correr("kubectl", "get", "netpol", "salida-del-control", "-n", NS, "-o", "json")
if rc == 0:
    reglas = json.loads(pol)["spec"]["egress"]
    destinos = []
    for r in reglas:
        for t in r.get("to", []):
            if "ipBlock" in t:
                destinos.append("%s%s" % (t["ipBlock"]["cidr"], " menos privadas" if t["ipBlock"].get("except") else ""))
            elif "podSelector" in t:
                destinos.append("rol " + t["podSelector"]["matchLabels"].get("ore.dev/rol", "?"))
            elif "namespaceSelector" in t:
                destinos.append("ns " + list(t["namespaceSelector"]["matchLabels"].values())[0])
    fila("`salida-del-control` deja salir a", ", ".join(destinos))
fila("lo que E1 necesita", "`control` → `%s/32:9000` (la regla de clase de ④, la mitad del plano de control); E2 la lleva a la plantilla, E1 la aplica a mano como E0" % MODELOS)
rc, args, _ = correr("kubectl", "get", "deploy", "ore-serve", "-n", NS, "-o", "jsonpath={.spec.template.spec.containers[0].args}")
fila("cómo sabe `ore-serve` dónde está el cofre", "`--cofre host:puerto` en el Deployment (rendido por el aprovisionador) → `--modelos %s:9000` es la misma figura" % MODELOS if "--cofre" in args else "?")

# ── E · la resolución ────────────────────────────────────────────────────────
print("\n  E · RESOLVER `modelo/v2-lite`")
fila("lo que E0 hizo", "el Job tomó la puerta (`MODELOS:8000`) y el id (`deepseek-ai/DeepSeek-V2-Lite`) de variables, y lo dijo en el log")
fila("lo que E1 hace", "`modelos/v2-lite.yaml` → `profile: g1/deepseek-v2-lite` → `perfiles.json[profile].model` = id; la url es `--modelos` → `GET /modelos/v2-lite` devuelve {url, model, profile, tier, task}")
fila("quién lo pregunta", "quien ejecute la Function (hoy el Job a mano; F4 cuando exista): un `GET` con el token de agente, no una variable")

# ── F · la aceptación ────────────────────────────────────────────────────────
print("\n  F · LA ACEPTACIÓN (los-verbos.sh)")
verbos_sh = leer("pruebas-de-fuego/los-verbos.sh")
fila("`los-verbos.sh` hoy", "%d `curl`, contra `ore-serve` en local con `--identidad cabecera`" % verbos_sh.count("curl"))
casos = [
    "POST /modelos {name, profile certificado, tier, task} → 201 + commit, y en el gateway `allowed_models` lo lleva",
    "POST con `profile` que no está en perfiles.json → 422 con el motivo, y NADA en el árbol ni en el gateway",
    "POST con `digest` distinto del perfil → 422 (cuando el perfil tenga digest; hoy: 422 si se manda uno)",
    "POST `tier: dedicated` → 422 «sin cuota de máquina» (E5)",
    "GET /modelos → la lista con lo declarado; GET /modelos/{n} → la resolución (url, model)",
    "DELETE /modelos/{n} → 204, el fichero desaparece, y el gateway → 401 a la celda",
    "el gateway caído en el POST → el documento queda y 502 «declarado, pero la suscripción NO se hizo» (la figura de la credencial)",
]
for c in casos:
    fila("·", c)
print()
