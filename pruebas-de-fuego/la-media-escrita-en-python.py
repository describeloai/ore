"""LA MEDIA ESCRITA EN PYTHON (0049 B4b·3), sin clúster: el banco (`banco_media.py`).

   1  crear_coleccion(): el documento v1alpha19 sin `from`, por /documentos
   2  ya existe: error; con si_no_existe, `creada: False` y no se escribe nada
   3  un código OOS del servidor vuelve como ValueError
   4  lo que no es un medio o unos formatos, ValueError sin preguntar
   5  transaccion() como `with`: put de bytes (tipo por los bytes, Repr-Digest) y commit al salir
   6  put de una ruta, en flujo, con su largo
   7  put de un fichero que no se rebobina: se copia antes y sube igual
   8  el token de ORE nunca va a `upload`; la rama del puesto sí va a la celda
   9  una subida cortada se reintenta, y entra
  10  un commit que pierde la carrera de la forja (409 sin type) se vuelve a confirmar
  11  una excepción dentro del `with`: abort, nada escrito, y la excepción sigue
  12  dentro de un transform, sólo su `output`: otra colección es PermissionError sin preguntar
  13  una colección mantenida no se escribe (MediaNoEscribible); los tipos nuevos de error

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-escrita-en-python.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
import hashlib  # noqa: E402
import io  # noqa: E402
import tempfile  # noqa: E402

from banco_media import BYTES, DOCUMENTOS, LAGO, MODOS, PUNTEROS, RAMA, SERVE, bien, cabeceras, caso  # noqa: E402

celda, medios_ = banco.arrancar()

import ore  # noqa: E402

ore.puesto._proveedor = lambda: {"authorization": "Bearer secreto-de-ore"}
print("la media escrita en python")


def e1():
    r = ore.crear_coleccion("legal.archivo.paginas", media="image", formatos=["PNG", ".webp"],
                            etiquetas={"gdpr.sensitivity": "high"})
    assert r == {"coleccion": "legal.archivo.paginas", "creada": True}, r
    y = DOCUMENTOS[("MediaCollection", "legal", "archivo", "paginas")]
    assert "apiVersion: oos.dev/v1alpha19" in y and "from:" not in y, y
    assert "formats: [png, webp]" in y and "owner: team:legal" in y and "gdpr.sensitivity: high" in y, y
    bien("1 · crear_coleccion(): v1alpha19 sin `from`, formatos en minúscula, dueño por defecto, etiquetas")


def e2():
    try:
        ore.crear_coleccion("legal.archivo.paginas", media="image", formatos=["png"])
    except RuntimeError as e:
        assert "ya hay una colección" in str(e), e
    else:
        raise AssertionError("debía ser RuntimeError")
    SERVE.clear()
    r = ore.crear_coleccion("legal.archivo.paginas", media="image", formatos=["png"], si_no_existe=True)
    assert r["creada"] is False, r
    assert not [m for m, _, _ in SERVE if m == "PUT"], SERVE
    bien("2 · ya existe: error; con si_no_existe, `creada: False` y no se escribe nada")


def e3():
    try:
        ore.crear_coleccion("legal.rota", media="document", formatos=["pdf"])
    except ValueError as e:
        assert "OOS1004" in str(e), e
        bien("3 · un código OOS del servidor vuelve como ValueError (%s)" % str(e)[:60])
        return
    raise AssertionError("debía ser ValueError")


def e4():
    SERVE.clear()
    for media, formatos in (("binary", ["bin"]), ("image", []), ("image", ["png", "png"]), ("image", ["p ng"])):
        try:
            ore.crear_coleccion("legal.fotos", media=media, formatos=formatos)
        except ValueError:
            continue
        raise AssertionError("%s %s debía ser ValueError" % (media, formatos))
    assert not SERVE, SERVE
    bien("4 · un medio que no es, formatos vacíos, repetidos o raros: ValueError sin preguntar")


PNG = b"\x89PNG\r\n\x1a\n-una-pagina-"
PAG = "legal.archivo.paginas"


def e5():
    BYTES.clear()
    with ore.coleccion(PAG).transaccion() as t:
        ref = t.put("c1/p0.png", PNG, tipo="text/html")
    sha = hashlib.sha256(PNG).hexdigest()
    assert ref.digest == "sha256:" + sha and ref.content_type == "image/png", ref
    assert t.cerrada and t.resultado["transaccion"] == 1 and PUNTEROS[PAG] == 1, t.resultado
    h = cabeceras(BYTES[-1][2])
    assert h["repr-digest"].startswith("sha-256=:") and h["content-type"] == "text/html", h
    assert "c1/p0.png" in BYTES[-1][1], BYTES[-1][1]
    bien("5 · with transaccion(): put de bytes con Repr-Digest, tipo por los bytes (image/png), commit al salir")


def e6():
    d = tempfile.mkdtemp()
    ruta = os.path.join(d, "grande.bin")
    datos = os.urandom(300_000)
    with open(ruta, "wb") as f:
        f.write(datos)
    BYTES.clear()
    with ore.coleccion(PAG).transaccion() as t:
        ref = t.put("c1/grande.bin", ruta)
    assert LAGO[hashlib.sha256(datos).hexdigest()] == datos
    assert cabeceras(BYTES[-1][2])["content-length"] == "300000", BYTES[-1][2]
    assert ref.size == 300_000
    bien("6 · put de una ruta: en flujo, con su largo (300 000 bytes, mismos bytes en el lago)")


class _SinRebobinar(io.RawIOBase):
    def __init__(self, b):
        self.b, self.i = b, 0

    def readable(self):
        return True

    def seekable(self):
        return False

    def readinto(self, buf):
        n = min(len(buf), len(self.b) - self.i)
        buf[:n] = self.b[self.i:self.i + n]
        self.i += n
        return n


def e7():
    datos = b"%PDF-" + os.urandom(50_000)
    with ore.coleccion(PAG).transaccion() as t:
        ref = t.put("c1/flujo.pdf", io.BufferedReader(_SinRebobinar(datos)))
    assert ref.digest == "sha256:" + hashlib.sha256(datos).hexdigest() and ref.content_type == "application/pdf"
    bien("7 · put de un fichero que no se rebobina: se copia antes y sube igual (application/pdf)")


def e8():
    RAMA["r"] = "r1/paginas"
    SERVE.clear()
    BYTES.clear()
    try:
        with ore.coleccion(PAG).transaccion() as t:
            t.put("c1/p1.png", PNG)
    finally:
        RAMA["r"] = None
    puts = [cabeceras(h) for m, _, h in BYTES if m == "PUT"]
    assert puts and all("authorization" not in h and "x-ore-puesto" not in h for h in puts), puts
    a_la_celda = [cabeceras(h) for m, r, h in SERVE if "/transactions" in r]
    assert all(h.get("x-ore-rama") == "r1/paginas" and "authorization" in h for h in a_la_celda), a_la_celda
    assert t.id in repr(t) and "permiso" not in repr(t) and "subida" not in repr(t), repr(t)
    bien("8 · el token de ORE nunca va a `upload` (%d subidas); la rama y el token, a la celda (%d)"
         % (len(puts), len(a_la_celda)))


def e9():
    MODOS["cortar_subidas"] = 2
    BYTES.clear()
    with ore.coleccion(PAG).transaccion() as t:
        ref = t.put("c1/p2.png", PNG + b"-2")
    intentos = [r for m, r, _ in BYTES if m == "PUT"]
    assert len(intentos) == 3 and ref.path == "c1/p2.png", intentos
    bien("9 · una subida cortada dos veces se reintenta y entra a la tercera")


def e10():
    MODOS["conflictos"] = 2
    SERVE.clear()
    antes = PUNTEROS[PAG]
    with ore.coleccion(PAG).transaccion() as t:
        t.put("c1/p3.png", PNG + b"-3")
    commits = [r for m, r, _ in SERVE if r.endswith("/commit")]
    assert len(commits) == 3 and PUNTEROS[PAG] == antes + 1, commits
    bien("10 · el commit perdió la carrera de la forja dos veces (409 sin type): confirmado a la tercera")


def e11():
    SERVE.clear()
    antes = PUNTEROS[PAG]
    try:
        with ore.coleccion(PAG).transaccion() as t:
            t.put("c1/p4.png", PNG + b"-4")
            raise KeyError("algo se rompió")
    except KeyError:
        pass
    else:
        raise AssertionError("la excepción tenía que seguir")
    assert [r for m, r, _ in SERVE if r.endswith("/abort")] and PUNTEROS[PAG] == antes
    assert t.cerrada and t.resultado is None
    try:
        t.put("c1/p5.png", PNG)
    except ore.MediaTransaccion:
        pass
    else:
        raise AssertionError("cerrada, put debía ser MediaTransaccion")
    bien("11 · una excepción dentro: abort, el puntero no se mueve, la excepción sigue; cerrada no admite put")


def e12():
    SERVE.clear()

    @ore.transform(inputs=[ore.coleccion("legal.archivo.contratos")], output=ore.coleccion(PAG))
    def paginar():
        try:
            ore.coleccion("legal.archivo.otra").transaccion()
        except PermissionError as e:
            assert "legal.archivo.otra" in str(e), e
        else:
            raise AssertionError("debía ser PermissionError")
        with ore.coleccion(PAG).transaccion() as t:
            t.put("c2/p0.png", PNG + b"-c2")
        return t.resultado

    r = paginar()
    assert r["transaccion"] >= 1, r
    assert not [x for _, x, _ in SERVE if x.startswith("/media/legal/archivo/otra")], SERVE
    bien("12 · dentro de un transform: su output se escribe; otra colección, PermissionError sin preguntar")


def e13():
    try:
        ore.coleccion("legal.archivo.contratos").transaccion()
    except ore.MediaNoEscribible as e:
        assert isinstance(e, PermissionError) and e.status == 409, e
    else:
        raise AssertionError("una mantenida debía ser MediaNoEscribible")
    from ore import medios
    assert isinstance(medios._error(422, {"type": "media/digest-no-casa"}, "x"), ore.MediaCorrupto)
    assert isinstance(medios._error(404, {"type": "media/transaccion"}, "x"), ore.MediaTransaccion)
    assert isinstance(medios._error(401, {"type": "media/permiso"}, "x"), ore.MediaSinPermiso)
    bien("13 · una mantenida no se escribe (MediaNoEscribible, 409); digest-no-casa, transaccion y permiso, por su tipo")


for n, f in enumerate([e1, e2, e3, e4, e5, e6, e7, e8, e9, e10, e11, e12, e13], 1):
    caso(n, f)
celda.shutdown()
medios_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
