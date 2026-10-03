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


for n, f in enumerate([e1, e2, e3, e4, e5, e6, e7, e8, e9, e10, e11, e12], 1):
    caso(n, f)
celda.shutdown()
medios_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
