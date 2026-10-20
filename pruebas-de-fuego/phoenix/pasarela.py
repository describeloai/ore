"""El conector de Phoenix, como binario y por la pasarela de verdad (0071 M2).

    PHOENIX_URL=phoenix://127.0.0.1:8765 \\
    ORE_READ_PHOENIX=C:/ore-target/release/ore-read-phoenix.exe \\
    ORE_FEDERATION=C:/ore-target/release/ore-federation.exe \\
    python pruebas-de-fuego/phoenix/pasarela.py

Necesita el laboratorio (`docker run -d --name phx -p 8765:8765 ore-phoenix-lab` y `semilla.py`) y `pyarrow`.
Lo que comprueba no se fía del conector: abre el flujo Arrow que sale con `pyarrow` y cuenta.
"""
import http.client
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time

import pyarrow.ipc as ipc

URL = os.environ.get("PHOENIX_URL", "phoenix://127.0.0.1:8765")
LECTOR = os.environ["ORE_READ_PHOENIX"]
PASARELA = os.environ["ORE_FEDERATION"]
SECRETO = "Pw-xq7-secreta-4821"

ok = mal = 0


def comprobar(nombre, cond, detalle=""):
    global ok, mal
    if cond:
        ok += 1
        print(f"  ✓ {nombre}")
    else:
        mal += 1
        print(f"  ✗ {nombre}  {detalle}")


def peticion(objeto, proy, **extra):
    p = {"url": URL, "objeto": objeto, "proyeccion": proy, "formato": "arrow"}
    p.update(extra)
    return json.dumps(p)


def directo(verbo, entrada):
    """Un verbo del binario: (código de salida, stdout en bytes, stderr en texto)."""
    r = subprocess.run([LECTOR, verbo], input=entrada.encode(), capture_output=True, timeout=120)
    return r.returncode, r.stdout, r.stderr.decode("utf-8", "replace")


def arrow(datos):
    t = ipc.open_stream(datos).read_all()
    return t


print("== 1 · el binario, directo")
c, out, err = directo("capacidades", "")
cap = json.loads(out)
comprobar("capacidades: ore-read-phoenix, sin operadores, con limit y servir",
          cap["conector"] == "ore-read-phoenix" and cap["operadores"] == [] and cap["limit"] and cap["servir"] and not cap["estimar"],
          out.decode())

t0 = time.time()
c, out, err = directo("leer", peticion("ORE_X0.PEDIDOS", {"id": "ID", "cliente": "CLIENTE", "importe": "IMPORTE", "estado": "ESTADO"}))
t = arrow(out)
comprobar("leer 262 144 filas", c == 0 and t.num_rows == 262144, f"código {c}, {err[:200]}")
comprobar("la proyección es cerrada: id, cliente, importe, estado", t.schema.names == ["id", "cliente", "importe", "estado"], str(t.schema.names))
comprobar("importe es decimal128(12, 2)", str(t.schema.field("importe").type) == "decimal128(12, 2)", str(t.schema.field("importe").type))
comprobar("el tipo de OOS viaja en los metadatos", t.schema.field("importe").metadata.get(b"ore:tipo") == b"Decimal<12, 2>")
ids = t.column("id").to_pylist()
comprobar("ni una fila de más ni de menos, y en orden", sum(ids) == 262144 * 262145 // 2 and ids == sorted(ids))
print(f"    {t.num_rows} filas en {time.time() - t0:.1f}s por el binario")

c, out, err = directo("leer", peticion("ORE_X0.SALADA", {"id": "ID"}, limit=5))
comprobar("limit 5 se empuja y llegan 5", c == 0 and arrow(out).num_rows == 5, err[:200])

c, out, err = directo("leer", peticion("ore_x0.salada", {"id": "ID", "v": "V"}))
comprobar("un nombre en minúsculas se resuelve como Phoenix", c == 0 and arrow(out).num_rows == 40, err[:200])

print("== 2 · lo que se rechaza, con su código")
def fallo(err):
    # stderr: `ore-read-phoenix: {"codigo":…}` (una línea JSON)
    try:
        return json.loads(err.strip().split("ore-read-phoenix: ", 1)[1])
    except Exception:
        return {}

c, out, err = directo("leer", peticion("ORE_X0.PEDIDOS", {"id": "ID"}, filtros=[{"columna": "ID", "operador": "eq", "valor": "1"}]))
comprobar("un filtro se rechaza con `operador` y no sale ni un byte de datos", c != 0 and fallo(err).get("codigo") == "operador" and not out, err[:200])
c, out, err = directo("leer", peticion("ORE_X0.NO_EXISTE", {"a": "A"}))
comprobar("una tabla que no existe es `objeto`", fallo(err).get("codigo") == "objeto", err[:200])
c, out, err = directo("leer", peticion("ORE_X0.PEDIDOS", {"x": "NO_EXISTE"}))
comprobar("una columna que no existe es `esquema`", fallo(err).get("codigo") == "esquema", err[:200])
c, out, err = directo("leer", json.dumps({"url": "phoenix://127.0.0.1:1", "objeto": "T", "proyeccion": {"a": "A"}}))
comprobar("un servidor que no está es `conexion` y reintentable", fallo(err).get("codigo") == "conexion" and fallo(err).get("reintentable") is True, err[:200])
c, out, err = directo("leer", json.dumps({"url": f"phoenix://ana:{SECRETO}@127.0.0.1:1?tls=true&verify=false", "objeto": "T", "proyeccion": {"a": "A"}}))
comprobar("verify=false se rechaza y la contraseña no sale", c != 0 and SECRETO not in err, err[:200])
c, out, err = directo("leer", json.dumps({"url": f"phoenix://ana:{SECRETO}@127.0.0.1:8765", "objeto": "T", "proyeccion": {"a": "A"}}))
comprobar("una contraseña sin TLS se rechaza y no sale", c != 0 and SECRETO not in err and "tls=true" in err, err[:200])

print("== 3 · los otros verbos")
c, out, err = directo("check", json.dumps({"url": URL + "/ORE_X0"}))
comprobar("check con esquema: ok", json.loads(out).get("ok") is True, out.decode())
c, out, err = directo("check", json.dumps({"url": URL + "/NO_EXISTE"}))
comprobar("check de un esquema que no existe: no ok, con su causa", json.loads(out).get("ok") is False and "NO_EXISTE" in out.decode(), out.decode())
c, out, err = directo("testigo", json.dumps({"url": URL}))
comprobar("testigo: none", json.loads(out).get("modo") == "none", out.decode())
c, out, err = directo("cambio", json.dumps({"url": URL, "objeto": "ORE_X0.PEDIDOS"}))
comprobar("cambio: no respalda", json.loads(out).get("respalda") is False, out.decode())
c, out, err = directo("catalogo", URL)
comprobar("catalogo: dice que es de M3 y no se inventa un catálogo", c != 0 and "M3" in err and not out, err[:200])

print("== 4 · la pasarela")
carpeta = tempfile.mkdtemp(prefix="ore-conectores-")
shutil.copy(LECTOR, os.path.join(carpeta, "ore-read-phoenix" + (".exe" if LECTOR.endswith(".exe") else "")))
puerto = 18099
g = subprocess.Popen([PASARELA, "--escucha", f"127.0.0.1:{puerto}", "--conectores", carpeta],
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def http_(metodo, ruta, cuerpo=None):
    h = http.client.HTTPConnection("127.0.0.1", puerto, timeout=180)
    h.request(metodo, ruta, body=cuerpo, headers={"Content-Type": "application/json"} if cuerpo else {})
    r = h.getresponse()
    datos = r.read()
    h.close()
    return r.status, datos


try:
    for _ in range(100):
        try:
            if http_("GET", "/v1/health")[0] == 200:
                break
        except Exception:
            time.sleep(0.2)
    else:
        sys.exit("la pasarela no quedó sana")
    comprobar("la pasarela quedó sana", True)
    s, d = http_("GET", "/v1/connectors")
    cs = json.loads(d)
    comprobar("descubrió `phoenix` sin que nadie lo nombrara", "phoenix" in json.dumps(cs), d.decode()[:300])

    GRANDE = {"filas": 10_000_000, "bytes": 8 * 2**30, "ms": 600_000}

    def leer_pasarela(id_, objeto, proy, presupuesto=GRANDE, **extra):
        cuerpo = {"id": id_, "origen": "prueba", "tipo": "phoenix", "url": URL,
                  "peticion": dict({"objeto": objeto, "proyeccion": proy}, **extra)}
        if presupuesto:
            cuerpo["presupuesto"] = presupuesto
        return http_("POST", "/v1/read", json.dumps(cuerpo))

    # El presupuesto de la pasarela manda sobre el conector: con el de por defecto (100 000 filas) corta una
    # lectura de 262 144, y lo dice (`cortado`, motivo `filas`). Es la pasarela, no el conector.
    s, d = leer_pasarela("fx-0", "ORE_X0.PEDIDOS", {"id": "ID", "cliente": "CLIENTE"}, presupuesto=None)
    s0, e0 = http_("GET", "/v1/read/fx-0")
    comprobar("el presupuesto por defecto de la pasarela corta a las 100 000 filas", s == 200 and arrow(d).num_rows == 100000 and b"cortado" in e0 and b'"motivo":"filas"' in e0, f"{s} {e0[:200]!r}")

    t0 = time.time()
    s, d = leer_pasarela("fx-1", "ORE_X0.PEDIDOS", {"id": "ID", "cliente": "CLIENTE", "importe": "IMPORTE"})
    t = arrow(d)
    comprobar("POST /v1/read: 262 144 filas por la pasarela", s == 200 and t.num_rows == 262144, f"{s} {d[:200]!r}")
    print(f"    {t.num_rows} filas en {time.time() - t0:.1f}s por la pasarela")
    s, d = http_("GET", "/v1/read/fx-1")
    comprobar("GET /v1/read/{id}: cómo terminó", s == 200 and b"completo" in d and b"\"filas\":262144" in d.replace(b" ", b""), d.decode()[:200])

    s, d = leer_pasarela("fx-2", "ORE_X0.SALADA", {"id": "ID"}, limit=3)
    comprobar("limit 3 por la pasarela", s == 200 and arrow(d).num_rows == 3, f"{s} {d[:200]!r}")

    # Lo que el conector no declaró no llega al conector: la pasarela lo rechaza.
    s, d = leer_pasarela("fx-3", "ORE_X0.PEDIDOS", {"id": "ID"}, filtros=[{"columna": "ID", "operador": "eq", "valor": "1"}])
    comprobar("un filtro por la pasarela se rechaza (no es 200)", s != 200, f"{s} {d[:200]!r}")

    # Cuatro a la vez: cada una su sesión caliente o su cola.
    res = {}
    def una(i):
        s_, d_ = leer_pasarela(f"fx-par-{i}", "ORE_X0.PEDIDOS", {"id": "ID", "cliente": "CLIENTE"})
        res[i] = (s_, arrow(d_).num_rows if s_ == 200 else -1)
    hilos = [threading.Thread(target=una, args=(i,)) for i in range(4)]
    t0 = time.time()
    [h.start() for h in hilos]
    [h.join() for h in hilos]
    comprobar("cuatro lecturas a la vez, todas enteras", all(v == (200, 262144) for v in res.values()), str(res))
    print(f"    4 × 262 144 filas en {time.time() - t0:.1f}s")

    # Cancelar a mitad: la lectura acaba pronto, y la pasarela sigue sirviendo.
    resultado = {}
    def larga():
        resultado["r"] = leer_pasarela("fx-cancel", "ORE_X0.PEDIDOS", {"id": "ID", "cliente": "CLIENTE", "estado": "ESTADO"})
    h = threading.Thread(target=larga)
    t0 = time.time()
    h.start()
    time.sleep(0.8)
    s, d = http_("DELETE", "/v1/read/fx-cancel")
    h.join(30)
    comprobar("DELETE /v1/read/{id} corta la lectura antes de terminarla", not h.is_alive() and time.time() - t0 < 6, f"{time.time() - t0:.1f}s")
    s, d = leer_pasarela("fx-4", "ORE_X0.SALADA", {"id": "ID"})
    comprobar("y la pasarela sigue sirviendo después", s == 200 and arrow(d).num_rows == 40, f"{s} {d[:200]!r}")

    # Los otros verbos por la pasarela, sin la credencial en ninguna respuesta.
    cuerpo = json.dumps({"origen": "prueba", "tipo": "phoenix", "url": URL + "/ORE_X0"})
    s, d = http_("POST", "/v1/check", cuerpo)
    comprobar("POST /v1/check: ok", s == 200 and b'"ok":true' in d.replace(b" ", b""), f"{s} {d[:200]!r}")
    cuerpo = json.dumps({"origen": "prueba", "tipo": "phoenix", "url": f"phoenix://ana:{SECRETO}@127.0.0.1:1?tls=true"})
    s, d = http_("POST", "/v1/check", cuerpo)
    comprobar("POST /v1/check con un servidor que no está: sin la contraseña", SECRETO.encode() not in d, d.decode()[:200])
    s, d = leer_pasarela("fx-5", "T", {"a": "A"})
    s2, d2 = http_("POST", "/v1/read", json.dumps({"id": "fx-6", "origen": "prueba", "tipo": "phoenix", "url": f"phoenix://ana:{SECRETO}@127.0.0.1:1?tls=true",
                                                  "peticion": {"objeto": "T", "proyeccion": {"a": "A"}}}))
    s3, d3 = http_("GET", "/v1/read/fx-6")
    s4, d4 = http_("GET", "/v1/origins")
    comprobar("ni el registro de la pasarela ni /v1/origins llevan la contraseña", SECRETO.encode() not in d2 + d3 + d4, "")
    comprobar("/v1/origins cuenta las lecturas del origen", b'"origen":"prueba"' in d4 and b'"lecturas"' in d4, d4.decode()[:300])
finally:
    g.terminate()
    try:
        g.wait(10)
    except Exception:
        g.kill()
    errores = g.stderr.read().decode("utf-8", "replace")
    comprobar("la contraseña no está en lo que escribió la pasarela", SECRETO not in errores)
    shutil.rmtree(carpeta, ignore_errors=True)

print(f"\n{ok} ✓ · {mal} ✗")
sys.exit(1 if mal else 0)
