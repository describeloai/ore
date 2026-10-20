"""Graba la conversación Avatica real contra el PQS del laboratorio (0071 X0).

    python trazas.py [http://127.0.0.1:8765/] [carpeta]

Cada traza es un JSON con los intercambios en orden: el cuerpo HTTP de la petición y el de la
respuesta (un `WireMessage` protobuf) en base64, más el estado HTTP. Son los fixtures del cliente
`ore-avatica` (M1): el servidor falso los reproduce, y los bytes son los de un PQS de verdad.
"""
import base64
import json
import os
import sys
import types

sys.modules.setdefault("gssapi", types.ModuleType("gssapi"))
_m = types.ModuleType("requests_gssapi")
_m.HTTPSPNEGOAuth = object
sys.modules.setdefault("requests_gssapi", _m)
import phoenixdb  # noqa: E402
from phoenixdb.avatica import client as avatica  # noqa: E402
from phoenixdb.avatica.proto import requests_pb2  # noqa: E402

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8765/"
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "trazas")
os.makedirs(OUT, exist_ok=True)

cinta = []
_post = avatica.AvaticaClient._post_request


def grabando(self, body):
    resp = _post(self, body)
    # `stream=True`: se lee el cuerpo aquí y se deja listo para quien venga detrás
    datos = resp.raw.read()
    cinta.append({"peticion": base64.b64encode(body).decode(), "estado": resp.status_code,
                  "respuesta": base64.b64encode(datos).decode()})
    import io
    resp.raw = io.BytesIO(datos)
    return resp


avatica.AvaticaClient._post_request = grabando


def traza(nombre, guion):
    cinta.clear()
    try:
        guion()
    except Exception as e:  # el error también es el contenido de la traza
        cinta.append({"excepcion": type(e).__name__, "args": [str(a) for a in e.args]})
    with open(os.path.join(OUT, nombre + ".json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(cinta, f, indent=1)
    print("%-28s %d intercambios" % (nombre, len(cinta)))


def abrir():
    c = phoenixdb.connect(URL, autocommit=True)
    return c, c._client, c._id


def lectura_paginada():
    c, cl, cid = abrir()
    sid = cl.create_statement(cid)
    cl.prepare_and_execute(cid, sid, "SELECT id, cliente, importe FROM ore_x0.pedidos ORDER BY id", first_frame_max_size=3)
    cl.fetch(cid, sid, offset=3, frame_max_size=3)
    cl.close_statement(cid, sid)
    c.close()


def todos_los_tipos():
    c, cl, cid = abrir()
    sid = cl.create_statement(cid)
    cl.prepare_and_execute(cid, sid, "SELECT * FROM ore_x0.tipos ORDER BY k1", first_frame_max_size=10)
    c.close()


def catalogo():
    c, cl, cid = abrir()
    cl.get_schemas(cid)
    cl.get_tables(cid, schemaPattern="ORE_X0")
    cl.get_columns(cid, schemaPattern="ORE_X0", tableNamePattern="TIPOS")
    c.close()


def error_tabla():
    c, cl, cid = abrir()
    sid = cl.create_statement(cid)
    cl.prepare_and_execute(cid, sid, "SELECT * FROM no_existe", first_frame_max_size=10)


def statement_perdido():
    c, cl, cid = abrir()
    sid = cl.create_statement(cid)
    cl.prepare_and_execute(cid, sid, "SELECT id FROM ore_x0.pedidos ORDER BY id", first_frame_max_size=3)
    cl.close_statement(cid, sid)
    req = requests_pb2.FetchRequest()
    req.connection_id, req.statement_id, req.offset, req.frame_max_size = cid, sid, 3, 3
    cl._apply(req)  # la respuesta trae missing_statement=true


def tiempo_limite():
    c = phoenixdb.connect(URL, autocommit=True, **{"phoenix.query.timeoutMs": "300"})
    cl, cid = c._client, c._id
    sid = cl.create_statement(cid)
    cl.prepare_and_execute(cid, sid, "SELECT COUNT(*) FROM ore_x0.pedidos a, ore_x0.pedidos b WHERE a.importe = b.importe", first_frame_max_size=10)


for nombre, f in [("lectura_paginada", lectura_paginada), ("todos_los_tipos", todos_los_tipos), ("catalogo", catalogo),
                  ("error_tabla", error_tabla), ("statement_perdido", statement_perdido), ("tiempo_limite", tiempo_limite)]:
    traza(nombre, f)
