"""EL SQL DEL ARBOL · medido antes de construir nada (el paradigma SQL, con nuestro indice).

La idea anotada en 0037 ⓪ era que en SQL «el servidor de lenguaje somos
nosotros»: el esquema no esta en una base, esta en nuestro indice. Antes de
escribir una linea se mide si eso es verdad, contra que se compara y que le
falta a un `.sql` para ser una unidad del arbol como lo son un `.py` o un
`.java`.

  §1  LO QUE EL INDICE YA SABE    `ore assets --json` sobre arboles reales: que
                                  nombres podria ofrecer un `from `, cuales se
                                  pueden LEER de verdad, y con que columnas y tipos.
  §2  COMO RESUELVE HOY `sql()`   la regex del SDK frente al analizador del propio
                                  DuckDB, caso a caso.
  §3  EL MOTOR COMO COMPROBADOR   DuckDB con tablas VACIAS sacadas del indice:
                                  que dice su binder de una consulta, sin datos.
  §4  LOS SERVIDORES QUE EXISTEN  sql-language-server, postgres-language-server,
                                  sqlfluff y sqruff, corridos contra las semillas.
  §5  QUE LE FALTA A UN `.sql`    entorno, semilla, ejecutor y la forma de
                                  declarar lo que escribe.

    python pruebas-de-fuego/medida-el-sql-del-arbol.py --local <arbol> [--local <arbol> ...]

Los arboles son clones de la forja (demo, victor): la medida no toca el cluster.
"""
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import time
from collections import Counter

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# Los tipos de OOS en DuckDB (0032). Lo que no esta en la tabla —una columna sin
# tipo en el indice— se crea como VARCHAR: el binder sigue viendo el NOMBRE.
TIPO = {"Integer": "BIGINT", "String": "VARCHAR", "Decimal": "DECIMAL(38,9)", "DateTime": "TIMESTAMP",
        "DateTimeTz": "TIMESTAMPTZ", "Date": "DATE", "Boolean": "BOOLEAN", "Float": "DOUBLE",
        "Time": "TIME", "Opaque": "BLOB", "list<String>": "VARCHAR[]"}

# Lo que `sql()` tiene que entender: casos escritos a mano, cada uno por una razon.
CASOS = [
    ("la semilla", "select pais, count(*) as n\nfrom mi_paquete.mi_dataset\ngroup by pais"),
    ("from a, b", "select * from hr.a, hr.b where a.id = b.id"),
    ("comentario", "-- antes: from viejo.tabla\nselect * from hr.a"),
    ("cadena", "select 'from x.y' as s from hr.a"),
    ("comillas", 'select * from "hr"."espanoles"'),
    ("tres partes", "select * from lago.hr.espanoles"),
    ("cte", "with t as (select * from hr.a) select * from t join hr.b using (id)"),
    ("from primero", "from hr.espanoles select nombre"),
    ("subconsulta", "select * from hr.a where id in (select id from hr.b)"),
    ("insert", "insert into hr.salida select * from hr.a"),
    ("create as", "create table hr.salida as select * from hr.a"),
    ("funcion", "select * from read_parquet('x.parquet')"),
    ("unnest", "select * from hr.a, unnest(a.xs) as u(x)"),
]
# Lo que un analizador de verdad tiene que dar: [(lee)], y si escribe, a donde.
ORACULO = {
    "la semilla": ["mi_paquete.mi_dataset"], "from a, b": ["hr.a", "hr.b"], "comentario": ["hr.a"],
    "cadena": ["hr.a"], "comillas": ["hr.espanoles"], "tres partes": ["lago.hr.espanoles"],
    "cte": ["hr.a", "hr.b"], "from primero": ["hr.espanoles"], "subconsulta": ["hr.a", "hr.b"],
    "insert": ["hr.a"], "create as": ["hr.a"], "funcion": [], "unnest": ["hr.a"],
}

# ─────────────────────────────────────────────────────────────────────────────
# Lo medido el 2026-09-24 CORRIENDO cada herramienta (un cliente LSP de verdad
# por stdio con su `Content-Length`, el mismo que midio pyright en 0037 ③a) sobre
# cuatro semillas: `semilla.sql` (la consulta de `transforms-sql`), `victor.sql`
# (una consulta de victor con `customer_stat` mal escrita: EL error de verdad),
# `duckdb.sql` (FROM primero, `exclude`, `group by all`, `qualify`) y `ctas.sql`
# (`create table hr.salida as select ...`). Sin base de datos detras: es lo
# unico que un servidor generico tendria en el puesto.
#
#   sql-language-server 1.7.1   (npm; publicado por ultima vez 2024-11)
#   postgres-language-server 0.25.7  (npm + binario nativo; 2026-07)
#   sqlfluff 4.3.0 · sqruff 0.40.0  (pip; linters, sqruff es un binario Rust)
#   sqlglot 30.19.0             (pip; analizador puro Python con dialecto duckdb)
# ─────────────────────────────────────────────────────────────────────────────
SERVIDORES = {
    "sql-language-server": {
        "disco_mb": 96, "arranca": "NO tal cual: `ERR_PACKAGE_PATH_NOT_EXPORTED` (una dependencia "
                                   "suelta ya rompio su API); arranca fijando el protocolo a 3.17.5 a mano",
        "initialize_s": 1.78, "memoria_mb": 76,
        "semillas": {
            "semilla.sql": "5 avisos de ESTILO (mayusculas, saltos de linea), nada del nombre",
            "victor.sql": "1 ERROR FALSO: `group by 1` «Expected ... but \"1\" found». De `customer_stat`, nada",
            "duckdb.sql": "1 ERROR FALSO en L1: no conoce FROM primero",
            "ctas.sql": "1 ERROR FALSO: `create table hr.salida` «... but \".\" found»",
        },
        "completion": "tras `from `: 5 palabras clave (--, /*, WITH, SELECT, ( ), cero tablas; en una columna: 0",
        "hover": "null",
    },
    "postgres-language-server": {
        "disco_mb": 29, "arranca": "si, pero npm no instala el binario de Windows sin `--force`",
        "initialize_s": 0.15, "memoria_mb": 25,
        "semillas": {
            "semilla.sql": "0",
            "victor.sql": "0 — `customer_stat` pasa: sin una base Postgres conectada no sabe de columnas",
            "duckdb.sql": "3 ERRORES FALSOS: «syntax error at or near \"from\"», «...\"exclude\"», «...\"row_number\"»",
            "ctas.sql": "0",
        },
        "completion": "0 en los dos sitios", "hover": "null",
    },
    "sqlfluff (dialecto duckdb)": {
        "disco_mb": 10, "arranca": "si (linter, no LSP)", "initialize_s": None, "memoria_mb": None,
        "semillas": {
            "semilla.sql": "1 de estilo (LT09)", "victor.sql": "1 de estilo (LT09); de `customer_stat`, nada",
            "duckdb.sql": "3 de estilo; ENTIENDE FROM primero, exclude, group by all y qualify",
            "ctas.sql": "1 de estilo",
        },
        "completion": "-", "hover": "-", "tarda_s": 2.04,
    },
    "sqruff (dialecto duckdb)": {
        "disco_mb": 29, "arranca": "si (linter con modo LSP)", "initialize_s": None, "memoria_mb": None,
        "semillas": {"semilla.sql": "0", "victor.sql": "0", "duckdb.sql": "0 (lo entiende)", "ctas.sql": "0"},
        "completion": "-", "hover": "-", "tarda_s": 0.21,
    },
}
# sqlparser-rs 0.63.0 (Rust; `default-features = false, features = ["std", "visitor"]`:
# sin `recursive`, que arrastra `stacker`/`psm`; queda `log` + su derive), medido
# el 2026-09-24 sobre los 13 casos de aqui MAS seis de DuckDB (exclude, group by
# all, qualify, create or replace, insert or replace, insert ... by name).
SQLPARSER = {
    "duckdb": ("18/19", "`insert ... by name` no analiza"),
    "databricks": ("17/19", "`exclude` y `insert ... by name` no analizan"),
    "hive": ("16/19", "FROM primero, `exclude` y `insert ... by name`"),
    "generic": ("18/19", "`insert ... by name`"),
    "ms_por_consulta": 0.046,
}
SQLGLOT = {"disco_mb": 8.1, "ms_por_consulta": 0.61, "aciertos": "13/13", "destino": "insert y create as: hr.salida"}


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(*p):
    return open(os.path.join(*p), encoding="utf-8", errors="replace").read()


def ore_bin():
    for c in ("target/release/ore.exe", "target/release/ore", "target/debug/ore.exe", "target/debug/ore"):
        if os.path.exists(os.path.join(RAIZ, c)):
            return os.path.join(RAIZ, c)
    return shutil.which("ore")


def indice(arbol):
    t = time.perf_counter()
    r = subprocess.run([ore_bin(), "assets", "--json", arbol], capture_output=True)
    if r.returncode != 0:
        print("     MAL `ore assets` en %s: %s" % (arbol, r.stderr.decode(errors="replace")[:200]))
        return None, 0
    return json.loads(r.stdout.decode("utf-8")), (time.perf_counter() - t) * 1000


def catalogo_de_discover(arbol):
    """Los tipos que `discover` dejo junto al paquete (`discover.catalog.json`)."""
    cat = {}
    for f in glob.glob(os.path.join(arbol, "packages", "*", "discover.catalog.json")):
        try:
            d = json.load(open(f, encoding="utf-8"))
        except Exception:
            continue
        for t in d.get("tables", []):
            cat[(d.get("source"), t.get("name"))] = {c["name"]: c.get("type") for c in t.get("columns", [])}
    return cat


def leible(it, items):
    """Lo que `sql()` puede leer HOY (`datos_de` en ore-serve): un Dataset con
    puntero vivo, o una View cuya raiz de lectura es un Dataset. Una Table es
    409 («se lee por un Dataset que la copie») y una Entity no se nombra."""
    if it["kind"] == "Dataset":
        return (it.get("puntero") or {}).get("estado") in ("copiada", "al-dia")  # `datos_de`
    if it["kind"] == "View":
        de = ((it.get("define") or {}).get("from") or "")
        return de.startswith("dataset:") and leible(items.get(de, {"kind": "?"}), items)
    return False


def seccion_indice(arboles):
    titulo("  §1 LO QUE EL INDICE YA SABE (`ore assets --json`, de verdad)")
    datos = {}
    for arbol in arboles:
        j, ms = indice(arbol)
        if not j:
            continue
        items = j["items"]
        nombre = os.path.basename(os.path.normpath(arbol))
        if nombre == "arbol":
            nombre = os.path.basename(os.path.dirname(os.path.normpath(arbol)))
        # Un clon sin `.git` no dice su commit: se dice de cuando es la copia.
        copia = time.strftime("%Y-%m-%d", time.localtime(os.path.getmtime(os.path.join(arbol, "packages"))))
        por_kind = Counter(i["kind"] for i in items.values())
        nombrables = [i for i in items.values() if i["kind"] in ("Dataset", "View", "Table", "Entity")]
        leibles = [i for i in nombrables if leible(i, items)]
        cols = [(i, c) for i in nombrables for c in (i.get("expone") or [])]
        con_tipo = sum(1 for _, c in cols if c.get("type"))
        cat = catalogo_de_discover(arbol)
        recuperables = 0
        for i, c in cols:
            if not c.get("type") and i["kind"] == "Table" and i.get("detalle"):
                if cat.get((i["detalle"].get("datasource"), i["detalle"].get("object")), {}).get(c["name"]):
                    recuperables += 1
        tabla_sin = sum(1 for i, c in cols if i["kind"] == "Table" and not c.get("type"))
        tabla_todas = sum(1 for i, c in cols if i["kind"] == "Table")
        proyeccion = json.dumps({"%s.%s" % (i["namespace"], i["name"]): {
            "kind": i["kind"], "leible": leible(i, items),
            "columnas": [[c["name"], c.get("type")] for c in (i.get("expone") or [])]} for i in nombrables},
            separators=(",", ":"))
        print("     %s (copia del %s): %d items en %.0f ms (%s)" % (nombre, copia, len(items), ms,
              ", ".join("%s %d" % kv for kv in por_kind.most_common())))
        print("       nombrables tras un `from `: %d  ·  LEIBLES hoy por `sql()`: %d" % (len(nombrables), len(leibles)))
        print("       columnas: %d, con tipo en el indice %d (%.0f %%)" % (len(cols), con_tipo,
              100.0 * con_tipo / max(1, len(cols))))
        print("       de las Table: %d de %d SIN tipo en el indice; %d estan en `discover.catalog.json`"
              % (tabla_sin, tabla_todas, recuperables))
        print("       el indice entero %.0f KB; lo que el SQL necesita de el %.0f KB"
              % (len(json.dumps(j)) / 1024, len(proyeccion) / 1024))
        datos[nombre] = (arbol, j)
    print("     ⇒ Los NOMBRES y las columnas ya estan: es lo que un `from ` y un `select `")
    print("       ofrecerian, y ningun servidor generico lo tiene (§4).")
    print("     ⛔ Los TIPOS dependen de cuando se descubrio la fuente: un arbol descubierto con")
    print("       el `discover` de antes deja la Table con `columns: {x: {}}` y los tipos solo en")
    print("       `discover.catalog.json` y en las Entity; uno de ahora los trae en la Table.")
    print("       Sin tipo el binder (§3) sigue viendo el nombre, pero no la suma de un")
    print("       entero con una cadena.")
    print("     ⛔ Y NOMBRABLE NO ES LEIBLE: una Table es 409 y una Entity no se nombra;")
    print("       ofrecer las dos en un `from ` es ofrecer un error. El autocompletado tiene")
    print("       que decir cual se lee y, de la que no, por que (declara un Dataset).")
    return datos


def refs_duckdb(con, q):
    j = json.loads(con.execute("select json_serialize_sql(?)", [q]).fetchone()[0])
    if j.get("error"):
        return None
    out = set()

    def walk(n, ctes):
        if isinstance(n, dict):
            nombres = set(ctes)
            cm = n.get("cte_map")
            if isinstance(cm, dict):
                nombres |= {e.get("key") for e in cm.get("map", [])}
            if n.get("type") == "BASE_TABLE":
                s, t = n.get("schema_name") or "", n.get("table_name")
                if s or t not in nombres:
                    out.add(".".join(x for x in (n.get("catalog_name") or "", s, t) if x))
            for v in n.values():
                walk(v, nombres)
        elif isinstance(n, list):
            for v in n:
                walk(v, ctes)

    walk(j, set())
    return sorted(out)


def seccion_resolucion():
    titulo("  §2 COMO RESUELVE HOY `sql()` (la regex del SDK frente al analizador de DuckDB)")
    import duckdb

    sdk = leer(RAIZ, "puesto", "python", "ore", "__init__.py")
    regex = re.compile(re.search(r'_VISTAS_EN_SQL = re\.compile\(r"(.+?)"\)', sdk).group(1))
    con = duckdb.connect()
    mal_regex = mal_duck = 0
    for n, q in CASOS:
        r = sorted(set(".".join(m) for m in regex.findall(q)))
        d = refs_duckdb(con, q)
        ok_r, ok_d = r == ORACULO[n], d == ORACULO[n]
        mal_regex += not ok_r
        mal_duck += not ok_d
        print("     %-12s regex %-4s %-26s duckdb %-4s %s" % (n, "ok" if ok_r else "MAL", ",".join(r) or "-",
              "ok" if ok_d else "MAL", "NO ANALIZA (solo SELECT)" if d is None else (",".join(d) or "-")))
    t = time.perf_counter()
    for _ in range(500):
        refs_duckdb(con, CASOS[0][1])
    ms = (time.perf_counter() - t) * 1000 / 500
    print("     regex: %d de %d mal · duckdb: %d de %d mal · duckdb analiza en %.2f ms"
          % (mal_regex, len(CASOS), mal_duck, len(CASOS), ms))
    print("     sqlglot (dialecto duckdb, %.1f MB, NO esta en la imagen): %s, %.2f ms, y el destino"
          % (SQLGLOT["disco_mb"], SQLGLOT["aciertos"], SQLGLOT["ms_por_consulta"]))
    print("       de lo que escribe (%s)" % SQLGLOT["destino"])
    print("     sqlparser-rs 0.63 (Rust, sin `stacker`), %.3f ms, destino incluido:" % SQLPARSER["ms_por_consulta"])
    for d in ("duckdb", "databricks", "hive", "generic"):
        print("       %-11s %s  (%s)" % (d, SQLPARSER[d][0], SQLPARSER[d][1]))
    print("       ⇒ el MISMO analizador, con el dialecto como dato: lo que separa DuckDB de")
    print("         Databricks (Spark) en estos casos es exactamente lo propio de DuckDB.")
    print("     ⛔ Los fallos de la regex no son cosmeticos: lo que resuelve DE MAS (un nombre")
    print("       en un comentario o en una cadena) va a `ore-serve`, es 404 y la celda MUERE")
    print("       por un comentario; lo que resuelve DE MENOS (`from a, b`, `\"hr\".\"x\"`) llega")
    print("       a DuckDB sin vista y es «Catalog Error». Y `lago.hr.x` pide `lago.hr`.")
    print("     ⇒ `json_serialize_sql` es el propio motor y ya esta en las tres imagenes, pero")
    print("       SOLO analiza SELECT: para un `.sql` que escribe hace falta otra cosa.")


def seccion_binder(datos):
    titulo("  §3 EL MOTOR COMO COMPROBADOR (DuckDB, tablas VACIAS con el esquema del indice)")
    import duckdb

    if not datos:
        print("     (sin arbol: pasa --local <arbol>)")
        return
    nombre, (arbol, j) = max(datos.items(), key=lambda kv: len(kv[1][1]["items"]))
    cat = catalogo_de_discover(arbol)
    con = duckdb.connect()
    t = time.perf_counter()
    tablas, columnas = 0, 0
    candidata = None
    for it in j["items"].values():
        if it["kind"] not in ("Dataset", "View", "Table", "Entity") or not it.get("expone"):
            continue
        del_cat = {}
        if it["kind"] == "Table" and it.get("detalle"):
            del_cat = cat.get((it["detalle"].get("datasource"), it["detalle"].get("object")), {})
        defs, tipadas = [], []
        for c in it["expone"]:
            tipo = c.get("type") or del_cat.get(c["name"])
            defs.append('"%s" %s' % (c["name"].replace('"', '""'), TIPO.get(tipo, "VARCHAR")))
            if tipo in ("Integer", "Float", "Decimal"):
                tipadas.append(("num", c["name"]))
            elif tipo == "String":
                tipadas.append(("txt", c["name"]))
        con.execute('create schema if not exists "%s"' % it["namespace"])
        con.execute('create or replace table "%s"."%s" (%s)' % (it["namespace"], it["name"], ", ".join(defs)))
        tablas += 1
        columnas += len(defs)
        kinds = {k for k, _ in tipadas}
        if candidata is None and it["kind"] == "Table" and kinds == {"num", "txt"} and re.fullmatch(r"[a-z_][a-z0-9_]*", it["name"]):
            candidata = (it, dict((k, n) for k, n in reversed(tipadas)))
    print("     %s: %d tablas vacias, %d columnas, en %.0f ms" % (nombre, tablas, columnas, (time.perf_counter() - t) * 1000))
    if not candidata:
        print("     (ninguna Table con una columna de texto y una numerica: no hay consultas que probar)")
        return
    it, c = candidata
    ref = "%s.%s" % (it["namespace"], it["name"])
    txt, num = c["txt"], c["num"]
    pruebas = [
        ("buena", "select %s, count(*) as n from %s group by all" % (txt, ref)),
        ("columna mal", "select %s, count(*) from %s group by 1" % (txt[:-1], ref)),
        ("tabla mal", "select * from %s" % ref[:-1]),
        ("paquete mal", "select * from nadie.%s" % it["name"]),
        ("tipos", "select %s + %s from %s" % (num, txt, ref)),
        ("sintaxis", "select from where %s" % ref),
    ]
    for n, q in pruebas:
        t = time.perf_counter()
        try:
            r = con.sql(q)
            dice = "OK → " + ", ".join("%s %s" % (a, b) for a, b in zip(r.columns, r.types))
        except Exception as e:
            dice = str(e).split("\n")[0][:96]
            extra = [l for l in str(e).split("\n")[1:] if l.strip()][:1]
            if extra:
                dice += " | " + extra[0].strip()[:70]
        print("     %-12s %5.1f ms  %s" % (n, (time.perf_counter() - t) * 1000, dice))
    print("     ⭐ Es exactamente lo que ningun servidor de §4 dice: la columna que no existe,")
    print("       CON LA QUE SE QUERIA DECIR, y el tipo de lo que sale — sin una fila, sin red,")
    print("       con el motor que luego corre la consulta (0 MB: DuckDB ya esta en el puesto).")
    print("     ⛔ La excepcion de Python NO trae la posicion de un error de binder; el AST de")
    print("       `json_serialize_sql` si trae `query_location` en cada nodo: el subrayado sale")
    print("       de cruzar el nombre del mensaje con el nodo que lo lleva.")


def seccion_servidores():
    titulo("  §4 LOS SERVIDORES QUE EXISTEN, CONTRA NUESTRAS SEMILLAS (corridos de verdad)")
    for n, s in SERVIDORES.items():
        vivo = ""
        if s.get("initialize_s") is not None:
            vivo = "arranque %.2fs · %d MB residentes · " % (s["initialize_s"], s["memoria_mb"])
        elif s.get("tarda_s"):
            vivo = "%.2fs por fichero · " % s["tarda_s"]
        print("     %s — %d MB en disco · %sarranca: %s" % (n, s["disco_mb"], vivo, s["arranca"]))
        for f, d in s["semillas"].items():
            print("         %-12s %s" % (f, d))
        if s["completion"] != "-":
            print("         completion   %s · hover %s" % (s["completion"], s["hover"]))
    print("     ⇒ NINGUNO ve el error de verdad de `victor.sql` (`customer_stat`): no pueden,")
    print("       el esquema no esta en ninguna base que puedan abrir. Los dos LSP marcan como")
    print("       error SQL VALIDO del motor que corre (sql-language-server hasta `group by 1`):")
    print("       un subrayado falso es peor que ninguno, ensena a no mirar los subrayados.")
    print("     ⇒ Lo unico que aportan es ESTILO (sqlfluff/sqruff, y entienden DuckDB). Eso es")
    print("       una decision de la casa del cliente, no un servidor de lenguaje.")


def seccion_unidad():
    titulo("  §5 QUE LE FALTA A UN `.sql` PARA SER UNA UNIDAD DEL ARBOL")
    import duckdb

    p = leer(RAIZ, "crates", "ore-serve", "src", "puestos.rs")
    m = re.search(r"no es de ningún entorno: ([^\"]+)\"", p)
    print("     EL EJECUTOR. `POST /trabajos` acepta %s" % (m.group(1) if m else "?"))
    print("       ⇒ un `.sql` es 422: no hay «el mismo fichero corriendo como Job» (0031 §9).")
    c = leer(RAIZ, "crates", "ore-core", "src", "clases.rs")
    sem = re.search(r'id: "transforms-sql".*?semilla: &\[(.*?)\]', c, re.S)
    rutas = re.findall(r'\("([^"]+)"', sem.group(1)) if sem else []
    print("     LA SEMILLA. `transforms-sql` siembra %s: la consulta vive en una CADENA" % ", ".join(rutas))
    print("       de Python, y dentro de una cadena no hay subrayado ni autocompletado posible.")
    con = duckdb.connect()
    con.execute("create schema hr")
    con.execute("create table hr.a as select 1 as id")
    r = con.execute("create table hr.salida as select * from hr.a")
    print("     LO QUE ESCRIBE. Hoy un `.sql` en la sesion va entero a `sql()`. Un")
    print("       `create table hr.salida as select ...` DEVUELVE %s —parece un exito— y no"
          % ([d[0] for d in r.description], ) + "")
    print("       escribe NADA: ni lago, ni puntero, ni linaje. La tabla vive en la memoria de")
    print("       DuckDB y muere con el pod. Es el cartel que viii.b quiso evitar, en silencio.")
    tipos = [(s.type.name, s.query.strip()[:40]) for s in duckdb.extract_statements(
        "create or replace table hr.x as select 1; insert into hr.x select 2; insert or replace into hr.x select 3")]
    print("     LA FORMA. Los tres modos de `write()` ya tienen su frase en SQL, y DuckDB las")
    print("       distingue sin ejecutarlas (`extract_statements`):")
    for (t, q), modo in zip(tipos, ("sobrescribir", "anexar", "upsert")):
        print("         %-12s %-7s %s" % (modo, t, q))
    print("       ⇒ el `.sql` puede DECLARAR lo que escribe en su propio SQL —el destino es")
    print("         `hr.x` y lo que lee sale del FROM—, sin cabecera ni plantilla, como el")
    print("         `@transform(inputs, output)` de Python pero sin escribirlo dos veces.")
    print("     EL ENTORNO. `sql` corre donde corre python (no hay imagen de SQL): DuckDB esta,")
    print("       sqlglot no (%.1f MB). Leer el destino y las entradas de un `create ... as` o un"
          % SQLGLOT["disco_mb"])
    print("       `insert` necesita un analizador que acepte algo mas que SELECT (§2).")
    print("     COMO LO HACEN OTROS (de su documentacion; no medido aqui):")
    print("       dbt       un fichero = un modelo; el nombre del fichero es la salida y las")
    print("                 entradas se escriben `{{ ref('x') }}` (plantilla Jinja: el SQL ya no es SQL)")
    print("       SQLMesh   `MODEL (name ..., kind ...);` arriba y la SELECT debajo; las entradas")
    print("                 NO se declaran: se sacan del SQL con sqlglot")
    print("       Dataform  `.sqlx` con `config { }` y `${ref('x')}`")
    print("       Foundry   `CREATE TABLE `/ruta/salida` AS SELECT ... FROM `/ruta/entrada``: el")
    print("                 destino y las entradas en el propio SQL, por su ruta")
    print("     ⇒ Lo que las cuatro comparten: UN fichero, UNA salida, y las entradas sabidas")
    print("       ANTES de correr (para el grafo, el linaje y el permiso). Y lo que las separa es")
    print("       si el fichero sigue siendo SQL que un editor entiende (SQLMesh, Foundry) o no.")


def main():
    arboles = []
    a = sys.argv[1:]
    while a:
        x = a.pop(0)
        if x == "--local" and a:
            arboles.append(a.pop(0))
    print("=== el SQL del arbol, medido")
    datos = seccion_indice(arboles) if arboles else {}
    seccion_resolucion()
    seccion_binder(datos)
    seccion_servidores()
    seccion_unidad()
    return 0


if __name__ == "__main__":
    sys.exit(main())
