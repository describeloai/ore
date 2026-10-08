"""Las salidas de una celda, S1 · `json`: un valor compuesto sale como árbol.

`Kernel.salida_de` decide por el último valor de la celda: tabla, `json`,
texto o vacía; `como_json` lo convierte con sus topes.
"""
import dataclasses
import datetime
import decimal
import json
import os
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import agente  # noqa: E402
from ore.medios import MediaRef  # noqa: E402

# `pillow` y `matplotlib` vienen en la imagen del puesto (`provisto.txt`), pero no
# en todas partes donde corre esta suite (el plano de CI): sin ellas, lo suyo se salta.
import importlib.util  # noqa: E402
CON_PIL = importlib.util.find_spec("PIL") is not None
CON_MPL = importlib.util.find_spec("matplotlib") is not None
sin_pil = unittest.skipUnless(CON_PIL, "sin pillow")
sin_mpl = unittest.skipUnless(CON_PIL and CON_MPL, "sin matplotlib")


@dataclasses.dataclass
class Ancla:
    kind: str
    page: int


def salida(valor, texto=""):
    return agente.Kernel.salida_de(valor, texto, time.time())


class Json(unittest.TestCase):
    def test_un_dict_es_json_con_lo_impreso_debajo(self):
        s = salida({"a": 1, "b": [1, 2], "c": None}, "impreso\n")
        self.assertEqual((s["tipo"], s["valor"], s["texto"], s["recortado"]),
                         ("json", {"a": 1, "b": [1, 2], "c": None}, "impreso\n", False))

    def test_listas_tuplas_conjuntos_y_dataclasses(self):
        self.assertEqual(salida([1, (2, 3)])["valor"], [1, [2, 3]])
        self.assertEqual(salida({3, 1, 2})["valor"], [1, 2, 3])
        self.assertEqual(salida(Ancla("page", 1))["valor"], {"kind": "page", "page": 1})
        self.assertEqual(salida({"anchor": Ancla("page", 2)})["valor"], {"anchor": {"kind": "page", "page": 2}})

    def test_un_mediaref_dentro_de_un_dict_y_lo_que_devuelve_el_sdk(self):
        r = MediaRef(uri="ore://a.b.c/x.pdf?v=1", collection="a.b.c", path="x.pdf", version="1", size=717)
        v = salida({"ref": r})["valor"]["ref"]
        self.assertEqual((v["path"], v["size"], v["digest"]), ("x.pdf", 717, None))
        from ore import _Result
        self.assertEqual(salida(_Result({"items": 4, "written": False}))["valor"], {"items": 4, "written": False})

    def test_las_hojas_van_como_el_contrato(self):
        v = salida({"d": decimal.Decimal("1.50"), "f": datetime.date(2026, 10, 8),
                    "t": datetime.datetime(2026, 10, 8, 12, 0, tzinfo=datetime.timezone.utc),
                    "grande": 2 ** 60, "nan": float("nan"), "b": b"\x89PNG" * 10})["valor"]
        self.assertEqual(v, {"d": "1.50", "f": "2026-10-08", "t": "2026-10-08T12:00:00Z",
                             "grande": str(2 ** 60), "nan": "NaN", "b": "bytes · 40 B"})
        json.dumps(v)   # y es JSON

    def test_un_objeto_que_no_es_json_va_por_su_repr(self):
        class Raro:
            def __repr__(self):
                return "<Raro>"
        self.assertEqual(salida({"x": Raro()})["valor"], {"x": "<Raro>"})

    def test_los_topes_recortan_y_lo_dicen(self):
        s = salida(list(range(agente.JSON_POR_NIVEL + 7)))
        self.assertTrue(s["recortado"])
        self.assertEqual(s["valor"][-1], "… 7 more items")
        self.assertEqual(len(s["valor"]), agente.JSON_POR_NIVEL + 1)
        d = salida({str(i): i for i in range(agente.JSON_POR_NIVEL + 2)})["valor"]
        self.assertEqual(d["…"], "2 more keys")
        largo = salida({"s": "x" * (agente.JSON_CADENA + 3)})
        self.assertTrue(largo["recortado"] and largo["valor"]["s"].endswith("(3 more characters)"))
        hondo = []
        for _ in range(agente.JSON_HONDO + 5):
            hondo = [hondo]
        self.assertTrue(salida(hondo)["recortado"])

    def test_lo_que_no_es_compuesto_sigue_como_antes(self):
        self.assertEqual(salida(2)["tipo"], "texto")
        self.assertEqual(salida(2)["texto"], "2")
        self.assertEqual(salida("hola")["texto"], "'hola'")
        self.assertEqual(salida(None, "impreso")["tipo"], "texto")
        self.assertEqual(salida(None)["tipo"], "vacia")
        self.assertEqual(salida(Ancla)["tipo"], "texto")   # la clase, no una instancia

    def test_una_tabla_sigue_siendo_tabla(self):
        import pyarrow as pa
        s = salida(pa.table({"a": [1, 2]}))
        self.assertEqual((s["tipo"], s["total"]), ("tabla", 2))

    def test_lo_que_no_cabe_ni_recortado_sale_como_texto(self):
        viejo = agente.JSON_BYTES
        agente.JSON_BYTES = 100
        try:
            self.assertEqual(salida({"a": "x" * 500})["tipo"], "texto")
        finally:
            agente.JSON_BYTES = viejo

    def test_la_celda_entera_por_el_kernel(self):
        k = agente.Kernel.__new__(agente.Kernel)
        k.espacio = {}
        s = k._correr("import dataclasses\nprint('hola')\n{'n': 1, 'l': [1, 2]}")
        self.assertEqual((s["tipo"], s["valor"], s["texto"]), ("json", {"n": 1, "l": [1, 2]}, "hola\n"))


def png(ancho=4, alto=3, color=(200, 10, 10)):
    import io
    from PIL import Image
    b = io.BytesIO()
    Image.new("RGB", (ancho, alto), color).save(b, "PNG")
    return b.getvalue()


class Media(unittest.TestCase):
    """S2 · un ítem de una colección: su referencia, sin URL ni bytes."""

    def ref(self, path="x.png", tipo="image/png"):
        return MediaRef(uri="ore://a.b.c/%s?v=1" % path, collection="a.b.c", path=path, version="1",
                        size=10, content_type=tipo, digest="sha256:aa")

    def test_un_mediaref_un_item_y_una_lista(self):
        from ore.medios import Item, collection
        s = salida(self.ref())
        self.assertEqual((s["tipo"], s["total"], s["recortado"]), ("media", 1, False))
        self.assertEqual(s["items"], [{"collection": "a.b.c", "path": "x.png", "version": "1",
                                       "content_type": "image/png", "size": 10, "digest": "sha256:aa"}])
        self.assertNotIn("url", json.dumps(s))   # ni una URL firmada en el historial
        self.assertEqual(salida(Item(collection("a.b.c"), self.ref("y.pdf")))["items"][0]["path"], "y.pdf")
        self.assertEqual([i["path"] for i in salida([self.ref("a.png"), self.ref("b.png")])["items"]],
                         ["a.png", "b.png"])

    def test_una_lista_larga_se_recorta_y_una_mezcla_no_es_media(self):
        s = salida([self.ref("p%d.png" % i) for i in range(agente.MEDIA_MAXIMOS + 3)])
        self.assertEqual((len(s["items"]), s["total"], s["recortado"]), (agente.MEDIA_MAXIMOS, agente.MEDIA_MAXIMOS + 3, True))
        self.assertEqual(salida([self.ref(), 1])["tipo"], "json")
        self.assertEqual(salida([])["tipo"], "json")


class Imagen(unittest.TestCase):
    """S2 · una imagen: sus bytes en base64, con su tipo y su tamaño."""

    @sin_pil
    def test_bytes_de_png_jpeg_gif_webp_y_svg(self):
        import base64
        b = png()
        s = salida(b)
        self.assertEqual((s["tipo"], s["mime"], s["ancho"], s["alto"], s["bytes"], s["reducida"]),
                         ("imagen", "image/png", 4, 3, len(b), False))
        self.assertEqual(base64.b64decode(s["base64"]), b)
        self.assertEqual(agente.tipo_de_imagen(bytes([0xFF, 0xD8, 0xFF, 0xE0]) + b"xx"), "image/jpeg")
        self.assertEqual(agente.tipo_de_imagen(b"GIF89a..."), "image/gif")
        self.assertEqual(agente.tipo_de_imagen(b"RIFF\x00\x00\x00\x00WEBPVP8 "), "image/webp")
        self.assertEqual(agente.tipo_de_imagen(b'  <svg xmlns="http://www.w3.org/2000/svg"/>'), "image/svg+xml")
        self.assertEqual(salida(b"%PDF-1.7 no es una imagen")["tipo"], "texto")

    @sin_mpl
    def test_pil_matplotlib_y_ore_file(self):
        from PIL import Image
        import ore
        self.assertEqual(salida(Image.new("RGB", (5, 2)))["ancho"], 5)
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
        fig, ax = plt.subplots(figsize=(2, 1))
        ax.plot([1, 2, 3])
        s = salida(fig)
        plt.close(fig)
        self.assertEqual((s["tipo"], s["mime"]), ("imagen", "image/png"))
        f = salida(ore.File("p001.png", png(), "image/png", {"kind": "page", "page": 1}))
        self.assertEqual((f["tipo"], f["nombre"]), ("imagen", "p001.png"))
        self.assertEqual(salida(ore.File("a.txt", b"hola"))["tipo"], "texto")

    @sin_pil
    def test_una_grande_se_reduce_por_debajo_del_tope(self):
        import os as _os
        from PIL import Image
        import io
        ruido = Image.frombytes("RGB", (1400, 1400), _os.urandom(1400 * 1400 * 3))
        b = io.BytesIO()
        ruido.save(b, "PNG")
        s = salida(b.getvalue())
        self.assertTrue(s["reducida"], s.get("mime"))
        self.assertLessEqual(len(s["base64"]) * 3 // 4, agente.IMAGEN_BYTES)
        self.assertEqual((s["ancho"], s["bytes"]), (1400, len(b.getvalue())))   # lo de antes de reducirla
        self.assertLess(len(json.dumps(s)), 1 << 20)   # cabe en el cuerpo de ore-serve


class Html(unittest.TestCase):
    """S4 · lo que se dibuja como HTML: un `_repr_html_`, `ore.HTML`, `ore.Markdown`."""

    def test_ore_html_y_un_objeto_con_repr_html(self):
        import ore
        s = salida(ore.HTML("<h3>Hola</h3>"))
        self.assertEqual((s["tipo"], s["html"]), ("html", "<h3>Hola</h3>"))

        class Mapa:
            def _repr_html_(self):
                return "<div id='mapa'></div><script>1</script>"
        self.assertEqual(salida(Mapa())["html"], "<div id='mapa'></div><script>1</script>")

    def test_markdown_se_convierte_y_escapa_lo_que_no_es_markdown(self):
        import ore
        h = salida(ore.Markdown("### Páginas\n**4** PNG <b>x</b>\n\n- a\n- [b](https://x.y)"))["html"]
        self.assertIn("<h3>Páginas</h3>", h)
        self.assertIn("<strong>4</strong> PNG &lt;b&gt;x&lt;/b&gt;", h)
        self.assertIn('<li><a href="https://x.y" target="_blank" rel="noopener">b</a></li>', h)
        # sólo http(s) es un enlace: lo demás queda como texto
        self.assertNotIn('href="javascript', agente.markdown_a_html("[x](javascript:alert(1))"))

    def test_el_styler_de_pandas(self):
        try:
            import jinja2  # noqa: F401 — el Styler lo necesita (viene en la imagen)
        except ImportError:
            self.skipTest("sin jinja2")
        import pandas as pd
        s = salida(pd.DataFrame({"kb": [1.0, 2.5]}).style.format({"kb": "{:.1f} KB"}))
        self.assertEqual(s["tipo"], "html")
        self.assertIn("2.5 KB", s["html"])

    def test_lo_de_antes_va_antes_y_lo_grande_es_texto(self):
        import ore
        import pandas as pd
        self.assertEqual(salida(pd.DataFrame({"a": [1]}))["tipo"], "tabla")   # también tiene _repr_html_
        self.assertEqual(salida("<b>una cadena</b>")["tipo"], "texto")        # una cadena no es HTML
        self.assertEqual(salida(ore.HTML("x" * (agente.HTML_BYTES + 1)))["tipo"], "texto")

        class Roto:
            def _repr_html_(self):
                raise ValueError("no")
        self.assertEqual(salida(Roto())["tipo"], "texto")

    def test_en_varias_partes(self):
        k = agente.Kernel.__new__(agente.Kernel)
        k.espacio = {"ore": __import__("ore")}
        s = k._correr("ore.display(ore.Markdown('**a**'))\nprint('b')\nore.HTML('<i>c</i>')")
        self.assertEqual([p["tipo"] for p in s["partes"]], ["html", "texto", "html"])


class Varias(unittest.TestCase):
    """S3 · `ore.display()`: varias salidas en una celda, en orden, con lo impreso entre medias."""

    def celda(self, texto):
        k = agente.Kernel.__new__(agente.Kernel)
        k.espacio = {"ore": __import__("ore")}
        return k._correr(texto)

    def test_partes_en_orden_con_el_texto_entre_medias(self):
        s = self.celda("import pyarrow as pa\n"
                       "print('uno')\n"
                       "ore.display({'a': 1})\n"
                       "print('dos')\n"
                       "ore.display(pa.table({'x': [1, 2]}), [3, 4])\n"
                       "'fin'")
        self.assertEqual(s["tipo"], "varias")
        tipos = [(p["tipo"], p.get("texto") or p.get("valor") or p.get("total")) for p in s["partes"]]
        self.assertEqual(tipos, [("texto", "uno\n"), ("json", {"a": 1}), ("texto", "dos\n"), ("tabla", 2),
                                 ("json", [3, 4]), ("texto", "'fin'")])
        self.assertEqual(s["fuera"], 0)
        self.assertTrue(all("ms" not in p for p in s["partes"]))

    def test_sin_display_todo_sigue_como_antes(self):
        self.assertEqual(self.celda("print('hola')\n{'n': 1}")["tipo"], "json")
        self.assertEqual(self.celda("1 + 1")["texto"], "2")

    def test_una_sola_parte_sale_sola(self):
        s = self.celda("ore.display({'n': 1})")
        self.assertEqual((s["tipo"], s["valor"]), ("json", {"n": 1}))

    @sin_mpl
    def test_plt_show_y_las_figuras_que_quedan_abiertas(self):
        import matplotlib.pyplot as plt
        # en el puesto lo pone `MPLBACKEND`; aquí otro test ya eligió Agg
        plt.switch_backend("module://ore._mpl")
        s = self.celda("import matplotlib.pyplot as plt\n"
                       "for i in range(2):\n"
                       "    plt.plot([1, i])\n"
                       "    print('figura', i)\n"
                       "    plt.show()\n"
                       "plt.figure(); _ = plt.plot([3, 3])\n")
        self.assertEqual([p["tipo"] for p in s["partes"]], ["texto", "imagen", "texto", "imagen", "imagen"])
        self.assertEqual(plt.get_fignums(), [])   # cerradas
        # una figura como último valor, una vez
        s = self.celda("import matplotlib.pyplot as plt\nf = plt.figure(); plt.plot([1, 2])\nf")
        self.assertEqual(s["tipo"], "imagen")

    def test_los_topes_dejan_fuera_lo_que_no_cabe_y_lo_cuentan(self):
        s = self.celda("for i in range(%d):\n    ore.display({'i': i})" % (agente.PARTES_MAXIMAS + 5))
        self.assertEqual((len(s["partes"]), s["fuera"]), (agente.PARTES_MAXIMAS, 5))

    @sin_pil
    def test_varias_imagenes_grandes_se_reducen_para_caber(self):
        # varias imágenes grandes: se reducen para caber todas, por debajo del cuerpo de ore-serve
        s = self.celda("import os, io\nfrom PIL import Image\n"
                       "for i in range(3):\n"
                       "    b = io.BytesIO(); Image.frombytes('RGB', (900, 900), os.urandom(900*900*3)).save(b, 'PNG')\n"
                       "    ore.display(b.getvalue())")
        self.assertEqual([p["tipo"] for p in s["partes"]], ["imagen"] * 3)
        self.assertLess(len(json.dumps(s)), 1 << 20)

    def test_display_fuera_de_una_celda_imprime(self):
        import contextlib
        import io
        import ore
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ore.display({"a": 1}, "texto")
        self.assertEqual(out.getvalue(), "{'a': 1}\ntexto\n")


if __name__ == "__main__":
    unittest.main()
