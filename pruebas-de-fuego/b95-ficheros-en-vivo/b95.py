# 0049 B9·5 · Ficheros que dan ficheros, en vivo: cada contrato, una imagen por página.
#
# Con Run en el repositorio Models de victor, cuando el puesto ya trae el SDK de
# B9·4 (se comprueba abajo). Lee de nuestro lago: no pide la credencial de S3.
#
#   ① copia 3 contratos de `s3_stuff.nueva_carpeta.contratos` a una colección
#      escrita de entrada (`sandbox.default.b95_contratos_<n>`), para poder
#      cambiarla sin tocar la de S3;
#   ② un @transform: entrada → `sandbox.default.b95_paginas_<n>`, un PNG por
#      página (pypdfium2), con su linaje;
#   ③ otra vez: nada que hacer, nada escrito;
#   ④ cambia la entrada: un contrato cambia, otro se va, entra uno nuevo;
#   ⑤ otra vez: sólo lo que cambió, y los PNG de lo que se fue, retirados.
#
# `<n>` es la hora: cada vez que lo corras, colecciones nuevas.
import io
import time

import ore

if not hasattr(ore, "File"):
    raise SystemExit("✗ el SDK del puesto aún no tiene `ore.File` (0049 B9·4): espera al despliegue")
import pypdfium2 as pdfium

ORIGEN = "s3_stuff.nueva_carpeta.contratos"
N = time.strftime("%H%M")
ENTRADA = "sandbox.default.b95_contratos_%s" % N
SALIDA = "sandbox.default.b95_paginas_%s" % N
PPP = 100


def a_png(item):
    pdf = pdfium.PdfDocument(item.read_bytes())
    for n, pagina in enumerate(pdf, 1):
        png = io.BytesIO()
        pagina.render(scale=PPP / 72).to_pil().save(png, "PNG")
        yield ore.File("p%03d.png" % n, png.getvalue(), "image/png", {"kind": "page", "page": n})


@ore.transform(inputs=[ore.collection(ENTRADA)], output=ore.collection(SALIDA))
def paginas():
    # `@transform` declara lo que se lee y lo que se escribe; no pasa argumentos.
    return ore.collection(ENTRADA).apply(a_png, version="1", save_every_s=60)


def resumen(etiqueta, r):
    claves = ("items", "new", "recomputed", "skipped", "errors", "removed", "files_written", "files_retired", "written")
    print("  %-10s %s" % (etiqueta, " · ".join("%s %s" % (k, r.get(k)) for k in claves)))


# ── ① la entrada ─────────────────────────────────────────────────────────────
pdfs = [it for it in ore.collection(ORIGEN).items() if it.ref.path.lower().endswith(".pdf")][:4]
if len(pdfs) < 4:
    raise SystemExit("✗ hacen falta 4 PDF en %s y hay %d" % (ORIGEN, len(pdfs)))
bytes_de = {"c%d.pdf" % i: it.read_bytes() for i, it in enumerate(pdfs, 1)}
ore.create_collection(ENTRADA, "document", ["pdf"], comment="0049 B9·5: la entrada, que se cambia")
ore.create_collection(SALIDA, "image", ["png"], comment="0049 B9·5: una imagen por página")
with ore.collection(ENTRADA).transaction() as t:
    for c in ("c1.pdf", "c2.pdf", "c3.pdf"):
        t.put(c, bytes_de[c], "application/pdf")
print("① %s: c1, c2, c3 (%s)" % (ENTRADA, ", ".join("%d B" % len(bytes_de[c]) for c in ("c1.pdf", "c2.pdf", "c3.pdf"))))

# ── ② ③ ───────────────────────────────────────────────────────────────────────
t0 = time.time()
r = paginas()
resumen("② primera", r)
print("             %.1f s" % (time.time() - t0))
resumen("③ otra vez", paginas())

# ── ④ la entrada cambia ──────────────────────────────────────────────────────
with ore.collection(ENTRADA).transaction() as t:
    t.put("c1.pdf", bytes_de["c1.pdf"] + b"\n% cambiado en B9.5\n", "application/pdf")   # cambia
    t.delete("c3.pdf")                                                                   # se va
    t.put("c4.pdf", bytes_de["c4.pdf"], "application/pdf")                               # entra
print("④ c1 cambia, c3 se va, entra c4")

# ── ⑤ ────────────────────────────────────────────────────────────────────────
resumen("⑤ después", paginas())

# ── lo que quedó ─────────────────────────────────────────────────────────────
print("\n── %s" % SALIDA)
por_origen = {}
for it in ore.collection(SALIDA).items():
    por_origen.setdefault(it.ref.path.split("/")[0], []).append(it.ref.path)
for c in sorted(por_origen):
    print("  %-8s %d páginas" % (c, len(por_origen[c])))
print("  (c3 no debe estar; c1 y c4 sí)")
for d in ore.collection(SALIDA).derivations():
    print("  registro · %-60s %s %s" % (d["source"]["uri"][-60:], d["state"], len(d.get("files") or [])))
uno = next(iter(ore.collection(SALIDA).items()), None)
if uno is not None:
    print("\n  un fichero:", uno.ref.path)
    print("    source:    ", uno.ref.source)
    print("    derivation:", {k: (uno.ref.derivation or {}).get(k) for k in ("fn", "fn_version", "key")})
