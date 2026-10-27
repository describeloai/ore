"""0064 · marimo, lanzado como subproceso del agente — SÓLO existe en la imagen
`puesto-python-notebooks:1` (`agente.py` importa este módulo con un `try` y se
comporta como siempre si falta: el resto de entornos no lo llevan).

La identidad del pod no la pone este módulo: la coge sola `sitecustomize.py`
(horneado al lado, en `/opt/ore`), que `python -m marimo` importa solo al
arrancar, antes de que corra cualquier celda de la persona.
"""
import os
import subprocess
import sys
import threading
import time
import urllib.request

PUERTO = 2718
HOST = "127.0.0.1"

# ⛔ El entorno del hijo es EXPLÍCITO y MÍNIMO, no heredado (como
#   `repositorio.py::correr_pytest`, no como `Correa.encender`): el kernel de
#   marimo corre código que escribe la persona, no un binario nuestro de
#   confianza como pyright.
_VARIABLES = (
    "HOME", "PATH", "LANG",
    "ORE_SERVE", "PUESTO", "BUCKET", "ORE_ALMACEN", "ORE_MEMORIA_MB",
    "ORE_TOKEN_DEL_POD",
)


def log(*a):
    print("marimo ·", *a, flush=True)


class Marimo:
    def __init__(self, proceso):
        self.proceso = proceso
        # Nadie la toca todavía: hace falta el relevo (Pieza 3) para que haya
        # tráfico de verdad que observar. Ya enchufada al TTL del agente
        # (`max(ultimo, correa.actividad, marimo.actividad)`).
        self.actividad = 0.0

    def detener(self):
        try:
            self.proceso.terminate()
            self.proceso.wait(15)
        except Exception as e:  # noqa: BLE001 — mejor esfuerzo, como el resto del agente
            log("no se pudo parar limpio (%s)" % e)


def _esperar_listo(t0):
    while time.time() - t0 < 120:
        try:
            urllib.request.urlopen("http://%s:%d/api/status" % (HOST, PUERTO), timeout=2)
            log("listo en %.1f s" % (time.time() - t0))
            return
        except Exception:  # noqa: BLE001 — sigue intentando hasta el tope
            time.sleep(0.2)
    log("no respondió en 120 s: sigue vivo, pero algo va mal")


def arrancar(directorio):
    """Lanza marimo sobre `directorio` (el árbol materializado del
    repositorio: la persona elige qué `.py` abrir como cuaderno desde la
    propia portada de marimo). No bloquea: un hilo aparte sólo COMPRUEBA y
    LOGUEA que arrancó — nada depende todavía de que esté listo."""
    env = {k: os.environ[k] for k in _VARIABLES if k in os.environ}
    # Para que `import ore` (y el `sitecustomize.py` que lo autoconfigura) y
    # lo que el repositorio declaró en su capa se vean desde el kernel.
    env["PYTHONPATH"] = "/opt/ore:/capa"
    # `--watch` (0064): lo que la consola escribe en un cuaderno ya abierto
    #   (`ore/escribir`: descartar cambios, otra rama) marimo lo recarga.
    proceso = subprocess.Popen(
        [sys.executable, "-m", "marimo", "edit", "--headless", "--host", HOST,
         "--port", str(PUERTO), "--no-token", "--no-sandbox", "--no-skew-protection",
         "--skip-update-check", "--watch", directorio],
        env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    log("arrancado (pid %d) sobre %s" % (proceso.pid, directorio))
    threading.Thread(target=_esperar_listo, args=(time.time(),), daemon=True).start()
    return Marimo(proceso)
