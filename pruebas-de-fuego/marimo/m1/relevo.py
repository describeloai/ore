"""M1·a · EL RELEVO DE LABORATORIO — el papel que hará ore-serve.

El navegador nunca llega al pod (deny-all-ingress, y el puesto no escucha):
llega aquí, y esto se lo pasa al pod por la conexión que EL POD abrió hacia
fuera (`/tunel`). Lo que vuelve —una página, un JSON, un flujo SSE que no
acaba— se devuelve trozo a trozo, tal como llega.

Marco de cada mensaje por el túnel (binario): 4 bytes con la longitud de una
cabecera JSON, la cabecera, y los bytes del cuerpo si los hay.
  relevo → pod:  {t: req, id, method, path, headers} + cuerpo · {t: cancel, id}
  pod → relevo:  {t: head, id, status, headers} · {t: chunk, id} + bytes · {t: end, id}

Lo que ya hace como lo hará ore-serve (D7, D4 de 0064): niega lo que no sale
del puesto (paquetes, terminal, MCP, IA) y anota cada guardado. Desechable:
sin autenticación, para un `kubectl port-forward`.
"""
import asyncio
import json
import os
import struct
import time

import uvicorn
from starlette.applications import Starlette
from starlette.responses import JSONResponse, RedirectResponse, StreamingResponse
from starlette.routing import Route, WebSocketRoute

BASE = os.environ.get("M1_BASE", "/m")
# M1·b: detrás de la puerta pública el relevo vive bajo un prefijo y exige un token
# (cookie `m1lab`, o la cabecera `x-m1-token` para los guiones). Sin M1_TOKEN, abierto
# (M1·a, por `port-forward`).
PREFIJO = os.environ.get("M1_PREFIJO", "")
TOKEN = os.environ.get("M1_TOKEN", "")
# Lo que propone el diseño (0064 M1·a): los ficheros del frontend llevan hash en el
# nombre y son iguales para todos los puestos de una versión → inmutables.
INMUTABLES = os.environ.get("M1_INMUTABLES", "1") == "1"
# M1·b: cortar los SSE a los N s, sin `close`, como hace la puerta (requestTimeout 300 s),
# para ver qué hace el frontend de marimo cuando se le corta el flujo.
CORTE_SSE = float(os.environ.get("M1_CORTE_SSE", "0") or 0)
NEGADAS = ("/api/packages", "/api/kernel/install_missing_packages", "/terminal", "/mcp", "/api/ai", "/lsp")
SALTO = {"host", "connection", "keep-alive", "transfer-encoding", "upgrade", "proxy-connection", "te", "trailer"}

estado = {"tunel": None, "seq": 0, "conectado": None, "reconexiones": 0}
pendientes = {}
candado = asyncio.Lock()
REGISTRO = []   # una fila por petición: método, ruta, estado, ms al primer byte, ms total, bytes
GUARDADOS = []  # cada POST de guardar: cuándo y cuánto
NEGADOS = []


def marco(cab, datos=b""):
    j = json.dumps(cab).encode()
    return struct.pack(">I", len(j)) + j + datos


def abrir(b):
    n = struct.unpack(">I", b[:4])[0]
    return json.loads(b[4:4 + n]), b[4 + n:]


async def enviar(cab, datos=b""):
    ws = estado["tunel"]
    if ws is None:
        raise ConnectionError("sin túnel")
    async with candado:
        await ws.send_bytes(marco(cab, datos))


async def tunel(ws):
    await ws.accept()
    if estado["tunel"] is not None:
        estado["reconexiones"] += 1
    estado["tunel"], estado["conectado"] = ws, time.time()
    try:
        while True:
            cab, datos = abrir(await ws.receive_bytes())
            q = pendientes.get(cab.get("id"))
            if q is not None:
                cab["_datos"] = datos
                q.put_nowait(cab)
    except Exception:  # noqa: BLE001 — el pod se fue
        pass
    finally:
        if estado["tunel"] is ws:
            estado["tunel"] = None
        for q in list(pendientes.values()):
            q.put_nowait({"t": "end", "error": "el túnel se cayó"})


def autorizado(request):
    # `kubectl port-forward` entra por el propio pod (127.0.0.1): no pasa por la puerta.
    if not TOKEN or (request.client and request.client.host == "127.0.0.1"):
        return True
    return request.cookies.get("m1lab") == TOKEN or request.headers.get("x-m1-token") == TOKEN


async def entrar(request):
    if not TOKEN or request.query_params.get("t") != TOKEN:
        return JSONResponse({"error": "no"}, status_code=403)
    r = RedirectResponse(BASE + "/?file=" + request.query_params.get("file", "cuaderno.py"), status_code=303)
    r.set_cookie("m1lab", TOKEN, path=PREFIJO or "/", httponly=True, secure=True, samesite="lax")
    return r


async def pasar(request):
    ruta = request.url.path
    if not autorizado(request):
        return JSONResponse({"error": "no"}, status_code=403)
    sub = ruta[len(BASE):]
    if any(sub.startswith(n) for n in NEGADAS):
        NEGADOS.append({"t": time.time(), "ruta": ruta})
        return JSONResponse({"error": "ORE: esto no sale del puesto (%s)" % sub}, status_code=403)
    if estado["tunel"] is None:
        return JSONResponse({"error": "el puesto no está conectado"}, status_code=503)
    cuerpo = await request.body()
    estado["seq"] += 1
    rid = estado["seq"]
    q = asyncio.Queue()
    pendientes[rid] = q
    t0 = time.perf_counter()
    cabeceras = [(k, v) for k, v in request.headers.items() if k.lower() not in SALTO]
    camino = ruta + ("?" + request.url.query if request.url.query else "")
    await enviar({"t": "req", "id": rid, "method": request.method, "path": camino, "headers": cabeceras}, cuerpo)
    if request.method == "POST" and sub.endswith("/api/kernel/save"):
        GUARDADOS.append({"t": round(time.time(), 3), "bytes": len(cuerpo)})
    head = await q.get()
    if head.get("t") != "head":
        pendientes.pop(rid, None)
        return JSONResponse({"error": head.get("error", "sin respuesta")}, status_code=502)
    primer = (time.perf_counter() - t0) * 1000
    fila = {"m": request.method, "ruta": sub[:80], "st": head["status"], "ms_cab": round(primer, 1)}

    async def flujo():
        total, fin = 0, False
        try:
            while True:
                if CORTE_SSE and sub.startswith("/sse") and time.perf_counter() - t0 > CORTE_SSE:
                    fila["cortado"] = True
                    break
                # M1·b: no esperar al siguiente latido para saber que el navegador se fue
                # (M1·a: hasta 15 s, y la pestaña que recargaba entraba como lectora).
                try:
                    m = await asyncio.wait_for(q.get(), timeout=0.5)
                except asyncio.TimeoutError:
                    if await request.is_disconnected():
                        fila["desconectado"] = True
                        break
                    continue
                if m["t"] == "chunk":
                    total += len(m["_datos"])
                    yield m["_datos"]
                else:
                    fin = True
                    break
        finally:
            pendientes.pop(rid, None)
            fila.update(ms=round((time.perf_counter() - t0) * 1000, 1), bytes=total)
            REGISTRO.append(fila)
            if not fin and estado["tunel"] is not None:
                try:
                    await enviar({"t": "cancel", "id": rid})
                except Exception:  # noqa: BLE001
                    pass

    r = StreamingResponse(flujo(), status_code=head["status"])
    r.raw_headers = [(k.lower().encode("latin-1"), v.encode("latin-1"))
                     for k, v in head["headers"] if k.lower() not in SALTO | {"date", "server"}]
    if INMUTABLES and sub.startswith("/assets/") and head["status"] == 200:
        r.raw_headers.append((b"cache-control", b"public, max-age=31536000, immutable"))
    return r


async def registro(request):
    if not autorizado(request):
        return JSONResponse({"error": "no"}, status_code=403)
    return JSONResponse({"conectado_desde": estado["conectado"], "reconexiones": estado["reconexiones"],
                         "abiertas": len(pendientes), "peticiones": REGISTRO[-2000:],
                         "guardados": GUARDADOS, "negados": NEGADOS})


app = Starlette(routes=[
    WebSocketRoute("/tunel", tunel),
    Route(PREFIJO + "/m1/registro", registro),
    Route(PREFIJO + "/entrar", entrar),
    Route(BASE, pasar, methods=["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"]),
    Route(BASE + "/{resto:path}", pasar, methods=["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"]),
])

if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=8080, log_level="warning", ws_max_size=64 * 2**20)
