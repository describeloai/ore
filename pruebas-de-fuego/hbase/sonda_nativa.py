"""X0 de HBase, ruta nativa: lo minimo para leer una tabla hablando el protocolo de HBase desde fuera de la JVM.

    python sonda_nativa.py <carpeta_con_los_pb2> [tabla]

Los mensajes salen de los .proto que trae el propio jar de HBase (hbase-protocol-2.4.18.jar), compilados con
protoc. Pasos medidos: 1) ZooKeeper -> donde esta hbase:meta; 2) RPC SIMPLE a ese RegionServer; 3) Scan de
hbase:meta -> regiones de la tabla; 4) Scan de cada region.  Sin Kerberos, sin cellblock, sin compresion.
"""
import socket
import struct
import sys
import time

sys.path.insert(0, sys.argv[1])
from google.protobuf.internal.decoder import _DecodeVarint32  # noqa: E402
from google.protobuf.internal.encoder import _VarintBytes  # noqa: E402
import Client_pb2 as C  # noqa: E402
import HBase_pb2 as H  # noqa: E402
import RPC_pb2 as R  # noqa: E402
import ZooKeeper_pb2 as Z  # noqa: E402

TABLA = sys.argv[2] if len(sys.argv) > 2 else "TestTable"


def zk_meta(host="127.0.0.1", puerto=2181):
    from kazoo.client import KazooClient
    zk = KazooClient(hosts=f"{host}:{puerto}")
    zk.start()
    datos, _ = zk.get("/hbase/meta-region-server")
    zk.stop()
    # 0xFF, longitud del id (4 bytes), id, "PBUF", protobuf
    n = struct.unpack(">I", datos[1:5])[0]
    resto = datos[5 + n:]
    assert resto[:4] == b"PBUF", resto[:8]
    m = Z.MetaRegionServer()
    m.ParseFromString(resto[4:])
    return m.server.host_name, m.server.port


class Rpc:
    def __init__(self, host, puerto):
        self.s = socket.create_connection((host, puerto), timeout=60)
        self.s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.id = 0
        # preambulo: "HBas", version 0, autenticacion SIMPLE (80)
        self.s.sendall(b"HBas" + bytes([0, 80]))
        h = R.ConnectionHeader()
        h.user_info.effective_user = "ore"
        h.service_name = "ClientService"
        h.version_info.version = "2.4.18"
        h.version_info.url = ""
        h.version_info.revision = ""
        h.version_info.user = ""
        h.version_info.date = ""
        h.version_info.src_checksum = ""
        b = h.SerializeToString()
        self.s.sendall(struct.pack(">I", len(b)) + b)

    def _leer(self, n):
        buf = bytearray()
        while len(buf) < n:
            x = self.s.recv(n - len(buf))
            if not x:
                raise EOFError("el servidor cerro la conexion")
            buf += x
        return bytes(buf)

    def llamar(self, metodo, peticion, respuesta):
        self.id += 1
        cab = R.RequestHeader(call_id=self.id, method_name=metodo, request_param=True)
        cuerpo = peticion.SerializeToString()
        c = cab.SerializeToString()
        msg = _VarintBytes(len(c)) + c + _VarintBytes(len(cuerpo)) + cuerpo
        self.s.sendall(struct.pack(">I", len(msg)) + msg)
        total = struct.unpack(">I", self._leer(4))[0]
        datos = self._leer(total)
        n, p = _DecodeVarint32(datos, 0)
        rc = R.ResponseHeader()
        rc.ParseFromString(datos[p:p + n])
        p += n
        if rc.HasField("exception"):
            raise RuntimeError(f"{rc.exception.exception_class_name}: {rc.exception.stack_trace[:200]}")
        n, p = _DecodeVarint32(datos, p)
        respuesta.ParseFromString(datos[p:p + n])
        return respuesta


def scan(rpc, region, ini=b"", fin=b"", filas=1000, tipo=H.RegionSpecifier.REGION_NAME, max_filas=10**12):
    """Un scan entero de una region; devuelve (n_filas, n_celdas)."""
    req = C.ScanRequest()
    req.region.type = tipo
    req.region.value = region
    req.scan.start_row = ini
    req.scan.include_start_row = True
    if fin:
        req.scan.stop_row = fin
    req.scan.max_versions = 1
    req.scan.caching = filas
    req.number_of_rows = filas
    req.client_handles_partials = True
    req.client_handles_heartbeats = True
    n_filas = n_celdas = 0
    r = rpc.llamar("Scan", req, C.ScanResponse())
    sid = r.scanner_id
    seq = 0
    while True:
        for res in r.results:
            n_filas += 1 if not res.partial else 0
            n_celdas += len(res.cell)
        # el servidor cierra solo el scanner al acabar la region: more_results_in_region lo dice
        if r.HasField("more_results_in_region") and not r.more_results_in_region:
            return n_filas, n_celdas
        if not r.more_results or n_filas >= max_filas:
            break
        nxt = C.ScanRequest(scanner_id=sid, number_of_rows=filas, next_call_seq=seq, client_handles_partials=True,
                            client_handles_heartbeats=True)
        seq += 1
        r = rpc.llamar("Scan", nxt, C.ScanResponse())
    cl = C.ScanRequest(scanner_id=sid, close_scanner=True, number_of_rows=0, next_call_seq=seq)
    rpc.llamar("Scan", cl, C.ScanResponse())
    return n_filas, n_celdas


def regiones(meta_rpc, tabla):
    """Las regiones de la tabla, leidas de hbase:meta."""
    # hbase:meta NO se ordena por bytes: empezar en "tabla," se salta en silencio la region de clave vacia.
    ini = tabla.encode() + b",,"
    fin = tabla.encode() + b"-"  # ',' + 1 = '-'
    req = C.ScanRequest()
    req.region.type = H.RegionSpecifier.REGION_NAME
    req.region.value = b"hbase:meta,,1"
    req.scan.start_row = ini
    req.scan.include_start_row = True
    req.scan.stop_row = fin
    req.scan.max_versions = 1
    req.number_of_rows = 100
    req.close_scanner = True
    r = meta_rpc.llamar("Scan", req, C.ScanResponse())
    sal = []
    for res in r.results:
        celdas = {c.qualifier: c.value for c in res.cell}
        ri = H.RegionInfo()
        ri.ParseFromString(celdas[b"regioninfo"][4:])  # tras "PBUF"
        host, puerto = celdas[b"server"].decode().split(":")
        sal.append((res.cell[0].row, ri.start_key, ri.end_key, host, int(puerto), ri.offline))
    return sal


if __name__ == "__main__":
    t0 = time.time()
    host, puerto = zk_meta()
    print(f"1. ZooKeeper -> hbase:meta en {host}:{puerto}  ({time.time() - t0:.2f}s)")
    meta = Rpc(host, puerto)
    t1 = time.time()
    regs = regiones(meta, TABLA)
    print(f"3. hbase:meta -> {len(regs)} regiones de {TABLA}  ({time.time() - t1:.2f}s)")
    for nombre, ini, fin, h, p, off in regs:
        print(f"     {ini[:10]!r:>16} .. {fin[:10]!r:<16} {h}:{p}")
    conexiones = {}
    t2 = time.time()
    filas = celdas = 0
    for nombre, ini, fin, h, p, off in regs:
        c = conexiones.setdefault((h, p), Rpc(h, p))
        f, ce = scan(c, nombre)
        filas += f
        celdas += ce
    dt = time.time() - t2
    print(f"4. scan nativo, secuencial: {filas} filas, {celdas} celdas en {dt:.1f}s = {filas / dt:,.0f} filas/s")
    from concurrent.futures import ThreadPoolExecutor
    def una(r):
        c = Rpc(r[3], r[4])
        return scan(c, r[0])
    t3 = time.time()
    with ThreadPoolExecutor(len(regs)) as ex:
        tot = sum(f for f, _ in ex.map(una, regs))
    dt = time.time() - t3
    print(f"5. scan nativo, {len(regs)} regiones en paralelo (hilos de Python): {tot} filas en {dt:.1f}s = {tot / dt:,.0f} filas/s")
