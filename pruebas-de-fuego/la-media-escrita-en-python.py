"""LA MEDIA ESCRITA EN PYTHON (0049 B4b·3), sin clúster: el banco (`banco_media.py`).

   1  crear_coleccion(): el documento v1alpha19 sin `from`, por /documentos
   2  ya existe: error; con si_no_existe, `creada: False` y no se escribe nada
   3  un código OOS del servidor vuelve como ValueError
   4  lo que no es un medio o unos formatos, ValueError sin preguntar

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-escrita-en-python.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
from banco_media import DOCUMENTOS, SERVE, bien, caso  # noqa: E402

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


for n, f in enumerate([e1, e2, e3, e4], 1):
    caso(n, f)
celda.shutdown()
medios_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
