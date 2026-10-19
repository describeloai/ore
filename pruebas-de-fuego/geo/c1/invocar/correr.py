"""ORE 0068 · cierre de C1 · La celda DE VERDAD de una función de Python que devuelve una geometría (la
escribe `funciones.rs`, prueba `las_celdas_de_una_funcion_que_da_una_geometria`), corrida como el agente:
su última expresión, la tabla de Arrow, y lo que el agente devuelve de ella (`ore.table`).
La spec (v1alpha32 01 §5): en texto, WKB en hexadecimal."""
import json
import sys

sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402

import struct  # noqa: E402

ESPERA = ("0101000000" + struct.pack("<dd", 440001, 4474000).hex())  # POINT (440001 4474000), x = 440000 + 1
texto = open("/celdas/celda.py", encoding="utf-8").read()
ns = {}
try:
    cuerpo, _, ultima = texto.rstrip().rpartition("\n")
    exec(compile(cuerpo, "celda", "exec"), ns)
    t = eval(ultima, ns)
    tabla = ore.table(t)
    v = tabla["filas"][0][0]
    print("%s · python · %s · %s" % ("OK   " if v == ESPERA else "FALLA", tabla["columnas"], json.dumps(v)))
except Exception as e:  # noqa: BLE001
    print("FALLA · python · %s: %s" % (type(e).__name__, str(e).splitlines()[0][:200]))
