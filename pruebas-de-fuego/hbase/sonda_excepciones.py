"""N2 de 0072: provoca en el laboratorio los fallos que un lector tiene que saber tratar y graba, de cada uno, la
excepcion tal como llega (clase, primera linea del mensaje, `do_not_retry`).

    python sonda_excepciones.py <carpeta_pb2> > trazas/excepciones.json

Hay que tener TestTable (hbase pe) y un HBase con `hbase.client.scanner.timeout.period` corto (10 s).
"""
import json
import subprocess
import sys
import time

sys.argv = [sys.argv[0], sys.argv[1]]
import sonda_nativa as S  # noqa: E402
import Client_pb2 as C  # noqa: E402
import HBase_pb2 as H  # noqa: E402

CONTENEDOR = "hb"
sal = []


def shell(*ordenes):
    txt = "\n".join(ordenes) + "\nexit\n"
    subprocess.run(["docker", "exec", "-i", CONTENEDOR, "bash", "-c", "$HBASE_HOME/bin/hbase shell -n"],
                   input=txt, capture_output=True, text=True)


def anotar(escenario, f):
    try:
        f()
        sal.append({"escenario": escenario, "excepcion": None})
    except Exception as e:  # noqa: BLE001
        ex = getattr(e, "ex", None)
        if ex is None:
            sal.append({"escenario": escenario, "error_no_hbase": str(e)[:200]})
            return
        primera = (ex.stack_trace.splitlines() or [""])[0]
        sal.append({
            "escenario": escenario,
            "clase": ex.exception_class_name,
            "mensaje": primera.split(": ", 1)[1] if ": " in primera else primera,
            "do_not_retry": ex.do_not_retry if ex.HasField("do_not_retry") else None,
            "hostname": ex.hostname if ex.HasField("hostname") else None,
        })


host, puerto = S.zk_meta()
meta = S.Rpc(host, puerto)


def regiones(t):
    return S.regiones(meta, t)


def abrir(rpc, nombre, filas=10, ini=b"", fin=b""):
    return S.scan.__globals__["C"]  # no se usa


def peticion_scan(nombre, **kw):
    r = C.ScanRequest()
    r.region.type = H.RegionSpecifier.REGION_NAME
    r.region.value = nombre
    r.scan.include_start_row = True
    r.scan.max_versions = 1
    r.number_of_rows = kw.get("filas", 10)
    return r


# 1. una region que no existe en ese servidor
rs = S.Rpc(host, puerto)
anotar("region_inexistente", lambda: rs.llamar("Scan", peticion_scan(b"TestTable,zzz,1.deadbeefdeadbeefdeadbeefdeadbeef."), C.ScanResponse()))

# 2. un scanner que no existe
anotar("scanner_desconocido", lambda: rs.llamar("Scan", C.ScanRequest(scanner_id=424242, number_of_rows=10, next_call_seq=0), C.ScanResponse()))

# 3. numero de secuencia equivocado
nombre = regiones("TestTable")[1][0]
r = rs.llamar("Scan", peticion_scan(nombre), C.ScanResponse())
anotar("secuencia_equivocada", lambda: rs.llamar("Scan", C.ScanRequest(scanner_id=r.scanner_id, number_of_rows=10, next_call_seq=7), C.ScanResponse()))

# 4. el arriendo caduca (10 s en el laboratorio)
rs2 = S.Rpc(host, puerto)
r = rs2.llamar("Scan", peticion_scan(nombre), C.ScanResponse())
time.sleep(14)
anotar("arriendo_caducado", lambda: rs2.llamar("Scan", C.ScanRequest(scanner_id=r.scanner_id, number_of_rows=10, next_call_seq=0), C.ScanResponse()))

# 5. clave fuera de la region (Get)
def get_fuera():
    g = C.GetRequest()
    g.region.type = H.RegionSpecifier.REGION_NAME
    g.region.value = nombre
    g.get.row = b"zzz"
    rs.llamar("Get", g, C.GetResponse())
anotar("clave_fuera_de_la_region", get_fuera)

# 6. una tabla deshabilitada
shell("create 'caos1', 'f'", "put 'caos1','a','f:x','1'", "put 'caos1','b','f:x','2'")
n_caos1 = regiones("caos1")[0][0]
shell("disable 'caos1'")
anotar("tabla_deshabilitada_scan_a_la_region", lambda: rs.llamar("Scan", peticion_scan(n_caos1), C.ScanResponse()))

# 7. una region partida: el nombre de la region madre ya no se sirve
shell("create 'caos2', 'f'", *[f"put 'caos2','{c}','f:x','1'" for c in "abcdefgh"])
n_caos2 = regiones("caos2")[0][0]
shell("flush 'caos2'", "split 'caos2', 'e'")
time.sleep(8)
anotar("region_partida_scan_a_la_madre", lambda: rs.llamar("Scan", peticion_scan(n_caos2), C.ScanResponse()))

json.dump(sal, sys.stdout, indent=1, ensure_ascii=False)
