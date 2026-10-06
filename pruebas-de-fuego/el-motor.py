"""EL MOTOR (ORE 0057 B4·3·1): `ore-motor` calcula lo que ore-serve le da, sin
identidad y sin hablar con nadie.

  1  una consulta sin fuentes: DuckDB solo
  2  una foreign table leída en vivo (su Arrow, en la petición) por el nombre
     que la consulta usa, con un filtro que hace el motor
  3  una foreign view con junta y agregado sobre dos lecturas en vivo
  4  `limite`: el resultado se corta y la respuesta lo dice (`ore-truncada`)
  5  lo que no se calcula se dice con su código, sin calcular a medias: una
     lectura que la petición no trae, una sentencia que escribe o dos, un
     fichero del contenedor, SQL roto
  6  dos peticiones a la vez: cada una con lo suyo

    PYTHONUTF8=1 python pruebas-de-fuego/el-motor.py
"""
import base64
import http.server
import io
import json
import os
import sys
import threading
import urllib.request

import pyarrow as pa

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))
import ore_motor  # noqa: E402

s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ore_motor.Manejador)
threading.Thread(target=s.serve_forever, daemon=True).start()
BASE = "http://127.0.0.1:%d" % s.server_address[1]
MAL = []


def arrow(t):
    b = io.BytesIO()
    with pa.ipc.new_stream(b, t.schema) as w:
        w.write_table(t)
    return base64.b64encode(b.getvalue()).decode()


CLIENTES = pa.table({"id": pa.array([1, 2, 3, 4, 5], pa.int64()), "pais": ["ES", "PT", "ES", "FR", "ES"]})
PEDIDOS = pa.table({"id": pa.array(range(1, 7), pa.int64()), "cliente": pa.array([1, 1, 2, 3, 4, 5], pa.int64()),
                    "importe": [10.0, 20.0, 5.0, 7.5, 12.0, 8.0]})
FUENTES = {
    "vivo.datos.clientes": {"federada": {"tabla": "s3.datos.clientes"}},
    "vivo.datos.pedidos": {"federada": {"tabla": "s3.datos.pedidos"}},
    "vivo.informes.por_pais": {"vistaFederada":
                               "SELECT c.pais, count(*) AS n, sum(p.importe) AS total FROM vivo.datos.pedidos p "
                               "JOIN vivo.datos.clientes c ON p.cliente = c.id GROUP BY c.pais"},
}
VIVAS = {"s3.datos.clientes": arrow(CLIENTES), "s3.datos.pedidos": arrow(PEDIDOS)}


def calcula(texto, fuentes=None, vivas=None, limite=100):
    cuerpo = json.dumps({"texto": texto, "fuentes": fuentes or {}, "vivas": vivas or {}, "limite": limite}).encode()
    req = urllib.request.Request(BASE + "/v1/calcular", data=cuerpo, method="POST",
                                 headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            t = pa.ipc.open_stream(io.BytesIO(r.read())).read_all()
            return 200, t, r.headers.get("ore-truncada")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}"), None


def caso(nombre, bien, detalle=""):
    print(("  ✓ " if bien else "  ✗ ") + nombre + ("" if bien else " · %s" % (detalle,)))
    if not bien:
        MAL.append(nombre)


c, t, _ = calcula("select 41 + 1 as x")
caso("1 · sin fuentes, DuckDB solo", c == 200 and t.to_pylist() == [{"x": 42}], (c, t))

c, t, _ = calcula("select id from vivo.datos.clientes where pais = 'ES' order by id", FUENTES, VIVAS)
caso("2 · una foreign table en vivo, por su nombre, filtrada en el motor",
     c == 200 and [r["id"] for r in t.to_pylist()] == [1, 3, 5], (c, t))

c, t, _ = calcula("select * from vivo.informes.por_pais order by pais", FUENTES, VIVAS)
caso("3 · una foreign view con junta y agregado",
     c == 200 and t.to_pylist() == [{"pais": "ES", "n": 4, "total": 45.5}, {"pais": "FR", "n": 1, "total": 12.0},
                                    {"pais": "PT", "n": 1, "total": 5.0}], (c, t))

c, t, tr = calcula("select * from vivo.datos.pedidos", FUENTES, VIVAS, limite=4)
caso("4 · limite: 4 filas y la respuesta dice que había más", c == 200 and t.num_rows == 4 and tr == "1", (c, tr))

c, d, _ = calcula("select * from vivo.datos.clientes", FUENTES, {})
caso("5 · una lectura que no llega: 422 motor/falta-lectura", c == 422 and d.get("codigo") == "motor/falta-lectura", (c, d))
c, d, _ = calcula("create table x as select 1")
caso("5 · una sentencia sin filas: 422 motor/sin-resultado", c == 422 and d.get("codigo") == "motor/sin-resultado", (c, d))
c, d, _ = calcula("select 1; select 2")
caso("5 · dos sentencias: 422", c == 422 and d.get("codigo") == "motor/sin-resultado", (c, d))
c, d, _ = calcula("copy (select 1) to '/tmp/x.csv'")
caso("5 · copy … to (escribe un fichero): 422", c == 422 and d.get("codigo") == "motor/sin-resultado", (c, d))
c, d, _ = calcula("select * from read_csv('/etc/passwd')")
caso("5 · leer un fichero del contenedor: 422", c == 422 and d.get("codigo") == "motor/sql", (c, d))
c, d, _ = calcula("select * from nada")
caso("5 · SQL que no corre: 422 motor/sql", c == 422 and d.get("codigo") == "motor/sql", (c, d))

res = {}


def hilo(k, q):
    res[k] = calcula(q, FUENTES, VIVAS)


hs = [threading.Thread(target=hilo, args=(k, "select count(*) n from vivo.datos.%s" % k)) for k in ("clientes", "pedidos")]
[h.start() for h in hs]
[h.join() for h in hs]
caso("6 · dos a la vez, cada una lo suyo",
     res["clientes"][1].to_pylist() == [{"n": 5}] and res["pedidos"][1].to_pylist() == [{"n": 6}], res)

print()
if MAL:
    print("✗ el motor: " + " · ".join(MAL))
    sys.exit(1)
print("✓ el motor (0057 B4·3·1): calcula lo que le dan, sin identidad")
