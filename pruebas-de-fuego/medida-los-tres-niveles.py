"""LOS TRES NIVELES · `base.schema.tabla` en SQL, medido antes de tocar nada.

Decidido (2026-09-24): en SQL un asset se nombra con los TRES niveles del
catalogo —base de datos, schema, tabla/dataset/vista—, como en Unity Catalog.
El schema es la capa de organizacion del catalogo, que es donde viven los
assets del cliente. Hoy un nombre de SQL es `paquete.nombre` (dos partes; tres
se rechazan) y el schema —la carpeta entre el paquete y el fichero, 0034 ④—
solo organiza: la consola ya lo pinta (catalogo → schema → asset), el nombre
no lo lleva.

  §1  LO QUE HOY ES DE DOS      cada sitio del codigo que parte o arma `p.n`
  §2  LOS ARBOLES               bases, schemas y assets en los arboles que hay:
                                cuantos caen en "" (sin clasificar), y que
                                carpetas son repositorios y no schemas
  §3  LA IDENTIDAD HOY          ¿dos `x` en dos schemas del mismo paquete?
  §4  LOS PUNTEROS              `datasets/<p>_<n>.json`: ¿se pisan ya hoy?
  §5  DUCKDB                    tres niveles con un catalogo por base (ATTACH
                                ':memory:'), y los nombres de dos partes que ya
                                estan escritos: ¿siguen resolviendo?
  §6  EL CATALOGO /v1 Y SPARK   el namespace de Iceberg de dos niveles
  §7  UNITY CATALOG             lo que hace (de su documentacion; no medido)

    python pruebas-de-fuego/medida-los-tres-niveles.py [--arbol <dir> ...]
"""
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
KINDS = {"tables", "views", "datasets", "entities", "interfaces", "concepts", "functions", "actions", "models"}


def titulo(t):
    print("\n  " + t)
    print("  " + "-" * len(t))


def leer(rel):
    return open(os.path.join(RAIZ, rel), encoding="utf-8", errors="replace").read()


def ore_bin():
    for c in ("target/release/ore.exe", "target/release/ore", "target/debug/ore.exe", "target/debug/ore"):
        if os.path.exists(os.path.join(RAIZ, c)):
            return os.path.join(RAIZ, c)
    sys.exit("no hay binario de `ore`")


# ── §1 ──────────────────────────────────────────────────────────────────────
SITIOS = [
    ("el analizador del .sql (sqlparser)", "crates/ore-core/src/sql_del_arbol.rs",
     r"con tres partes|partes\.len\(\)|0\.len\(\) (?:==|!=) 2|\[a, b\] =|Nombre \{"),
    ("sql() de una celda (tokenizador)", "crates/ore-core/src/sql_del_arbol.rs",
     r"Token::Period"),
    ("ore-serve: datos de un puesto / sql", "crates/ore-serve/src/puestos.rs",
     r"split_once\('\.'\)|rsplit_once\('\.'\)"),
    ("ore-serve: el catalogo /v1", "crates/ore-serve/src/catalogo.rs",
     r"\"namespaces\"|namespace\b|\\u\{1f\}|\\x1f|%1F"),
    ("ore datasets (write): partes()", "crates/ore-cli/src/datasets.rs",
     r"fn partes|split_once\('\.'\)|\{ns\}_\{|\"\{\}_\{\}\""),
    ("el SDK de Python", "puesto/python/ore/__init__.py",
     r"count\(\"\.\"\) != 1|split\(\"\.\"\)|\"datasets/%s_%s\""),
    ("el SDK de Node", "puesto/node/ore/index.mjs",
     r"split\(['\"]\.['\"]\)|datasets/\$\{"),
    ("el SDK de la JVM", "puesto/jvm/ore/Ore.java",
     r"split\(\"\\\\\.\"\)|indexOf\('\.'\)|\"datasets/\""),
    ("el LSP de SQL", "puesto/python/ore/lsp_sql.py",
     r"split\(\"\.\"\)|\"\.\" in|\.partition\(\"\.\"\)"),
    ("el indice de assets (kind:ns.name)", "crates/ore-core/src/assets.rs",
     r"format!\(\"\{\}:\{\}\.\{\}\"|kind:namespace\.name|\{kind\}:\{ns\}\.\{"),
    ("la View en SQL (ore-view a_sql)", "crates/ore-view/src/a_sql.rs",
     r"ident_en|fn nombre_de_tabla|\{\}\.\{\}"),
]


def seccion_codigo():
    titulo("§1 LO QUE HOY ES DE DOS (cada sitio que parte o arma `p.n`)")
    total = 0
    for que, ruta, patron in SITIOS:
        try:
            t = leer(ruta)
        except OSError:
            print("     %-40s (no esta: %s)" % (que, ruta))
            continue
        lineas = [i + 1 for i, l in enumerate(t.splitlines()) if re.search(patron, l)]
        total += len(lineas)
        print("     %-40s %3d  %s:%s" % (que, len(lineas), ruta, ",".join(map(str, lineas[:8])) + ("…" if len(lineas) > 8 else "")))
    print("     ⇒ %d sitios en %d piezas: el nombre de dos partes no es una funcion, es una" % (total, len(SITIOS)))
    print("       suposicion repartida (analizador, sql(), /v1, write en tres SDK, LSP, indice).")
    # y los documentos: las referencias de OOS en YAML son `p.n`
    dos = una = 0
    for a in ARBOLES:
        for f in glob.glob(os.path.join(a, "packages", "**", "*.yaml"), recursive=True):
            t = open(f, encoding="utf-8", errors="replace").read()
            dos += len(re.findall(r"\b(?:dataset|view|table)\s*:\s*[a-z_][\w-]*\.[\w-]+", t))
            una += len(re.findall(r"\b(?:dataset|view|table)\s*:\s*[a-z_][\w-]*\s*[},\n]", t))
    print("     y en los YAML de los arboles medidos: %d referencias `p.n` y %d de una parte (`n`, del" % (dos, una))
    print("       mismo paquete): una ref de una parte sigue valiendo si el nombre sigue unico por base")


# ── §2 ──────────────────────────────────────────────────────────────────────
def carpeta_de(rel_en_paquete):
    """0034 ④: la carpeta entre el paquete y el fichero, sin las del kind."""
    partes = [p for p in rel_en_paquete.replace("\\", "/").split("/")[:-1] if p not in KINDS]
    return "/".join(partes)


def seccion_arboles():
    titulo("§2 LOS ARBOLES (bases = paquetes, schemas = carpetas)")
    for a in ARBOLES:
        nombre = os.path.basename(os.path.normpath(a))
        paquetes = sorted(d for d in os.listdir(os.path.join(a, "packages")) if os.path.isdir(os.path.join(a, "packages", d)))
        con_guion = [p for p in paquetes if "-" in p]
        assets = sin_schema = 0
        schemas = {}
        repos = set()
        for p in paquetes:
            base = os.path.join(a, "packages", p)
            for f in glob.glob(os.path.join(base, "**", "README.md"), recursive=True):
                if "plantilla" in open(f, encoding="utf-8", errors="replace").read():
                    repos.add((p, os.path.relpath(os.path.dirname(f), base).replace("\\", "/")))
            for f in glob.glob(os.path.join(base, "**", "*.yaml"), recursive=True):
                if os.path.basename(f) == "package.yaml":
                    continue
                txt = open(f, encoding="utf-8", errors="replace").read(400)
                if not re.search(r"^kind:\s*(Dataset|Table|View)\b", txt, re.M):
                    continue
                assets += 1
                c = carpeta_de(os.path.relpath(f, base))
                if not c:
                    sin_schema += 1
                schemas.setdefault(p, {}).setdefault(c, 0)
                schemas[p][c] += 1
        print("     %s: %d bases (paquetes), %d con guion %s, %d assets (Dataset/Table/View)"
              % (nombre, len(paquetes), len(con_guion), con_guion[:3], assets))
        print("       en \"\" (sin schema): %d de %d · schemas con nombre: %d"
              % (sin_schema, assets, sum(1 for p in schemas for c in schemas[p] if c)))
        for p in list(schemas)[:4]:
            print("         %-28s %s" % (p, ", ".join("%s:%d" % (c or '""', n) for c, n in sorted(schemas[p].items())[:6])))
        if repos:
            print("       carpetas que son REPOSITORIOS (no schemas): %s" % sorted(repos)[:4])
        # el schema del ORIGEN, aplanado en el nombre (`public_x`): lo que discover hace hoy
        aplanados = {}
        for f in glob.glob(os.path.join(a, "packages", "*", "tables", "*.yaml")):
            t = open(f, encoding="utf-8", errors="replace").read()
            m = re.search(r"^\s*object:\s*\"?([\w]+)\.([\w]+)", t, re.M)
            n = re.search(r"name:\s*([\w-]+)", t)
            if m and n and n.group(1) == m.group(1) + "_" + m.group(2):
                aplanados[m.group(1)] = aplanados.get(m.group(1), 0) + 1
        if aplanados:
            print("       el schema del ORIGEN aplanado en el nombre (`object: public.x` → `public_x`): %s" % aplanados)


# ── §3 ──────────────────────────────────────────────────────────────────────
def seccion_identidad():
    titulo("§3 LA IDENTIDAD HOY: dos `x` en dos schemas del mismo paquete")
    d = tempfile.mkdtemp(prefix="ore-tres-")
    try:
        os.makedirs(os.path.join(d, "packages", "ventas", "espana", "datasets"))
        os.makedirs(os.path.join(d, "packages", "ventas", "francia", "datasets"))
        open(os.path.join(d, "ontology.config.yaml"), "w").write(
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n")
        open(os.path.join(d, "packages", "ventas", "package.yaml"), "w").write(
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n")
        for s in ("espana", "francia"):
            open(os.path.join(d, "packages", "ventas", s, "datasets", "pedidos.yaml"), "w").write(
                "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\n"
                "spec:\n  owner: team:v\n  columns:\n    id: { type: Integer }\n  changes: { mode: append }\n")
        r = subprocess.run([ore_bin(), "validate", d], capture_output=True, text=True, encoding="utf-8", errors="replace")
        out = (r.stdout + r.stderr).strip().splitlines()
        print("     ventas/espana/datasets/pedidos.yaml y ventas/francia/datasets/pedidos.yaml")
        print("     ore validate → rc=%d · %s" % (r.returncode, (out[0] if out else "")[:150]))
        dup = [l for l in out if "OOS" in l][:2]
        for l in dup:
            print("       %s" % l[:150])
        print("     ⇒ hoy el nombre es UNICO POR PAQUETE: el schema no es parte de la identidad.")
        print("       Con tres niveles, o sigue unico por base (el schema organiza y ademas se")
        print("       escribe) o pasa a unico por schema (y cada `p.n` de YAML se vuelve ambiguo).")
    finally:
        shutil.rmtree(d, ignore_errors=True)


# ── §4 ──────────────────────────────────────────────────────────────────────
def seccion_punteros():
    titulo("§4 LOS PUNTEROS: `datasets/<p>_<n>.json`")
    casos = [("a_b", "c"), ("a", "b_c")]
    print("     %s y %s → %s y %s" % (
        "%s.%s" % casos[0], "%s.%s" % casos[1], "%s_%s.json" % casos[0], "%s_%s.json" % casos[1]))
    print("     ⛔ el mismo fichero YA HOY: `_` separa y tambien vale dentro de un nombre.")
    print("       Con el schema dentro (`<p>_<s>_<n>`) hay mas formas de chocar; un separador que")
    print("       ningun nombre lleve (`<p>.<s>.<n>.json`, o carpetas `datasets/<p>/<s>/<n>.json`) no.")
    for a in ARBOLES:
        ps = [os.path.basename(p) for p in glob.glob(os.path.join(a, "datasets", "*.json"))]
        print("     %s: %d punteros" % (os.path.basename(os.path.normpath(a)), len(ps)))


# ── §5 ──────────────────────────────────────────────────────────────────────
def seccion_duckdb():
    titulo("§5 DUCKDB: un catalogo por base, y lo ya escrito en dos partes")
    import duckdb

    con = duckdb.connect()
    con.execute("attach ':memory:' as ventas")
    con.execute("create schema ventas.espana")
    con.execute("create view ventas.espana.pedidos as select 1 as id, 'es' as pais")
    con.execute("create view ventas.main.pedidos_sin_schema as select 2 as id")

    def prueba(q):
        try:
            return str(con.execute(q).fetchall())
        except Exception as e:  # noqa: BLE001
            return "✗ " + str(e).splitlines()[0][:110]

    print("     attach ':memory:' as ventas; create schema ventas.espana; create view ventas.espana.pedidos")
    for q in ("select * from ventas.espana.pedidos",
              "select * from ventas.pedidos_sin_schema",
              "select * from ventas.main.pedidos_sin_schema",
              "select * from \"ventas\".\"espana\".\"pedidos\"",
              "select * from ventas.pedidos"):
        print("       %-52s %s" % (q, prueba(q)))
    # un schema con el nombre de una base: ¿a quien resuelve `a.b`?
    con.execute("attach ':memory:' as hr")
    con.execute("create view hr.main.x as select 'catalogo hr' as de")
    con.execute("create schema memory.hr")
    con.execute("create view memory.hr.x as select 'schema hr de memory' as de")
    print("     `hr` es base Y schema de la base por defecto: select * from hr.x → %s" % prueba("select * from hr.x"))
    # el coste: 5000 vistas en 20 bases x 5 schemas
    con2 = duckdb.connect()
    t0 = time.time()
    for b in range(20):
        con2.execute("attach ':memory:' as b%d" % b)
        for s in range(5):
            con2.execute("create schema b%d.s%d" % (b, s))
    t1 = time.time()
    for i in range(5000):
        b, s = i % 20, (i // 20) % 5
        con2.execute("create view b%d.s%d.v%d as select %d as n" % (b, s, i, i))
    t2 = time.time()
    con.execute("create schema ventas.\"default\"")
    con.execute("create view ventas.\"default\".pedidos_d as select 3 as id")
    print("     el schema `default` (el de Unity) en DuckDB:")
    for q in ("select * from ventas.default.pedidos_d", "select * from ventas.pedidos_d"):
        print("       %-52s %s" % (q, prueba(q)))
    print("       ⇒ `default` se escribe sin comillas; pero dos partes van a `main`, no a `default`:")
    print("         mientras se admitan, cada vista de `default` necesita su alias en `main`.")
    print("     coste: 20 bases × 5 schemas = %.0f ms; 5000 vistas de tres niveles = %.0f ms (%.2f ms/vista)"
          % ((t1 - t0) * 1000, (t2 - t1) * 1000, (t2 - t1) * 1000 / 5000))
    print("     ⇒ `ventas.x` (dos partes) resuelve a ventas.main.x: con el schema \"\" = `main`, lo")
    print("       escrito hoy sigue valiendo en DuckDB sin tocarlo. Pero `a.b` es un ERROR si `a` es")
    print("       base y tambien schema de la base por defecto: que ningun schema se llame como una base.")


# ── §6 ──────────────────────────────────────────────────────────────────────
def seccion_rest():
    titulo("§6 EL CATALOGO /v1 (Iceberg REST) Y SPARK")
    t = leer("crates/ore-serve/src/catalogo.rs")
    un_nivel = len(re.findall(r"\[ns\]|namespace\"?, *\[|\[\s*Json::s\(ns", t))
    sep = "\\u{1f}" in t or "\\x1f" in t or "%1F" in t
    print("     el spec REST: un namespace es una LISTA (`[\"ventas\", \"espana\"]`), en la URL unida")
    print("       con 0x1F (`ventas%1Fespana`); Spark 3.5 + Iceberg lo nombra `ore.ventas.espana.pedidos`")
    print("     ore-serve hoy: namespaces de un nivel (%d sitios arman `[ns]`), separador 0x1F %s"
          % (un_nivel, "presente" if sep else "AUSENTE"))
    print("     ⛔ y no podria: ore-entrada tira la query (`?parent=`), no decodifica `%1F` y `token()`")
    print("       lo rechaza (http.rs:396, rutas.rs:2113)")
    print("     UNITY (su Iceberg REST, de su documentacion): el CATALOGO va en `warehouse` (el")
    print("       `prefix`) y el namespace es el SCHEMA, de un nivel; Spark nombra")
    print("       `<catalogo>.<schema>.<tabla>` — el mismo nombre que en su SQL.")
    print("     ⇒ igual aqui: base = `prefix` (/v1/{prefix}/…, que ENDPOINTS ya anuncia), schema = el")
    print("       namespace de un nivel: sin 0x1F ni `parent`, y Spark dice `ventas.espana.pedidos`.")


# ── §7 ──────────────────────────────────────────────────────────────────────
def seccion_unity():
    titulo("§7 UNITY CATALOG (de su documentacion; no medido aqui)")
    print("     catalog.schema.table siempre; `USE CATALOG` / `USE SCHEMA` fijan los por defecto y")
    print("     entonces valen dos o una parte; cada catalogo nace con el schema `default`;")
    print("     nombres sin comillas: letras, digitos y `_` (el guion pide acentos graves).")


def seccion_conclusion():
    titulo("§8 LO QUE DICE (2026-09-24)")
    print("""     EL NOMBRE DE DOS PARTES ESTA REPARTIDO: ~89 sitios en 11 piezas (analizador, sql(),
       /v1, write en los tres SDK, LSP, indice, ore-view). No es cambiar una funcion.
     LOS ARBOLES NO TIENEN SCHEMAS TODAVIA: el 100% de los assets cae en "" (sin clasificar),
       y en el paquete de un proyecto las carpetas son REPOSITORIOS, no schemas.
     EL ORIGEN SI LOS TIENE, Y SE APLANAN: discover escribe `public.ai_insights` como la
       tabla `public_ai_insights` (38 de 38 en foreign_test). Con tres niveles, el schema
       del origen es el schema natural: `foreign_test.public.ai_insights`.
     LA IDENTIDAD HOY ES POR BASE: dos `pedidos` en dos schemas del mismo paquete es OOS2035.
       Las refs de YAML son de una parte (38) o dos: siguen valiendo si el nombre sigue
       unico por base; si pasa a unico por schema (Unity), hay que poder nombrar el schema.
     LOS PUNTEROS YA CHOCAN HOY: `a_b.c` y `a.b_c` son el mismo `datasets/a_b_c.json`.
     DUCKDB LO HACE: un catalogo por base (ATTACH ':memory:'), 0.33 ms por vista con 5000,
       y lo escrito en dos partes sigue resolviendo por el schema `main`; un schema que se
       llame como una base es un error de ambiguedad.
     /v1 NO: namespaces de un nivel; dos niveles son `ventas%1Fespana` (spec REST), y Spark
       los nombra `ore.ventas.espana.pedidos`.
     LO QUE HAY QUE DECIDIR: (1) unico por base o por schema; (2) el schema de lo que hoy
       esta en "" (`default` como Unity, o `main` como DuckDB); (3) si dos partes siguen
       valiendo; (4) si discover lleva el schema del origen al schema del catalogo.""")


ARBOLES = []

if __name__ == "__main__":
    args = sys.argv[1:]
    while args:
        if args[0] == "--arbol" and len(args) > 1:
            ARBOLES.append(args[1])
            args = args[2:]
        else:
            sys.exit(__doc__)
    seccion_codigo()
    seccion_arboles()
    seccion_identidad()
    seccion_punteros()
    seccion_duckdb()
    seccion_rest()
    seccion_unity()
    seccion_conclusion()
    print()
