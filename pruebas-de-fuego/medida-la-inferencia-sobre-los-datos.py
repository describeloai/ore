# -*- coding: utf-8 -*-
"""MEDIDA · de un modelo de verdad a inferencia consistente sobre los datos de la ontología.

La pregunta del 2026-09-16, después de aceptar 0027 E3: ¿qué falta —en orden— para que
un modelo REAL corra inferencia, de forma consistente, sobre los conjuntos de datos de
la ontología (los assets), en forma de Function o de lo que sea? Esto mira el terreno,
eslabón por eslabón, y lo que no está lo dice con el fichero que lo diría.

La cadena, tal como los documentos la nombran:

    máquina + modelo (Bastion B1/B2)  →  gateway (B3)  →  la celda lo alcanza (0027 E0–E3)
      →  los datos: la COPIA en el almacén (sustrato, ADR 0015/0018; `ore materialize`)
      →  quien invoca: F4 (`ore-invoke`, docs/functions.md)  →  la Propuesta (F1, `ore verify`)
      →  quien aplica: F5 (por la vista, sobre la copia)  →  la consola lo enseña

    uso:  python pruebas-de-fuego/medida-la-inferencia-sobre-los-datos.py [celda]
"""
import json
import os
import re
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "demo")
PROYECTO = "project-8853a180-450d-47be-b83"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"


def sh(*cmd, entrada=None):
    r = subprocess.run(list(cmd), input=entrada, capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return (r.stdout or "").strip()


def lee(rel):
    try:
        return open(os.path.join(RAIZ, rel), encoding="utf-8").read()
    except OSError:
        return ""


def fila(que, res):
    print("  %-46s %s" % (que, res))


print("\n  ═══ MEDIDA · la inferencia sobre los datos · %s ═══\n" % CELDA)

# ── A · el modelo: qué sirve hoy, y con qué máquina ─────────────────────────
print("A · el modelo de verdad")
vm = sh(GCLOUD, "compute", "instances", "describe", "modelos-e0", "--zone", "europe-west1-b", "--project", PROYECTO, "--format=value(status,machineType.basename())")
fila("la máquina en 10.10.0.100 (modelos-e0)", vm or "?")
arranque = open("C:/bastion/env/e0/startup-modelos.sh", encoding="utf-8").read() if os.path.exists("C:/bastion/env/e0/startup-modelos.sh") else ""
fila("qué sirve detrás del gateway", "un vLLM DE MENTIRA (vllm-de-mentira.py, contesta `pyme`)" if "de-mentira" in arranque else "?")
try:
    import urllib.request
    perfiles = json.load(urllib.request.urlopen("https://storage.googleapis.com/bastion-perfiles/perfiles.json", timeout=10))
    fila("perfiles certificados publicados", ", ".join("%s (%s, %s $/M)" % (p["profile"], p["status"], p["usd_per_mtok"]) for p in perfiles["profiles"]))
except Exception as e:  # noqa: BLE001
    fila("perfiles certificados publicados", "no se pudo leer: %s" % e)
cuota = sh(GCLOUD, "compute", "regions", "describe", "europe-west1", "--project", PROYECTO, "--format=json")
gpus = [q for q in (json.loads(cuota).get("quotas", []) if cuota else []) if "GPU" in q["metric"] or "RTX" in q["metric"]]
con = [q for q in gpus if q["limit"] > 0]
fila("cuota de GPU en europe-west1", ", ".join("%s=%g" % (q["metric"], q["limit"]) for q in con) if con else "NINGUNA con límite > 0 (%d métricas de GPU a 0)" % len(gpus))
fila("dónde corrió el g1 real", "Vast (E0 b, 20 min, 1,5 $/h; el gateway lo alcanzó por túnel ssh desde la máquina)")
fila("quién enciende una máquina cuando hace falta", "NADIE: `bastion launch` es un verbo a mano (B1 I1); no hay reconciliador ni cola")

# ── B · los datos: qué hay en el árbol de la celda, y si hay COPIA ─────────
print("\nB · los datos de la ontología (%s)" % CELDA)
sys.path.insert(0, os.path.join(RAIZ, "pruebas-de-fuego"))
src = lee("pruebas-de-fuego/deployments-tiene-filas.py").split('print("\\n  ═══ E3 I5')[0]
g = {}
exec(compile(src, "ayudas", "exec"), g)  # noqa: S102 — las mismas ayudas que la aceptación de E3
g["CELDA"] = CELDA
tok = g["token_del_agente"](CELDA)
cod, _, cuerpo = g["serve"]("GET", "/fuentes", tok)
fuentes = json.loads(cuerpo).get("datasources", []) if cod == "200" else []
fila("fuentes declaradas", "%d · %s" % (len(fuentes), ", ".join("%s (%s)" % (f["name"], f["type"]) for f in fuentes)[:120]))
cod, _, cuerpo = g["serve"]("GET", "/paquetes", tok)
paquetes = json.loads(cuerpo).get("packages", []) if cod == "200" else []
fila("paquetes (catálogo leído del origen)", "%d · %s" % (len(paquetes), ", ".join("%s%s" % (p["name"], " [elegido]" if p.get("scoped") else "") for p in paquetes)[:120]))
for p in paquetes[:1]:
    cod, _, cuerpo = g["serve"]("GET", "/paquetes/%s/esquema" % p["name"], tok)
    ent = json.loads(cuerpo).get("entities", []) if cod == "200" else []
    fila("  entidades de `%s`" % p["name"], "%d · filas: el inductor no cuenta (se dice «—»)" % len(ent))
jobs = sh("kubectl", "get", "jobs", "-n", "t-" + CELDA, "-o", "jsonpath={range .items[*]}{.metadata.name} {end}")
fila("Jobs que corren en la celda", (jobs or "ninguno") + "  → sólo `catalogo-*` (discover) y los de las pruebas")
fila("¿hay COPIA de algún dato en un almacén?", "NO: ningún manifiesto de la malla llama a `ore materialize`; `ore-store-r2` sólo corre en `refresco.sh` (local)")
fila("¿la celda tiene almacén (bucket/R2)?", "NO: sólo `-copias` (backups de la forja y del IdP)")

# ── C · el vocabulario: qué puede decir el árbol ─────────────────────────────
print("\nC · lo que el árbol puede decir")
doc = lee("crates/ore-core/src/document.rs")
fila("Function `runtime: model` + `model: modelo/<n>` + `prompt`", "SÍ (v1alpha9, OOS1004, OOS2005)" if "modelo/" in doc else "?")
fila("`effects: writes: <entidad>.<propiedad>`", "SÍ (OOS7xxx, effect.rs)" if os.path.exists(os.path.join(RAIZ, "crates/ore-core/src/effect.rs")) else "?")
fila("la Propuesta y `ore verify`", "SÍ (propuesta.rs, verificar.rs): coteja, no ejecuta, no escribe")
fila("una Function que corre SOBRE UNA VISTA (qué filas)", "NO: la Function nombra el modelo y el efecto, no de qué vista salen las filas ni la clave (el `Plan` de functions.md §3 no tiene forma en la gramática)")

# ── D · quién ejecuta, quién aplica ─────────────────────────────────────────
print("\nD · quién invoca y quién aplica")
fn = lee("docs/functions.md")
for k in ("F4", "F5"):
    m = re.search(r"\*\*%s\*\*[^|]*\|\s*([^|]*)\|" % k, fn)
    fila("%s según docs/functions.md" % k, (m.group(1).strip() if m else "?") + " · «No existe»")
fila("ore-invoke (binario)", "NO existe: " + ", ".join(sorted(d for d in os.listdir(os.path.join(RAIZ, "crates")) if "invoke" in d)) if any("invoke" in d for d in os.listdir(os.path.join(RAIZ, "crates"))) else "NO existe (crates/: ningún ore-invoke)")
fila("cómo se ejecutó `segmentar` en E0/E2/E3", "un Job escrito a mano en la prueba: una fila fija (C-0001), el prompt, el gateway, la Propuesta, `ore verify`, commit")
fila("quién decide CUÁNDO corre una Function", "NADIE: ni el convergedor (sólo catálogo), ni la consola, ni un CronJob")
fila("dónde aterriza el resultado", "`propuestas/<f>-<fila>.json` en el árbol (F1) · aplicar por la vista sobre la copia es F5, y no hay copia")

# ── E · la consola ───────────────────────────────────────────────────────────
print("\nE · la consola")
forge = os.path.exists("C:/rubix-platform/components/ontology/secciones/Functions.tsx")
fila("Ontology Forge · sección Functions", "existe (WIP sin commitear, sobre `acme`: datos de mentira)" if forge else "?")
fila("Deployments / Hub (0027 E3)", "hechas: filas reales de `GET /modelos`, alta y retiro")
fila("ver una Propuesta / un resultado de inferencia", "NO hay pantalla")

print("\n  ═══ lo que sale de aquí, en orden, está en la ADR 0027 §«Después de E3» ═══\n")
