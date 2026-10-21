"""Graba las tramas entre un cliente y el laboratorio de HBase: un proxy TCP para ZooKeeper y para el RegionServer.

    docker run -d --name hb -p 22181:2181 -p 26020:16020 ore-hbase-lab      # el laboratorio, en puertos desplazados
    python grabar_trazas.py trazas/nativo.json                              # el proxy, en 2181 y 16020
    HBASE_LAB_ZK=127.0.0.1:2181 cargo test -p ore-hbase-rpc --test laboratorio escenario_grabado
    (parar el proxy con Ctrl-C: escribe el fichero cada vez que se cierra una conexion)

El RegionServer se anuncia como 127.0.0.1:16020 (ZooKeeper y `hbase:meta`), asi que el cliente llega al proxy sin
tocar nada. El fichero guarda, por conexion y en orden, cada trama completa (con su longitud delante; la
apertura del RPC, preambulo y cabecera, entera) y de que lado viene: `c` el cliente, `s` el servidor.
Son los bytes de un HBase de verdad: las pruebas sin laboratorio los reproducen (`tests/trazas.rs`).
"""
import base64
import itertools
import json
import socket
import struct
import sys
import threading

SALIDA = sys.argv[1]
PUENTES = [("zk", 2181, 22181), ("rpc", 16020, 26020)]

orden = itertools.count()
cerrojo = threading.Lock()
conexiones = []  # [{"tipo": ..., "id": n, "tramas": [(orden, d, bytes)]}]


def volcar():
    with cerrojo:
        sal = {"conexiones": []}
        for c in sorted(conexiones, key=lambda c: c["id"]):
            if not c["cerrada"]:
                continue
            tramas = sorted(c["tramas"], key=lambda t: t[0])
            sal["conexiones"].append(
                {"tipo": c["tipo"], "tramas": [{"d": d, "b": base64.b64encode(b).decode()} for _, d, b in tramas]}
            )
        with open(SALIDA, "w") as f:
            json.dump(sal, f, indent=1)


class Cortador:
    """Parte un flujo en tramas: `[longitud de 4 bytes][cuerpo]`; en el RPC, el primer mensaje del cliente es la
    apertura: preambulo de 6 bytes, longitud y cabecera."""

    def __init__(self, apertura):
        self.buf = b""
        self.apertura = apertura

    def tramas(self, datos):
        self.buf += datos
        while True:
            if self.apertura:
                if len(self.buf) < 10:
                    return
                n = 10 + struct.unpack(">I", self.buf[6:10])[0]
            else:
                if len(self.buf) < 4:
                    return
                n = 4 + struct.unpack(">I", self.buf[:4])[0]
            if len(self.buf) < n:
                return
            t, self.buf = self.buf[:n], self.buf[n:]
            self.apertura = False
            yield t


def bombear(origen, destino, conn, d, apertura):
    cort = Cortador(apertura)
    try:
        while True:
            datos = origen.recv(65536)
            if not datos:
                break
            destino.sendall(datos)
            for t in cort.tramas(datos):
                with cerrojo:
                    conn["tramas"].append((next(orden), d, t))
    except OSError:
        pass
    finally:
        try:
            destino.shutdown(socket.SHUT_WR)
        except OSError:
            pass


identificador = itertools.count()


def atender(cliente, tipo, arriba):
    conn = {"tipo": tipo, "id": next(identificador), "tramas": [], "cerrada": False}
    with cerrojo:
        conexiones.append(conn)
    servidor = socket.create_connection(("127.0.0.1", arriba))
    a = threading.Thread(target=bombear, args=(cliente, servidor, conn, "c", tipo == "rpc"))
    b = threading.Thread(target=bombear, args=(servidor, cliente, conn, "s", False))
    a.start()
    b.start()
    a.join()
    b.join()
    cliente.close()
    servidor.close()
    conn["cerrada"] = True
    volcar()


def escuchar(tipo, puerto, arriba):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", puerto))
    s.listen()
    while True:
        c, _ = s.accept()
        threading.Thread(target=atender, args=(c, tipo, arriba), daemon=True).start()


for tipo, puerto, arriba in PUENTES:
    threading.Thread(target=escuchar, args=(tipo, puerto, arriba), daemon=True).start()
print("proxy listo: zk 2181 -> 22181, rpc 16020 -> 26020; Ctrl-C para acabar", flush=True)
try:
    threading.Event().wait()
except KeyboardInterrupt:
    volcar()
