#!/usr/bin/env python3
"""
EL AGENTE DEL PUESTO (0031 W3.1) — lo que corre dentro de la sesión Python.

Es un kernel pequeño y un cliente: pide trabajo a `ore-serve` por HTTP (polling
largo: el puesto no acepta conexiones, «sin entrada»), ejecuta cada celda en un
espacio de nombres que dura toda la sesión, y devuelve la salida TIPADA —tabla,
texto, error, vacía— al mismo sitio. La consola nunca habla con este proceso.

  GET  /puestos/{id}/pendiente            → 200 {pendiente, celda, lenguaje, texto}
  POST /puestos/{id}/celdas/{n}/salida    ← {tipo, ms, …}

Quién es: dentro del clúster, el agente de la celda (`ore-agente-<n>`, client
credentials contra el IdP, `rubix_tipo: agente`); en las pruebas, la cabecera
`x-ore-sujeto: agente:…` (`ORE_SUJETO`). `ore-serve` ata el puesto al primer
agente que lo reclama y contesta 403 a cualquier otro.

Cuándo muere: sin celdas durante `TTL` segundos (el TTL de inactividad es del
agente, no de la cola: medido en 0031), o cuando `ore-serve` dice que el puesto
está cerrado (410). El tope de sesión lo pone el Job (`activeDeadlineSeconds`).
"""
import ast
import contextlib
import io
import json
import os
import sys
import time
import traceback
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ore  # noqa: E402 — el SDK, al lado de este fichero

FILAS_MAXIMAS = 200


def log(*a):
    print("agente ·", *a, flush=True)


# ── Quién soy: el token ────────────────────────────────────────────────────
class Testigo:
    def __init__(self):
        self.sujeto = os.environ.get("ORE_SUJETO")
        self.direccion = os.environ.get("DIRECCION", "").rstrip("/")
        self.realm = os.environ.get("REALM", "rubix")
        self.cliente = self._fichero("agente-cliente")
        self.secreto = self._fichero("agente-secreto")
        self.token = None
        self.caduca = 0

    @staticmethod
    def _fichero(nombre):
        ruta = os.path.join(os.environ.get("PUESTO_DIR", "/puesto"), nombre)
        try:
            with open(ruta, encoding="utf-8") as f:
                return f.read().strip()
        except OSError:
            return None

    def cabeceras(self):
        if self.sujeto:
            return {"x-ore-sujeto": self.sujeto}
        if not (self.cliente and self.secreto and self.direccion):
            raise SystemExit("agente · sin identidad: ni ORE_SUJETO ni /puesto/agente-{cliente,secreto} con DIRECCION")
        if time.time() > self.caduca - 60:
            datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": self.cliente, "client_secret": self.secreto}).encode()
            with urllib.request.urlopen("%s/realms/%s/protocol/openid-connect/token" % (self.direccion, self.realm), data=datos, timeout=20) as r:
                t = json.load(r)
            self.token = t["access_token"]
            self.caduca = time.time() + int(t.get("expires_in", 300))
            log("token del agente renovado · caduca en %ds" % int(t.get("expires_in", 300)))
        return {"authorization": "Bearer " + self.token}


# ── El kernel: un espacio de nombres para toda la sesión ───────────────────
class Kernel:
    def __init__(self):
        self.espacio = {"__name__": "__main__", "over": ore.over, "sql": ore.sql, "write": ore.write, "declare": ore.declare, "persona": ore.persona, "ore": ore}

    def correr(self, texto, lenguaje="python"):
        t0 = time.time()
        salida = io.StringIO()
        valor = None
        # Una celda SQL (W3.3): la consulta entera va a `ore.sql`, sobre las copias.
        if lenguaje == "sql":
            try:
                with contextlib.redirect_stdout(salida), contextlib.redirect_stderr(salida):
                    valor = ore.sql(texto)
            except Exception as e:  # noqa: BLE001
                return {"tipo": "error", "nombre": type(e).__name__, "mensaje": str(e), "traza": traceback.format_exc(),
                        "texto": salida.getvalue(), "ms": ms(t0)}
            return self.salida_de(valor, salida.getvalue(), t0)
        try:
            arbol = ast.parse(texto, mode="exec")
            ultimo = None
            if arbol.body and isinstance(arbol.body[-1], ast.Expr):
                ultimo = ast.Expression(arbol.body.pop().value)
            with contextlib.redirect_stdout(salida), contextlib.redirect_stderr(salida):
                exec(compile(arbol, "<celda>", "exec"), self.espacio)
                if ultimo is not None:
                    valor = eval(compile(ultimo, "<celda>", "eval"), self.espacio)
        except Exception as e:  # noqa: BLE001 — la celda puede fallar como quiera
            traza = traceback.format_exc()
            return {"tipo": "error", "nombre": type(e).__name__, "mensaje": str(e), "traza": traza,
                    "texto": salida.getvalue(), "ms": ms(t0)}
        return self.salida_de(valor, salida.getvalue(), t0)

    @staticmethod
    def salida_de(valor, texto, t0):
        tabla = como_tabla(valor)
        if tabla is not None:
            tabla.update({"tipo": "tabla", "texto": texto, "ms": ms(t0)})
            return tabla
        if valor is None:
            if texto.strip():
                return {"tipo": "texto", "texto": texto, "ms": ms(t0)}
            return {"tipo": "vacia", "ms": ms(t0)}
        return {"tipo": "texto", "texto": texto + repr(valor), "ms": ms(t0)}


def ms(t0):
    return int((time.time() - t0) * 1000)


def como_tabla(valor):
    """La salida `tabla` del contrato: la hace el SDK (`ore.tabla`), que es lo que
    una celda también puede pedir; aquí sólo se le pone el límite de la consola."""
    return ore.tabla(valor, FILAS_MAXIMAS)


def llano(v):
    """Un valor suelto (el resultado de una celda que no es tabla) → JSON."""
    return ore.json_de(v)


# ── El bucle ───────────────────────────────────────────────────────────────
def main():
    p = ore.puesto
    if not p.id:
        raise SystemExit("agente · sin PUESTO en el entorno")
    ttl = int(os.environ.get("TTL", "1800"))
    testigo = Testigo()
    kernel = Kernel()
    log("puesto %s · ore-serve %s · TTL %ds · almacén %s" % (p.id, p.servidor, ttl, p.almacen))
    ultimo = time.time()
    while True:
        if time.time() - ultimo > ttl:
            log("sin celdas durante %ds: cierro" % ttl)
            return 0
        p._cabeceras = testigo.cabeceras()
        try:
            codigo, r = p.pedir("GET", "/puestos/%s/pendiente" % p.id, plazo=40)
        except Exception as e:  # noqa: BLE001 — la red se cae; se reintenta
            log("ore-serve no contesta (%s): reintento en 5 s" % e)
            time.sleep(5)
            continue
        # Sin celda en la espera: `{pendiente: false}` (era `not r`, y un dict
        # con `pendiente: false` NO está vacío: el agente moría con KeyError
        # al primer poll vacío, y el puesto se daba por perdido a los 90 s).
        if codigo == 200 and not (r and r.get("pendiente")):
            continue
        if codigo == 410:
            log("el puesto está cerrado: adiós")
            return 0
        if codigo != 200:
            log("pendiente contestó %s: %s · reintento en 5 s" % (codigo, r))
            time.sleep(5)
            continue
        if not p.persona:
            c2, ficha = p.pedir("GET", "/puestos/%s" % p.id)
            if c2 == 200 and (ficha or {}).get("persona"):
                p.persona = ficha["persona"]
                log("el puesto es de %s" % p.persona)
        n, texto, lenguaje = r["celda"], r.get("texto", ""), r.get("lenguaje") or "python"
        log("celda %s · %s · %d bytes" % (n, lenguaje, len(texto)))
        salida = kernel.correr(texto, lenguaje)
        ultimo = time.time()
        p._cabeceras = testigo.cabeceras()
        try:
            codigo, r2 = p.pedir("POST", "/puestos/%s/celdas/%s/salida" % (p.id, n), cuerpo=salida)
            if codigo not in (200, 201):
                log("la salida de la celda %s no se aceptó: %s %s" % (n, codigo, r2))
        except Exception as e:  # noqa: BLE001
            log("no pude entregar la salida de la celda %s: %s" % (n, e))
        log("celda %s · %s · %d ms" % (n, salida.get("tipo"), salida.get("ms", 0)))


if __name__ == "__main__":
    sys.exit(main())
