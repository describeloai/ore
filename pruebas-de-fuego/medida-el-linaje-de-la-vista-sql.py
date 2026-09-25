"""MEDIDA · ADR 0040 paso 0 · ¿el linaje derivado del SQL es el del motor?

Decidido: dentro de ORE habrá una sola View, la SQL; las v1alpha8–13 se traducen
a SQL al cargarlas. La puerta para quitar la forma estructurada es que su
traducción, analizada como SQL, dé **el mismo linaje** que el motor de hoy.

  L1  cada View del repositorio (casos, ejemplos, conformance) se traduce a SQL
      de un nivel —la tabla de §7 de `01-la-vista-es-sql`—: cuántas se traducen,
      y las que no, por qué
  L2  el linaje por columna de esa consulta (el prototipo del paso 2,
      `ore_core::vista_sql` por `examples/vista_sql`, sqlparser sin motor), compuesto por la
      cadena de fuentes hasta las columnas raíz, contra el que `ore view`
      imprime (DIRECT / INDIRECT, por raíz): cuántas coinciden, y las que no
  L3  quién supone UNA raíz en el núcleo: los usos de `vistas::raiz*` y de
      `Raiz` fuera de `vistas.rs`, que con varias fuentes cambian de forma

Mide y dice; no falla. Necesita `ore` y `examples/vista_sql` en target/release
y python con pyyaml.
"""
import collections
import glob
import os
import re
import subprocess
import json

import yaml

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
ORE = os.path.join(RAIZ, "target", "release", "ore" + EXE)
PROTO = os.path.join(RAIZ, "target", "release", "examples", "vista_sql" + EXE)

AGREGADO = re.compile(r"^\s*(\w+)\((.*)\)\s*$")
COMPARADOR = re.compile(r"^\s*(>=|<=|!=|==|>|<)\s*(.+)$")


def q(ident):
    return '"%s"' % str(ident).replace('"', '""')


def lit(v):
    if v is None:
        return None
    if isinstance(v, bool):
        return "TRUE" if v else "FALSE"
    if isinstance(v, (int, float)):
        return str(v)
    return "'%s'" % str(v).replace("'", "''")


def paquetes():
    """Cada paquete (un `package.yaml`) con sus documentos."""
    out = {}
    for pk in glob.glob(os.path.join(RAIZ, "**", "package.yaml"), recursive=True):
        if os.sep + "target" + os.sep in pk or "node_modules" in pk:
            continue
        raiz = os.path.dirname(pk)
        docs = []
        for f in glob.glob(os.path.join(raiz, "**", "*.yaml"), recursive=True):
            try:
                for d in yaml.safe_load_all(open(f, encoding="utf-8")):
                    if isinstance(d, dict) and d.get("kind") in ("View", "Table", "Dataset", "Entity"):
                        docs.append(d)
            except Exception:  # noqa: BLE001
                continue
        if any(d["kind"] == "View" for d in docs):
            out[raiz] = docs
    return out


def docs_del_arbol(raiz, docs):
    out = list(docs)
    for f in glob.glob(os.path.join(raiz, "**", "*.yaml"), recursive=True):
        try:
            for d in yaml.safe_load_all(open(f, encoding="utf-8")):
                if isinstance(d, dict) and d.get("kind") in ("View", "Table", "Dataset"):
                    out.append(d)
        except Exception:  # noqa: BLE001
            continue
    return out


def qn(d):
    m = d.get("metadata") or {}
    s = m.get("schema")
    base = "%s.%s.%s" % (m.get("namespace"), s, m.get("name")) if s and s != "default" else "%s.%s" % (m.get("namespace"), m.get("name"))
    return base


def nombre_de_fuente(v):
    """La fuente de una View como nombre del árbol (o el marcador v1alpha7)."""
    f = (v.get("spec") or {}).get("from") or {}
    m = v.get("metadata") or {}
    ns, sc = m.get("namespace"), m.get("schema")
    for k in ("table", "view", "dataset"):
        if k in f:
            n = str(f[k])
            if "." not in n:
                n = "%s.%s.%s" % (ns, sc, n) if sc and sc != "default" else "%s.%s" % (ns, n)
            return n, k
    if "datasource" in f:
        return "@%s·%s" % (f["datasource"], f.get("object")), "datasource"
    return None, None


def traducir(v):
    """La tabla de §7: forma estructurada → consulta SQL de un nivel."""
    s = v.get("spec") or {}
    fuente, clase = nombre_de_fuente(v)
    if not fuente:
        raise ValueError("sin from")
    campos = s.get("fields") or {}
    items, agregados = [], {}
    for sal, val in campos.items():
        col = val.get("column") if isinstance(val, dict) else val
        if not isinstance(col, str):
            raise ValueError("campo %s no es texto" % sal)
        m = AGREGADO.match(col)
        if m:
            fn, arg = m.group(1), m.group(2).strip()
            e = "count(*)" if fn == "count" and not arg else "%s(%s)" % (fn, q(arg))
            agregados[sal] = e
            items.append("%s AS %s" % (e, q(sal)))
        elif col == sal:
            items.append(q(col))
        else:
            items.append("%s AS %s" % (q(col), q(sal)))
    if not items:
        raise ValueError("sin fields")
    desde = ".".join(q(p) for p in fuente.split(".")) if not fuente.startswith("@") else q(fuente)
    sql = "SELECT %s\nFROM %s" % (", ".join(items), desde)
    w = s.get("where") or {}
    conds = []
    for col, val in w.items():
        if isinstance(val, list):
            nulos = any(x is None for x in val)
            vals = [lit(x) for x in val if x is not None]
            partes = []
            if vals:
                partes.append("%s IN (%s)" % (q(col), ", ".join(vals)))
            if nulos:
                partes.append("%s IS NULL" % q(col))
            conds.append("(" + " OR ".join(partes) + ")" if len(partes) > 1 else partes[0])
        elif val is None:
            conds.append("%s IS NULL" % q(col))
        else:
            conds.append("%s = %s" % (q(col), lit(val)))
    if conds:
        sql += "\nWHERE " + " AND ".join(conds)
    g = s.get("groupBy") or []
    if g:
        sql += "\nGROUP BY " + ", ".join(q(c) for c in g)
    h = s.get("having") or {}
    hs = []
    for campo, cond in h.items():
        m = COMPARADOR.match(str(cond))
        if not m or campo not in agregados:
            raise ValueError("having %s: %s" % (campo, cond))
        op = "=" if m.group(1) == "==" else m.group(1)
        hs.append("%s %s %s" % (agregados[campo], op, m.group(2)))
    if hs:
        sql += "\nHAVING " + " AND ".join(hs)
    return sql, clase


def raiz_del_arbol(pkg):
    """El árbol de un paquete: el `ontology.config.yaml` más cercano hacia arriba."""
    d = pkg
    while True:
        if os.path.exists(os.path.join(d, "ontology.config.yaml")):
            return d
        p = os.path.dirname(d)
        if p == d or not p.startswith(RAIZ):
            return pkg
        d = p


def lineaje_del_motor(pkg):
    """`ore view` → {vista: {(salida, raíz, D|I)}}; y las que no siguen."""
    r = subprocess.run([ORE, "view", raiz_del_arbol(pkg)], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120)
    out, fallos, vista = {}, {}, None
    for linea in r.stdout.splitlines():
        if linea and not linea.startswith(" "):
            vista = linea.strip()
            continue
        m = re.match(r"^  linaje    (.+?) ← (.+?)  (DIRECT|INDIRECT) · (.+)$", linea)
        if m and vista:
            out.setdefault(vista, set()).add((m.group(1), m.group(2), m.group(3)[0]))
        elif vista and re.match(r"^  (plan|linaje|esquema)\s+(no se|no tipa)", linea):
            fallos[vista] = linea.strip()
    return out, fallos, r.returncode, (r.stderr or r.stdout).strip().splitlines()[:1]


def proto(consultas):
    entrada = "".join("-- @@ %d\n%s\n" % (i, s) for i, s in enumerate(consultas))
    r = subprocess.run([PROTO], input=entrada, capture_output=True, text=True, encoding="utf-8")
    res = {}
    for linea in r.stdout.splitlines():
        i, j = linea.split("\t", 1)
        res[int(i)] = json.loads(j)
    return [res.get(i, {"error": "sin salida"}) for i in range(len(consultas))]


class Arbol:
    def __init__(self, docs):
        self.docs = {}
        for d in docs:
            self.docs.setdefault(qn(d), d)
        self.cache = {}

    def doc(self, n):
        return self.docs.get(n) or self.docs.get(n.replace(".default.", "."))

    def columnas(self, n):
        """Columnas de una fuente, para expandir `*`."""
        d = self.doc(n)
        if not d:
            return []
        s = d.get("spec") or {}
        if d["kind"] == "Table":
            return list((s.get("columns") or {}).keys())
        if d["kind"] == "View":
            return list((s.get("fields") or {}).keys())
        return list((s.get("columns") or s.get("schema") or {}).keys()) if isinstance(s.get("columns") or s.get("schema"), dict) else []

    def raices(self, fuente, col):
        """(fuente, columna) → {(raíz, D|I)} compuesta por la cadena."""
        if fuente.startswith("@"):
            return {("%s.%s" % (fuente[1:], col), "D")}
        d = self.doc(fuente)
        if not d:
            return {("?%s.%s" % (fuente, col), "D")}
        s = d.get("spec") or {}
        if d["kind"] == "Table":
            return {("%s·%s.%s" % (s.get("datasource"), s.get("object"), col), "D")}
        if d["kind"] == "Dataset":
            m = d.get("metadata") or {}
            return {("lago·%s_%s.%s" % (m.get("namespace"), m.get("name"), col), "D")}
        lin = self.linaje(d)
        if lin is None:
            return {("?%s.%s" % (fuente, col), "D")}
        return {(r, c) for (o, r, c) in lin if o == col}

    def linaje(self, v):
        """View → {(salida, raíz, D|I)} derivado de su SQL traducido."""
        k = qn(v)
        if k in self.cache:
            return self.cache[k]
        self.cache[k] = None
        try:
            sql, _ = traducir(v)
        except ValueError:
            return None
        j = proto([sql])[0]
        if "error" in j:
            return None
        out = set()
        ind = set()
        for t, c in j["ind"]:
            for r, _ in self.raices(t, c):
                ind.add(r)
        for nombre, dirs, ders, inds in j["cols"]:
            for t, c in inds:
                for r, _ in self.raices(t, c):
                    out.add((nombre, r, "I"))
            pares = []
            for t, c in dirs:
                if c == "*":
                    for cc in self.columnas(t):
                        pares.append((cc, t, cc, "D"))
                else:
                    pares.append((nombre, t, c, "D"))
            for t, c in ders:
                pares.append((nombre, t, c, "d"))
            for sal, t, c, cls in pares:
                for r, cr in self.raices(t, c):
                    out.add((sal, r, "I" if cr == "I" else "D"))
        salidas = {o for (o, _, _) in out} | {n for n, _, _, _ in j["cols"] if n != "*"}
        for sal in salidas:
            for r in ind:
                out.add((sal, r, "I"))
        # una raíz indirecta de la fuente sube como indirecta a todas las salidas
        self.cache[k] = out
        return out


def l3():
    print("L3 · quién supone UNA raíz (fuera de `vistas.rs`)")
    usos = collections.Counter()
    for f in glob.glob(os.path.join(RAIZ, "crates", "**", "*.rs"), recursive=True):
        if f.endswith(os.sep + "vistas.rs") or os.sep + "tests" + os.sep in f:
            continue
        t = open(f, encoding="utf-8").read()
        n = len(re.findall(r"vistas::(raiz|raiz_de_lectura|suelo|datasources_de|Raiz|fuente|Fuente)\b", t))
        if n:
            usos[os.path.relpath(f, RAIZ).replace(os.sep, "/")] = n
    print("  · %d usos en %d ficheros" % (sum(usos.values()), len(usos)))
    for f, n in usos.most_common():
        print("    - %-50s %d" % (f, n))


def main():
    if not (os.path.exists(ORE) and os.path.exists(PROTO)):
        print("⛔ falta ore o examples/vista_sql en target/release")
        return
    pk = paquetes()
    print("L0 · %d paquetes con Views" % len(pk))
    traducibles, total, motivos = 0, 0, collections.Counter()
    clases = collections.Counter()
    todas = []
    for raiz, docs in pk.items():
        for d in docs:
            if d["kind"] != "View" or "sql" in (d.get("spec") or {}):
                continue
            total += 1
            try:
                sql, clase = traducir(d)
                traducibles += 1
                clases[clase] += 1
                todas.append(sql)
            except ValueError as e:
                motivos[re.sub(r"\s.*", "", str(e))] += 1
    js = proto(todas)
    analizadas = sum(1 for j in js if "error" not in j)
    print("L1 · %d Views · se traducen %d (por fuente: %s) · no: %s · el prototipo analiza %d de %d" % (total, traducibles, dict(clases), dict(motivos), analizadas, len(todas)))
    errores = collections.Counter(j["error"][:60] for j in js if "error" in j)
    for e, n in errores.most_common(5):
        print("    - %d × %s" % (n, e))

    print("L2 · el linaje del SQL contra el de `ore view`")
    iguales, distintas, sin_motor, sin_sql = 0, 0, 0, 0
    difs = collections.Counter()
    ejemplos = []
    paquetes_ok = 0
    for raiz, docs in pk.items():
        motor, fallos, rc, err = lineaje_del_motor(raiz)
        if not motor:
            sin_motor += sum(1 for d in docs if d["kind"] == "View" and "sql" not in (d.get("spec") or {}))
            continue
        paquetes_ok += 1
        arb = Arbol(docs_del_arbol(raiz_del_arbol(raiz), docs))
        for d in docs:
            if d["kind"] != "View" or "sql" in (d.get("spec") or {}):
                continue
            n = qn(d)
            m = motor.get(n)
            if m is None:
                sin_motor += 1
                continue
            s = arb.linaje(d)
            if s is None:
                sin_sql += 1
                continue
            if s == m:
                iguales += 1
            else:
                distintas += 1
                solo_m, solo_s = m - s, s - m
                clave = []
                if solo_m:
                    clave.append("motor+" + ",".join(sorted({c for _, _, c in solo_m})))
                if solo_s:
                    clave.append("sql+" + ",".join(sorted({c for _, _, c in solo_s})))
                difs[" ".join(clave)] += 1
                if len(ejemplos) < 20:
                    ejemplos.append((os.path.relpath(raiz, RAIZ), n, sorted(solo_m)[:3], sorted(solo_s)[:3]))
    comparadas = iguales + distintas
    print("  · %d paquetes que `ore view` sigue · %d Views comparadas: iguales %d · distintas %d · sin linaje en el motor %d · sin SQL %d" % (paquetes_ok, comparadas, iguales, distintas, sin_motor, sin_sql))
    for k, v in difs.most_common():
        print("    - %d × %s" % (v, k))
    for e in ejemplos:
        print("    · %s %s\n        sólo motor %s\n        sólo SQL   %s" % e)
    l3()


if __name__ == "__main__":
    main()
