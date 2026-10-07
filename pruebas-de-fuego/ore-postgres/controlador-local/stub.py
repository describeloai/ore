# el plano de control de juguete: acepta /notify-attach y /notify-safekeepers y los apunta
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    def do_PUT(self): self._ok()
    def do_POST(self): self._ok()
    def _ok(self):
        n = int(self.headers.get('Content-Length', 0)); b = self.rfile.read(n)
        print(self.command, self.path, b.decode(), flush=True)
        self.send_response(200); self.send_header('Content-Length', '0'); self.end_headers()
http.server.HTTPServer(('0.0.0.0', 8000), H).serve_forever()
