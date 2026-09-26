#!/usr/bin/env python3
"""
MEDIDA · migrar las vistas estructuradas a v1alpha14 (ADR 0040 paso 6, antes de escribirlo).

Cada árbol con vistas de v1alpha8–13 —los casos de la conformance, válidos e
inválidos, y `casos/`— se copia y cada View estructurada se reescribe como
v1alpha14: su consulta de `linaje::como_sql` (la misma traducción con la que el
núcleo las lee desde el paso 3) y su contrato del esquema que `ore view` da de
ella (el plan del motor). Luego `ore validate`, antes y después:

  M1  cuántas se traducen, y cuántas tienen contrato entero (cada columna tipada)
  M2  los válidos siguen válidos: 0 errores después
  M3  los inválidos siguen diciendo su código (el de `case.yaml`)
  M4  lo que cambia: los códigos nuevos o perdidos, con el árbol que los da
  M5  los tipos del contrato que no son de la fuente (Money<…>: el afinado de la
      Entity se colaba en el esquema) y los que faltan

Y después, la de verdad (paso 6a): `ore migrate v1alpha14` sobre cada árbol.

  M6  los válidos migran (código 0), quedan sin una vista estructurada y con
      0 errores; los que no migra, por qué
  M7  los inválidos: o migran y siguen diciendo su código, o no migran

Uso:  python pruebas-de-fuego/medida-migrar-a-v14.py [--detalle] [--solo-ore]
Necesita target/release/ore (o ORE=…) y el ejemplo `como_sql`
(cargo build --release -p ore-core --example como_sql).
"""
import collections
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile

import yaml

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
ORE = os.environ.get("ORE") or os.path.join(RAIZ, "target", "release", "ore" + EXE)
COMO_SQL = os.path.join(RAIZ, "target", "release", "examples", "como_sql" + EXE)
DETALLE = "--detalle" in sys.argv
SOLO_ORE = "--solo-ore" in sys.argv
FORMA = {"from", "fields", "where", "groupBy", "having", "aggregates", "materialized", "select"}


def correr(*args, cwd=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=cwd)
    return r.returncode, r.stdout + r.stderr


def codigos(arbol):
    _, out = correr(ORE, "validate", arbol)
    return collections.Counter(m for m in re.findall(r"error\[(OOS\d+)\]", out))


def esquemas(arbol):
    """qn → {col: tipo} de `ore view` (la línea `esquema`)."""
    _, out = correr(ORE, "view", arbol)
    out_, qn = {}, None
    for l in out.splitlines():
        if l and not l.startswith(" "):
            qn = l.strip()
        elif l.startswith("  esquema") and qn:
            txt = l[len("  esquema"):].strip()
            if txt.startswith("no tipa") or not txt:
                continue
            cols = {}
            for par in txt.split(" · "):
                if ": " in par:
                    c, t = par.split(": ", 1)
                    cols[c.strip()] = t.strip()
            out_[qn] = cols
    return out_


def consultas(arbol):
    _, out = correr(COMO_SQL, arbol)
    r = {}
    for l in out.splitlines():
        p = l.split("\t")
        if len(p) == 3:
            r[p[0]] = p[2].replace("\\n", "\n")
    return r


def qn_de(doc):
    m = doc.get("metadata") or {}
    s = m.get("schema")
    return ".".join([m.get("namespace", "")] + ([s] if s and s != "default" else []) + [m.get("name", "")])


def migrar(arbol, cuenta):
    """Reescribe en sitio las View estructuradas de `arbol`. Devuelve lo que pasó."""
    sql, esq = consultas(arbol), esquemas(arbol)
    for f in glob.glob(os.path.join(arbol, "**", "*.yaml"), recursive=True):
        try:
            texto = open(f, encoding="utf-8").read()
            docs = list(yaml.safe_load_all(texto))
        except Exception:
            continue
        if len(docs) != 1 or not isinstance(docs[0], dict) or docs[0].get("kind") != "View":
            continue
        d = docs[0]
        spec = d.get("spec") or {}
        if "sql" in spec:
            cuenta["ya SQL"] += 1
            continue
        if not (FORMA & set(spec)):
            continue
        cuenta["estructuradas"] += 1
        qn = qn_de(d)
        if qn not in sql:
            cuenta["sin traducción"] += 1
            cuenta.setdefault("_sin_traduccion", []).append(qn)
            continue
        cuenta["traducidas"] += 1
        cols = esq.get(qn)
        if not cols:
            cuenta["sin contrato (el plan no tipa)"] += 1
            cuenta.setdefault("_sin_contrato", []).append(qn)
            continue
        for t in cols.values():
            if "<" in t and not t.startswith("list<"):
                cuenta["columnas con tipo afinado (Money/Quantity)"] += 1
        nuevo = {"apiVersion": "oos.dev/v1alpha14", "kind": "View", "metadata": d.get("metadata")}
        s2 = {}
        for k in ("owner", "moved", "reserved"):
            if k in spec:
                s2[k] = spec[k]
        s2["dialect"] = "duckdb"
        s2["sql"] = sql[qn] + "\n"
        s2["columns"] = {c: {"type": t} for c, t in cols.items()}
        nuevo["spec"] = s2
        open(f, "w", encoding="utf-8", newline="\n").write(yaml.safe_dump(nuevo, sort_keys=False, allow_unicode=True))
        cuenta["migradas"] += 1


def casos():
    out = []
    for v in range(8, 14):
        for clase in ("valid", "invalid"):
            for c in sorted(glob.glob(os.path.join(RAIZ, "vendor", "oos", "conformance", "v1alpha%d" % v, clase, "*"))):
                i = os.path.join(c, "input")
                if os.path.isdir(i) and glob.glob(os.path.join(i, "**", "*.yaml"), recursive=True):
                    txt = " ".join(open(f, encoding="utf-8", errors="replace").read() for f in glob.glob(os.path.join(i, "**", "*.yaml"), recursive=True))
                    if "kind: View" in txt:
                        espera = None
                        cy = os.path.join(c, "case.yaml")
                        if os.path.isfile(cy):
                            m = re.search(r"OOS\d{4}", open(cy, encoding="utf-8").read())
                            espera = m.group(0) if m else None
                        out.append(("v1alpha%d/%s/%s" % (v, clase, os.path.basename(c)), i, clase, espera))
    for c in sorted(glob.glob(os.path.join(RAIZ, "casos", "*"))):
        if os.path.isfile(os.path.join(c, "ontology.config.yaml")):
            out.append(("casos/" + os.path.basename(c), c, "valid", None))
    return out


def estructuradas(arbol):
    n = 0
    for f in glob.glob(os.path.join(arbol, "**", "*.yaml"), recursive=True):
        try:
            for d in yaml.safe_load_all(open(f, encoding="utf-8")):
                if isinstance(d, dict) and d.get("kind") == "View" and FORMA & set(d.get("spec") or {}):
                    n += 1
        except Exception:
            pass
    return n


def con_ore():
    """M6 y M7: el migrador de verdad."""
    lista = casos()
    tmp = tempfile.mkdtemp(prefix="ore-migrate-v14-")
    migrados, rechazos, rotos, quedan, mudos, rechazados_inv = 0, [], [], [], [], 0
    for nombre, arbol, clase, espera in lista:
        copia = os.path.join(tmp, nombre.replace("/", "__"))
        shutil.copytree(arbol, copia)
        rc, out = correr(ORE, "migrate", "v1alpha14", copia)
        despues = codigos(copia)
        if clase == "valid":
            if rc != 0:
                rechazos.append((nombre, rc, out.strip().splitlines()[-3:]))
                continue
            migrados += 1
            if sum(despues.values()):
                rotos.append((nombre, dict(despues)))
            if estructuradas(copia):
                quedan.append((nombre, estructuradas(copia)))
        else:
            if rc != 0:
                rechazados_inv += 1
            elif espera and espera not in despues:
                mudos.append((nombre, espera, dict(despues)))
    shutil.rmtree(tmp, ignore_errors=True)
    validos = sum(1 for x in lista if x[2] == "valid")
    print("M6 · válidos: %d de %d migran · %d con errores después · %d con vistas estructuradas aún" % (
        migrados, validos, len(rotos), len(quedan)))
    for n, rc, cola in rechazos:
        print("  no migra   %-60s rc=%d" % (n, rc))
        for l in cola:
            print("             %s" % l)
    for n, d in rotos:
        print("  roto       %-60s %s" % (n, d))
    for n, k in quedan:
        print("  quedan     %-60s %d" % (n, k))
    print()
    print("M7 · inválidos: %d no migran (el árbol no carga o no se migra solo) · %d migran y callan su código" % (
        rechazados_inv, len(mudos)))
    for n, e, d in mudos:
        print("  mudo       %-60s esperaba %s · da %s" % (n, e, d))


def main():
    if SOLO_ORE:
        if not os.path.isfile(ORE):
            sys.exit("falta %s" % ORE)
        con_ore()
        return
    for p in (ORE, COMO_SQL):
        if not os.path.isfile(p):
            sys.exit("falta %s" % p)
    total = collections.Counter()
    sin_traduccion, sin_contrato = [], []
    cambios = []
    validos_rotos = []
    invalidos_mudos = []
    tmp = tempfile.mkdtemp(prefix="ore-migrar-")
    lista = casos()
    for nombre, arbol, clase, espera in lista:
        copia = os.path.join(tmp, nombre.replace("/", "__"))
        shutil.copytree(arbol, copia)
        antes = codigos(copia)
        cuenta = collections.Counter()
        migrar(copia, cuenta)
        sin_traduccion += [(nombre, q) for q in cuenta.pop("_sin_traduccion", [])]
        sin_contrato += [(nombre, q) for q in cuenta.pop("_sin_contrato", [])]
        total.update(cuenta)
        despues = codigos(copia)
        if antes != despues:
            cambios.append((nombre, clase, dict(antes), dict(despues)))
        if clase == "valid" and sum(despues.values()):
            validos_rotos.append((nombre, dict(despues)))
        if clase == "invalid" and espera and espera not in despues:
            invalidos_mudos.append((nombre, espera, dict(despues)))
    shutil.rmtree(tmp, ignore_errors=True)

    print("corpus: %d árboles con vistas (%d válidos, %d inválidos)" % (
        len(lista), sum(1 for x in lista if x[2] == "valid"), sum(1 for x in lista if x[2] == "invalid")))
    print()
    print("M1 · las vistas")
    for k in ("estructuradas", "ya SQL", "traducidas", "sin traducción", "sin contrato (el plan no tipa)", "migradas",
              "columnas con tipo afinado (Money/Quantity)"):
        print("  %-44s %d" % (k, total[k]))
    print()
    print("M2 · válidos que dejan de serlo: %d" % len(validos_rotos))
    for n, d in validos_rotos:
        print("  %-70s %s" % (n, d))
    print()
    print("M3 · inválidos que dejan de decir su código: %d" % len(invalidos_mudos))
    for n, e, d in invalidos_mudos:
        print("  %-70s esperaba %s · da %s" % (n, e, d))
    print()
    nuevos, perdidos = collections.Counter(), collections.Counter()
    for n, clase, a, d in cambios:
        for c in set(a) | set(d):
            x = d.get(c, 0) - a.get(c, 0)
            if x > 0:
                nuevos[c] += x
            elif x < 0:
                perdidos[c] += -x
    print("M4 · árboles cuyo validate cambia: %d · códigos nuevos %s · perdidos %s" % (len(cambios), dict(nuevos), dict(perdidos)))
    if DETALLE:
        for n, clase, a, d in cambios:
            print("  %-70s %s → %s" % (n, a, d))
    print()
    print("M5 · sin traducción: %d · sin contrato: %d" % (len(sin_traduccion), len(sin_contrato)))
    for n, q in (sin_traduccion + sin_contrato)[:20]:
        print("  %-70s %s" % (n, q))
    print()
    con_ore()


if __name__ == "__main__":
    main()
