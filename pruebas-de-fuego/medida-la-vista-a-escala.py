"""UNA VIEW A ESCALA · el punto de referencia antes de arreglar las Views.

`medida-la-vista-con-filtro.py` midio que leer una View desde un puesto NO
aplica su `where` ni sus `fields`: `datos` da el puntero del dataset raiz y el
SDK hace `select *` sobre el. El arreglo propuesto es que la View llegue a
DuckDB como SQL sobre su dataset (`select id, pais from <raiz> where pais =
'ES'`). Antes de construirlo se mide, con un dataset grande, lo que cuesta HOY
y lo que costaria con el SQL de la View —escrito a mano aqui, porque todavia no
lo escribe nadie—:

  §1  EL SDK       `over()` de la View (lo que devuelve, y lo que cuesta) y
                   `sql()` con un `count(*)` sobre ella; antes del arreglo era el
                   dataset entero, y el numero de entonces se imprime al lado
  §2  EL ARREGLO   el mismo `iceberg_scan` que el SDK arma, con la proyeccion
                   y el filtro de la View encima
  §3  ORDENADO     lo mismo sobre una copia de los datos ordenada por `pais`:
                   ¿aparece el recorte por filas (las estadisticas de Parquet)?

Por caso: filas y columnas, segundos, peticiones y BYTES servidos por el S3, y
la memoria pico del proceso. Cada caso corre en un proceso aparte (sin cache de
DuckDB entre casos, y con su propio pico).

    python pruebas-de-fuego/medida-la-vista-a-escala.py [--filas 20000000]

El banco es el de `medida-el-catalogo-como-resolutor.py` (ore-serve de verdad,
S3 de mentira, puesto reclamado), con un contador del S3 que cuenta bytes. No
toca el cluster.
"""
import importlib.util
import json
import os
import subprocess
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
if "--filas" not in sys.argv:
    sys.argv += ["--filas", "20000000"]
sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
m = importlib.util.module_from_spec(sp)
sp.loader.exec_module(m)

# El contador del banco, que ademas suma los bytes que sirve cada GET.
m.CONTADOR_S3 = r'''
import importlib.util, sys, threading
REG = sys.argv[3]; L = threading.Lock()
sp = importlib.util.spec_from_file_location("dm", sys.argv[1]); dm = importlib.util.module_from_spec(sp)
sys.argv = [sys.argv[0], "s3", "0"]
sp.loader.exec_module(dm)
class Cuenta:
    def __init__(self, w): self.w, self.n = w, 0
    def write(self, b): self.n += len(b); return self.w.write(b)
    def flush(self): return self.w.flush()
    def __getattr__(self, k): return getattr(self.w, k)
class C(dm.S3):
    def _n(self, n=0):
        with L:
            with open(REG, "a") as f:
                f.write("%s %s %d\n" % (self.command, self.path.split("?")[0], n))
    def do_GET(self):
        c = Cuenta(self.wfile); self.wfile = c
        try: dm.S3.do_GET(self)
        finally: self.wfile = c.w; self._n(c.n)
    def do_HEAD(self): self._n(); dm.S3.do_HEAD(self)
    def do_PUT(self): self._n(); dm.S3.do_PUT(self)
    def do_POST(self): self._n(); dm.S3.do_POST(self)
    def do_DELETE(self): self._n(); dm.S3.do_DELETE(self)
s = dm.ThreadingHTTPServer(("127.0.0.1", 0), C)
print("listo %d" % s.server_address[1], flush=True)
s.serve_forever()
'''

VISTA = """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: %s, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.%s }
  where: { pais: ES }
  fields: { id: id, pais: pais }
"""

# Un caso en un proceso propio: el SDK de verdad con el testigo del agente.
CASO = r'''
import json, os, sys, time, psutil
sys.path.insert(0, sys.argv[1])
import ore
ore.puesto.servidor, ore.puesto.id = os.environ["ORE_SERVE"], os.environ["PUESTO"]
ore.puesto._cabeceras = {"x-ore-sujeto": "agente:local"}
modo, nombre, raiz = sys.argv[2], sys.argv[3], sys.argv[4]
p = psutil.Process()
con = ore._duckdb()
con.execute("set enable_external_file_cache = false")
t = time.perf_counter()
if modo == "over":
    r = ore.over(nombre, como="arrow")
elif modo == "count":
    r = ore.sql("select count(*) as n from %s" % nombre, como="arrow")
else:
    fuente, _ = ore._fuente_de(raiz)
    q = 'select "id" as "id", "pais" as "pais" from %s where "pais" = \'ES\'' % fuente
    if modo == "arreglo_count":
        q = "select count(*) as n from (%s)" % q
    r = ore._arrow(con.sql(q))
s = time.perf_counter() - t
pico = getattr(p.memory_info(), "peak_wset", None) or p.memory_info().rss
print(json.dumps({"filas": r.num_rows if "count" not in modo else r.column(0)[0].as_py(),
                  "columnas": r.column_names, "s": s, "pico_mb": pico / 1e6}))
'''


def correr(b, modo, nombre, raiz):
    i = b.s3reg.marca()
    f = b.tmp + "/caso.py"
    m.escribir(f, CASO)
    env = dict(os.environ, ORE_SERVE=b.directo, PUESTO=m.PUESTO, ORE_ALMACEN="dir:" + b.tmp,
               ORE_MEMORIA_MB="8192")
    p = subprocess.run([m.PY, f, m.RAIZ + "/puesto/python", modo, nombre, raiz],
                       capture_output=True, text=True, env=env, encoding="utf-8", errors="replace")
    if p.returncode != 0:
        return {"error": (p.stderr or p.stdout).strip().splitlines()[-1][:200]}
    r = json.loads(p.stdout.strip().splitlines()[-1])
    gets = [l.split() for l in b.s3reg.desde(i) if l.startswith("GET ")]
    r["peticiones"] = len(gets)
    r["mb_s3"] = sum(int(g[2]) for g in gets if len(g) > 2) / 1e6
    return r


def linea(nombre, r):
    if "error" in r:
        print("     %-34s ✗ %s" % (nombre, r["error"]))
        return
    cols = "%d col" % len(r["columnas"]) if isinstance(r.get("columnas"), list) else ""
    print("     %-34s %11s %-6s %7.2f s %5d pet. %9.1f MB del S3 %8.0f MB pico"
          % (nombre, "{:,}".format(r["filas"]).replace(",", "."), cols, r["s"], r["peticiones"], r["mb_s3"], r["pico_mb"]))


def main():
    print("=== una View a escala  (%s)" % time.strftime("%Y-%m-%d"))
    b = m.Banco()
    try:
        b.levantar()
        # la copia ORDENADA por pais: mismas filas, otra disposicion en Parquet
        import pyarrow.compute as pc
        from pyiceberg.catalog import load_catalog

        cat = load_catalog("ore", **{"type": "rest", "uri": b.directo, "header.x-ore-sujeto": "persona:ana"})
        t0 = time.perf_counter()
        v = cat.load_table(("hr", "ventas")).scan().to_arrow()
        ordenada = v.take(pc.sort_indices(v, sort_keys=[("pais", "ascending")]))
        cat.create_table(("hr", "ventas_ord"), schema=ordenada.schema).append(ordenada)
        es = pc.sum(pc.equal(v.column("pais"), "ES")).as_py()
        del v, ordenada
        m.fila("hr.ventas_ord (la misma, ordenada)", "%.0f ms" % m.ms(t0), "")
        m.escribir(b.A + "/packages/hr/views/ventasES.yaml", VISTA % ("ventasES", "ventas"))
        m.escribir(b.A + "/packages/hr/views/ventasOrdES.yaml", VISTA % ("ventasOrdES", "ventas_ord"))
        print()
        print("  %s filas en hr.ventas (4 columnas); pais=ES: %s. La View: where {pais: ES}, fields {id, pais}"
              % ("{:,}".format(m.FILAS).replace(",", "."), "{:,}".format(es).replace(",", ".")))
        print("     %-34s %11s %-6s %9s %10s %22s %15s" % ("", "filas", "", "tiempo", "", "", ""))
        print()
        print("  §1 EL SDK (over() y sql() de la View, por `datos`)")
        print("     antes del arreglo, 2026-09-24, 20 M: over() 20.000.000 filas · 4 col · 1,58 s · 112,5 MB del S3 · 1425 MB pico")
        linea("over(hr.ventasES)", correr(b, "over", "hr.ventasES", "hr.ventas"))
        linea("sql(count(*) from hr.ventasES)", correr(b, "count", "hr.ventasES", "hr.ventas"))
        print("  §2 EL ARREGLO (el SQL de la View sobre su dataset)")
        linea("select id, pais … where ES", correr(b, "arreglo", "", "hr.ventas"))
        linea("count(*) de eso", correr(b, "arreglo_count", "", "hr.ventas"))
        print("  §3 ORDENADO por pais (el recorte por filas, si lo hay)")
        linea("hoy: over(hr.ventasOrdES)", correr(b, "over", "hr.ventasOrdES", "hr.ventas_ord"))
        linea("arreglo: select … where ES", correr(b, "arreglo", "", "hr.ventas_ord"))
        linea("arreglo: count(*)", correr(b, "arreglo_count", "", "hr.ventas_ord"))
    finally:
        b.cerrar()
    return 0


if __name__ == "__main__":
    sys.exit(main())
