"""0049 B5·2 · La derivación incremental en Python: `collection.apply(fn, …)`.

El listado y el lago son de mentira (en memoria): lo que se prueba es la lógica
de `apply()` —qué se calcula, qué se queda, qué se va— y la tabla anclada que
escribe (v1alpha17 `03`). Que el lago y el documento la acepten se prueba en el
CLI (`una_tabla_anclada_declara_su_coleccion_y_su_carga`) y en vivo (B5·3).

   1  primera vez: todo, y la tabla anclada con sus seis columnas de sistema
   2  otra vez sin cambios: nada que hacer, y no se escribe
   3  un ítem nuevo: sólo ése
   4  otra `version`: todo otra vez (recalculados), los ids de ancla no cambian
   5  un error es un resultado: su fila `error`, y los demás siguen
   6  el error no se reintenta solo; con `reintentar_errores`, sí, y cuenta el intento
   7  mover un ítem sin cambiar su contenido: no se recalcula; su fila dice la ruta nueva
   8  un ítem que ya no está: sus filas se van
   9  una virtual sin `digest`: la identidad es el localizador, y moverla sí recalcula
  10  `params` y la versión del código entran en la clave
  11  dentro de un transform: la salida es su `output`, y leerla no es una entrada
  12  lo que `fn` da mal: una columna `_…` o un ancla sin `kind` son el error de ese ítem
  13  una copia con otra ruta, listada antes: su fila no cambia de ruta y no se escribe
  14  la colección como relación de `sql()` (B7·1): una fila por ítem, sin leer bytes
  15  funciones del árbol en SQL (B7·2): de tabla en un lateral, escalar, con su contrato
  16  un dataset desde una colección en SQL (B7·3): ítem a ítem por apply(), anclado
  17  B8: set managed|virtual, y una colección con origen desde el SDK

    PYTHONUTF8=1 python pruebas-de-fuego/la-derivacion-en-python.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
from banco_media import TRANSFORMS, bien, caso  # noqa: E402

celda, medios_ = banco.arrancar()

import ore  # noqa: E402
from ore import medios  # noqa: E402

ore.session._proveedor = lambda: {"authorization": "Bearer secreto-de-ore"}
print("la derivación en python")

COL = "legal.archivo.contratos"
SAL = "legal.archivo.textos"
LISTADO = []                       # las MediaRef que el listado da
LAGO = {}                          # nombre → pyarrow.Table
ESCRITURAS = []                    # (nombre, filas, mode, anchored_to)
LLAMADAS = []                      # los paths que fn calculó


def ref(path, contenido, coleccion=COL, virtual=False):
    return medios.MediaRef(uri="ore://%s/%s?v=v1" % (coleccion, path), collection=coleccion, path=path,
                           version="v1", digest=None if virtual else "sha256:" + contenido, size=10,
                           content_type="application/pdf", checksum="etag:" + contenido)


def items(self, prefix=None, state=None, limit=1000):
    for r in LISTADO:
        yield medios.Item(self, r)


def over(nombre, format="pandas"):
    ore._lee(nombre)
    if nombre not in LAGO:
        raise LookupError("no hay ninguna `View` ni `Dataset` `%s` en el árbol" % nombre)
    return LAGO[nombre]


def write(nombre, t, mode="overwrite", key=None, anchored_to=None):
    LAGO[nombre] = t
    ESCRITURAS.append((nombre, t.num_rows, mode, anchored_to))
    return {"rows": t.num_rows}


medios.Collection.items = items
ore.over, ore.write = over, write


def paginas(item):
    """Dos páginas por contrato; `fallar.pdf` falla."""
    LLAMADAS.append(item.ref.path)
    if item.ref.path == "fallar.pdf" and not os.environ.get("ARREGLADO"):
        raise ValueError("pdf roto")
    for p in (1, 2):
        yield {"anchor": {"kind": "page", "page": p}, "texto": "%s p%d" % (item.ref.path, p)}


def aplicar(**kw):
    del LLAMADAS[:]
    return ore.collection(COL).apply(paginas, output=SAL, **{"version": "1", **kw})


def filas():
    return LAGO[SAL].to_pylist()


def e1():
    LISTADO[:] = [ref("a.pdf", "aa"), ref("b.pdf", "bb"), ref("c.pdf", "cc")]
    r = aplicar()
    assert r == {"items": 3, "new": 3, "recomputed": 0, "skipped": 0, "errors": 0, "removed": 0,
                 "rows": 6, "written": True}, r
    assert ESCRITURAS[-1] == (SAL, 6, "overwrite", COL), ESCRITURAS
    t = LAGO[SAL]
    assert t.column_names[:6] == ["_item", "_anchor", "_anchor_id", "_anchor_parent", "_derivation", "_status"], t.column_names
    assert str(t.schema.field("_derivation").type.field("created").type) == "timestamp[us, tz=UTC]"
    f = filas()[0]
    assert f["_anchor"]["kind"] == "page" and f["_anchor"]["page"] in (1, 2) and f["_anchor"]["bbox"] is None, f
    assert f["_item"]["digest"].startswith("sha256:") and f["_status"]["state"] == "ok" and f["_status"]["attempts"] == 1, f
    assert f["_derivation"]["fn"] == "paginas" and f["_derivation"]["fn_version"] == "1", f
    assert len({x["_anchor_id"] for x in filas()}) == 6
    bien("1 · primera vez: 3 ítems, 6 filas, tabla anclada a la colección con sus seis columnas de sistema")


def e2():
    n = len(ESCRITURAS)
    r = aplicar()
    assert r["skipped"] == 3 and r["new"] == 0 and not r["written"] and r["rows"] == 6, r
    assert len(ESCRITURAS) == n and LLAMADAS == [], (ESCRITURAS, LLAMADAS)
    bien("2 · otra vez sin cambios: 0 calculados, y no se escribe (un commit menos)")


def e3():
    LISTADO.append(ref("d.pdf", "dd"))
    r = aplicar()
    assert r["new"] == 1 and r["skipped"] == 3 and r["rows"] == 8 and LLAMADAS == ["d.pdf"], (r, LLAMADAS)
    bien("3 · un ítem nuevo: sólo ése (d.pdf), y la tabla tiene sus 8 filas")


def e4():
    antes = {(x["_item"]["path"], x["_anchor"]["page"]): x["_anchor_id"] for x in filas()}
    r = aplicar(version="2")
    assert r["recomputed"] == 4 and r["skipped"] == 0 and sorted(LLAMADAS) == ["a.pdf", "b.pdf", "c.pdf", "d.pdf"], (r, LLAMADAS)
    despues = {(x["_item"]["path"], x["_anchor"]["page"]): x["_anchor_id"] for x in filas()}
    assert antes == despues and {x["_derivation"]["fn_version"] for x in filas()} == {"2"}
    bien("4 · otra `version`: los 4 recalculados; los `_anchor_id` son los mismos (no dependen de la versión)")


def e5():
    LISTADO.append(ref("fallar.pdf", "ff"))
    r = aplicar(version="2")
    assert r["errors"] == 1 and r["skipped"] == 4 and r["written"], r
    e = [x for x in filas() if x["_item"]["path"] == "fallar.pdf"]
    assert len(e) == 1 and e[0]["_status"]["state"] == "error" and e[0]["_status"]["error_type"] == "ValueError", e
    assert e[0]["_status"]["error_message"] == "pdf roto" and e[0]["_anchor"]["kind"] == "item", e
    bien("5 · un error es un resultado: una fila `error` (ValueError: pdf roto, ancla item), y los otros 4 siguen")


def e6():
    r = aplicar(version="2")
    assert r["skipped"] == 5 and not r["written"] and LLAMADAS == [], (r, LLAMADAS)
    os.environ["ARREGLADO"] = "1"
    try:
        r = aplicar(version="2", retry_errors=True)
    finally:
        del os.environ["ARREGLADO"]
    assert LLAMADAS == ["fallar.pdf"] and r["recomputed"] == 1 and r["errors"] == 0, (r, LLAMADAS)
    e = [x for x in filas() if x["_item"]["path"] == "fallar.pdf"]
    assert len(e) == 2 and {x["_status"]["state"] for x in e} == {"ok"} and e[0]["_status"]["attempts"] == 2, e
    bien("6 · el error no se reintenta solo; con `retry_errors=True` sólo ése, sale bien, intento 2")


def e7():
    LISTADO[0] = ref("archivo/a-renombrado.pdf", "aa")
    r = aplicar(version="2")
    assert LLAMADAS == [] and r["skipped"] == 5 and r["written"], (r, LLAMADAS)
    rutas = {x["_item"]["path"] for x in filas()}
    assert "archivo/a-renombrado.pdf" in rutas and "a.pdf" not in rutas, rutas
    bien("7 · mover a.pdf sin cambiar su contenido: 0 calculados; sus filas dicen la ruta nueva")


def e8():
    del LISTADO[1]   # b.pdf
    r = aplicar(version="2")
    assert r["removed"] == 1 and LLAMADAS == [] and r["rows"] == 8, r
    assert "b.pdf" not in {x["_item"]["path"] for x in filas()}
    bien("8 · b.pdf ya no está: sus 2 filas se van, sin calcular nada")


def e9():
    global COL
    viejo = COL
    COL = "legal.archivo.virtual"
    try:
        LISTADO[:] = [ref("v.pdf", "vv", COL, virtual=True)]
        LAGO.pop(SAL, None)
        aplicar()
        r = aplicar()
        assert r["skipped"] == 1 and not r["written"], r
        LISTADO[:] = [ref("movido/v.pdf", "vv", COL, virtual=True)]
        r = aplicar()
        assert LLAMADAS == ["movido/v.pdf"] and r["removed"] == 1, (r, LLAMADAS)
    finally:
        COL = viejo
        LAGO.pop(SAL, None)
    bien("9 · una virtual sin digest: su identidad es (colección, ruta, versión); otra vez nada, movida sí recalcula")


def e10():
    LISTADO[:] = [ref("a.pdf", "aa")]
    aplicar(params={"idioma": "es"})
    r = aplicar(params={"idioma": "es"})
    assert r["skipped"] == 1, r
    r = aplicar(params={"idioma": "en"})
    assert r["recomputed"] == 1, r
    k = filas()[0]["_derivation"]
    assert k["params_hash"] and len(k["params_hash"]) == 64, k

    def sin_version(item):
        return [{"texto": "x"}]
    LAGO.pop(SAL, None)
    r = ore.collection(COL).apply(sin_version, output=SAL)
    v = filas()[0]["_derivation"]["fn_version"]
    assert v.startswith("codigo:") and filas()[0]["_anchor"]["kind"] == "item", filas()
    r = ore.collection(COL).apply(sin_version, output=SAL)
    assert r["skipped"] == 1, r
    bien("10 · `params` entran en la clave (es→en recalcula); sin `version`, la del código (%s), y es estable" % v)


def e11():
    LAGO.pop(SAL, None)
    del TRANSFORMS[:]
    del ore._leidas[:]   # lo que los casos de antes leyeron fuera de un transform

    @ore.transform(inputs=[ore.collection(COL)], output=SAL)
    def textos(contratos):
        a = contratos.apply(paginas, version="1")
        b = contratos.apply(paginas, version="1")
        return a, b
    a, b = textos(ore.collection(COL))
    assert a["new"] == 1 and b["skipped"] == 1, (a, b)
    assert ESCRITURAS[-1][0] == SAL and ESCRITURAS[-1][3] == COL, ESCRITURAS[-1]
    assert SAL not in ore._leidas, ore._leidas
    bien("11 · dentro de un transform: escribe su `output`, lo lee para saltar lo hecho, y leerlo no es una entrada")


def e12():
    LAGO.pop(SAL, None)

    def mala(item):
        return [{"_status": "x"}] if item.ref.path == "a.pdf" else [{"texto": "y", "anchor": {"page": 1}}]
    LISTADO[:] = [ref("a.pdf", "aa"), ref("b.pdf", "bb")]
    r = ore.collection(COL).apply(mala, output=SAL, version="1")
    assert r["errors"] == 2, r
    m = {x["_item"]["path"]: x["_status"]["error_message"] for x in filas()}
    assert "system columns" in m["a.pdf"] and "without `kind`" in m["b.pdf"], m
    bien("12 · `fn` que da una columna `_…` o un ancla sin `kind`: el error de ese ítem, con su porqué")


def e13():
    """B5·3 en vivo: una copia con otra ruta que el listado da ANTES que la de
    siempre. Mismo ítem; la fila conserva su ruta y no se escribe nada."""
    LAGO.pop(SAL, None)
    LISTADO[:] = [ref("copia/a.pdf", "aa"), ref("copia/b.pdf", "bb")]
    aplicar()
    n = len(ESCRITURAS)
    LISTADO.insert(0, ref("copia-b5/a.pdf", "aa"))
    r = aplicar()
    assert r["skipped"] == 2 and r["items"] == 2 and not r["written"] and LLAMADAS == [], r
    assert len(ESCRITURAS) == n and {x["_item"]["path"] for x in filas()} == {"copia/a.pdf", "copia/b.pdf"}, filas()
    # y si la de siempre se va, la copia la hereda: se escribe con la ruta nueva, sin calcular
    del LISTADO[1]
    r = aplicar()
    assert r["skipped"] == 2 and r["written"] and LLAMADAS == [], r
    assert {x["_item"]["path"] for x in filas()} == {"copia-b5/a.pdf", "copia/b.pdf"}, filas()
    bien("13 · una copia con otra ruta, listada antes: el mismo ítem, su fila no cambia de ruta y no se escribe; "
         "si la de siempre se va, la copia la hereda")


def e14():
    """B7·1: la colección como relación de `sql()`: una fila por ítem, sin bytes."""
    import duckdb
    LISTADO[:] = [ref("copia/a.pdf", "aa"), ref("copia/b.pdf", "bb")]
    t = medios._relacion(ore.collection(COL))
    assert t.column_names == list(medios.COLUMNAS_DE_LA_RELACION), t.column_names
    assert t.schema.field("_item").type == dict(medios._esquema_de_sistema())["_item"]
    con = duckdb.connect()
    con.register("c", t)
    r = con.execute("select path, _item.digest, size from c where path like '%b.pdf'").fetchall()
    assert r == [("copia/b.pdf", "sha256:bb", 10)], r
    assert LLAMADAS == [], LLAMADAS
    bien("14 · la colección en `FROM` (B7·1): una fila por ítem —item, path, digest, size, "
         "content_type, modified—, con `item` del mismo tipo que `_item`, y sin leer un byte")


def e15():
    """B7·2: funciones del árbol llamadas desde SQL, con su contrato: una de
    tabla (filas por ítem, en un lateral) y una escalar, sobre la colección."""
    import duckdb
    from dataclasses import dataclass
    from ore import sql_functions
    from ore.tipos import Media

    @dataclass
    class Pagina:
        page: int
        texto: str

    @ore.function
    def paginas_sql(item: Media["legal.archivo.contratos"]) -> list[Pagina]:
        n = 2 if item.path.endswith("a.pdf") else 1
        return [Pagina(p, "%s p%d" % (item.path, p)) for p in range(1, n + 1)]

    @ore.function
    def idioma(item: Media["legal.archivo.contratos"], defecto: str = "en") -> str:
        return "es" if item.path.endswith("a.pdf") else defecto

    specs = {
        "legal.paginas": (paginas_sql, {"input": {"item": {"type": "Media<legal.archivo.contratos>"}},
                                        "output": {"type": "list<Struct<page: Integer, texto: String>>"}}),
        "legal.idioma": (idioma, {"input": {"item": {"type": "Media<legal.archivo.contratos>"},
                                            "defecto": {"type": "String"}},
                                  "output": {"type": "String"}}),
    }
    LISTADO[:] = [ref("copia/a.pdf", "aa"), ref("copia/b.pdf", "bb")]
    con = duckdb.connect()
    con.register("c", medios._relacion(ore.collection(COL)))
    sql_functions.register(con, [{"name": "legal.paginas", "internal": "__ore_fn_1", "arity": 1, "table": True},
                                 {"name": "legal.idioma", "internal": "__ore_fn_2", "arity": 1, "table": False}],
                           lambda n: specs[n])
    r = con.execute("select c.path, p.page, p.texto, __ore_fn_2(c._item) as lang from c "
                    "cross join lateral (select unnest(__ore_fn_1(c._item), max_depth := 2)) as p "
                    "order by 1, 2").fetchall()
    assert r == [("copia/a.pdf", 1, "copia/a.pdf p1", "es"), ("copia/a.pdf", 2, "copia/a.pdf p2", "es"),
                 ("copia/b.pdf", 1, "copia/b.pdf p1", "en")], r
    # lo que el contrato no admite es un error de la consulta, con su porqué
    sql_functions.register(con, [{"name": "legal.idioma", "internal": "__ore_fn_3", "arity": 2, "table": False}],
                           lambda n: specs[n])
    try:
        con.execute("select __ore_fn_3(c._item, 7) from c").fetchall()
        raise AssertionError("un 7 no es un str")
    except duckdb.Error as e:
        # los tipos del contrato son los de la función en DuckDB: un 7 no es un String
        assert "__ore_fn_3" in str(e) and "VARCHAR" in str(e), e
    bien("15 · funciones del árbol en SQL (B7·2): una de tabla da sus filas por ítem en un lateral "
         "(dataclass → struct), una escalar con un argumento opcional; el contrato manda también aquí")


def e16():
    """B7·3: `create or replace dataset … as select … from <colección>`, lo que
    corre: la consulta ítem a ítem por `apply()`, anclada, con su registro."""
    from dataclasses import dataclass
    from ore.tipos import Media

    @dataclass
    class Ancla:
        kind: str
        page: int

    @dataclass
    class Pag:
        page: int
        texto: str
        anchor: Ancla

    @ore.function
    def pags(item: Media["legal.archivo.contratos"]) -> list[Pag]:
        n = 2 if item.path.endswith("a.pdf") else 1
        return [Pag(p, "%s p%d" % (item.path, p), Ancla("page", p)) for p in range(1, n + 1)]

    spec = {"input": {"item": {"type": "Media<legal.archivo.contratos>"}},
            "output": {"type": "list<Struct<page: Integer, texto: String, anchor: Struct<kind: String, page: Integer>>>"}}
    ore._FUNCIONES["legal.pags"], ore._FUNCIONES_SPEC["legal.pags"] = pags, spec
    pedidas = []

    def query(q):
        return ("select p.page, p.texto, p.anchor from legal.archivo.contratos as c cross join lateral "
                "(select unnest(__ore_fn_1(c._item), max_depth := 2)) as p" + q)

    def pedir(metodo, ruta, cuerpo=None):
        assert ruta.endswith("/sql"), ruta
        pedidas.append(cuerpo["texto"])
        extra = " where c.size > 0" if "size" in cuerpo["texto"] else ""
        return 200, {"fuentes": {COL: {"collection": COL}}, "query": query(extra),
                     "functions": [{"name": "legal.pags", "internal": "__ore_fn_1", "arity": 1, "table": True}]}

    antes_pedir, antes_id = ore.session.pedir, ore.session.id
    ore.session.pedir, ore.session.id = pedir, "puesto-prueba"
    try:
        LAGO.pop(SAL, None)
        LISTADO[:] = [ref("copia/a.pdf", "aa"), ref("copia/b.pdf", "bb")]
        q = "select p.page, p.texto, p.anchor from legal.archivo.contratos as c cross join lateral legal.pags(c._item) as p"
        r = ore._sql_per_item(SAL, COL, q, "paginas")
        assert r["new"] == 2 and r["rows"] == 3 and r["written"], r
        assert ESCRITURAS[-1][3] == COL, ESCRITURAS[-1]
        f = sorted(filas(), key=lambda x: (x["_item"]["path"], x["page"]))
        assert [(x["_item"]["path"], x["_anchor"]["kind"], x["_anchor"]["page"], x["texto"]) for x in f] == [
            ("copia/a.pdf", "page", 1, "copia/a.pdf p1"), ("copia/a.pdf", "page", 2, "copia/a.pdf p2"),
            ("copia/b.pdf", "page", 1, "copia/b.pdf p1")], f
        assert {x["_derivation"]["fn"] for x in f} == {"paginas"} and "anchor" not in t_cols(), t_cols()
        assert f[0]["_derivation"]["fn_version"].startswith("sql:"), f[0]["_derivation"]
        r = ore._sql_per_item(SAL, COL, q, "paginas")
        assert r["skipped"] == 2 and not r["written"], r
        # otra consulta: otra versión, todo otra vez
        r = ore._sql_per_item(SAL, COL, q + " where c.size > 0", "paginas")
        assert r["recomputed"] == 2 and r["written"], r
        res = pd_res(ore._resultado_de_aplicar(r))
        assert res == {"items": 2, "new": 0, "recomputed": 2, "skipped": 0, "errors": 0, "removed": 0,
                       "rows": 3}, res
    finally:
        ore.session.pedir, ore.session.id = antes_pedir, antes_id
    bien("16 · un dataset desde una colección en SQL (B7·3): la consulta ítem a ítem por apply(), "
         "anclada (las anclas de la función), 3 filas; otra vez nada; otra consulta, todo de nuevo")


def t_cols():
    return LAGO[SAL].column_names


def pd_res(t):
    return {k: v[0] for k, v in t.to_pydict().items()}


def e17():
    """B8: `alter media collection … set managed|virtual` y `create … from
    object table`: el documento que el SDK escribe, y lo que dice."""
    doc = {"yaml": "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata:\n  name: c\n"
                   "  namespace: legal\n  schema: archivo\nspec:\n  owner: team:legal\n  media: document\n"
                   "  formats: [pdf]\n  from:\n    objectTable: s3.docs.t\n  virtual: true\n  retention: 30d\n"}
    puestos = []
    copia = [{"queued": True, "detail": "encolado"}]

    def pedir(metodo, ruta, cuerpo=None, **_):
        if metodo == "GET":
            return (200, doc) if ruta.endswith("/c") else (404, {"error": "no"})
        puestos.append(cuerpo["yaml"])
        doc["yaml"] = cuerpo["yaml"]
        # B8·3: el servidor encola la copia de lo que pasa a mantenida
        if "objectTable" in cuerpo["yaml"] and "virtual: true" not in cuerpo["yaml"]:
            return 200, {"copy": copia[0]}
        return 200, {}

    antes = ore.session.pedir
    ore.session.pedir = pedir
    try:
        r = ore.alter_collection("legal.archivo.c", managed=True)
        assert r == {"collection": "legal.archivo.c", "status": "managed · copy queued (follow it: describe media collection legal.archivo.c)"}, r
        assert "virtual" not in puestos[-1] and "objectTable: s3.docs.t" in puestos[-1], puestos[-1]
        assert ore.alter_collection("legal.archivo.c")["status"] == "already managed" and len(puestos) == 1
        r = ore.alter_collection("legal.archivo.c", managed=False)
        lineas = puestos[-1].split("\n")
        i = lineas.index("  virtual: true")
        assert r["status"] == "virtual" and lineas[i - 1] == "    objectTable: s3.docs.t", lineas
        assert lineas[i + 1] == "  retention: 30d", lineas
        try:
            ore.alter_collection("legal.archivo.nada")
            raise AssertionError("no existe")
        except LookupError as e:
            assert "in this branch" in str(e), e
        r = ore.create_collection("legal.archivo.nueva", "document", ["pdf"], source="s3.docs.t", virtual=True)
        assert r["created"] and "  from: { objectTable: s3.docs.t }\n  virtual: true" in puestos[-1], puestos[-1]
        assert "copy" not in r, r
        copia[0] = {"queued": False, "reason": "the conduit waits for the package owner (OOS4011)"}
        r = ore.create_collection("legal.archivo.mantenida", "document", ["pdf"], source="s3.docs.t")
        assert r["copy"] == "copy NOT queued: the conduit waits for the package owner (OOS4011)", r
        try:
            ore.create_collection("legal.archivo.otra", "document", ["pdf"], virtual=True)
            raise AssertionError("virtual sin origen")
        except ValueError as e:
            assert "no origin" in str(e), e
    finally:
        ore.session.pedir = antes
    bien("17 · B8: `set managed` quita `virtual` y dice su copia (otra vez: already managed, sin escribir), "
         "`set virtual` lo pone tras su `from`; `create … from object table` escribe su origen y dice si su "
         "copia quedó encolada; sin origen, no")

def e18():
    """B8·3: `describe <kind>` pide las filas a ore-serve (`/describe/…`, en la
    rama) y las da como `DESCRIBE TABLE EXTENDED`: las columnas y `# Detail`."""
    filas = [["item", "String", ""], ["path", "String", ""], ["", "", ""], ["# Detail", "", ""],
             ["kind", "media collection", ""], ["type", "managed", ""], ["status", "copied", ""],
             ["items", "4", ""], ["pending", "0", ""]]
    pedidas = []

    def pedir(metodo, ruta, cuerpo=None, cabeceras=None, **_):
        pedidas.append((metodo, ruta))
        if ruta.startswith("/puestos/"):
            return 404, {}
        if ruta == "/describe/media-collection/legal/archivo/c":
            return 200, {"rows": filas}
        if ruta == "/describe/view/legal/default/v":
            return 422, {"error": "`legal.v` is a `Dataset`, not a view"}
        return 404, {"error": "no"}

    antes = ore.session.pedir
    ore.session.pedir = pedir
    try:
        r = ore.describe("legal.archivo.c", "media collection")
        assert r[0] == {"col_name": "item", "data_type": "String", "comment": ""}, r
        assert ore._detalle(r) == {"kind": "media collection", "type": "managed", "status": "copied",
                                   "items": "4", "pending": "0"}, ore._detalle(r)
        assert ore.describe_collection("legal.archivo.c") == r
        t = ore._resultado_de_describir(r)
        assert t.column_names == ["col_name", "data_type", "comment"] and t.num_rows == len(filas), t
        try:
            ore.describe("legal.v", "view")
            raise AssertionError("no es una vista")
        except RuntimeError as e:
            assert "not a view" in str(e), e
        try:
            ore.describe("legal.archivo.nada", "dataset")
            raise AssertionError("no existe")
        except LookupError as e:
            assert "in this branch" in str(e), e
        try:
            ore.describe("legal.archivo.c", "model")
            raise AssertionError("kind")
        except ValueError as e:
            assert "`kind`" in str(e), e
    finally:
        ore.session.pedir = antes
    bien("18 · B8·3: `describe <kind>` da las filas de ore-serve como DESCRIBE TABLE EXTENDED (columnas y "
         "# Detail); `describe_collection` es la de una colección; otro kind, no; lo que no es, lo dice")

for n, f in enumerate([e1, e2, e3, e4, e5, e6, e7, e8, e9, e10, e11, e12, e13, e14, e15, e16, e17, e18], 1):
    caso(n, f)
celda.shutdown()
medios_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
