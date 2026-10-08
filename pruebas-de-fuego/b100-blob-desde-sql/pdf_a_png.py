# 0049 B10·0 · La función que da los bytes: un PNG por página de un contrato.
#
# En tu repositorio functions-python de victor, publicada como cualquier otra.
# En SQL es `functions.pdf_a_png(c.item)` (0056: espacio propio, fuera de toda
# base). Devuelve `list[Pagina]`: con `cross join lateral`, una fila por página.
import io
from dataclasses import dataclass

import ore
from ore.tipos import Media

PPP = 100  # puntos por pulgada al renderizar


@dataclass
class Ancla:
    kind: str
    page: int


@dataclass
class Pagina:
    name: str
    data: bytes  # Opaque en el contrato, BLOB en SQL
    anchor: Ancla


@ore.function
def pdf_a_png(item: Media["s3_stuff.nueva_carpeta.contratos"]) -> list[Pagina]:
    import pypdfium2 as pdfium

    # `item` es el MediaRef (sin bytes): se leen fijados a su versión.
    datos = ore.Item(ore.collection(item.collection), item).read_bytes()
    paginas = []
    for n, pagina in enumerate(pdfium.PdfDocument(datos), 1):
        png = io.BytesIO()
        pagina.render(scale=PPP / 72).to_pil().save(png, "PNG")
        paginas.append(Pagina("p%03d.png" % n, png.getvalue(), Ancla("page", n)))
    return paginas
