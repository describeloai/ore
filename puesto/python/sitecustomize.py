"""0064 · la identidad del pod, para un proceso que el agente no controla por
dentro (el kernel de marimo). SÓLO vive en la imagen `puesto-python-notebooks:1`.

`sitecustomize.py` es un enganche de CPython, no de marimo: cualquier proceso
de Python que arranque con este directorio en `PYTHONPATH` lo importa solo,
antes de ejecutar nada más —`python -m marimo` incluido—. Así el SDK se
autoconfigura SIN que haga falta una celda de arranque visible en el `.py` de
la persona (el cuaderno es código del árbol; esto no es suyo).

Es el mismo camino que ya demostró M0·d (`pruebas-de-fuego/marimo/m0d.py`,
una celda escrita a mano para la medida), pero sin la celda: deliberadamente
más simple que `Testigo.cabeceras()` del agente (sin `x-ore-sujeto` ni el
respaldo de OAuth — rutas de desarrollo que no aplican dentro de un kernel).
"""
import os


def _cabeceras_del_pod():
    try:
        with open(os.environ.get("ORE_TOKEN_DEL_POD", "/var/run/ore/pod/token"), encoding="utf-8") as f:
            token = f.read().strip()
    except OSError:
        return {}
    return {"x-ore-pod": token} if token else {}


try:
    import ore
    ore.session._proveedor = _cabeceras_del_pod
except ImportError:
    pass
