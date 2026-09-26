"""
`python -m ore.calcular DIR` — **la copia de una vista SQL, el paso de en medio**
(ADR 0040 paso 4c).

`ore materialize --preparar DIR` deja en `DIR/<copia>/` la consulta de la vista
—servida: cada dataset como `"__ore_dataset"."<p>.<n>"`, el mismo nombre con que
lo registra un puesto— y los datasets que lee, en Arrow. Esto la ejecuta con
DuckDB y deja lo que devuelve en `DIR/<copia>/salida.arrow`; `ore materialize
--calculado DIR` la sella. Es un contenedor de la imagen del puesto dentro del
Job de la copia, entre los dos de `ore-drivers`.

⛔ **Sin credencial.** El contenedor no monta el testigo del Job, y esto no lee
el bucket ni habla con `ore-serve`: lo que la consulta lee ya está en el disco,
y en cuanto está registrado DuckDB se cierra al exterior
(`enable_external_access = false`). Una consulta que
intentara leer un fichero o una URL por su cuenta —el compilador ya lo niega
(`OOS2038`)— tampoco podría aquí.

Una copia que falla deja `error.txt` con su motivo y las demás siguen: la
pasada la informa en su puntero, sin perder el dataset que había. Sale con 0
salvo que `DIR` no se pueda leer.
"""

import json
import os
import sys

from ore import _arrow, _derrame, _q, _tropo_mb


def _conexion():
    import duckdb

    con = duckdb.connect()
    # El reparto del pod y dónde derramar: lo mismo que una celda (`_duckdb`).
    con.execute("set memory_limit='%dMB'" % _tropo_mb())
    con.execute("set temp_directory='%s'" % _derrame().replace("'", "''"))
    con.execute("set autoinstall_known_extensions = false")
    con.execute("set autoload_known_extensions = false")
    # Un instante es un instante (0032 §1): la sesión es UTC.
    con.execute("set TimeZone = 'UTC'")
    return con


def calcular(aqui):
    """Una copia: `aqui` es `DIR/<copia>/`, con `peticion.json` y `entradas/`."""
    import pyarrow.ipc as ipc

    with open(os.path.join(aqui, "peticion.json"), encoding="utf-8") as f:
        p = json.load(f)
    con = _conexion()
    esquema = p["esquema_de_datasets"]
    con.execute("create schema if not exists %s" % _q(esquema))
    for i, (nombre, rel) in enumerate(sorted(p["entradas"].items())):
        tabla = ipc.open_stream(os.path.join(aqui, rel)).read_all()
        # La tabla de Arrow, sin copiarla, bajo el nombre que la consulta usa.
        con.register("__entrada_%d" % i, tabla)
        con.execute(
            "create view %s.%s as select * from %s"
            % (_q(esquema), _q(nombre), _q("__entrada_%d" % i))
        )
    con.execute("set enable_external_access = false")
    resultado = _arrow(con.sql(p["consulta"]))
    # Las columnas en el orden del contrato; lo que sobre o falte lo dice
    # `ore-store sellar-arrow`, que es quien sabe el contrato de tipos.
    orden = [c for c in p["columnas"] if c in resultado.column_names]
    orden += [c for c in resultado.column_names if c not in orden]
    resultado = resultado.select(orden)
    salida = os.path.join(aqui, "salida.arrow")
    with ipc.new_stream(salida, resultado.schema) as w:
        w.write_table(resultado)
    con.close()
    return resultado.num_rows


def main(argv):
    if len(argv) != 2:
        print("uso: python -m ore.calcular DIR", file=sys.stderr)
        return 2
    raiz = argv[1]
    # Lo que se cuenta va en UTF-8 aunque la consola no lo sea (Windows, cp1252):
    # un `✓` que no se puede imprimir no puede tumbar un cálculo que salió bien.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    try:
        copias = sorted(
            d for d in os.listdir(raiz) if os.path.isfile(os.path.join(raiz, d, "peticion.json"))
        )
    except OSError as e:
        print("✗ no se puede leer `%s`: %s" % (raiz, e), file=sys.stderr)
        return 1
    if not copias:
        print("· ninguna copia por consulta que calcular")
    for c in copias:
        aqui = os.path.join(raiz, c)
        for viejo in ("salida.arrow", "error.txt"):
            try:
                os.remove(os.path.join(aqui, viejo))
            except FileNotFoundError:
                pass
        try:
            n = calcular(aqui)
            print("✓ %s · %d filas" % (c, n), flush=True)
        except Exception as e:  # la de esta copia, no la pasada
            motivo = "%s: %s" % (type(e).__name__, str(e).strip().splitlines()[0] if str(e).strip() else "")
            with open(os.path.join(aqui, "error.txt"), "w", encoding="utf-8") as f:
                f.write(motivo + "\n")
            print("✗ %s · %s" % (c, motivo), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
