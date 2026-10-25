"""ORE 0072 F2·3·2 · sirve ver.html y pmtiles/ en http://localhost:8765 con Range (lo que PMTiles necesita).

  python pruebas-de-fuego/geo/f2/piramide/servir.py [puerto]
"""
import http.server
import os
import re
import sys

D = os.path.dirname(os.path.abspath(__file__))


class Con(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=D, **kw)

    def end_headers(self):
        self.send_header("access-control-allow-origin", "*")
        self.send_header("accept-ranges", "bytes")
        self.send_header("cache-control", "no-store")
        super().end_headers()

    def do_GET(self):
        r = self.headers.get("range")
        ruta = self.translate_path(self.path.split("?")[0])
        m = re.match(r"bytes=(\d+)-(\d*)", r or "")
        if not m or not os.path.isfile(ruta):
            return super().do_GET()
        largo = os.path.getsize(ruta)
        a = int(m.group(1))
        b = min(int(m.group(2)) if m.group(2) else largo - 1, largo - 1)
        with open(ruta, "rb") as f:
            f.seek(a)
            cuerpo = f.read(b - a + 1)
        self.send_response(206)
        self.send_header("content-type", "application/octet-stream")
        self.send_header("content-range", "bytes %d-%d/%d" % (a, b, largo))
        self.send_header("content-length", str(len(cuerpo)))
        self.end_headers()
        self.wfile.write(cuerpo)

    def log_message(self, *a):
        pass


puerto = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
print("pirámide PoC en http://localhost:%d/ver.html" % puerto, flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", puerto), Con).serve_forever()
