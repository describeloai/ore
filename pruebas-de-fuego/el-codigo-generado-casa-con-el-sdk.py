"""S3 · EL CÓDIGO QUE ORE GENERA CASA CON LA FIRMA DEL SDK.

`cargo test` (con `ORE_CELDAS_GENERADAS=<dir>`) vuelca cada forma que el servidor
genera: una celda por sentencia del guion SQL y el arnés de una Function. Allí ya
se comprueba que importan nombres que existen y que no llevan ninguno de antes.
Aquí, lo que Rust no puede ver sin ejecutar Python: que cada **llamada** a una
función del SDK casa con su firma de verdad —`inspect.signature(f).bind(...)`—,
así que un argumento mal escrito (`if_not_exist=`) o uno de más es un fallo aquí y
no en vivo. Sin servidor y sin clúster: nada se ejecuta, se analiza.

    ORE_CELDAS_GENERADAS=target/celdas-generadas cargo test -p ore-serve --bin ore-serve
    PYTHONUTF8=1 python pruebas-de-fuego/el-codigo-generado-casa-con-el-sdk.py target/celdas-generadas
"""
import ast
import inspect
import os
import sys

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(RAIZ, "puesto", "python"))
import ore  # noqa: E402

fallos = []


def bien(m):
    print("  ✓", m)


def mal(m):
    fallos.append(m)
    print("  ✗", m)


def comprobar(nombre, texto):
    try:
        compile(texto, nombre, "exec")
        arbol = ast.parse(texto, nombre)
    except SyntaxError as e:
        mal("%s: no compila: %s" % (nombre, e))
        return 0
    # Lo importado de `ore`, y a qué objeto del SDK es.
    del_sdk = {}
    for n in ast.walk(arbol):
        if isinstance(n, ast.ImportFrom) and n.module == "ore":
            for a in n.names:
                if not hasattr(ore, a.name):
                    mal("%s: importa `%s`, que el SDK no tiene" % (nombre, a.name))
                    continue
                del_sdk[a.asname or a.name] = getattr(ore, a.name)
    llamadas = 0
    for n in ast.walk(arbol):
        if not (isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id in del_sdk):
            continue
        f = del_sdk[n.func.id]
        if not callable(f):
            continue
        # Los valores dan igual: la firma decide por posición y por nombre.
        args = [None] * len(n.args)
        kw = {k.arg: None for k in n.keywords if k.arg is not None}
        try:
            inspect.signature(f).bind(*args, **kw)
        except TypeError as e:
            mal("%s: `%s(…)` no casa con su firma: %s" % (nombre, n.func.id, e))
            continue
        # Y por su nombre de hoy: lo que se llama es el nombre inglés, no un alias.
        if getattr(f, "__name__", n.func.id) != n.func.id and not n.func.id.startswith("_"):
            mal("%s: `%s` es un alias de `%s`" % (nombre, n.func.id, f.__name__))
        llamadas += 1
    return llamadas


def main(dir_):
    ficheros = sorted(f for f in os.listdir(dir_) if f.endswith(".py"))
    if len(ficheros) < 12:
        mal("sólo %d formas en %s: ¿se volcaron con ORE_CELDAS_GENERADAS?" % (len(ficheros), dir_))
    total = 0
    for f in ficheros:
        with open(os.path.join(dir_, f), encoding="utf-8") as fh:
            total += comprobar(f, fh.read())
    if not fallos:
        bien("%d formas generadas compilan, y sus %d llamadas al SDK casan con la firma de hoy (API %s)"
             % (len(ficheros), total, ore.API))
    print("todo bien" if not fallos else "%d fallos" % len(fallos))
    return 1 if fallos else 0


if __name__ == "__main__":
    print("el código generado casa con el SDK")
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(RAIZ, "target", "celdas-generadas")))
