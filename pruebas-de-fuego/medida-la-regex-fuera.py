"""LA REGEX FUERA DE `sql()` · el espectro entero de la iteracion, medido antes.

`sql()` encuentra los nombres con una regex (`VISTAS_EN_SQL`, igual en los tres
SDK) y `medida-el-sql-del-arbol.py` §2 midio donde falla: 5 de 13 casos, y lo
que resuelve de mas (un nombre en un comentario) mata la celda. Quitarla no es
cambiar una linea: la regex es la primera pieza de un camino que ademas ANOTA
lo que se lee (la procedencia), ACOTA a los inputs de un transform, pide la
CREDENCIAL de cada dataset y sabe DESENVOLVER el sobre heredado. Antes de tocar
nada se mide todo lo que se mueve.

  §1  LO QUE LA REGEX ARRASTRA    en cada SDK, las piezas del camino de lectura.
  §2  EL CONTRATO QUE SE FIJO     lo que `el-puesto.sh` afirma de `sql()`/`over()`:
                                  los errores con su tipo, en los tres lenguajes.
  §3  LOS DATOS DE VERDAD         que clase de puntero hay en los arboles reales
                                  (¿queda algun sobre `ORECOPY1`?).
  §4  LOS TRES MOTORES            ATTACH del catalogo en Python, Node y la JVM.
  §5  EL CORPUS                   cada SQL que el repositorio manda de verdad a
                                  `sql()`: regex, DuckDB y `ore sql` (sqlparser).
  §6  LAS DOS RUTAS, PIEZA A PIEZA  (A) el motor pregunta al catalogo; (B) ore-serve
                                  analiza el texto y resuelve de una vez.

    python pruebas-de-fuego/medida-la-regex-fuera.py [--local <arbol> ...]
"""
import glob
import json
import os
import re
import subprocess
import sys
import tempfile

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

SDK = {
    "python": "puesto/python/ore/__init__.py",
    "node": "puesto/node/ore/index.mjs",
    "jvm": "puesto/jvm/ore/Ore.java",
}
# Las piezas del camino de lectura, por lo que las nombra en cada SDK.
PIEZAS = [
    ("la regex", r"VISTAS_EN_SQL"),
    ("resolver (GET datos)", r"/datos/"),
    ("anotar/acotar lo leido", r"\bleidas\b|_lee\(|\blee\("),
    ("credencial por raiz (secretos)", r"create or replace secret"),
    ("el sobre ORECOPY1", r"ORECOPY1|MAGIA|_parquet_de|parquetDe"),
]

# ─────────────────────────────────────────────────────────────────────────────
# §4 · lo medido por `medida-el-catalogo-como-resolutor.py` (Python, en vivo,
# 2026-09-24) y su §8 (Node y la JVM). Se rellena con lo que esa medida dio.
# ─────────────────────────────────────────────────────────────────────────────
MOTORES = {
    "python": "duckdb 1.5.4 · ATTACH si · 2a consulta 69-103 ms · join 152-213 ms · el 403: "
              "`HTTPException: HTTP Error: GetTableInformation … Forbidden_403 with message \"OOS4002: …\"`",
    "node": "@duckdb/node-api 1.5.5-r.5 · ATTACH si · 2a 72-94 ms · join 183-215 ms · el 403: "
            "`Error: HTTP Error: … Forbidden_403 with message \"OOS4002: …\"`",
    "jvm": "JDBC 1.5.5.1 · ATTACH si · 2a 66-116 ms · join 139-238 ms · el 403: `SQLException: Invalid Input "
           "Error: Attempting to execute an unsuccessful or closed pending query result Error: HTTP Error: …`",
}
# El MISMO SQL en los tres (TOKEN + secreto http con `x-ore-puesto`, ATTACH READ_ONLY,
# search_path 'memory.main,ore.<p>,…'); /v1: attach 1 peticion, 1a consulta 2, 2a 1.
# Lo unico que pide codigo distinto por SDK: el prefijo del error en la JVM, y la
# forma de ejecutar de cada API. Medido en `medida-el-catalogo-en-los-tres.py`.


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(rel):
    return open(os.path.join(RAIZ, rel), encoding="utf-8", errors="replace").read()


def seccion_arrastre():
    titulo("  §1 LO QUE LA REGEX ARRASTRA (en cada SDK, contado del codigo)")
    print("     %-32s %8s %8s %8s" % ("", "python", "node", "jvm"))
    for nombre, patron in PIEZAS:
        fila = []
        for lenguaje, ruta in SDK.items():
            fila.append(len(re.findall(patron, leer(ruta))))
        print("     %-32s %8d %8d %8d" % (nombre, *fila))
    print("     ⇒ No es una linea: en los tres, la regex alimenta un camino que resuelve por")
    print("       `datos` (y ahi deciden el conducto, la rama y lo declarado), anota la")
    print("       procedencia, acota un transform, presta credenciales por raiz y sabe leer")
    print("       el sobre heredado. Lo que se haga, se hace TRES veces, o se sube al servidor.")


def seccion_contrato():
    titulo("  §2 EL CONTRATO QUE SE FIJO (lo que `el-puesto.sh` afirma de la lectura)")
    t = leer("pruebas-de-fuego/el-puesto.sh")
    lineas = [l for l in t.splitlines() if re.search(r"celda_sql |sql\(|over\(", l) and "tiene" in l]
    print("     afirmaciones sobre sql()/over(): %d" % len(lineas))
    for tipo in ("LookupError", "RuntimeError", "PermissionError", "OOS4002"):
        n = sum(1 for l in lineas if tipo in l)
        print("       nombran %-16s %d" % (tipo, n))
    print("     ⛔ el fallback de la rama a `main` (`datos_del_puesto`) no lo afirma NINGUNA")
    print("       prueba de fuego: si el camino se mueve, no hay red que lo sostenga.")
    print("     ⛔ Ese es el contrato de hoy, en los tres lenguajes: un nombre que no esta es")
    print("       LookupError (404), uno sin copia RuntimeError (409), uno que el conducto")
    print("       niega PermissionError con OOS4002. Por ATTACH los errores son los de DuckDB")
    print("       («Catalog Error …», «HTTP Error … 403 …»): o el SDK los traduce, o el")
    print("       contrato cambia — y cambia en tres sitios.")


def seccion_datos(arboles):
    titulo("  §3 LOS DATOS DE VERDAD (los punteros de `datasets/`)")
    if not arboles:
        print("     (pasa --local <arbol>)")
    for a in arboles:
        clases = {}
        for f in glob.glob(os.path.join(a, "datasets", "*.json")):
            try:
                j = json.load(open(f, encoding="utf-8"))
            except Exception:
                continue
            k = "iceberg (metadata_location)" if j.get("metadata_location") else (
                "sobre ORECOPY1 (clave)" if j.get("clave") else "sin datos")
            k += " · " + str(j.get("estado"))
            clases[k] = clases.get(k, 0) + 1
        nombre = os.path.basename(os.path.normpath(a))
        print("     %s: %s" % (nombre, ", ".join("%s %d" % kv for kv in sorted(clases.items())) or "ninguno"))
    t = leer("pruebas-de-fuego/el-puesto.sh")
    print("     en las pruebas: `ORECOPY1` aparece %d veces en el-puesto.sh (el caso 4 lee hr.espanoles asi)"
          % t.count("ORECOPY1"))
    print("     ⇒ Si ningun arbol real tiene ya un sobre, su lectura (en los tres SDK) es codigo")
    print("       que solo ejercitan las pruebas: quitarla junto con la regex es una decision,")
    print("       y cambia el caso 4.")


def seccion_motores():
    titulo("  §4 LOS TRES MOTORES (ATTACH del catalogo; de la medida del catalogo)")
    if not MOTORES:
        print("     (sin rellenar)")
    for m, d in MOTORES.items():
        print("     %-8s %s" % (m, d))


def corpus():
    """Cada SQL que el repositorio manda a `sql()`: las celdas SQL de el-puesto.sh y
    las consultas dentro de `sql(\"...\")` en los tres lenguajes."""
    t = leer("pruebas-de-fuego/el-puesto.sh")
    out = []
    for m in re.finditer(r"celda_sql '([^']+)'", t):
        out.append(m.group(1))
    for m in re.finditer(r'sql\(\\"(.+?)\\"', t):
        out.append(m.group(1).replace("$$", "'"))
    # y los casos de la medida del SQL del arbol, que son los que la regex falla
    spec = open(os.path.join(AQUI, "medida-el-sql-del-arbol.py"), encoding="utf-8").read()
    for m in re.finditer(r'\("[^"]+", "((?:[^"\\]|\\.)*)"\)', spec.split("ORACULO")[0]):
        out.append(m.group(1).encode().decode("unicode_escape"))
    vistos, unicos = set(), []
    for q in out:
        if q not in vistos:
            vistos.add(q)
            unicos.append(q)
    return unicos


def ore_bin():
    for c in ("target/release/ore.exe", "target/release/ore", "target/debug/ore.exe", "target/debug/ore"):
        if os.path.exists(os.path.join(RAIZ, c)):
            return os.path.join(RAIZ, c)
    return None


def seccion_corpus():
    titulo("  §5 EL CORPUS (lo que el repositorio manda de verdad a `sql()`)")
    import duckdb

    regex = re.compile(re.search(r'_VISTAS_EN_SQL = re\.compile\(r"(.+?)"\)', leer(SDK["python"])).group(1))
    con = duckdb.connect()
    ore = ore_bin()
    qs = corpus()
    dif_regex = dif_ore = no_duck = no_ore = 0
    for q in qs:
        r = sorted(set(".".join(m) for m in regex.findall(q)))
        j = json.loads(con.execute("select json_serialize_sql(?)", [q]).fetchone()[0])
        d = None
        if not j.get("error"):
            d = set()

            def walk(n):
                if isinstance(n, dict):
                    if n.get("type") == "BASE_TABLE" and n.get("schema_name"):
                        d.add("%s.%s" % (n["schema_name"], n["table_name"]))
                    for v in n.values():
                        walk(v)
                elif isinstance(n, list):
                    for v in n:
                        walk(v)

            walk(j)
            d = sorted(d)
        o = None
        if ore:
            with tempfile.NamedTemporaryFile("w", suffix=".sql", delete=False, encoding="utf-8") as f:
                f.write(q)
            p = subprocess.run([ore, "sql", "--json", f.name], capture_output=True)
            os.unlink(f.name)
            try:
                oj = json.loads(p.stdout.decode("utf-8"))
                o = sorted(x["ref"] for x in oj["lee"]) if not oj["fallos"] else None
            except Exception:
                o = None
        verdad = d if d is not None else o
        no_duck += d is None
        no_ore += o is None
        if verdad is not None:
            dif_regex += r != verdad
            dif_ore += o is not None and o != verdad
    print("     %d consultas distintas (el-puesto.sh + los casos de medida-el-sql-del-arbol.py)" % len(qs))
    print("     DuckDB no las analiza (no son SELECT o estan rotas a proposito): %d" % no_duck)
    print("     `ore sql` no las acepta como unidad (y lo dice): %d" % no_ore)
    print("     la regex difiere de la verdad en %d · `ore sql` en %d" % (dif_regex, dif_ore))
    print("     ⛔ `ore sql` NO es `sql()`: la unidad del arbol niega lo que una celda puede")
    print("       querer (varias sentencias, leer por funcion). Si ore-serve analiza el texto de")
    print("       una celda, hace falta un modo que SOLO saque nombres, sin las reglas de unidad.")


def seccion_rutas():
    titulo("  §6 LAS DOS RUTAS, PIEZA A PIEZA")
    print("     (A) EL MOTOR PREGUNTA AL CATALOGO (ATTACH Iceberg REST, medido viable):")
    for p in [
        "[XS] ore-serve: listTables filtra por clase `dataset` y `ore datasets` dice escrito|mantenido",
        "[S-M] ore-serve: loadTable resuelve como `datos_de` (la copia mantenida por su puntero, la View sobre dataset por su raiz)",
        "[S-M] ore-serve: la credencial de LEER en loadTable (hoy presta la de escribir, y solo a quien escribio)",
        "[S] ore-serve: la regla ⑤ deja pasar los inputs del transform en GET (hoy solo el output)",
        "[S] ore-serve: el fallback de la rama a `main`, como `datos`",
        "[S-M] ore-serve: la procedencia `leidas` sale de los loadTable del puesto (el SDK ya no ve los nombres)",
        "[S] ×3 SDK: ATTACH READ_ONLY + search_path + renovar el token del agente",
        "[S] ×3 SDK: traducir los errores de DuckDB al contrato de §2",
        "[S] ×3 SDK: `over()` tambien por ATTACH, o hay dos lectores (0033: un lector, un camino)",
        "[?] el sobre ORECOPY1 no es Iceberg: quitarlo (§3) o mantener `datos` solo para el",
    ]:
        print("       · " + p)
    print("     (B) ORE-SERVE ANALIZA EL TEXTO (`POST /puestos/{id}/sql`, una ida y vuelta):")
    for p in [
        "[S] ore-core: un modo de `sql_del_arbol` que solo saque los nombres que se leen (§5)",
        "[S] ore-serve: la ruta, que llama a `datos_del_puesto` por cada nombre (conducto, rama, lo declarado, credencial: lo de hoy, sin tocar)",
        "[S] ×3 SDK: la regex fuera; lo que devuelve la ruta entra por el mismo `fuenteDe` de hoy",
        "     la procedencia, el contrato de errores y el sobre ORECOPY1 NO cambian",
    ]:
        print("       · " + p)
    print("     ⇒ Las dos quitan la regex. (B) es la regex fuera y nada mas: el gobierno sigue")
    print("       en `datos`. (A) es ademas mover el camino de lectura al estandar: cierra el")
    print("       hueco de raiz (el motor resuelve lo que va a leer, sin analizador de por medio)")
    print("       y es lo que Spark hablaria, pero lleva a `loadTable` todo lo que hoy hace `datos`.")


def main():
    arboles = []
    a = sys.argv[1:]
    while a:
        x = a.pop(0)
        if x == "--local" and a:
            arboles.append(a.pop(0))
    print("=== la regex fuera de sql(), el espectro medido")
    seccion_arrastre()
    seccion_contrato()
    seccion_datos(arboles)
    seccion_motores()
    seccion_corpus()
    seccion_rutas()
    return 0


if __name__ == "__main__":
    sys.exit(main())
