# ore-iam de mentira para el laboratorio de P5: dos celdas de dos organizaciones.
# `POST /access/v1/celda` con `Authorization: Bearer <celda>` → la celda y su organización.
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
CELDAS = {"demo": "org_demo", "victor": "org_victor"}
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        t = self.headers.get("Authorization", "").removeprefix("Bearer ").strip()
        if self.path != "/access/v1/celda" or t not in CELDAS:
            c, r = 401, {"error": "no es una celda"}
        else:
            c, r = 200, {"celda": {"id": "cel_" + t, "nombre": t}, "organizacion": CELDAS[t]}
        b = json.dumps(r).encode()
        self.send_response(c); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
HTTPServer(("0.0.0.0", 8090), H).serve_forever()
