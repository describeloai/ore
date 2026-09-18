#!/usr/bin/env python3
"""
MEDIDA · el paquete de la fuente (0027, después de «el dueño es la organización»)

La pregunta del 18 de septiembre: *«cuando se crea el source, ¿se crean tablas,
vistas o entidades automáticamente, o sólo la conexión?»*. Las dos cosas: el
alta escribe la conexión, y el Job de catálogo (`44-el-catalogo.yaml`) lee el
origen y hace `ore discover --from catalogo.json --out packages/<fuente>` SIN
`--only`, SIN `--no-model`, SIN `--owner`: una inducción entera —Table + View +
Entity por tabla, la cola con todas las decisiones de modelado, `cambiame`—
de la que la database sólo usa `discover.catalog.json`.

Se mide, sobre los árboles reales (`--arbol DIR`, repetible: un `git archive`
de la forja del inquilino) y sobre el código:

  §A  qué contiene hoy cada paquete de fuente: documentos por clase y bytes,
      decisiones abiertas por clase, dueño, y con cuántos diagnósticos NO
      compila (`ore validate`, atribuido por paquete)
  §B  quién lee qué del paquete de la fuente (ore-serve y la consola): por
      fichero, para saber qué tiene que seguir existiendo
  §C  qué cuesta la inducción entera frente a «sólo el catálogo»: tiempo de
      `ore discover` con el catálogo real más grande, bytes escritos, ficheros;
      y `ore validate` del árbol con y sin los paquetes de fuente
  §D  lo que pesan en el árbol: bytes y ficheros de los paquetes de fuente
      frente al resto

Uso:  python pruebas-de-fuego/medida-el-paquete-de-la-fuente.py --arbol <dir> [--arbol <dir>] [--ore <bin>]
"""
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def arg(nombre, todos=False):
    out = []
    a = sys.argv[1:]
    for i, x in enumerate(a):
        if x == nombre and i + 1 < len(a):
            out.append(a[i + 1])
    return out if todos else (out[0] if out else None)


def fila(k, v, nota=""):
    print("  %-46s %-28s %s" % (k, v, nota))


def ore_bin():
    b = arg("--ore")
    if b:
        return b
    for n in ("target/debug/ore.exe", "target/debug/ore", "target/release/ore.exe", "target/release/ore"):
        p = os.path.join(RAIZ, n)
        if os.path.exists(p):
            return p
    sys.exit("no hay binario de `ore`: cargo build -p ore-cli, o --ore")


ORE = ore_bin()


def leer(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


def clase_de_paquete(d):
    if os.path.exists(os.path.join(d, "discover.scope.json")):
        return "base"
    if os.path.exists(os.path.join(d, "discover.catalog.json")):
        return "fuente"
    return "a mano"


def contar(d, sub):
    fs = glob.glob(os.path.join(d, sub, "*.yaml"))
    return len(fs), sum(os.path.getsize(f) for f in fs)


def pendientes_por_clase(d):
    p = os.path.join(d, "discover.pending.json")
    if not os.path.exists(p):
        return {}
    try:
        cola = json.loads(leer(p))
    except Exception:
        return {"?": -1}
    out = {}
    for x in cola.get("pending", []):
        c = str(x.get("id", "?")).split("/")[0]
        out[c] = out.get(c, 0) + 1
    return out


def dueno(d):
    p = os.path.join(d, "package.yaml")
    if not os.path.exists(p):
        return "—"
    m = re.search(r"owner:\s*\"?([^\"\n}]+)", leer(p))
    return m.group(1).strip() if m else "?"


def validar(arbol):
    """`ore validate` y los diagnósticos atribuidos a su paquete."""
    t0 = time.perf_counter()
    r = subprocess.run([ORE, "validate", "."], cwd=arbol, capture_output=True, text=True, encoding="utf-8", errors="replace")
    dt = time.perf_counter() - t0
    salida = r.stdout + r.stderr
    por_paquete = {}
    codigo_actual = None
    for linea in salida.splitlines():
        m = re.match(r"(error|warning)\[(OOS\d{4})\]", linea)
        if m:
            codigo_actual = m.group(2)
            continue
        m = re.search(r"packages/([A-Za-z0-9_\-]+)/", linea)
        if m and codigo_actual:
            por_paquete.setdefault(m.group(1), {}).setdefault(codigo_actual, 0)
            por_paquete[m.group(1)][codigo_actual] += 1
            codigo_actual = None
    return dt, r.returncode, por_paquete, salida


def seccion_a(arbol):
    nombre = os.path.basename(arbol.rstrip("/\\"))
    print("\n§A · %s: qué hay en cada paquete" % nombre)
    print("  %-30s %-8s %-7s %-7s %-7s %-9s %-18s %s" % ("paquete", "clase", "tables", "views", "entit.", "KB", "dueño", "decisiones abiertas"))
    dt, rc, diags, _ = validar(arbol)
    for d in sorted(glob.glob(os.path.join(arbol, "packages", "*"))):
        if not os.path.isdir(d):
            continue
        p = os.path.basename(d)
        c = clase_de_paquete(d)
        t, tb = contar(d, "tables")
        v, vb = contar(d, "views")
        e, eb = contar(d, "entities")
        kb = sum(os.path.getsize(f) for f in glob.glob(os.path.join(d, "**", "*"), recursive=True) if os.path.isfile(f)) / 1024
        pend = pendientes_por_clase(d)
        pend_s = " ".join("%s:%d" % kv for kv in sorted(pend.items())) or "—"
        print("  %-30s %-8s %-7d %-7d %-7d %-9.0f %-18s %s" % (p, c, t, v, e, kb, dueno(d), pend_s))
        dg = diags.get(p)
        if dg:
            print("  %-30s %s" % ("", "NO compila: " + " ".join("%s×%d" % kv for kv in sorted(dg.items()))))
    fila("ore validate del árbol entero", "%.2f s · rc=%d" % (dt, rc))
    return diags


def seccion_b():
    print("\n§B · quién lee qué del paquete de la fuente (grep sobre el código)")
    lect = [
        ("discover.catalog.json", "crates/ore-serve/src/rutas.rs", r"discover\.catalog\.json", "POST /paquetes induce la database DESDE él; GET /paquetes lee `source`"),
        ("tables/*.yaml", "crates/ore-serve/src/rutas.rs", r"tablas_del_paquete|\"tables\"", "GET /paquetes/{n}/esquema → la consola pinta el esquema de la CONEXIÓN con esto (elegir tablas al crear la base)"),
        ("entities/*.yaml", "crates/ore-serve/src/rutas.rs", r"join\(\"entities\"\)", "GET /esquema los devuelve debajo (`entities`); GET /paquetes cuenta `modeladas`"),
        ("views/*.yaml", "crates/ore-serve/src/rutas.rs", r"join\(\"views\"\)", "objetos_fisicos: sólo para resolver entidad→objeto; vistas_con_copia (una fuente no declara copia)"),
        ("discover.pending.json", "crates/ore-serve/src/rutas.rs", r"COLA\b|discover\.pending", "GET /paquetes `decisionesPendientes`; GET/POST /decisiones"),
        ("package.yaml", "crates/ore-serve/src/copia.rs", r"package\.yaml", "autorizar_conducto lee `owner` (sólo al copiar: una fuente no copia)"),
        ("consola: esquemaDeFuente", "../rubix-platform/app/(workspace)/clusters/[celda]/catalog/acciones.ts", r"esquemaDelPaquete", "la ficha de la conexión y CreateDatabaseModal listan tablas desde GET /esquema"),
        ("consola: paquetesDelArbol", "../rubix-platform/app/(workspace)/clusters/[celda]/catalog/page.tsx", r"\.scoped", "el catálogo enseña sólo los `scoped` (con discover.scope.json): la fuente NO se ve como database"),
    ]
    print("  %-26s %-6s %s" % ("fichero del paquete", "usos", "quién y para qué"))
    for fichero, ruta, patron, nota in lect:
        p = os.path.join(RAIZ, ruta)
        n = len(re.findall(patron, leer(p))) if os.path.exists(p) else -1
        print("  %-26s %-6s %s" % (fichero, n if n >= 0 else "n/a", nota))
    print("  ⇒ lo que tiene que sobrevivir: el CATÁLOGO (para inducir la base) y un ESQUEMA de la conexión")
    print("    (hoy sale de tables/; el catálogo trae lo mismo: objeto, columnas, physicalType, claves).")


def seccion_c(arbol):
    nombre = os.path.basename(arbol.rstrip("/\\"))
    print("\n§C · %s: qué cuesta la inducción entera frente a sólo el catálogo" % nombre)
    # el catálogo de fuente más grande del árbol
    cats = []
    for d in glob.glob(os.path.join(arbol, "packages", "*")):
        if clase_de_paquete(d) == "fuente":
            c = os.path.join(d, "discover.catalog.json")
            cats.append((os.path.getsize(c), c, os.path.basename(d)))
    if not cats:
        fila("catálogos de fuente", "ninguno", "—")
        return
    cats.sort(reverse=True)
    tam, cat, fuente = cats[0]
    try:
        n_tablas = len(json.loads(leer(cat)).get("tables", []))
    except Exception:
        n_tablas = -1
    fila("catálogo más grande", "%s · %d tablas · %.0f KB" % (fuente, n_tablas, tam / 1024))
    tmp = tempfile.mkdtemp(prefix="medida-fuente-")
    try:
        for modo, extra in (("entera (el Job hoy)", []), ("--no-model --type foreign (como una base)", ["--type", "foreign", "--no-model"])):
            out = os.path.join(tmp, re.sub(r"[^a-z]", "", modo)[:12] or "x")
            args = [ORE, "discover", "--from", cat, "--out", out, "--name", "medida"] + extra
            if extra:
                # --type pide un alcance: todas las tablas del catálogo
                objetos = [t.get("name") for t in json.loads(leer(cat)).get("tables", []) if t.get("name")]
                lista = os.path.join(tmp, "todas.txt")
                with open(lista, "w", encoding="utf-8") as f:
                    f.write("\n".join(objetos))
                args += ["--only-file", lista]
            t0 = time.perf_counter()
            r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace")
            dt = time.perf_counter() - t0
            if r.returncode != 0:
                fila("discover " + modo, "rc=%d" % r.returncode, (r.stderr or r.stdout).strip().splitlines()[0][:80] if (r.stderr or r.stdout).strip() else "")
                continue
            fs = [f for f in glob.glob(os.path.join(out, "**", "*"), recursive=True) if os.path.isfile(f)]
            b = sum(os.path.getsize(f) for f in fs)
            pend = pendientes_por_clase(out)
            fila("discover " + modo, "%.0f ms · %d ficheros · %.0f KB" % (dt * 1000, len(fs), b / 1024),
                 "decisiones " + (" ".join("%s:%d" % kv for kv in sorted(pend.items())) or "0"))
        fila("sólo el catálogo (copiar el json)", "0 ms · 1 fichero · %.0f KB" % (tam / 1024), "decisiones 0 · nada que compilar")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    # validate con y sin los paquetes de fuente
    copia = tempfile.mkdtemp(prefix="medida-arbol-")
    try:
        dst = os.path.join(copia, "a")
        shutil.copytree(arbol, dst)
        dt1, rc1, d1, _ = validar(dst)
        n_diag1 = sum(sum(v.values()) for v in d1.values())
        quitados = []
        for d in glob.glob(os.path.join(dst, "packages", "*")):
            if clase_de_paquete(d) == "fuente":
                shutil.rmtree(d)
                quitados.append(os.path.basename(d))
        dt2, rc2, d2, _ = validar(dst)
        n_diag2 = sum(sum(v.values()) for v in d2.values())
        fila("ore validate CON los paquetes de fuente", "%.2f s · %d diagnósticos" % (dt1, n_diag1), "rc=%d" % rc1)
        fila("ore validate SIN ellos (%d quitados)" % len(quitados), "%.2f s · %d diagnósticos" % (dt2, n_diag2), "rc=%d · quedan: %s" % (rc2, " ".join("%s(%d)" % (k, sum(v.values())) for k, v in sorted(d2.items())) or "ninguno"))
    finally:
        shutil.rmtree(copia, ignore_errors=True)


def seccion_d(arbol):
    nombre = os.path.basename(arbol.rstrip("/\\"))
    print("\n§D · %s: lo que pesan en el árbol" % nombre)
    tot_f = tot_b = fue_f = fue_b = cat_b = 0
    for f in glob.glob(os.path.join(arbol, "**", "*"), recursive=True):
        if not os.path.isfile(f):
            continue
        b = os.path.getsize(f)
        tot_f += 1
        tot_b += b
        rel = os.path.relpath(f, arbol).replace("\\", "/")
        m = re.match(r"packages/([^/]+)/", rel)
        if m and clase_de_paquete(os.path.join(arbol, "packages", m.group(1))) == "fuente":
            fue_f += 1
            fue_b += b
            if rel.endswith("discover.catalog.json"):
                cat_b += b
    fila("árbol entero", "%d ficheros · %.0f KB" % (tot_f, tot_b / 1024))
    fila("paquetes de fuente", "%d ficheros · %.0f KB" % (fue_f, fue_b / 1024), "%.0f %% de los ficheros · %.0f %% de los bytes" % (100.0 * fue_f / max(tot_f, 1), 100.0 * fue_b / max(tot_b, 1)))
    fila("  de eso, los catálogos", "%.0f KB" % (cat_b / 1024), "lo único que la database usa")


def main():
    arboles = arg("--arbol", todos=True)
    print("medida · el paquete de la fuente · ore=%s" % os.path.relpath(ORE, RAIZ))
    if not arboles:
        print("  (sin --arbol: sólo §B)")
    for a in arboles:
        seccion_a(a)
    seccion_b()
    for a in arboles:
        seccion_c(a)
        seccion_d(a)
    print("""
Lectura:
  · el Job de catálogo hace la inducción del 30 de agosto (entidades de todo); 0027 C1 decidió
    que el catálogo NO modela, y se aplicó a las databases pero no a la fuente
  · nada usa las Tables/Views/Entities del paquete de la fuente salvo GET /esquema (tables/),
    que puede leer el catálogo; la database se induce del catálogo
  · el paquete de la fuente nace sin dueño y con decisiones de modelado que nadie pidió, y
    NO compila: cada fuente es un paquete roto en el árbol hasta que alguien lo revise
  ⇒ el Job deja `discover.catalog.json` y un `package.yaml` con el dueño de la organización,
    y nada gobernado: lo gobernado nace al crear una database
""")


if __name__ == "__main__":
    main()
