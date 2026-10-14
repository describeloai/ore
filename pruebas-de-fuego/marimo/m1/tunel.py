"""M1·a · EL TÚNEL, del lado del pod: el puesto llama hacia fuera.

Abre UN WebSocket al relevo y por él recibe las peticiones del navegador; cada
una la hace a marimo en 127.0.0.1 en su hilo y devuelve la respuesta a trozos
según llega (un SSE no termina: se manda mientras dure). Si el relevo se cae,
reintenta. Nada entra al pod: el pod sale, como el agente sale a ore-serve.
"""
import http.client
import json
import socket
import os
import struct
import threading
import time

from websockets.sync.client import connect

RELEVO = os.environ.get("M1_RELEVO", "ws://relevo-m1:8080/tunel")
MARIMO = int(os.environ.get("M1_MARIMO_PORT", "2718"))
SALTO = {"connection", "keep-alive", "transfer-encoding", "proxy-connection"}
cancelados = set()
# M1·a: si el navegador se va, marimo tiene que enterarse YA (si no, la pestaña que
# recarga entra como lectora). Al cancelar se cierra la conexión con marimo.
conexiones = {}


def marco(cab, datos=b""):
    j = json.dumps(cab).encode()
    return struct.pack(">I", len(j)) + j + datos


def abrir(b):
    n = struct.unpack(">I", b[:4])[0]
    return json.loads(b[4:4 + n]), b[4 + n:]


class Canal:
    def __init__(self, ws):
        self.ws, self.candado = ws, threading.Lock()

    def enviar(self, cab, datos=b""):
        with self.candado:
            self.ws.send(marco(cab, datos))


def atender(canal, cab, cuerpo):
    rid = cab["id"]
    c = http.client.HTTPConnection("127.0.0.1", MARIMO, timeout=24 * 3600)
    conexiones[rid] = c
    try:
        cabeceras = {k: v for k, v in cab["headers"] if k.lower() not in SALTO | {"host", "content-length"}}
        c.request(cab["method"], cab["path"], body=cuerpo or None, headers=cabeceras)
        r = c.getresponse()
        canal.enviar({"t": "head", "id": rid, "status": r.status,
                      "headers": [(k, v) for k, v in r.getheaders() if k.lower() not in SALTO]})
        while rid not in cancelados:
            d = r.read1(65536)
            if not d:
                break
            canal.enviar({"t": "chunk", "id": rid}, d)
        canal.enviar({"t": "end", "id": rid})
    except Exception as e:  # noqa: BLE001
        try:
            canal.enviar({"t": "head", "id": rid, "status": 502, "headers": [("content-type", "text/plain")]})
            canal.enviar({"t": "chunk", "id": rid}, str(e).encode())
            canal.enviar({"t": "end", "id": rid})
        except Exception:  # noqa: BLE001
            pass
    finally:
        cancelados.discard(rid)
        conexiones.pop(rid, None)
        c.close()


def main():
    while True:
        try:
            with connect(RELEVO, max_size=64 * 2**20, open_timeout=10, ping_interval=20) as ws:
                print("túnel abierto con", RELEVO, flush=True)
                canal = Canal(ws)
                for msg in ws:
                    cab, cuerpo = abrir(msg)
                    if cab["t"] == "req":
                        threading.Thread(target=atender, args=(canal, cab, cuerpo), daemon=True).start()
                    elif cab["t"] == "cancel":
                        cancelados.add(cab["id"])
                        c = conexiones.get(cab["id"])
                        if c is not None and c.sock is not None:
                            try:
                                c.sock.shutdown(socket.SHUT_RDWR)
                            except OSError:
                                pass
        except Exception as e:  # noqa: BLE001
            print("túnel caído (%s): reintento en 1 s" % e, flush=True)
            time.sleep(1)


if __name__ == "__main__":
    main()
