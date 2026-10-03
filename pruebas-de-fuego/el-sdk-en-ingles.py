"""EL SDK EN INGLÉS (S1), sin clúster.

El SDK de Python habla inglés —nombres, argumentos, claves de lo que devuelve,
mensajes— y lo de antes sigue valiendo, callado. Contra el banco de la media
(`banco_media.py`, una celda de mentira) se comprueba:

   1  los nombres ingleses funcionan: create_collection, collection, items,
      stat, read_bytes, read_many, transaction/put/put_many/commit, apply
   2  cada nombre español es EL MISMO objeto que el inglés (módulo, medios,
      contrato) y `from ore import crear_coleccion` sigue valiendo
   3  los atributos y métodos de antes en las clases (nombre_corto, aplicar,
      transaccion, sha256_visto, resultado, cerrada, put_varios, de_json, persona…)
   4  los argumentos con nombre español valen en las funciones inglesas, y dar
      los dos nombres a la vez es TypeError
   5  lo que se devuelve tiene claves inglesas y contesta a las de antes por
      [], .get e `in`; al recorrerlo, sólo las inglesas
   6  write(): "anexar"/"sobrescribir" y "append"/"overwrite" mandan la MISMA
      palabra de siempre a ore-store (`modo`) y la misma semilla
   7  `except ore.MediaNoExiste` caza un `MediaNotFound` (y así cada error)
   8  los avisos de los alias: callados, y con `_AVISAR_ALIAS` un DeprecationWarning
   9  el código que genera ore-serve (lee `_hecho["creada"]`, `_escrito["filas"]`…)
      sigue funcionando con los dicts nuevos

    PYTHONUTF8=1 python pruebas-de-fuego/el-sdk-en-ingles.py
"""
import os
import sys
import warnings

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
from banco_media import A, BYTES, DOCUMENTOS, SERVE, SHA, bien, caso  # noqa: E402

celda, medios_ = banco.arrancar()

import ore  # noqa: E402
from ore import contrato, medios, tipos  # noqa: E402

ore.session._proveedor = lambda: {"authorization": "Bearer secreto-de-ore"}

print("el sdk en inglés")
COL = "legal.archivo.contratos"
PAG = "legal.archivo.ingles"
PNG = b"\x89PNG\r\n\x1a\n" + b"\x00" * 200


def e1():
    r = ore.create_collection(PAG, media="image", formats=["png"], labels={"gdpr.sensitivity": "high"})
    assert r == {"collection": PAG, "created": True}, r
    assert ("MediaCollection", "legal", "archivo", "ingles") in DOCUMENTOS
    c = ore.collection(COL)
    assert isinstance(c, ore.Collection) and c.short_name == COL, c
    its = list(c.items(prefix=None, limit=1000))
    assert [i.ref.path for i in its] == ["a.pdf", "b.pdf", "cambia.pdf"], its
    it = c.stat(path="a.pdf")
    assert it.current is True and it.collection is c, it
    assert it.read_bytes(threads=4) == A and it.sha256_seen == SHA["a.pdf"], it.sha256_seen
    leidos = {i.ref.path: (d, e) for i, d, e in ore.read_many(c.items(), threads=2)}
    assert leidos["a.pdf"][0] == A and isinstance(leidos["cambia.pdf"][1], ore.MediaChanged), leidos
    with ore.collection(PAG).transaction() as t:
        ref = t.put("x/a.png", PNG, content_type="image/png")
        assert isinstance(ref, ore.MediaRef) and ref.content_type_detected in (None, "image/png"), ref
        hechos = list(t.put_many([("x/b.png", PNG + b"b"), ("x/c.png", PNG + b"c")], threads=2))
        assert all(e is None for _, _, e in hechos), hechos
    assert t.closed and t.result["transaction"] >= 1 and len(t.uploaded) == 3, (t.closed, t.result, t.uploaded)
    bien("1 · create_collection, collection, items, stat, read_bytes(threads=), read_many, "
         "transaction/put/put_many/commit: los nombres ingleses funcionan")


def e2():
    for es, en in ore._ALIAS.items():
        assert getattr(ore, es) is getattr(ore, en), (es, en)
    for es, en in medios._ALIAS.items():
        assert getattr(medios, es) is getattr(medios, en), (es, en)
    assert contrato.ErrorDeContrato is contrato.ContractError
    from ore import crear_coleccion, coleccion, puesto, MediaNoExiste, Coleccion, json_de, tabla  # noqa: F401
    assert crear_coleccion is ore.create_collection and puesto is ore.session and tabla is ore.table
    assert ore.puesto is ore.session and ore.Puesto is ore.Session and ore.Modelo is ore.Model
    try:
        ore.no_existe
    except AttributeError:
        pass
    else:
        raise AssertionError("ore.no_existe debía ser AttributeError")
    star = {}
    exec("from ore import *", star)
    assert "create_collection" in star and "crear_coleccion" not in star, sorted(star)
    bien("2 · %d nombres de antes en `ore` y %d en `ore.medios`: el mismo objeto; `from ore import crear_coleccion` "
         "vale, y `import *` da sólo los ingleses" % (len(ore._ALIAS), len(medios._ALIAS)))


def e3():
    c = ore.coleccion(COL)
    assert c.nombre_corto == c.short_name == COL
    assert ore.Collection.aplicar is ore.Collection.apply and ore.Collection.transaccion is ore.Collection.transaction
    assert ore.Transaction.put_varios is ore.Transaction.put_many and ore.Model.pide is ore.Model.ask
    assert medios.MediaRef.de_json == medios.MediaRef.from_json
    it = c.stat(path="b.pdf")
    it.read_bytes()
    assert it.actual is it.current and it.coleccion is it.collection
    assert it.sha256_visto == it.sha256_seen
    it.sha256_visto = "x"
    assert it.sha256_seen == "x"
    with ore.coleccion(PAG).transaccion() as t:
        t.put("y/a.png", datos=PNG, tipo="image/png")
    assert t.cerrada is t.closed is True and t.resultado is t.result and t.subidos is t.uploaded
    ore.session.persona = "persona:ana"
    assert ore.session.person == "persona:ana" and ore.persona() == ore.person() == "persona:ana"
    assert tipos.Precision(5, escala=2).scale == 2 and tipos.Precision(5, 2).escala == 2
    m = ore.Model("x", "http://h", "servido")
    assert m.referencia == m.ref == "x" and m.servido == m.served == "servido"
    e = medios._error(404, {"type": "media/no-existe", "detail": "d"}, "x")
    assert e.tipo == e.type == "media/no-existe" and e.detalle == e.detail == "d"
    bien("3 · nombre_corto, aplicar, transaccion, sha256_visto, actual, coleccion, put(datos=, tipo=), "
         "cerrada, resultado, subidos, put_varios, de_json, persona, Precision(escala=), pide, tipo/detalle")


def e4():
    import pyarrow as pa

    antes = ore._fuente_de
    ore._fuente_de = lambda v: ("(select 1 as a)", {})
    try:
        t = ore.over("p.v", como="arrow")
        assert isinstance(t, pa.Table) and t.column_names == ["a"], t
        assert isinstance(ore.over(view="p.v", format="arrow"), pa.Table)
        try:
            ore.over("p.v", format="arrow", como="arrow")
        except TypeError as e:
            assert "como" in str(e) and "format" in str(e), e
        else:
            raise AssertionError("over(format=, como=) debía ser TypeError")
    finally:
        ore._fuente_de = antes
    assert isinstance(ore.sql("select 2 as b", como="arrow"), pa.Table)
    r = ore.create_collection(PAG, media="image", formatos=["png"], si_no_existe=True)
    assert r["created"] is False, r
    for llamada in (lambda: ore.create_collection(PAG, media="image", formats=["png"], formatos=["png"]),
                    lambda: ore.write("p.t", None, mode="append", modo="anexar"),
                    lambda: ore.collection(COL).items(prefix="a", prefijo="a"),
                    lambda: ore.drop_view("p.v", if_exists=True, si_existe=True)):
        try:
            llamada()
        except TypeError as e:
            assert "old name" in str(e), e
        else:
            raise AssertionError("dar los dos nombres debía ser TypeError")
    SERVE.clear()
    list(ore.collection(COL).items(prefijo="Nueva carpeta/", limite=5))
    q = [r for _, r, _ in SERVE if "/items?" in r][0]
    assert "prefix=Nueva+carpeta%2F" in q and "limit=5" in q, q
    c = ore.collection(COL)
    assert c.stat(path="a.pdf").read_bytes(hilos=4) == A
    assert len(list(ore.read_many(c.items(), hilos=2))) == 3
    with ore.collection(PAG).transaction() as t:
        list(t.put_many([("z/a.png", PNG)], hilos=2))
    bien("4 · over(como=), sql(como=), create_collection(formatos=, si_no_existe=), items(prefijo=, limite=), "
         "read_bytes(hilos=), read_many(hilos=), put_many(hilos=); los dos nombres a la vez: TypeError")


def e5():
    r = ore.create_collection(PAG, media="image", formats=["png"], if_not_exists=True)
    assert list(r) == ["collection", "created"] and repr(r) == repr({"collection": PAG, "created": False}), r
    assert r["coleccion"] == PAG and r.get("creada") is False and "creada" in r and "coleccion" in r
    assert r.get("nada") is None and r.get("nada", 7) == 7 and "nada" not in r
    try:
        r["nada"]
    except KeyError:
        pass
    else:
        raise AssertionError("r['nada'] debía ser KeyError")
    s = ore._Result({"items": 1, "new": 1, "recomputed": 0, "skipped": 0, "errors": 0, "removed": 0, "rows": 2,
                     "written": True})
    for es, en in (("nuevos", "new"), ("recalculados", "recomputed"), ("saltados", "skipped"),
                   ("errores", "errors"), ("borrados", "removed"), ("filas", "rows"), ("escrito", "written")):
        assert s[es] == s[en] and s.get(es) == s[en] and es in s, (es, en)
    bien("5 · claves inglesas al recorrer e imprimir; las de antes por [], .get e `in`; una que no es, KeyError")


def _write_de_mentira(capturadas):
    """`write()` sin ore-store ni catálogo: lo que mandaría queda en `capturadas`."""
    def pedir(metodo, ruta, cuerpo=None, **kw):
        if metodo == "GET" and ruta.startswith("/v1/"):
            return 404, {"error": {"message": "no hay"}}
        if metodo == "POST" and ruta.endswith("/tables"):
            return 200, {"metadata": {"location": "gs://lago/p/t"}, "config": {}}
        if metodo == "POST" and ruta.startswith("/v1/"):
            return 200, {"metadata": {"current-snapshot-id": 9}, "metadata-location": "gs://lago/p/t/m2.json"}
        return 404, None

    def escribir(binario, env, peticion, ipc):
        capturadas.append(peticion)
        return {"filas": 2, "anadidas": 2, "antes": 0, "operacion": "op1", "requirements": [], "updates": []}
    return pedir, escribir


def e6():
    import pyarrow as pa

    capturadas = []
    pedir, escribir = _write_de_mentira(capturadas)
    antes = (ore.session.pedir, ore._escribir_ficheros, ore._ore_store)
    ore.session.pedir, ore._escribir_ficheros = pedir, escribir
    ore._ore_store = lambda config, ubicacion: ("ore-store-gcs", {})
    try:
        t = pa.table({"a": [1, 2]})
        vistos = {}
        for modo in ("sobrescribir", "overwrite", "anexar", "append", "upsert"):
            r = ore.write("p.t", t, modo, key=["a"] if modo == "upsert" else None)
            vistos[modo] = (capturadas[-1]["modo"], capturadas[-1]["semilla"].split("|")[1], r["mode"])
        r = ore.write("p.t", t, modo="anexar")
        assert capturadas[-1]["modo"] == "anexar" and r["mode"] == "append", (capturadas[-1], r)
        try:
            ore.write("p.t", t, mode="borrar")
        except ValueError as e:
            assert "overwrite" in str(e), e
        else:
            raise AssertionError("mode='borrar' debía ser ValueError")
    finally:
        ore.session.pedir, ore._escribir_ficheros, ore._ore_store = antes
    assert vistos == {"sobrescribir": ("sobrescribir", "sobrescribir", "overwrite"),
                      "overwrite": ("sobrescribir", "sobrescribir", "overwrite"),
                      "anexar": ("anexar", "anexar", "append"), "append": ("anexar", "anexar", "append"),
                      "upsert": ("upsert", "upsert", "upsert")}, vistos
    assert list(r) == ["table", "rows", "snapshot", "metadata_location", "operation", "repeated", "mode", "added",
                       "before"], list(r)
    assert r["filas"] == r["rows"] == 2 and r["repetida"] is r["repeated"] is False and r["tabla"] == "p.t"
    bien("6 · write(): sobrescribir/overwrite → `sobrescribir`, anexar/append → `anexar`, upsert → `upsert` en la "
         "petición y en la semilla; `mode` del resultado, en inglés; claves de antes por []")


def e7():
    pares = [("media/no-existe", 404, "MediaNoExiste"), ("media/sin-permiso", 403, "MediaSinPermiso"),
             ("media/cambiado", 412, "MediaCambiado"), ("media/corrupto", 502, "MediaCorrupto"),
             ("media/rango", 416, "MediaRango"), ("media/no-escribible", 409, "MediaNoEscribible"),
             ("media/transaccion", 409, "MediaTransaccion")]
    for tipo, status, viejo in pares:
        try:
            raise medios._error(status, {"type": tipo}, "x")
        except getattr(ore, viejo) as e:
            assert type(e).__name__ == ore._ALIAS[viejo], (viejo, type(e).__name__)
    try:
        ore.collection(COL).transaction()   # una mantenida no se escribe
    except ore.MediaNoEscribible as e:
        assert isinstance(e, ore.MediaNotWritable) and isinstance(e, PermissionError)
    else:
        raise AssertionError("debía ser MediaNotWritable")
    bien("7 · `except ore.MediaNoExiste` caza un MediaNotFound, y así los siete")


def e8():
    with warnings.catch_warnings(record=True) as w:
        warnings.simplefilter("always", DeprecationWarning)
        ore.crear_coleccion
        ore.create_collection(PAG, media="image", formatos=["png"], si_no_existe=True)["creada"]
    assert not [x for x in w if issubclass(x.category, DeprecationWarning)], [str(x.message) for x in w]
    ore._AVISAR_ALIAS = True
    try:
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always", DeprecationWarning)
            ore.crear_coleccion
            ore.create_collection(PAG, media="image", formats=["png"], si_no_existe=True)
            medios.MediaNoExiste
            ore.collection(COL).nombre_corto
            ore.write
        mensajes = [str(x.message) for x in w if issubclass(x.category, DeprecationWarning)]
    finally:
        ore._AVISAR_ALIAS = False
    assert len(mensajes) == 4 and "ore.create_collection" in mensajes[0], mensajes
    bien("8 · los alias, callados; con `_AVISAR_ALIAS = True`, un DeprecationWarning por uso (%s)" % mensajes[0])


def e9():
    # Lo que `ore-serve` genera para un guion SQL (puestos.rs), tal cual.
    import pyarrow as pa

    ns = {}
    exec("from ore import crear_coleccion, _resultado_de_crear\n\n"
         "_hecho = crear_coleccion(%r, 'image', ['png'], comentario=None, si_no_existe=True)\n"
         "print('%%s · media collection · %%s' %% (_hecho['coleccion'], 'creada' if _hecho['creada'] else 'ya estaba'))\n"
         "_r = _resultado_de_crear('media collection ' + _hecho['coleccion'], _hecho['creada'])\n" % PAG, ns)
    assert ns["_r"].to_pylist() == [{"object": "media collection " + PAG, "status": "already exists"}], ns["_r"]
    v = ore._Result({"view": "p.v", "status": "replaced", "columns": {}})
    assert ore._resultado_de_crear("view " + v["vista"], v["estado"]).to_pylist()[0]["status"] == "replaced"
    escrito = ore._Result({"table": "p.t", "rows": 5, "repeated": False, "mode": "upsert", "added": 3, "before": 4})
    assert escrito.get("filas") == 5 and escrito["repetida"] is False
    t = ore._resultado_de_escritura(escrito)
    assert t.to_pylist() == [{"num_affected_rows": 3, "num_updated_rows": 2, "num_inserted_rows": 1}], t
    assert isinstance(t, pa.Table)
    bien("9 · el código que genera ore-serve (`_hecho['creada']`, `_escrito['filas']`, `_resultado_de_*`) "
         "sigue funcionando con los dicts nuevos")


for n, f in enumerate([e1, e2, e3, e4, e5, e6, e7, e8, e9], 1):
    caso(n, f)
celda.shutdown()
medios_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
