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

from ore import _arrow, _derrame, _ejes_xy, _q, _tropo_mb


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


def _es_geo(consulta, tablas):
    from ore import _es_geo as campo_geo
    from ore.lsp_sql import GEO_EN_SQL

    return bool(GEO_EN_SQL.search(consulta)) or any(campo_geo(f) for _, t in tablas for f in t.schema)


def _cargar_geo(con):
    """`spatial` y `h3` de la imagen (sin red, como todo aquí)."""
    from ore import EXTENSIONES

    if os.path.isdir(EXTENSIONES):
        con.execute("set extension_directory = '%s'" % EXTENSIONES)
    con.execute("load spatial")
    # 0068 · x es la longitud, siempre (`ore._ejes_xy`): lo mismo que en la celda.
    _ejes_xy(con)
    try:
        con.execute("load h3")
    except Exception:  # noqa: BLE001 — de community: si no está, lo geo sigue
        pass


def calcular(aqui):
    """Compute one copy: `aqui` is `DIR/<copy>/`, with `peticion.json` and `entradas/`.
    Writes `salida.arrow` and returns the number of rows."""
    import pyarrow.ipc as ipc

    with open(os.path.join(aqui, "peticion.json"), encoding="utf-8") as f:
        p = json.load(f)
    con = _conexion()
    esquema = p["esquema_de_datasets"]
    con.execute("create schema if not exists %s" % _q(esquema))
    tablas = [(nombre, ipc.open_stream(os.path.join(aqui, rel)).read_all())
              for nombre, rel in sorted(p["entradas"].items())]
    # 0068 C1·1: lo geo ANTES de registrar. `volcar` marca cada columna geo con su
    # CRS (`geoarrow.wkb`); sin `spatial` cargado, DuckDB la lee como GEOMETRY sin
    # CRS, en silencio. Y una consulta con `ST_*` o `h3_*` los necesita igual.
    if _es_geo(p["consulta"], tablas):
        _cargar_geo(con)
    for i, (nombre, tabla) in enumerate(tablas):
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
        print("usage: python -m ore.calcular DIR", file=sys.stderr)
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
        print("✗ cannot read `%s`: %s" % (raiz, e), file=sys.stderr)
        return 1
    if not copias:
        print("· no query copy to compute")
    for c in copias:
        aqui = os.path.join(raiz, c)
        for viejo in ("salida.arrow", "error.txt"):
            try:
                os.remove(os.path.join(aqui, viejo))
            except FileNotFoundError:
                pass
        try:
            n = calcular(aqui)
            print("✓ %s · %d rows" % (c, n), flush=True)
        except Exception as e:  # la de esta copia, no la pasada
            motivo = "%s: %s" % (type(e).__name__, str(e).strip().splitlines()[0] if str(e).strip() else "")
            with open(os.path.join(aqui, "error.txt"), "w", encoding="utf-8") as f:
                f.write(motivo + "\n")
            print("✗ %s · %s" % (c, motivo), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
