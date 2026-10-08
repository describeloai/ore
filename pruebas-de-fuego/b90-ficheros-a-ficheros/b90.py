# 0049 B9·0 · La medida de «ficheros que dan ficheros», en un puesto real.
#
# Con Run en un repositorio de Python de victor, cuando la imagen del puesto ya
# trae `pypdfium2` y `pillow` (se comprueba abajo). Mide, sin escribir nada en
# la colección de origen:
#
#   ① los contratos (`s3_stuff.nueva_carpeta.contratos`) a PNG, página a
#      página: leer, renderizar y codificar, con su tamaño;
#   ② una transacción de N ficheros pequeños con `put_many` en una colección
#      escrita de medida (`sandbox.default.b90_medida`): subir y confirmar;
#   ③ la misma transacción otra vez: lo que cuesta una pasada que no cambia
#      nada (lo que hará `apply()` al repetir).
#
# Deja `sandbox.default.b90_medida` con N ficheros: borrar un ítem de una
# colección escrita es justo lo que B9·2 añade.
import io
import time

import ore

ORIGEN = "s3_stuff.nueva_carpeta.contratos"
MEDIDA = "sandbox.default.b90_medida"
N = 1000
PPP = 150  # puntos por pulgada al renderizar

try:
    import PIL
    import pypdfium2 as pdfium
except ImportError as e:
    raise SystemExit("✗ la imagen del puesto aún no trae %s: espera al despliegue de la imagen nueva" % e.name)
from importlib.metadata import version  # noqa: E402
print("pypdfium2", version("pypdfium2"), "· pillow", version("pillow"))


def ms(t):
    return round((time.perf_counter() - t) * 1000)


# ── ① PDF → PNG por página ───────────────────────────────────────────────────
print("\n① %s → PNG a %d ppp" % (ORIGEN, PPP))
paginas, total_render, total_bytes = 0, 0, 0
for item in ore.collection(ORIGEN).items():
    if not (item.ref.path or "").lower().endswith(".pdf"):
        continue
    t = time.perf_counter()
    datos = item.read_bytes()
    leer = ms(t)
    t = time.perf_counter()
    doc = pdfium.PdfDocument(datos)
    tamanos = []
    for pagina in doc:
        png = io.BytesIO()
        pagina.render(scale=PPP / 72).to_pil().save(png, "PNG")
        tamanos.append(len(png.getvalue()))
    render = ms(t)
    paginas += len(tamanos)
    total_render += render
    total_bytes += sum(tamanos)
    print("  %-40s %7d B · leer %5d ms · %3d pág · render %6d ms (%d ms/pág) · PNG %d KB/pág"
          % (item.ref.path[-40:], len(datos), leer, len(tamanos), render,
             render // max(1, len(tamanos)), sum(tamanos) // max(1, len(tamanos)) // 1024))
if paginas:
    print("  total: %d páginas · %d ms/pág · %d KB/pág de media"
          % (paginas, total_render // paginas, total_bytes // paginas // 1024))

# ── ② una transacción de N ficheros ──────────────────────────────────────────
from PIL import Image  # noqa: E402

def png_pequeno(i):
    """Un PNG de 64×64 distinto por `i` (otro color): N blobs distintos."""
    b = io.BytesIO()
    Image.new("RGB", (64, 64), (i % 256, (i // 256) % 256, 128)).save(b, "PNG")
    return b.getvalue()

ore.create_collection(MEDIDA, "image", ["png"], if_not_exists=True,
                      comment="0049 B9·0: la medida de una transacción de muchos ficheros")
col = ore.collection(MEDIDA)


def pasada(etiqueta):
    t0 = time.perf_counter()
    errores = 0
    with col.transaction() as tx:
        t = time.perf_counter()
        for _, _, e in tx.put_many((("medida/f%04d.png" % i, png_pequeno(i), "image/png") for i in range(N)),
                                   threads=8):
            errores += e is not None
        subir = ms(t)
        t = time.perf_counter()
        r = tx.commit()
        confirmar = ms(t)
    print("  %s: %d ficheros · subir %d ms (%.1f ficheros/s) · confirmar %d ms · total %d ms · errores %d"
          % (etiqueta, N, subir, N / max(subir, 1) * 1000, confirmar, ms(t0), errores))
    print("    el servidor:", {k: r.get(k) for k in ("transaction", "items", "changes", "unchanged") if k in r})


print("\n② una transacción de %d ficheros en %s" % (N, MEDIDA))
pasada("primera")
print("\n③ la misma, otra vez (nada cambia)")
pasada("repetida")
