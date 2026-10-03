"""LA MEDIA EN PYTHON (0049 B3·5), sin clúster.

Un `ore-serve` de mentira (`items`, `item`, y el `307` de `content`) y un servidor
de bytes de mentira (lo que sería `ore-medios` o el lago: rangos, permisos que
caducan, 412, flujos cortados). Se comprueba el SDK:

   1  items(): el listado por cursor, perezoso
   2  stat(): el ítem fresco y si es el actual
   3  open(): leer, seek desde el final con un Range, tell
   4  una lectura entera se verifica y da el sha256 visto
   5  el token de ORE va a la celda y NUNCA a la URL de los bytes
   6  un permiso caducado (401) se renueva con la misma versión y se sigue
   7  la versión ya no está (412) → MediaChanged
   8  el digest no casa, o el flujo se corta → MediaCorrupt
   9  read_bytes() de uno grande, por rangos en paralelo
  10  leer_varios(): muchos a la vez, el error de uno es un valor
  11  cerrar a medias no baja el resto
  12  sin tamaño en el listado, se aprende y el seek desde el final va
  13  la rama del puesto va en x-ore-rama (la ficha, preguntada una vez)
  14  B4·3: una colección es un input de un transform, y se lee dentro
  15  B4·3: dentro de un transform, una colección no declarada es PermissionError, sin preguntar
  16  B4·3: fuera de un transform, leer una colección queda anotado en lo leído
  17  B4·3: `as_of` dice la transacción que el listado leyó

    PYTHONUTF8=1 python pruebas-de-fuego/la-media-en-python.py
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import banco_media as banco  # noqa: E402
from banco_media import A, BYTES, CONTADOS, OBJETOS, RAMA, SERVE, SHA, bien, caso  # noqa: E402

celda, bytes_ = banco.arrancar()

import ore  # noqa: E402
from ore import medios  # noqa: E402

ore.session._proveedor = lambda: {"authorization": "Bearer secreto-de-ore"}


print("la media en python")
c = ore.collection("legal.archivo.contratos")


def c1():
    its = list(c.items())
    assert [i.ref.path for i in its] == ["a.pdf", "b.pdf", "cambia.pdf"], its
    assert sum(1 for m, r, _ in SERVE if "/items" in r) == 2
    bien("1 · items(): tres ítems en dos páginas, por cursor (MediaRef ignora lo desconocido)")


def c2():
    it = c.stat(path="a.pdf")
    assert it.current is True and it.ref.size == len(A)
    bien("2 · stat(): fresco y actual")


def c3():
    BYTES.clear()
    it = c.stat(path="a.pdf")
    with it.open() as f:
        assert f.read(5) == b"%PDF-"
        f.seek(-10, 2)
        cola = f.read()
        assert cola == A[-10:], cola
        assert f.tell() == len(A)
    rangos = [h.get("Range") for _, _, h in BYTES]
    assert "bytes=%d-" % (len(A) - 10) in rangos, rangos
    bien("3 · open(): read, seek desde el final con un Range (%d peticiones de bytes), tell" % len(BYTES))


def c4():
    it = c.stat(path="a.pdf")
    with it.open() as f:
        assert f.read() == A
    assert it.sha256_seen == SHA["a.pdf"], it.sha256_seen
    bien("4 · una lectura entera se verifica y da el sha256 visto")


def c5():
    malos = [h for _, _, h in BYTES if any(k.lower() in ("authorization", "x-ore-puesto") for k in h)]
    buenos = [h for _, _, h in SERVE
              if {k.lower(): v for k, v in h.items()}.get("authorization") == "Bearer secreto-de-ore"]
    assert not malos, "la URL de los bytes recibió el token: %s" % malos[:1]
    assert buenos and len(buenos) == len(SERVE)
    bien("5 · el token de ORE va a la celda (%d) y nunca a los bytes (%d)" % (len(SERVE), len(BYTES)))


def c6():
    renovadas = [r for _, r, _ in SERVE if "/content" in r and "a.pdf" in r]
    assert len(renovadas) >= 2 and all("version=v1" in r for r in renovadas), renovadas
    bien("6 · el primer permiso caducó a la primera: se pidió otro con version=v1 y se siguió")


def c7():
    try:
        c.stat(path="cambia.pdf").read_bytes()
    except ore.MediaChanged as e:
        assert e.status == 412
        bien("7 · la versión ya no está: MediaChanged (412)")
        return
    raise AssertionError("debía ser MediaChanged")


def c8():
    for path in ("corrupto.pdf", "cortado.pdf"):
        it = c.stat(path=path)
        if path == "corrupto.pdf":
            it.ref = medios.dataclasses.replace(it.ref, digest="sha256:" + "0" * 64)
        try:
            it.read_bytes()
        except ore.MediaCorrupt:
            continue
        raise AssertionError("%s debía ser MediaCorrupt" % path)
    bien("8 · el digest no casa, o el flujo se corta: MediaCorrupt")


def c9():
    medios.EN_PARALELO_DESDE, medios.TROZO = 50_000, 32_768
    BYTES.clear()
    it = c.stat(path="a.pdf")
    assert it.read_bytes(threads=4) == A
    rangos = [h.get("Range") for _, _, h in BYTES if h.get("Range")]
    assert len(rangos) >= len(A) // 32_768, rangos
    medios.EN_PARALELO_DESDE, medios.TROZO = 32 << 20, 8 << 20
    bien("9 · read_bytes() de uno grande: %d rangos en paralelo, verificado" % len(rangos))


def c10():
    r = {it.ref.path: (d, e) for it, d, e in ore.read_many(c.items(), threads=4)}
    assert r["a.pdf"][0] == A and r["b.pdf"][0] == OBJETOS["b.pdf"]
    assert isinstance(r["cambia.pdf"][1], ore.MediaChanged)
    bien("10 · leer_varios(): 3 a la vez, el 412 de uno es un valor")


def c11():
    CONTADOS["enviados"] = 0
    with c.stat(path="a.pdf").open() as f:
        f.read(10)
    import time
    time.sleep(0.3)
    assert CONTADOS["enviados"] < len(A), CONTADOS
    bien("11 · cerrar a medias: el servidor envió %d de %d bytes" % (CONTADOS["enviados"], len(A)))


def c12():
    # Un ítem cuyo listado no dice su tamaño (el hallazgo de B3·6 en un puesto
    # de victor): seek desde el final antes de leer nada, y verificar al final.
    it = c.stat(path="a.pdf")
    it.ref = medios.dataclasses.replace(it.ref, size=None)
    with it.open() as f:
        f.seek(-10, 2)
        assert f.read() == A[-10:]
    it = c.stat(path="a.pdf")
    it.ref = medios.dataclasses.replace(it.ref, size=None)
    with it.open() as f:
        assert f.read(5) == b"%PDF-"
        f.seek(-3, 2)
        assert f.read() == A[-3:]
    bien("12 · sin tamaño en el listado: se aprende de la respuesta (o de un byte) y el seek desde el final va")


def c13():
    # La rama del puesto viaja en las peticiones de la media, preguntada una vez.
    RAMA["r"] = "r1/trabajo"
    SERVE.clear()
    try:
        c2_ = ore.collection("legal.archivo.contratos")
        c2_.stat(path="a.pdf")
        c2_.stat(path="b.pdf")
    finally:
        RAMA["r"] = None
    fichas = [r for _, r, _ in SERVE if r == "/puestos/p1"]
    h = {k.lower(): v for k, v in SERVE[-1][2].items()}
    assert h.get("x-ore-rama") == "r1/trabajo", h
    assert len(fichas) == 1, fichas
    bien("13 · la rama del puesto va en x-ore-rama (la ficha, preguntada una vez)")


def c14():
    banco.TRANSFORMS.clear()

    @ore.transform(inputs=[ore.collection("legal.archivo.contratos")], output="legal.archivo.paginas")
    def paginar():
        return [i.ref.path for i in ore.collection("legal.archivo.contratos").items()]

    leidos = paginar()
    declarado = banco.TRANSFORMS[0][1]
    assert declarado["inputs"] == ["legal.archivo.contratos"], declarado
    assert declarado["output"] == "legal.archivo.paginas", declarado
    assert leidos == ["a.pdf", "b.pdf", "cambia.pdf"], leidos
    assert banco.TRANSFORMS[-1] == ("DELETE", None), banco.TRANSFORMS
    bien("14 · inputs=[ore.collection(…)]: se declara por su nombre, se lee dentro, y se retira al salir")


def c15():
    SERVE.clear()

    @ore.transform(inputs=[ore.collection("legal.archivo.contratos")], output="legal.archivo.paginas")
    def fuera():
        return list(ore.collection("legal.otra.fotos").items())

    try:
        fuera()
    except PermissionError as e:
        assert "legal.otra.fotos" in str(e), e
        assert not [r for _, r, _ in SERVE if r.startswith("/media/legal/otra")], SERVE
        bien("15 · una colección no declarada: PermissionError en la celda, sin preguntar al servidor")
        return
    raise AssertionError("debía ser PermissionError")


def c16():
    del ore._leidas[:]
    ore.collection("legal.archivo.contratos").stat(path="b.pdf")
    assert "legal.archivo.contratos" in ore._leidas, ore._leidas
    assert ore._procedencia()["leidas"] == ["legal.archivo.contratos"], ore._procedencia()
    bien("16 · fuera de un transform, leer una colección queda en lo leído (la procedencia de lo que se escriba)")


def c17():
    col = ore.collection("legal.archivo.contratos")
    assert col.as_of is None
    next(iter(col.items()))
    assert col.as_of == "7", col.as_of
    bien("17 · as_of: la transacción que el listado leyó (7)")


for n, f in enumerate([c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17], 1):
    caso(n, f)
celda.shutdown()
bytes_.shutdown()
print("todo bien" if banco.fallos["n"] == 0 else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
