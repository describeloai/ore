"""0046 E9·4 · el SDK de Python sirve un `Media<c>`: `media`, `medias` y `media_de`, contra un
ore-serve de mentira que dice lo que se le pidio (la ruta, la huella codificada, los lotes de cien,
`x-ore-puesto`). Sin red ni cluster:  python pruebas-de-fuego/el-sdk-sirve-un-media.py"""
import json, os, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
VISTO = []
class S(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _r(self, c, d):
        b = json.dumps(d).encode(); self.send_response(c); self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        VISTO.append(("GET", self.path, self.headers.get("x-ore-puesto")))
        if self.path.startswith("/puestos/p1/datos/legal.registro"):
            return self._r(200, {"dataset": "legal.registro", "media": {"documento": "legal.archivo.contratos"}})
        if self.path == "/colecciones/legal/archivo/contratos/items/crc64nvme%3Aab%2Fc%3D":
            return self._r(200, {"huella": "crc64nvme:ab/c=", "url": "https://x", "tipo": "application/pdf", "segundos": 300})
        return self._r(404, {"error": "ningún ítem de la colección lleva esa huella"})
    def do_POST(self):
        cuerpo = json.loads(self.rfile.read(int(self.headers["content-length"])))
        VISTO.append(("POST", self.path, len(cuerpo["huellas"])))
        items = [{"huella": h, "url": "https://" + h} for h in cuerpo["huellas"] if h != "nada"]
        return self._r(200, {"items": items, "segundos": int(cuerpo.get("ttl", 300)), "caduca_ms": 1, "no_estan": []})
srv = ThreadingHTTPServer(("127.0.0.1", 0), S); threading.Thread(target=srv.serve_forever, daemon=True).start()
os.environ["ORE_SERVE"] = "http://127.0.0.1:%d" % srv.server_address[1]; os.environ["PUESTO"] = "p1"
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "puesto", "python"))
import ore
assert ore.media_columns("legal.registro") == {"documento": "legal.archivo.contratos"}
assert ore.media_url("legal.archivo.contratos", "crc64nvme:ab/c=")["content_type"] == "application/pdf"
try:
    ore.media_url("legal.archivo.contratos", "otra"); raise SystemExit("debía ser LookupError")
except LookupError as e:
    assert "ningún ítem" in str(e)
m = ore.media_urls("legal.archivo.contratos", ["h%d" % i for i in range(250)] + ["h0", "nada"], ttl=60)
assert len(m) == 250 and m["h7"]["seconds"] == 60
assert [v for v in VISTO if v[0] == "POST"] == [("POST", "/colecciones/legal/archivo/contratos/items/resolver", 100),
                                               ("POST", "/colecciones/legal/archivo/contratos/items/resolver", 100),
                                               ("POST", "/colecciones/legal/archivo/contratos/items/resolver", 51)], VISTO
assert ore.media_url("legal.archivo.contratos", "h1", ttl=45)["url"] == "https://h1"
try:
    ore.media_url("contratos", "x"); raise SystemExit("debía ser ValueError")
except ValueError:
    pass
assert all(v[2] == "p1" for v in VISTO if v[0] == "GET"), "sin x-ore-puesto"
print("sdk python · media, medias y media_de en verde")
