"""MEDIDA · ADR 0040 paso 3 (1a) · ¿qué hacen con una vista SQL las que leían la forma?

Las consumidoras de `vistas::raiz()` y compañía se escribieron para una vista
con UNA fuente y forma estructurada. Una vista SQL les llega como
`SinRaiz::Consulta` o sin `fields`. Antes de tocarlas, lo que hace cada una:

  C1  cada mando del CLI que llega a una consumidora, sobre los árboles
      válidos de v1alpha14 (vistas SQL, una SQL sobre una estructurada, una
      copia de una SQL, una entidad respaldada por una SQL): sale bien, falla,
      o sale y calla lo que no ve
  C2  lo que dice de la vista SQL la faceta de `ore assets --json`: define,
      expone y relaciones (lo que el catálogo de la consola pinta)
  C3  `ore diff` de una vista SQL contra sí misma con la consulta cambiada, y
      con el contrato cambiado (decisión E: compara el contrato; un cambio del
      texto cambia las filas)

Mide y dice; no falla. Necesita `ore` en target/release.
"""
import json
import os
import shutil
import subprocess
import tempfile

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORE = os.path.join(RAIZ, "target", "release", "ore" + (".exe" if os.name == "nt" else ""))
CASOS = os.path.join(RAIZ, "vendor", "oos", "conformance", "v1alpha14", "valid")

ARBOLES = [
    "a-view-in-sql",
    "a-view-over-a-sql-view",
    "a-join-and-a-grouping",
    "a-sql-view-over-a-structured-view",
    "a-dataset-copies-a-sql-view",
]


def ore(*args, cwd=None):
    r = subprocess.run([ORE, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120, cwd=cwd)
    return r.returncode, r.stdout, r.stderr


def una(s):
    return " ".join(s.split())[:170]


def c1():
    print("C1 · los mandos sobre árboles con vistas SQL")
    for caso in ARBOLES:
        arbol = os.path.join(CASOS, caso, "input")
        vistas = []
        for raiz, _, fs in os.walk(arbol):
            if os.path.basename(raiz) == "views":
                for f in fs:
                    vistas.append(f[:-5])
        pkg = os.path.join(arbol, "packages", os.listdir(os.path.join(arbol, "packages"))[0])
        ns = os.path.basename(pkg)
        print("  · %s (%s)" % (caso, ", ".join(sorted(vistas))))
        mandos = [
            ("validate", ["validate", arbol]),
            ("lint", ["lint", arbol]),
            ("report", ["report", arbol]),
            ("view", ["view", arbol]),
            ("assets", ["assets", arbol, "--json"]),
            ("export odcs", ["export", arbol, "--format", "odcs"]),
            ("export graphql", ["export", arbol, "--format", "graphql"]),
            ("export cedar", ["export", arbol, "--format", "cedar"]),
            ("compile", ["compile", arbol, "--out", os.path.join(tempfile.gettempdir(), "ore-medida.oob")]),
        ]
        for v in sorted(vistas):
            mandos.append(("ask --sql %s" % v, ["ask", arbol, "--vista", "%s.%s" % (ns, v), "--sql"]))
            mandos.append(("ask --sql --catalogo %s" % v, ["ask", arbol, "--vista", "%s.%s" % (ns, v), "--sql", "--catalogo"]))
        for nombre, args in mandos:
            rc, out, err = ore(*args)
            texto = out if rc == 0 else (err or out)
            resumen = una(texto.splitlines()[0] if texto.strip() else "(nada)")
            if nombre == "view":
                # lo que `ore view` dice de cada vista: su bloque
                bloques = [l for l in out.splitlines() if l and not l.startswith(" ")]
                resumen = "rc=%d · vistas en el informe: %s · %s" % (rc, bloques[:6], una(err)[:80])
                print("    - %-26s %s" % (nombre, resumen))
                for l in out.splitlines():
                    if "no se" in l or "no tipa" in l or "SinRaiz" in l or "Consulta" in l:
                        print("        %s" % l.strip()[:150])
                continue
            print("    - %-26s rc=%d · %s" % (nombre, rc, resumen))


def c2():
    print("C2 · la faceta de `ore assets` de una vista SQL")
    for caso in ("a-view-in-sql", "a-join-and-a-grouping", "a-dataset-copies-a-sql-view"):
        arbol = os.path.join(CASOS, caso, "input")
        rc, out, err = ore("assets", arbol, "--json")
        if rc != 0:
            print("  · %s: rc=%d %s" % (caso, rc, una(err)))
            continue
        try:
            j = json.loads(out)
        except Exception:  # noqa: BLE001
            print("  · %s: no es JSON: %s" % (caso, una(out)))
            continue
        items = j.get("items", j) if isinstance(j, dict) else j
        for it in items if isinstance(items, list) else []:
            ident = it.get("id") or it.get("item") or ""
            if not str(ident).startswith(("view:", "dataset:", "entity:")):
                continue
            claves = {k: it.get(k) for k in ("define", "expone", "relaciones", "aristas", "acceso") if k in it}
            print("  · %s · %s" % (caso, ident))
            print("      %s" % una(json.dumps(claves, ensure_ascii=False))[:400])


def c3():
    print("C3 · `ore diff` de una vista SQL")
    base = os.path.join(CASOS, "a-view-over-a-sql-view", "input")
    tmp = tempfile.mkdtemp(prefix="ore-diff-")
    try:
        cambios = {
            "la consulta filtra otra cosa": ("WHERE pais IN ('ES', 'PT')", "WHERE pais IN ('ES')"),
            "el contrato pierde una columna": ("SELECT id, dni FROM", "SELECT id FROM"),
        }
        for nombre, (de, a) in cambios.items():
            antes = os.path.join(tmp, nombre.replace(" ", "_") + "_antes")
            despues = os.path.join(tmp, nombre.replace(" ", "_") + "_despues")
            shutil.copytree(base, antes)
            shutil.copytree(base, despues)
            f = os.path.join(despues, "packages", "hr", "views", "iberia.yaml")
            t = open(f, encoding="utf-8").read().replace(de, a)
            if "pierde" in nombre:
                t = t.replace("    dni: { type: String }\n", "")
                g = os.path.join(despues, "packages", "hr", "entities", "Employee.yaml")
                open(g, "w", encoding="utf-8").write(open(g, encoding="utf-8").read().replace("    dni:\n      type: String\n      labels: { gdpr.sensitivity: high }\n", ""))
            open(f, "w", encoding="utf-8").write(t)
            rc, out, err = ore("diff", antes, despues)
            print("  · %s: rc=%d" % (nombre, rc))
            for l in (out + err).splitlines()[:8]:
                if l.strip():
                    print("      %s" % l.strip()[:160])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    if not os.path.exists(ORE):
        print("⛔ no hay `ore` en target/release")
        return
    c1()
    c2()
    c3()


if __name__ == "__main__":
    main()
