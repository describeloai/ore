"""Lee por stdin lo que `ore-read-cassandra leer` escribió y dice qué es, en JSON.

    ore-read-cassandra leer < peticion.json | python arrow.py

Sirve para comprobar, sin fiarse del conector, QUÉ columnas salieron (la proyección es
cerrada), con qué tipo Arrow y cuántas filas. Con `--filas N` enseña también las N primeras.
"""
import json
import sys

import pyarrow.ipc as ipc


def main() -> int:
    n = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 0
    datos = sys.stdin.buffer.read()
    if not datos:
        print(json.dumps({"vacio": True}))
        return 3
    t = ipc.open_stream(datos).read_all()
    print(
        json.dumps(
            {
                "columnas": [
                    {"nombre": f.name, "tipo": str(f.type), "ore": (f.metadata or {}).get(b"ore:tipo", b"").decode()}
                    for f in t.schema
                ],
                "filas": t.num_rows,
                "primeras": json.loads(json.dumps(t.slice(0, n).to_pylist(), default=str)),
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
