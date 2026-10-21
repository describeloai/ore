"""X0b de 0072: negociar SASL/GSSAPI con un HBase con Kerberos desde fuera de la JVM, y leer una tabla.

    KRB5CCNAME=... python sonda_kerberos.py <carpeta_pb2> <tabla> [debug]

Usa la libreria GSSAPI del sistema (la misma que `libgssapi`/`cross-krb5` en Rust). El ticket sale de la cache
(`kinit -kt /keytabs/lector.keytab lector@ORE.TEST`).  Calidad de proteccion: la que pida el servidor
(`hbase.rpc.protection`): authentication, integrity o privacy.
"""
import socket
import struct
import sys
import time

sys.path.insert(0, sys.argv[1])
import gssapi  # noqa: E402
from google.protobuf.internal.decoder import _DecodeVarint32  # noqa: E402
from google.protobuf.internal.encoder import _VarintBytes  # noqa: E402
import Client_pb2 as C  # noqa: E402
import HBase_pb2 as H  # noqa: E402
import RPC_pb2 as R  # noqa: E402
import ZooKeeper_pb2 as Z  # noqa: E402

TABLA = sys.argv[2]
DEBUG = len(sys.argv) > 3
AUTH_SASL = 81


def zk_meta(host, puerto=2181):
    from kazoo.client import KazooClient
    zk = KazooClient(hosts=f"{host}:{puerto}")
    zk.start()
    datos, _ = zk.get("/hbase/meta-region-server")
    zk.stop()
    n = struct.unpack(">I", datos[1:5])[0]
    m = Z.MetaRegionServer()
    m.ParseFromString(datos[5 + n + 4:])
    return m.server.host_name, m.server.port


class Rpc:
    def __init__(self, host, puerto, servicio_krb="hbase"):
        self.s = socket.create_connection((host, puerto), timeout=30)
        self.id = 0
        self.ctx = None
        self.capa = 1  # 1 = nada, 2 = integridad, 4 = confidencialidad
        self.s.sendall(b"HBas" + bytes([0, AUTH_SASL]))
        self._sasl(f"{servicio_krb}@{host}")
        h = R.ConnectionHeader()
        h.user_info.effective_user = "lector"
        h.service_name = "ClientService"
        self._enviar(h.SerializeToString())

    # ---- bytes crudos
    def _leer(self, n):
        buf = bytearray()
        while len(buf) < n:
            x = self.s.recv(n - len(buf))
            if not x:
                raise EOFError("el servidor cerro la conexion")
            buf += x
        return bytes(buf)

    # ---- el apretón SASL / GSSAPI (RFC 4752)
    def _sasl(self, nombre):
        name = gssapi.Name(nombre, gssapi.NameType.hostbased_service)
        flags = gssapi.RequirementFlag.mutual_authentication | gssapi.RequirementFlag.out_of_sequence_detection \
            | gssapi.RequirementFlag.integrity | gssapi.RequirementFlag.confidentiality
        ctx = gssapi.SecurityContext(name=name, usage="initiate", flags=flags)
        token = ctx.step()
        pasos = 0
        while True:
            pasos += 1
            self.s.sendall(struct.pack(">I", len(token)) + token)
            estado = struct.unpack(">I", self._leer(4))[0]
            if DEBUG:
                print(f"   sasl paso {pasos}: estado={estado}")
            if estado != 0:
                # error: clase y mensaje como cadenas UTF de Hadoop (2 bytes de longitud)
                n = struct.unpack(">H", self._leer(2))[0]
                clase = self._leer(n).decode()
                n = struct.unpack(">H", self._leer(2))[0]
                raise RuntimeError(f"SASL rechazado: {clase}: {self._leer(n).decode()}")
            n = struct.unpack(">I", self._leer(4))[0]
            reto = self._leer(n) if n else b""
            if DEBUG:
                print(f"   sasl paso {pasos}: reto de {n} bytes; contexto completo={ctx.complete}")
            if not ctx.complete:
                token = ctx.step(reto)
                if token is None:  # el contexto acabo con este reto: falta la negociacion de capa
                    token = b""
                if ctx.complete and not token:
                    # sigue la negociacion de capa de seguridad: el servidor manda 4 bytes envueltos
                    pass
                continue
            # contexto completo: este reto es la oferta de capas de seguridad, envuelta
            oferta = ctx.unwrap(reto).message
            capas, maxbuf = oferta[0], oferta[1:4]
            if DEBUG:
                print(f"   capas ofrecidas={capas:#04x} (1 nada, 2 integridad, 4 confidencialidad), buffer={maxbuf.hex()}")
            elegida = 4 if capas & 4 else 2 if capas & 2 else 1
            self.capa = elegida
            respuesta = ctx.wrap(bytes([elegida]) + maxbuf, encrypt=False).message
            self.s.sendall(struct.pack(">I", len(respuesta)) + respuesta)
            # el servidor NO contesta a este ultimo mensaje (medido: leer aqui espera hasta el tiempo limite):
            # la negociacion acaba y lo siguiente es el ConnectionHeader.
            self.ctx = ctx
            return

    # ---- mensajes: con la capa elegida se envuelven
    def _enviar(self, cuerpo):
        if self.capa > 1:
            # con capa de seguridad, lo que se envuelve es el mensaje CON su longitud; la trama lleva la del envoltorio
            cuerpo = self.ctx.wrap(struct.pack(">I", len(cuerpo)) + cuerpo, encrypt=self.capa == 4).message
        self.s.sendall(struct.pack(">I", len(cuerpo)) + cuerpo)

    def _recibir(self):
        n = struct.unpack(">I", self._leer(4))[0]
        datos = self._leer(n)
        if self.capa > 1:
            datos = self.ctx.unwrap(datos).message[4:]
        return datos

    def llamar(self, metodo, peticion, respuesta):
        self.id += 1
        cab = R.RequestHeader(call_id=self.id, method_name=metodo, request_param=True)
        c, cu = cab.SerializeToString(), peticion.SerializeToString()
        self._enviar(_VarintBytes(len(c)) + c + _VarintBytes(len(cu)) + cu)
        datos = self._recibir()
        n, p = _DecodeVarint32(datos, 0)
        rc = R.ResponseHeader()
        rc.ParseFromString(datos[p:p + n])
        p += n
        if rc.HasField("exception"):
            raise RuntimeError(f"{rc.exception.exception_class_name}: {rc.exception.stack_trace[:160]}")
        n, p = _DecodeVarint32(datos, p)
        respuesta.ParseFromString(datos[p:p + n])
        return respuesta


def scan(rpc, region, filas=1000):
    req = C.ScanRequest()
    req.region.type = H.RegionSpecifier.REGION_NAME
    req.region.value = region
    req.scan.include_start_row = True
    req.scan.max_versions = 1
    req.number_of_rows = filas
    req.client_handles_partials = True
    req.client_handles_heartbeats = True
    n, seq = 0, 0
    r = rpc.llamar("Scan", req, C.ScanResponse())
    sid = r.scanner_id
    while True:
        n += sum(1 for x in r.results if not x.partial)
        if r.HasField("more_results_in_region") and not r.more_results_in_region:
            return n
        if not r.more_results:
            break
        r = rpc.llamar("Scan", C.ScanRequest(scanner_id=sid, number_of_rows=filas, next_call_seq=seq,
                                             client_handles_partials=True, client_handles_heartbeats=True), C.ScanResponse())
        seq += 1
    rpc.llamar("Scan", C.ScanRequest(scanner_id=sid, close_scanner=True, number_of_rows=0, next_call_seq=seq), C.ScanResponse())
    return n


def regiones(meta, tabla):
    req = C.ScanRequest()
    req.region.type = H.RegionSpecifier.REGION_NAME
    req.region.value = b"hbase:meta,,1"
    req.scan.start_row = tabla.encode() + b",,"
    req.scan.stop_row = tabla.encode() + b"-"
    req.scan.max_versions = 1
    req.number_of_rows = 100
    req.close_scanner = True
    sal = []
    for res in meta.llamar("Scan", req, C.ScanResponse()).results:
        celdas = {c.qualifier: c.value for c in res.cell}
        host, puerto = celdas[b"server"].decode().split(":")
        sal.append((res.cell[0].row, host, int(puerto)))
    return sal


if __name__ == "__main__":
    t0 = time.time()
    host, puerto = zk_meta("hb.ore.test")
    print(f"ZooKeeper -> hbase:meta en {host}:{puerto}")
    meta = Rpc(host, puerto)
    print(f"SASL/GSSAPI completo en {time.time() - t0:.2f}s; capa de seguridad elegida: "
          f"{ {1: 'nada (authentication)', 2: 'integridad', 4: 'confidencialidad (privacy)'}[meta.capa] }")
    regs = regiones(meta, TABLA)
    print(f"{TABLA}: {len(regs)} regiones")
    conexiones = {}
    t1 = time.time()
    total = 0
    for nombre, h, p in regs:
        total += scan(conexiones.setdefault((h, p), Rpc(h, p)), nombre)
    dt = time.time() - t1
    print(f"scan: {total} filas en {dt:.2f}s = {total / max(dt, 1e-9):,.0f} filas/s")
