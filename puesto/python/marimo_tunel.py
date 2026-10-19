"""0064 Pieza 3 · el túnel de marimo — SÓLO existe en la imagen
`puesto-python-notebooks:1`. Abre un flujo saliente hacia `ore-serve`
(`GET /puestos/{id}/marimo/entrada`), exactamente como `Correa._una_vuelta`
hace para el LSP, y por cada petición del navegador habla con marimo en
`127.0.0.1:2718` con `http.client` — el mismo patrón que medía
`pruebas-de-fuego/marimo/m1/tunel.py`, ahora de verdad.

Es también lo que cierra lo que la Pieza 2 dejó pendiente: cada vez que
marimo contesta algo, marca `marimo.actividad` — el TTL del agente por fin ve
tráfico real, no sólo celdas ni editor.
"""
import base64
import http.client
import json
import socket
import threading
import time
import urllib.request

PUERTO = 2718
HOST = "127.0.0.1"

# Calcado de `tunel.py` (el laboratorio M1): lo que no se reenvía nunca
# (cabecera por conexión, no por mensaje) — `HOST_CONTENT` además al
# construir la petición local, porque `http.client` pone las suyas.
_SALTO = {"connection", "keep-alive", "transfer-encoding", "proxy-connection"}
_SALTO_PETICION = _SALTO | {"host", "content-length"}

# La consola abre el iframe en cuanto el puesto existe, y marimo tarda ~2 s en
# escuchar (victor, 2026-10-19: la primera petición llegó 0,8 s antes y fue un
# 502). Mientras marimo no acepte conexiones, se espera hasta esto.
ESPERA_A_MARIMO = 30.0


def log(*a):
    print("marimo-tunel ·", *a, flush=True)


class Tunel:
    def __init__(self, p, testigo, marimo):
        self.p = p
        self.testigo = testigo
        self.marimo = marimo
        self.vivo = True
        self.candado = threading.Lock()
        self.salientes = []
        self.conexiones = {}
        self.cancelados = set()
        self.fallos = 0

    def escuchar(self):
        """Calco de `Correa.escuchar`: se reconecta sola."""
        threading.Thread(target=self._entregar, daemon=True).start()
        while self.vivo:
            try:
                self._una_vuelta()
            except Exception as e:  # noqa: BLE001
                log("el túnel se cortó (%s): vuelvo en 3 s" % e)
                time.sleep(3)

    def _una_vuelta(self):
        req = urllib.request.Request(
            "%s/puestos/%s/marimo/entrada" % (self.p.servidor, self.p.id), method="GET")
        req.add_header("accept", "text/event-stream")
        for k, v in self.testigo.cabeceras().items():
            req.add_header(k, v)
        with urllib.request.urlopen(req, timeout=300) as r:
            evento = None
            for linea in r:
                linea = linea.decode("utf-8", "replace").rstrip("\n").rstrip("\r")
                if linea.startswith("event: "):
                    evento = linea[7:]
                elif linea.startswith("data: "):
                    if evento == "marimo":
                        self._mensaje(linea[6:])
                    elif evento == "fin":
                        return

    def _mensaje(self, texto):
        try:
            m = json.loads(texto)
        except ValueError:
            return
        if m.get("t") == "peticion":
            threading.Thread(target=self._atender, args=(m,), daemon=True).start()
        elif m.get("t") == "cancelar":
            self._cancelar(m.get("id"))

    def _atender(self, m):
        """Una petición del navegador, llevada a marimo. Calco de
        `tunel.py::atender`: cabeceras/`chunk`/`end`, y si algo falla, un
        `head` 502 en vez de dejar la petición colgada."""
        rid = m["id"]
        c = http.client.HTTPConnection(HOST, PUERTO, timeout=24 * 3600)
        self.conexiones[rid] = c
        try:
            cabeceras = {k: v for k, v in m.get("cabeceras", []) if k.lower() not in _SALTO_PETICION}
            cuerpo = base64.b64decode(m["cuerpo"]) if m.get("cuerpo") else None
            limite = time.time() + ESPERA_A_MARIMO
            while True:
                try:
                    c.request(m["metodo"], m["ruta"], body=cuerpo, headers=cabeceras)
                    break
                except ConnectionRefusedError:
                    if time.time() >= limite or rid in self.cancelados:
                        raise
                    c.close()
                    time.sleep(0.25)
            r = c.getresponse()
            self.marimo.actividad = time.time()
            self._encolar({
                "id": rid, "t": "head", "estado": r.status,
                "cabeceras": [(k, v) for k, v in r.getheaders() if k.lower() not in _SALTO],
            })
            while rid not in self.cancelados:
                d = r.read1(65536)
                if not d:
                    break
                self.marimo.actividad = time.time()
                self._encolar({"id": rid, "t": "chunk", "datos": base64.b64encode(d).decode("ascii")})
            self._encolar({"id": rid, "t": "end"})
        except Exception as e:  # noqa: BLE001 — el navegador ve un 502, no un flujo colgado
            log("%s %s: 502 (%s: %s)" % (m.get("metodo"), m.get("ruta", "").split("?")[0], type(e).__name__, e))
            self._encolar({
                "id": rid, "t": "head", "estado": 502,
                "cabeceras": [("content-type", "text/plain")],
            })
            self._encolar({
                "id": rid, "t": "chunk",
                "datos": base64.b64encode(str(e).encode()).decode("ascii"),
            })
            self._encolar({"id": rid, "t": "end"})
        finally:
            self.cancelados.discard(rid)
            self.conexiones.pop(rid, None)
            c.close()

    def _cancelar(self, rid):
        """El navegador se fue: que marimo se entere YA (como `tunel.py`),
        no cuando note por su cuenta que nadie lee la respuesta."""
        if rid is None:
            return
        self.cancelados.add(rid)
        c = self.conexiones.get(rid)
        if c is not None and c.sock is not None:
            try:
                c.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def _encolar(self, mensaje):
        with self.candado:
            self.salientes.append(mensaje)

    def _entregar(self):
        """Cada 20 ms, lo que haya, de golpe — calco de `Correa._entregar`:
        `ore-entrada` no tiene keep-alive."""
        while self.vivo:
            time.sleep(0.02)
            self._entregar_una_vez()

    def _entregar_una_vez(self):
        """Un lote a ore-serve. Si no llega porque ore-serve está lleno (503) o
        la conexión se corta, el lote **vuelve delante** y se reintenta, en
        orden: tirarlo deja al navegador sin la cabecera o con un cuerpo a
        medias. En victor (2026-10-19) los ~30 *assets* de la portada de marimo
        llenaron las 64 conexiones de ore-serve, la salida dio 503, se tiró, y
        cada *asset* acabó en 504 a los 30 s: la página, en blanco."""
        with self.candado:
            lote, self.salientes = self.salientes, []
        if not lote:
            return True
        self.p._cabeceras = self.testigo.cabeceras()
        try:
            codigo, r = self.p.pedir(
                "POST", "/puestos/%s/marimo/salida" % self.p.id, cuerpo={"respuestas": lote})
        except Exception as e:  # noqa: BLE001 — la red se cae; se reintenta
            codigo, r = None, e
        if codigo in (200, 201, 202, 204):
            if self.fallos:
                log("la salida de marimo vuelve a entrar tras %d intento(s)" % self.fallos)
            self.fallos = 0
            return True
        if codigo is None or codigo in (429, 502, 503, 504):
            with self.candado:
                self.salientes = lote + self.salientes
            self.fallos += 1
            if self.fallos == 1:
                log("la salida de marimo no entró (%s): reintento" % (codigo or r))
            time.sleep(min(0.05 * self.fallos, 1.0))
            return False
        log("la salida de marimo no se aceptó: %s %s" % (codigo, r))
        return True


def arrancar(p, testigo, marimo):
    """Arranca el túnel en un hilo aparte y lo devuelve (para parar, en el
    futuro, si hiciera falta — hoy muere con el proceso, como `Correa`)."""
    t = Tunel(p, testigo, marimo)
    threading.Thread(target=t.escuchar, daemon=True).start()
    return t
