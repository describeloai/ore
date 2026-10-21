"""X0 de HBase, ruta Thrift2: scan completo de TestTable.  python sonda_thrift2.py <hbase.thrift> [caching]

El IDL sale del jar del propio HBase (hbase-thrift-2.4.18.jar, org/apache/hadoop/hbase/thrift2/hbase.thrift).
"""
import sys
import time

import thriftpy2
from thriftpy2.rpc import make_client

idl = thriftpy2.load(sys.argv[1], module_name="hbase_thrift")
caching = int(sys.argv[2]) if len(sys.argv) > 2 else 1000
c = make_client(idl.THBaseService, "127.0.0.1", 9090, timeout=120_000)
t0 = time.time()
sid = c.openScanner(b"TestTable", idl.TScan(caching=caching, maxVersions=1))
n = 0
while True:
    filas = c.getScannerRows(sid, caching)
    if not filas:
        break
    n += len(filas)
c.closeScanner(sid)
dt = time.time() - t0
print(f"thrift2 caching={caching}: {n} filas en {dt:.1f}s = {n / dt:,.0f} filas/s")
