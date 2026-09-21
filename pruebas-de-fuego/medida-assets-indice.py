"""0034 · LA FORMA DEL ÍNDICE DE ASSETS, MEDIDA ANTES DEL CÓDIGO (docs/assets.md, paso 0).

Sobre los árboles reales (demo, victor; un Job de lectura por forja, como
`medida-migrar-dataset.py`) cuenta LO QUE `ore_core::assets::indice` TIENE QUE DAR:
ítems por kind y por paquete, carpetas (regla de 0034 ⑤ 6), relaciones esperadas
en las dos direcciones, cuántos datasets son identidad, cuántas Tables de una
foreign tienen vista inducida, y los punteros. Son los oráculos del paso 1: el
índice sobre el mismo árbol tiene que darlos.

    python pruebas-de-fuego/medida-assets-indice.py [demo victor …] [--local <dir>]
"""
import importlib.util
import json
import os
import sys
import tempfile
from collections import Counter, defaultdict

import yaml

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
# La medida de la migracion vuelve a envolver stdout al importarse; se guardan
# las dos envolturas para que ninguna cierre el buffer al recogerse.
_ENVOLTURAS = [sys.stdout]
AQUI = os.path.dirname(os.path.abspath(__file__))
CARPETAS_DE_KIND = {"tables", "views", "datasets", "entities", "functions", "actions", "models", "interfaces", "concepts"}
KINDS_ITEM = {"Dataset", "Table", "View", "Entity", "Interface", "Concept", "Function", "Action", "TrainedModel", "Model"}


def traer(celda, destino):
    spec = importlib.util.spec_from_file_location("m", os.path.join(AQUI, "medida-migrar-dataset.py"))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    _ENVOLTURAS.append(sys.stdout)
    return m.traer_arbol(celda, destino)


def documentos(dir_):
    """Cada YAML del árbol con su ruta relativa y el documento (sólo los que tienen `kind`)."""
    out = []
    for raiz, _, fs in os.walk(dir_):
        for f in fs:
            if not f.endswith(".yaml"):
                continue
            p = os.path.join(raiz, f)
            rel = os.path.relpath(p, dir_).replace("\\", "/")
            try:
                d = yaml.safe_load(open(p, encoding="utf-8"))
            except Exception as e:
                out.append((rel, {"_roto": str(e)[:80]}))
                continue
            if isinstance(d, dict):
                out.append((rel, d))
    return out


def ref(kind, ns, name):
    return "%s:%s%s" % (kind.lower(), (ns + ".") if ns else "", name)


def carpeta_de(rel):
    partes = rel.split("/")
    if partes[0] != "packages" or len(partes) < 3:
        return None, None
    entre = partes[2:-1]
    if entre and entre[0] in CARPETAS_DE_KIND:
        entre = entre[1:]
    return partes[1], "/".join(entre)


def qn_ref(kind, texto, ns_por_defecto):
    """`p.x` o `x` → ref; el kind lo dice quien enlaza."""
    if not isinstance(texto, str):
        return None
    if "." in texto:
        ns, n = texto.split(".", 1)
    else:
        ns, n = ns_por_defecto, texto
    return ref(kind, ns, n)


def medir(nombre, dir_):
    print("  %s" % nombre)
    docs = documentos(dir_)
    por_ref = {}
    items = []
    for rel, d in docs:
        k = d.get("kind")
        if k not in KINDS_ITEM:
            continue
        meta = d.get("metadata") or {}
        ns, n = meta.get("namespace"), meta.get("name")
        paquete, carpeta = carpeta_de(rel)
        it = {"ref": ref(k, ns, n), "kind": k, "ns": ns, "name": n, "paquete": paquete, "carpeta": carpeta, "ruta": rel, "spec": d.get("spec") or {}}
        items.append(it)
        por_ref[it["ref"]] = it
    kinds = Counter(i["kind"] for i in items)
    print("     ítems: %d · %s" % (len(items), ", ".join("%s=%d" % kv for kv in sorted(kinds.items()))))
    por_paq = defaultdict(Counter)
    carpetas = defaultdict(Counter)
    for i in items:
        if i["paquete"]:
            por_paq[i["paquete"]][i["kind"]] += 1
            carpetas[i["paquete"]][i["carpeta"]] += 1
    for p in sorted(por_paq):
        print("       %-28s %s · carpetas: %s" % (p, ", ".join("%s=%d" % kv for kv in sorted(por_paq[p].items())), ", ".join("%r=%d" % kv for kv in sorted(carpetas[p].items()))))
    sin_paquete = [i for i in items if not i["paquete"]]
    print("     sin paquete (raíz/vendor): %d · %s" % (len(sin_paquete), ", ".join(sorted(set(i["kind"] for i in sin_paquete)))))

    # ── relaciones esperadas ──
    rel_count = Counter()
    rotas = 0
    def arista(desde, tipo, a):
        nonlocal rotas
        rel_count[tipo] += 1
        if a not in por_ref:
            rotas += 1
    for i in items:
        s = i["spec"]; k = i["kind"]; ns = i["ns"]
        fr = s.get("from")
        if isinstance(fr, dict):
            for kk in ("table", "view", "dataset"):
                if kk in fr:
                    arista(i["ref"], "sale_de", qn_ref(kk, fr[kk], ns))
        elif isinstance(fr, str):
            arista(i["ref"], "sale_de", qn_ref("table", fr, ns))
        if k == "Entity" and s.get("backedBy"):
            b = s["backedBy"]
            r = qn_ref("view", b, ns)
            if r not in por_ref:
                r = qn_ref("dataset", b, ns)
            arista(i["ref"], "respaldada_por", r)
        if k in ("Function", "Action"):
            if s.get("over"):
                arista(i["ref"], "lee", qn_ref("view", s["over"], ns))
            for r_ in s.get("reads") or []:
                arista(i["ref"], "lee", qn_ref("view", r_, ns))
            for e in s.get("effects") or []:
                if isinstance(e, dict) and e.get("writes"):
                    arista(i["ref"], "escribe", qn_ref("entity", str(e["writes"]).rsplit(".", 1)[0], ns))
            if s.get("model"):
                arista(i["ref"], "usa", "model:" + str(s["model"]).split("/")[-1])
        if k == "TrainedModel" and s.get("trainedFrom"):
            arista(i["ref"], "sale_de", qn_ref("view", s["trainedFrom"], ns))
        if k == "Entity":
            for im in s.get("implements") or []:
                arista(i["ref"], "satisface", qn_ref("interface", im, ns))
            for pn, pv in (s.get("properties") or {}).items():
                if isinstance(pv, dict) and pv.get("concept"):
                    arista(i["ref"], "nombra", "concept:" + str(pv["concept"]).split("/")[-1])
    total = sum(rel_count.values())
    print("     relaciones (una dirección; en el índice ×2): %d · %s · rotas: %d" % (total, ", ".join("%s=%d" % kv for kv in sorted(rel_count.items())), rotas))

    # ── identidad de los datasets y vista inducida de las tables ──
    def raiz_columns(i, vistos=()):
        s = i["spec"]; fr = s.get("from")
        if i["kind"] == "Table":
            return set((s.get("columns") or {}).keys())
        if i["kind"] == "Dataset" and "columns" in s:
            return set(s["columns"].keys())
        if isinstance(fr, dict):
            for kk in ("table", "view", "dataset"):
                if kk in fr:
                    r = qn_ref(kk, fr[kk], i["ns"])
                    if r in por_ref and r not in vistos:
                        abajo = por_ref[r]
                        cols = raiz_columns(abajo, vistos + (r,))
                        f = abajo["spec"].get("fields")
                        return set(f.keys()) if isinstance(f, dict) and abajo["kind"] != "Table" else cols
        return set()

    def es_identidad(i):
        s = i["spec"]
        if any(s.get(k) for k in ("where", "groupBy", "having")):
            return False
        f = s.get("fields")
        abajo = raiz_columns(i)
        if f is None:
            return True
        if not isinstance(f, dict) or not abajo:
            return False
        return all(k == v for k, v in f.items()) and set(f.keys()) == abajo

    ds = [i for i in items if i["kind"] == "Dataset"]
    ident = sum(1 for i in ds if "from" in i["spec"] and es_identidad(i))
    escritos = sum(1 for i in ds if "columns" in i["spec"])
    print("     datasets: %d · mantenidos %d (identidad %d) · escritos %d" % (len(ds), len(ds) - escritos, ident, escritos))
    vistas = [i for i in items if i["kind"] == "View"]
    inducidas = {}
    for v in vistas:
        fr = v["spec"].get("from")
        t = fr.get("table") if isinstance(fr, dict) else (fr if isinstance(fr, str) else None)
        if t and "__" in os.path.basename(v["ruta"]) and es_identidad(v):
            inducidas[qn_ref("table", t, v["ns"])] = v["ref"]
    tablas = [i for i in items if i["kind"] == "Table"]
    con_ind = sum(1 for t in tablas if t["ref"] in inducidas)
    print("     views: %d · inducidas identidad sobre una tabla (detalle de la Table): %d · propias: %d" % (len(vistas), len(inducidas), len(vistas) - len(inducidas)))
    print("     tables: %d · con vistaInducida: %d" % (len(tablas), con_ind))
    # punteros
    pd = os.path.join(dir_, "datasets")
    punteros = {}
    if os.path.isdir(pd):
        for f in os.listdir(pd):
            if f.endswith(".json"):
                try:
                    punteros[f[:-5]] = json.load(open(os.path.join(pd, f), encoding="utf-8"))
                except Exception:
                    punteros[f[:-5]] = {"estado": "?"}
    est = Counter(p.get("estado", "?") for p in punteros.values())
    con_puntero = sum(1 for i in ds if "%s_%s" % (i["ns"], i["name"]) in punteros)
    print("     punteros: %d · estados: %s · datasets con puntero: %d / %d" % (len(punteros), ", ".join("%s=%d" % kv for kv in sorted(est.items())), con_puntero, len(ds)))
    # lo que no es ítem
    otros = Counter(d.get("kind") or "(sin kind)" for _, d in docs if d.get("kind") not in KINDS_ITEM)
    print("     no ítems: %s" % ", ".join("%s=%d" % kv for kv in sorted(otros.items())))
    return {"items": len(items), "kinds": dict(kinds), "relaciones": total, "rotas": rotas, "identidad": ident, "inducidas": len(inducidas), "punteros": len(punteros)}


def main():
    args = sys.argv[1:]
    print("0034 · la forma del índice de assets, medida sobre los árboles reales")
    if "--local" in args:
        d = args[args.index("--local") + 1]
        medir(os.path.basename(d), os.path.abspath(d))
        return 0
    celdas = [a for a in args if not a.startswith("--")] or ["demo", "victor"]
    base = tempfile.mkdtemp(prefix="medida-assets-")
    for c in celdas:
        d = traer(c, os.path.join(base, c))
        if d:
            medir(c, d)
    print("  (los árboles quedan en %s; el clúster, como estaba)" % base)
    return 0


if __name__ == "__main__":
    sys.exit(main())
