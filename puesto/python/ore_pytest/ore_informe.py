"""El informe de pytest para la consola (ORE 0050 P4): el gemelo de
`puesto/node/informe-de-pruebas.mjs`.

pytest lo carga (`-p ore_informe`) en el proceso de las pruebas, y escribe en
`ORE_INFORME` una línea JSON por cosa —`prueba`, `fichero` (uno que no cargó),
`salida` (lo que una prueba imprimió)—, con los mismos campos que el de Node, así
que la consola los pinta igual. La salida de la terminal (`-v -rA`) es aparte:
es el registro, lo que vería quien corre `pytest` a mano.

Y tres cosas más, del entorno de `repositorio.py`:

- ``ORE_RUTAS``: el SDK y la capa del repositorio (`/capa`, `/capa/.dev`), al
  FINAL de `sys.path`, como los pone el agente: manda la imagen (0031 W3.2).
- ``ORE_NOMBRES``: si viene, sólo esas pruebas (por su nombre, con su caso:
  `test_x[2-4]`), como `--test-name-pattern` en Node.
- ``ORE_TOPE_PRUEBA``: los segundos de UNA prueba (30), con una alarma: un bucle
  infinito falla esa prueba, no la ejecución entera.
"""
import json
import os
import signal
import sys
import threading

import pytest

_salida = None
_ultima = None  # (izquierda, derecha) del último `assert a == b` que falló


def _escribir(**x):
    if _salida is not None:
        _salida.write(json.dumps(x, ensure_ascii=False) + "\n")
        _salida.flush()


def pytest_configure(config):
    global _salida
    destino = os.environ.get("ORE_INFORME")
    if destino:
        _salida = open(destino, "w", encoding="utf-8")
    for r in filter(None, os.environ.get("ORE_RUTAS", "").split(os.pathsep)):
        if os.path.isdir(r) and r not in sys.path:
            sys.path.append(r)


def pytest_unconfigure(config):
    global _salida
    if _salida is not None:
        _salida.close()
        _salida = None


def pytest_collection_modifyitems(config, items):
    try:
        nombres = set(json.loads(os.environ.get("ORE_NOMBRES") or "[]"))
    except ValueError:
        nombres = set()
    if not nombres:
        return
    quedan = [i for i in items if i.name in nombres]
    fuera = [i for i in items if i.name not in nombres]
    if fuera:
        config.hook.pytest_deselected(items=fuera)
        items[:] = quedan


class _Vencida(Exception):
    pass


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_call(item):
    tope = int(os.environ.get("ORE_TOPE_PRUEBA", "30") or 0)
    alarma = tope > 0 and hasattr(signal, "SIGALRM") and threading.current_thread() is threading.main_thread()
    if alarma:
        def vencer(*_):
            raise _Vencida("la prueba tardó más de %d s: se paró" % tope)
        anterior = signal.signal(signal.SIGALRM, vencer)
        signal.alarm(tope)
    try:
        yield
    finally:
        if alarma:
            signal.alarm(0)
            signal.signal(signal.SIGALRM, anterior)


def pytest_runtest_logstart(nodeid, location):
    global _ultima
    _ultima = None


def pytest_assertrepr_compare(config, op, left, right):
    # Sólo se apunta: la explicación de pytest sigue siendo la suya.
    global _ultima
    if op == "==":
        _ultima = (left, right)
    return None


def _texto(v, n=2000):
    t = repr(v)
    return t if len(t) <= n else t[:n] + "…"


def pytest_runtest_logreport(report):
    # Lo que la prueba imprimió, a su fichero (una vez: lo de la llamada).
    fichero = report.nodeid.split("::")[0]
    if report.when == "call":
        impreso = (report.capstdout or "") + (report.capstderr or "")
        if impreso:
            _escribir(e="salida", fichero=fichero, texto=impreso)
    # Una prueba se cuenta en su llamada, o en su preparación si no llegó a
    # llamarse (saltada con `skip`, o un fixture que falló).
    if not (report.when == "call" or (report.when == "setup" and report.outcome != "passed")):
        return
    partes = report.nodeid.split("::")
    if report.outcome == "passed":
        estado = "fallo" if getattr(report, "wasxfail", None) is not None and report.failed else "ok"
    elif report.outcome == "skipped":
        estado = "saltada"
    else:
        estado = "fallo"
    x = dict(
        e="prueba",
        fichero=fichero,
        ruta=partes[1:-1],
        nombre=partes[-1],
        estado=estado,
        ms=round(report.duration * 1000, 1),
        linea=(report.location[1] + 1) if report.location and report.location[1] is not None else None,
    )
    if estado == "fallo":
        crash = getattr(report.longrepr, "reprcrash", None)
        x["mensaje"] = (crash.message if crash else str(report.longrepr)).strip()[:4000]
        x["traza"] = report.longreprtext[-6000:]
        if _ultima is not None and crash and "AssertionError" in crash.message:
            x["obtenido"], x["esperado"] = _texto(_ultima[0]), _texto(_ultima[1])
    elif estado == "saltada" and isinstance(report.longrepr, tuple) and len(report.longrepr) == 3:
        x["mensaje"] = str(report.longrepr[2]).replace("Skipped: ", "", 1)
    _escribir(**x)


def pytest_collectreport(report):
    if report.failed:
        _escribir(e="fichero", fichero=report.nodeid.split("::")[0], estado="error",
                  mensaje=report.longreprtext[-4000:].strip())
