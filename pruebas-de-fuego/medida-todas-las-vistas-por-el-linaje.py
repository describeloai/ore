"""MEDIDA · ADR 0040 paso 3 (1b) · si TODAS las vistas van por el linaje, ¿qué cambia?

Hoy una vista de v1alpha7 a v1alpha13 va por `vistas::raiz()` y una cadena que
toca una vista SQL va por `linaje.rs`: dos caminos. La decisión A es uno. Antes
de quitar el viejo, lo que cambiaría de lo que ya compila:

  L1  `ore validate` de cada árbol del repositorio (conformance de todas las
      versiones, `casos/`, ejemplos): mismo resultado con los dos caminos, o no
  L2  la clasificación de cada asset (`ore assets --json`, `acceso`): la que
      pinta el catálogo y la que cotejan el puesto y el índice
  L3  `ore diff` de los casos de diff

El camino nuevo se fuerza con un binario de medida (`target/medida`) que lee
`ORE_MEDIDA_TODO_LINAJE`; no está en el código que se commitea.
"""
import glob
import json
import os
import subprocess

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
HOY = os.path.join(RAIZ, "target", "release", "ore" + EXE)
TODO = os.path.join(RAIZ, "target", "medida", "release", "ore" + EXE)


def correr(bin_, args, todo):
    env = dict(os.environ)
    if todo:
        env["ORE_MEDIDA_TODO_LINAJE"] = "1"
    r = subprocess.run([bin_, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120, env=env)
    return r.returncode, (r.stdout + r.stderr).strip()


def arboles():
    out = []
    for cfg in glob.glob(os.path.join(RAIZ, "**", "ontology.config.yaml"), recursive=True):
        if os.sep + "target" + os.sep in cfg or "node_modules" in cfg:
            continue
        out.append(os.path.dirname(cfg))
    return sorted(out)


def codigos(t):
    import re
    return sorted(set(re.findall(r"OOS\d{4}", t)))


def main():
    if not (os.path.exists(HOY) and os.path.exists(TODO)):
        print("⛔ faltan los binarios")
        return
    arb = arboles()
    print("L0 · %d árboles" % len(arb))
    iguales, distintos = 0, []
    for a in arb:
        h = correr(HOY, ["validate", a], False)
        t = correr(TODO, ["validate", a], True)
        if h == t:
            iguales += 1
        else:
            distintos.append((os.path.relpath(a, RAIZ), h, t))
    print("L1 · `ore validate`: iguales %d · distintos %d" % (iguales, len(distintos)))
    for rel, h, t in distintos:
        print("  - %s\n      hoy   rc=%d %s\n      todo  rc=%d %s" % (rel, h[0], codigos(h[1]) or h[1][:120], t[0], codigos(t[1]) or t[1][:120]))
        for linea in t[1].splitlines()[:3]:
            print("        %s" % linea[:170])

    cambios = 0
    ejemplos = []
    comparados = 0
    for a in arb:
        h = correr(HOY, ["assets", a, "--json"], False)
        t = correr(TODO, ["assets", a, "--json"], True)
        if h[0] != 0 or t[0] != 0:
            continue
        try:
            ih = json.loads(h[1])["items"]
            it = json.loads(t[1])["items"]
        except Exception:  # noqa: BLE001
            continue
        for k, v in ih.items():
            comparados += 1
            ah = (v.get("acceso") or {}).get("clasificacion")
            at = ((it.get(k) or {}).get("acceso") or {}).get("clasificacion")
            if ah != at:
                cambios += 1
                if len(ejemplos) < 12:
                    ejemplos.append((os.path.relpath(a, RAIZ), k, ah, at))
    print("L2 · clasificación de %d assets: cambian %d" % (comparados, cambios))
    for e in ejemplos:
        print("  - %s · %s: hoy %s → todo %s" % e)

    diffs = 0
    dist = []
    for antes in glob.glob(os.path.join(RAIZ, "vendor", "oos", "conformance", "*", "diff", "*", "before")):
        despues = os.path.join(os.path.dirname(antes), "after")
        h = correr(HOY, ["diff", antes, despues], False)
        t = correr(TODO, ["diff", antes, despues], True)
        diffs += 1
        if h != t:
            dist.append((os.path.relpath(os.path.dirname(antes), RAIZ), codigos(h[1]), codigos(t[1])))
    print("L3 · `ore diff` de %d casos: distintos %d" % (diffs, len(dist)))
    for d in dist:
        print("  - %s: hoy %s → todo %s" % d)


if __name__ == "__main__":
    main()
