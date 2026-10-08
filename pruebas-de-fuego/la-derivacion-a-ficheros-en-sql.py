"""0049 B10·3 · FICHEROS QUE DAN FICHEROS, EN SQL: lo que corre la celda de
`create or replace media collection … as select …`, contra el banco de la media.

La celda que ore-serve escribe (`celda_de_sentencia`, `S::ColeccionDerivada`)
crea la colección si no está y declara el transform; dentro, `_sql_a_ficheros`
corre la consulta ítem a ítem con DuckDB de verdad (`_consulta_por_item`, la de
B7) y da cada fila a `apply()` como un `ore.File` (B9). Aquí se corre eso mismo,
sobre la entrada de `derivar` (`docs/a.pdf`, `docs/c.pdf`, `img/b.png`):

  ① copiar sin función: `data` es el ítem (`c._item`), y sus bytes se copian;
  ② otra vez: nada que hacer, nada escrito;
  ③ bytes de una expresión (`encode(…)`, un BLOB) con `content_type` y ancla;
  ④ la consulta cambia: todo se recalcula y lo que ya no da se retira;
  ⑤ una fila sin `name`: el error de su ítem, y los demás siguen;
  ⑥ `_fichero_de_fila` y `_resultado_de_derivar`, sueltos.

    python pruebas-de-fuego/la-derivacion-a-ficheros-en-sql.py
"""
import base64
import json
import os
import sys

import banco_media as banco

banco.arrancar()
import ore  # noqa: E402
from ore import medios  # noqa: E402

RAIZ = banco.RAIZ
MUESTRA = json.load(open(os.path.join(RAIZ, "conformidad", "media", "muestra.json"), encoding="utf-8"))
ENTRADA = banco.DERIVAR


def contenido(item, version=None):
    if "texto" in item:
        return item["texto"].encode()
    if "base64" in item:
        return base64.b64decode(item["base64"])
    v = next(x for x in item["versiones"] if version is None or x["id"] == version)
    return v["texto"].encode()


por_ruta = {i["path"]: i for i in MUESTRA["items"] if "path" in i}
for s in MUESTRA["colecciones"]["derivar"]["solo"]:
    ruta, _, version = s.partition("@")
    banco.ENTRADA[ruta] = contenido(por_ruta[ruta], version or None)
RUTAS = sorted(banco.ENTRADA)
PDFS = [r for r in RUTAS if r.endswith(".pdf")]


def derivar(salida, media, formatos, consulta, nombre="consulta"):
    """Lo que hace la celda de ore-serve, tal cual."""
    ore.create_collection(salida, media, formatos, comment="B10·3", if_not_exists=True)

    @ore.transform(inputs=[ore.collection(ENTRADA)], output=ore.collection(salida))
    def consulta_():
        return ore._sql_a_ficheros(salida, ENTRADA, consulta, nombre)

    return consulta_()


def ficheros_de(salida):
    st = banco.ESCRITAS[salida]
    return {c: banco.LAGO[r["digest"].split(":", 1)[1]] for c, r in st["items"].items()}


def registro(salida):
    return {e["source"]["uri"].split("/", 3)[-1].split("?")[0]: e for e in banco.registro_de(banco.ESCRITAS[salida])}


def caso(n, f):
    banco.caso(n, lambda: (f(), banco.bien(n))[0])


# ── ① ② copiar, sin función ──────────────────────────────────────────────────
COPIA = "conformidad.default.b103_copia"
Q_COPIA = ("select 'copia.pdf' as name, c._item as data from %s as c where c.path like '%%.pdf'" % ENTRADA)


def copiar():
    r = derivar(COPIA, "document", ["pdf"], Q_COPIA)
    assert (r["items"], r["new"], r["skipped"], r["errors"], r["files_written"]) == (3, 3, 0, 0, 2), r
    assert r["written"], r
    fs = ficheros_de(COPIA)
    assert sorted(fs) == ["%s/copia.pdf" % p for p in PDFS], sorted(fs)
    for p in PDFS:
        assert fs["%s/copia.pdf" % p] == banco.ENTRADA[p], p   # los bytes del ítem, tal cual
    reg = registro(COPIA)
    assert reg["img/b.png"]["state"] == "empty", reg["img/b.png"]   # sin filas: su marca
    assert all(reg[p]["state"] == "files" for p in PDFS), reg


def otra_vez():
    r = derivar(COPIA, "document", ["pdf"], Q_COPIA)
    assert (r["skipped"], r["new"], r["files_written"], r["written"]) == (3, 0, 0, False), r


caso("① copiar: `data` es el ítem, y sus bytes se copian; sin filas, su marca", copiar)
caso("② otra vez: nada que hacer, ni una transacción", otra_vez)

# ── ③ ④ bytes de una expresión; la consulta cambia ──────────────────────────
TXT = "conformidad.default.b103_txt"


def q_txt(n):
    return ("select '%s' as name, encode(c.path) as data, 'text/plain' as content_type, "
            "{'kind': 'page', 'page': 1} as anchor from %s as c" % (n, ENTRADA))


def bytes_y_ancla():
    r = derivar(TXT, "document", ["txt"], q_txt("ruta.txt"))
    assert (r["items"], r["new"], r["files_written"]) == (3, 3, 3), r
    fs = ficheros_de(TXT)
    assert fs == {"%s/ruta.txt" % p: p.encode() for p in RUTAS}, fs
    for p in RUTAS:
        assert registro(TXT)[p]["files"] == [{"path": "%s/ruta.txt" % p, "anchor": {"kind": "page", "page": 1}}]


def cambia_la_consulta():
    r = derivar(TXT, "document", ["txt"], q_txt("ruta2.txt"))
    assert (r["recomputed"], r["skipped"], r["files_written"], r["files_retired"]) == (3, 0, 3, 3), r
    assert sorted(ficheros_de(TXT)) == ["%s/ruta2.txt" % p for p in RUTAS], sorted(ficheros_de(TXT))


caso("③ bytes de una expresión (BLOB), con su tipo y su ancla", bytes_y_ancla)
caso("④ la consulta cambia: todo se recalcula, lo que ya no da se retira", cambia_la_consulta)

# ── ⑤ una fila que no es un fichero ─────────────────────────────────────────
MAL = "conformidad.default.b103_mal"


def sin_nombre():
    q = ("select case when c.path like '%%.png' then null else 'x.txt' end as name, "
         "encode(c.path) as data from %s as c" % ENTRADA)
    r = derivar(MAL, "document", ["txt"], q)
    assert (r["items"], r["errors"], r["new"], r["files_written"]) == (3, 1, 2, 2), r
    e = registro(MAL)["img/b.png"]
    assert e["state"] == "error" and "no `name`" in e["error"]["message"], e


caso("⑤ una fila sin `name`: el error de su ítem, y los demás siguen", sin_nombre)

# ── ⑥ sueltos ───────────────────────────────────────────────────────────────


def sueltos():
    f = medios._fichero_de_fila({"name": "a.png", "data": bytearray(b"\x89PNG"), "content_type": None,
                                 "anchor": {"kind": "page", "page": 2, "bbox": None}})
    assert (f.name, f.data, f.content_type, f.anchor) == ("a.png", b"\x89PNG", None, {"kind": "page", "page": 2})
    f = medios._fichero_de_fila({"name": "b", "data": memoryview(b"xy"), "anchor": {"kind": None, "page": None}})
    assert (f.data, f.anchor) == (b"xy", None)
    for fila, que in [({"name": "", "data": b"x"}, "no `name`"), ({"name": "a", "data": None}, "not null"),
                      ({"name": "a", "data": 7}, "not int"), ({"name": "../a", "data": b"x"}, "relative path")]:
        try:
            medios._fichero_de_fila(fila)
            raise AssertionError("%r tenía que fallar" % fila)
        except (ValueError, TypeError) as e:
            assert que in str(e), (fila, str(e))
    t = ore._resultado_de_derivar({"items": 3, "new": 1, "files_written": 4, "written": True})
    assert t.column_names == ["items", "new", "recomputed", "skipped", "errors", "removed",
                              "files_written", "files_retired"], t.column_names
    assert t.to_pylist() == [{"items": 3, "new": 1, "recomputed": 0, "skipped": 0, "errors": 0, "removed": 0,
                              "files_written": 4, "files_retired": 0}]


caso("⑥ `_fichero_de_fila` y `_resultado_de_derivar`", sueltos)
print("\nB10·3: " + ("todo bien" if not banco.fallos["n"] else "%d fallos" % banco.fallos["n"]))
sys.exit(1 if banco.fallos["n"] else 0)
