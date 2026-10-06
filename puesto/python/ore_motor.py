"""**ore-motor** (ORE 0057 B4·3): el motor SQL de la celda, sin puesto.

Lo que el editor calcula en el puesto —DuckDB sobre los datasets del lago y lo
leído en vivo de un origen— para lo que pregunta **sin un puesto delante**: la
preview y los datos de una vista sin copia, la consola SQL de la ficha, `ask`.

**Un motor puro, sin identidad.** ore-serve decide todo antes de llamarlo, con la
identidad y la rama de quien pregunta: qué nombres lee la consulta, el gobierno,
la credencial de cada dataset (prestada y acotada a él) y las lecturas en vivo
(que trae él de la pasarela, con su presupuesto). Aquí sólo llega eso:

    POST /v1/calcular
      {"texto": "<sql>", "fuentes": {<nombre>: <lo que /puestos/{id}/sql da>},
       "vivas": {<tabla>: "<Arrow IPC en base64>"}, "limite": 1000}
    → 200, un flujo Arrow IPC (el resultado, cortado en `limite` filas;
      `ore-truncada: 1` si había más) · 422 {codigo, error} si no se calcula

El motor no habla con ore-serve, ni con la pasarela, ni con ningún origen: sólo
lee el lago con la credencial que la petición trae. Cada petición, en un DuckDB
nuevo; una a la vez (el SDK guarda su conexión en un global).

    ORE_MOTOR_ESCUCHA=0.0.0.0:8097 python ore_motor.py
"""
import base64
import http.server
import io
import json
import os
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ore  # noqa: E402

LIMITE_MAXIMO = 10_000
# El SDK guarda su conexión de DuckDB (y el secreto del lago) en globales: una
# petición a la vez, cada una con lo suyo.
CANDADO = threading.Lock()


class NoSeCalcula(Exception):
    def __init__(self, codigo, mensaje):
        super().__init__(mensaje)
        self.codigo, self.mensaje = codigo, mensaje


def calcular(texto, fuentes, vivas, limite):
    """El resultado de `texto` como `pyarrow.Table` (hasta `limite` filas) y si
    había más. Lo mismo que `ore.sql()` hace en un puesto, con las fuentes que
    ore-serve ya decidió y lo leído en vivo que ya trajo."""
    import duckdb
    import pyarrow as pa

    con = duckdb.connect()
    # Sólo lee: UNA sentencia, y un `select` (o un `with … select`). Lo demás
    # —crear, escribir, `attach`, `copy … to`, `set`— no se ejecuta aquí.
    try:
        sentencias = con.extract_statements(texto)
    except duckdb.Error as e:
        raise NoSeCalcula("motor/sql", str(e).splitlines()[0]) from None
    if len(sentencias) != 1 or sentencias[0].type != duckdb.StatementType.SELECT:
        raise NoSeCalcula("motor/sin-resultado", "el motor sólo lee: una sentencia `select`")
    ore._con, ore._s3 = con, None
    fuentes = dict(fuentes or {})
    fuentes.pop("__avisos", None)
    en_vivo = {}
    for nombre, rd in sorted(fuentes.items()):
        l = (rd or {}).get("federada")
        if l is None:
            continue
        t = l["tabla"]
        if t not in en_vivo:
            if t not in vivas:
                raise NoSeCalcula("motor/falta-lectura", "`%s` se lee en vivo y la petición no la trae" % t)
            tabla = pa.ipc.open_stream(io.BytesIO(base64.b64decode(vivas[t]))).read_all()
            interno = "__ore_vivo_%d" % len(en_vivo)
            con.register(interno, tabla)
            en_vivo[t] = interno
        ore._registra(con, nombre, ore._q(en_vivo[t]))
    for nombre, rd in sorted(fuentes.items()):
        rd = rd or {}
        if rd.get("federada") is not None or rd.get("vistaFederada") is not None:
            continue
        if rd.get("collection"):
            raise NoSeCalcula("motor/coleccion", "`%s` es una colección: su listado aún no llega al motor" % nombre)
        if not (rd.get("metadata_location") or rd.get("consulta")):
            raise NoSeCalcula("motor/copia-antigua", "`%s` no es un dataset de Iceberg: se lee en un puesto" % nombre)
        fuente, _ = ore._fuente_de_respuesta(nombre, rd)
        ore._registra(con, nombre, fuente)
    # Las vistas vivas, al final: DuckDB enlaza una vista al crearla, y una que
    # junta un origen con un dataset necesita los dos ya puestos.
    for nombre, rd in sorted(fuentes.items()):
        if (rd or {}).get("vistaFederada") is not None:
            ore._registra(con, nombre, "(%s)" % rd["vistaFederada"])
    # Lo del usuario no toca este contenedor: sin sistema de ficheros local y
    # con la configuración cerrada (no la puede volver a abrir un `set`). Lo
    # registrado arriba —Arrow en memoria, el lago por su credencial— sigue.
    con.execute("set disabled_filesystems = 'LocalFileSystem'")
    con.execute("set lock_configuration = true")
    try:
        r = con.execute(texto)
    except duckdb.Error as e:
        raise NoSeCalcula("motor/sql", str(e).splitlines()[0]) from None
    if r.description is None:
        raise NoSeCalcula("motor/sin-resultado", "la sentencia no devuelve filas: el motor sólo lee")
    lector = r.to_arrow_reader(8192) if hasattr(r, "to_arrow_reader") else r.fetch_record_batch(8192)
    lotes, n = [], 0
    for lote in lector:
        if n + lote.num_rows > limite:
            lotes.append(lote.slice(0, limite - n))
            return pa.Table.from_batches(lotes, schema=lector.schema), True
        lotes.append(lote)
        n += lote.num_rows
    return pa.Table.from_batches(lotes, schema=lector.schema), False


class Manejador(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # una línea propia por petición, abajo
        pass

    def _json(self, codigo, cuerpo):
        datos = json.dumps(cuerpo).encode("utf-8")
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(datos)))
        self.end_headers()
        self.wfile.write(datos)

    def do_GET(self):
        if self.path == "/salud":
            return self._json(200, {"estado": "ok"})
        return self._json(404, {"error": "no existe"})

    def do_POST(self):
        if self.path != "/v1/calcular":
            return self._json(404, {"error": "no existe"})
        t0 = time.time()
        try:
            n = int(self.headers.get("content-length") or 0)
            p = json.loads(self.rfile.read(n) or b"{}")
            texto = p.get("texto")
            if not isinstance(texto, str) or not texto.strip():
                raise NoSeCalcula("motor/peticion", "falta `texto`")
            limite = int(p.get("limite") or 1000)
            if not 1 <= limite <= LIMITE_MAXIMO:
                raise NoSeCalcula("motor/peticion", "`limite` va de 1 a %d" % LIMITE_MAXIMO)
            with CANDADO:
                tabla, truncada = calcular(texto, p.get("fuentes"), p.get("vivas") or {}, limite)
        except NoSeCalcula as e:
            print("calcular · %s · %d ms" % (e.codigo, (time.time() - t0) * 1000), flush=True)
            return self._json(422, {"codigo": e.codigo, "error": e.mensaje})
        except (ValueError, KeyError) as e:
            return self._json(400, {"codigo": "motor/peticion", "error": str(e)})
        import pyarrow as pa

        sumidero = io.BytesIO()
        with pa.ipc.new_stream(sumidero, tabla.schema) as w:
            w.write_table(tabla)
        datos = sumidero.getvalue()
        self.send_response(200)
        self.send_header("content-type", "application/vnd.apache.arrow.stream")
        self.send_header("content-length", str(len(datos)))
        self.send_header("ore-filas", str(tabla.num_rows))
        self.send_header("ore-truncada", "1" if truncada else "0")
        self.end_headers()
        self.wfile.write(datos)
        print("calcular · %d filas%s · %d ms" % (tabla.num_rows, " (cortada)" if truncada else "",
                                                 (time.time() - t0) * 1000), flush=True)


def main():
    host, _, puerto = os.environ.get("ORE_MOTOR_ESCUCHA", "127.0.0.1:8097").rpartition(":")
    s = http.server.ThreadingHTTPServer((host or "0.0.0.0", int(puerto)), Manejador)
    print("ore-motor · escucha en %s:%s" % (host, s.server_address[1]), flush=True)
    s.serve_forever()


if __name__ == "__main__":
    main()
