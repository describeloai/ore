# -*- coding: utf-8 -*-
"""
Medida W1 · ejecutar la pregunta (ADR 0030, segundo peldaño).

    Una vista es una pregunta sobre un hecho. W0 la puso en el editor; W1 tiene
    que CONTESTARLA: *Run* sobre una View = filas, en la celda, sin abrir el
    origen de una base estándar. Antes de escribir nada, cinco preguntas:

    §A  qué se puede preguntar hoy (la gramática cerrada de 02-view), y cuánto
        de eso pregunta alguien en demo y victor;
    §B  qué hay en la copia de verdad — se abre el artefacto y se cuenta;
    §C  cuánto cuesta el residuo escrito a mano sobre lo que ore-store ya da,
        frente a lo que pesaría traer un motor (DataFusion, Polars, DuckDB);
    §D  dónde correría: un Job por la cola, o un verbo servido en ore-serve,
        y qué permisos y latencias tiene cada sitio ya;
    §E  qué tiene la consola para enseñar filas.

  Uso:
    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-w1-ejecutar-la-pregunta.py \
        [--arbol DIR] [--copia FICHERO.ore ...] [--sin-cluster]

  `--arbol` es un clon del árbol de una celda (para §A y §B); `--copia` son
  artefactos `ore/v1/<sha>` bajados del bucket (para §B y §C, con el ejemplo
  `crates/ore-store/examples/medida-w1-residuo.rs` ya construido).
"""
import glob
import json
import os
import re
import subprocess
import sys

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ARGS = sys.argv[1:]
SIN_CLUSTER = "--sin-cluster" in ARGS


def arg(nombre):
    if nombre in ARGS:
        return ARGS[ARGS.index(nombre) + 1]
    return None


def args(nombre):
    return [ARGS[i + 1] for i, a in enumerate(ARGS) if a == nombre]


def fila(que, medido, veredicto=""):
    print("  %-46s %-58s %s" % (que, str(medido)[:58], veredicto))


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def leer(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


# ── §A · la gramática de la pregunta ────────────────────────────────────────
def seccion_a():
    print("\n§A · qué se puede preguntar (vendor/oos/schemas/v1alpha8/view.schema.json)")
    esquema = json.load(open(os.path.join(RAIZ, "vendor/oos/schemas/v1alpha8/view.schema.json"), encoding="utf-8"))
    spec = esquema["properties"]["spec"]["properties"]
    fila("A1 · claves de spec", ", ".join(spec.keys()))
    fila("A2 · from", "table | view (exactamente una): la cadena de vistas es la composición", "✓")
    fila("A3 · fields", "renombre, o count() sum(c) min(c) max(c) avg(c) — vocabulario cerrado", "✓")
    fila("A4 · where", "igualdad, pertenencia (lista), ausencia (null); conjunción implícita; sin rangos", "✓")
    fila("A5 · groupBy / having", "groupBy: columnas; having: `>= 8` sobre un agregado (OOS2032–2034)", "✓")
    fila("A6 · lo que NO hay", "join, orderBy, limit, rango en where: `Une` y `Limita` del IR sin vocabulario", "—")
    arbol = arg("--arbol")
    if not arbol:
        fila("A7 · uso en una celda", "sin --arbol", "—")
        return
    n = w = g = h = v = m = 0
    for p in glob.glob(os.path.join(arbol, "packages/*/views/*.yaml")):
        s = "\n".join(l for l in leer(p).splitlines() if not l.strip().startswith("#"))
        n += 1
        w += bool(re.search(r"^\s+where:", s, re.M))
        g += bool(re.search(r"^\s+groupBy:", s, re.M))
        h += bool(re.search(r"^\s+having:", s, re.M))
        v += bool(re.search(r"from:\s*\{?\s*view:", s))
        m += bool(re.search(r"^\s+materialized:", s, re.M))
    fila("A7 · uso en %s" % os.path.basename(arbol.rstrip("/\\")),
         "%d vistas · where %d · groupBy %d · having %d · sobre vista %d · materialized %d" % (n, w, g, h, v, m),
         "nadie pregunta todavía" if w + g + h + v == 0 else "✓")


# ── §B · la copia, abierta ──────────────────────────────────────────────────
def ejemplo():
    for d in ("release", "debug"):
        for n in ("medida-w1-residuo.exe", "medida-w1-residuo"):
            p = os.path.join(RAIZ, "target", d, "examples", n)
            if os.path.isfile(p):
                return p
    return None


def seccion_b():
    print("\n§B · qué hay en la copia (el informe dice filas; el artefacto dice columnas)")
    arbol = arg("--arbol")
    if arbol:
        for p in sorted(glob.glob(os.path.join(arbol, "copias/*.json"))):
            j = json.load(open(p, encoding="utf-8"))
            fila("B1 · %s" % j.get("vista"), "%s · %s filas · %s B · testigo %s" % (j.get("estado"), j.get("filas"), j.get("bytes"), (j.get("testigo") or {}).get("valor")))
    ex = ejemplo()
    copias = args("--copia")
    if not copias:
        fila("B2 · el artefacto", "sin --copia", "—")
        return
    if not ex:
        fila("B2 · el artefacto", "construye el ejemplo: cargo build -p ore-store --example medida-w1-residuo", "✗")
        return
    for c in copias:
        _, out = sh('"%s" "%s"' % (ex, c))
        lineas = out.splitlines()
        cab = next((l for l in lineas if l.startswith("abrir")), "")
        cols = next((l for l in lineas if l.startswith("columnas")), "")
        m = re.search(r"\{(.*)\} \(la cabecera declara (\d+)", cols)
        presentes = len(re.findall(r'"[^"]+": \d+', m.group(1))) if m else "?"
        declaradas = int(m.group(2)) if m else "?"
        fila("B2 · %s" % os.path.basename(c), cab)
        fila("B3 · columnas con algún valor", "%s de %s declaradas en la cabecera" % (presentes, declaradas),
             "✗ la copia calla columnas" if presentes != declaradas else "✓")
    fila("B4 · por qué faltan", "ore-read-postgres: try_get::<Option<String>>(i).unwrap_or(None) — un float8/int8 no es String → None", "✗ defecto")
    fila("B5 · lo que W1 exige", "una copia que conteste lo que la vista dice: castear a texto en el driver, y contarlo", "antes de ejecutar")


# ── §C · el residuo a mano, y el peso de un motor ──────────────────────────
def seccion_c():
    print("\n§C · el motor: el residuo a mano sobre lo que ore-store ya da, frente a traer uno")
    ex = ejemplo()
    for c in args("--copia"):
        if not ex:
            break
        _, out = sh('"%s" "%s"' % (ex, c))
        for l in out.splitlines():
            if l.startswith(("Q1", "Q2", "Q3", "Q4", "total")):
                fila("C1 · %s · %s" % (os.path.basename(c), l.split(" ")[0]), l)
    fila("C2 · lo que el residuo necesita", "eq/in/null · proyectar y renombrar · groupBy con 5 agregados · having: ~150 líneas, 0 crates nuevas", "✓")
    # Medido con `cargo generate-lockfile` sobre una crate vacía que depende de cada uno (2026-09-18).
    fila("C3 · DataFusion 55", "+283 crates en el lock (ORE entero: 248; ore-store: 131; ore-serve: 56)", "no compensa a 33 k filas")
    fila("C4 · Polars 0.55 (lazy+parquet)", "+337 crates", "no compensa")
    fila("C5 · DuckDB 1.10 (bundled)", "+168 crates y un C++ que la imagen alpine/scratch no trae", "no compensa")
    fila("C6 · el plan ya existe", "ore-view::Nodo (Proyecta/Filtra/Agrupa; Une y Limita esperan vocabulario) y ore-maintain lo corre en Δ", "el ejecutor consume el IR")


# ── §D · dónde corre ────────────────────────────────────────────────────────
def seccion_d():
    print("\n§D · dónde corre: la cola (un Job) o servido (ore-serve)")
    fila("D1 · ore-serve lee la copia", "aprovisionador: ore-serve-<n> tiene objectViewer «lee la copia, y no escribe»; el pod tiene metadata server", "✓ permitido ya")
    fila("D2 · lo que le falta a la imagen serve", "alpine con `ore` y `ore-serve`; ni ore-store-gcs ni parquet (ore-serve no enlaza ore-store)", "+1 binario o +75 crates")
    fila("D3 · la cola", "48 copiar / 49 invocar: Job por petición, clona, acuña, corre, empuja", "✓ existe")
    if SIN_CLUSTER:
        fila("D4 · latencia de un Job", "--sin-cluster", "—")
    else:
        cod, out = sh("kubectl get jobs -n t-demo -o json")
        try:
            import datetime as d
            t = lambda s: d.datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ")
            dur = []
            for it in json.loads(out)["items"]:
                m, s = it["metadata"], it["status"]
                if not m["name"].startswith(("copiar-", "invocar-")):
                    continue
                fin = s.get("completionTime") or next((c["lastTransitionTime"] for c in s.get("conditions", []) if c["type"] == "Failed"), None)
                if s.get("startTime") and fin:
                    dur.append((m["name"][:28], (t(fin) - t(s["startTime"])).seconds))
            fila("D4 · latencia de un Job en demo", " · ".join("%s %ds" % x for x in dur[-4:]) or "sin Jobs", "1,5–3 min: no es un Run")
        except Exception:
            fila("D4 · latencia de un Job", "kubectl no contesta", "—")
    fila("D5 · GET del artefacto (1,1 MB) desde aquí", "0,56–0,96 s desde España a europe-west1 (curl a la API JSON); en la celda menos", "medido 2026-09-18")
    fila("D6 · un verbo servido", "clon 1,0–1,3 s (W0) + GET de la copia + residuo < 70 ms ≈ 1,5–2,5 s", "sí es un Run")
    fila("D7 · el origen no se abre", "ore-serve no tiene credencial de ninguna fuente y no la tendrá: si no hay copia, 409 como en invocar", "✓ la frontera de siempre")


# ── §E · la consola ─────────────────────────────────────────────────────────
def seccion_e():
    print("\n§E · la consola (C:/rubix-platform/components/code-workspace)")
    cw = "C:/rubix-platform/components/code-workspace" if os.name == "nt" else os.path.expanduser("~/rubix-platform/components/code-workspace")
    if not os.path.isdir(cw):
        fila("E1 · code workspace", "no está en esta máquina", "—")
        return
    src = "".join(leer(p) for p in glob.glob(os.path.join(cw, "*.tsx")))
    fila("E1 · Run", "onRunRequested={() => {}} · Run all / Run all below sin manejador" if "onRunRequested={() => {}}" in src else "hay manejador", "no hace nada")
    fila("E2 · rejilla de resultados", "DataTable en el workspace" if "DataTable" in src else "no hay ninguna: nada enseña filas", "por hacer")
    fila("E3 · lo que W0 dejó", "ArbolFileView: Save = PUT /arbol; el mismo sitio para Run = POST …/ejecutar y una rejilla debajo", "✓ encaja")


if __name__ == "__main__":
    print("Medida W1 · ejecutar la pregunta")
    seccion_a()
    seccion_b()
    seccion_c()
    seccion_d()
    seccion_e()
    print("\nLo que sale: la gramática es cerrada y pequeña, la copia ya está en la celda y ore-serve ya puede leerla;")
    print("el residuo cabe en `ore` sin motor externo; lo que W1 no puede saltarse es que la copia hoy calla columnas.")
