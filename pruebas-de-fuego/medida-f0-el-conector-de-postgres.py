# -*- coding: utf-8 -*-
"""MEDIDA F0 · M3 (ADR 0053) · lo que cuesta leer un Postgres con el conector de hoy.

El ORE Federation Engine va a leer el origen en vivo. Antes de fijar cotas y
prioridades se mide el conector real (`ore-read-postgres leer`) contra un
Postgres de pruebas con una tabla de N filas, sin red de verdad ni credencial
de nadie:

  ① el coste fijo de una peticion (arrancar, conectar, consultar, contestar)
  ② la tabla entera: tiempo, bytes y memoria del conector
  ③ lo que ahorra empujar un filtro `eq` (sin indice y por la clave)
  ④ lo que cuesta no tener `limit`: diez filas, hoy, son la tabla entera
  ⑤ cuantas columnas: la proyeccion estrecha frente a la ancha
  ⑥ texto frente a Arrow: el mismo resultado en Arrow IPC
  ⑦ concurrencia: N peticiones a la vez contra `max_connections`

Corre DENTRO de un contenedor con el conector en el PATH y el Postgres en
`PG_URL` (ver `medida-f0-el-conector-de-postgres.sh`). Escribe un JSON con
las cifras en `M3_SALIDA` y un resumen por pantalla.
"""
import concurrent.futures as cf
import json
import os
import resource
import statistics
import subprocess
import sys
import time

URL = os.environ["PG_URL"]
CONECTOR = os.environ.get("CONECTOR", "ore-read-postgres")
SALIDA = os.environ.get("M3_SALIDA", "/tmp/m3.json")
N = int(os.environ.get("M3_FILAS", "1000000"))
cifras = {"filas_en_la_tabla": N}

ANCHA = {"id": "id", "pais": "pais", "importe": "importe", "creado": "creado", "nota": "nota"}
ESTRECHA = {"id": "id"}


def peticion(proyeccion, filtros=()):
    return json.dumps({
        "url": URL,
        "objeto": "public.ventas",
        "proyeccion": proyeccion,
        "filtros": [{"columna": c, "operador": o, "valor": v} for c, o, v in filtros],
    })


def leer(proyeccion, filtros=()):
    """Una peticion: (segundos, bytes de salida, filas, kB de memoria maxima del hijo, error)."""
    antes = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    t = time.perf_counter()
    p = subprocess.run([CONECTOR, "leer"], input=peticion(proyeccion, filtros).encode(),
                       capture_output=True)
    s = time.perf_counter() - t
    rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    err = p.stderr.decode(errors="replace").strip() if p.returncode else ""
    filas = sum(1 for l in p.stdout.splitlines() if l.strip())
    return s, len(p.stdout), filas, max(rss, antes), err, p.stdout


def mediana(xs):
    return round(statistics.median(xs) * 1000, 1)


def titulo(t):
    print("\n── " + t)


# ① el coste fijo: una fila por su clave, veinte veces
titulo("① coste fijo de una peticion (id = k, por la clave)")
ts = []
for k in range(1, 21):
    s, b, f, _, err, _ = leer(ANCHA, [("id", "eq", str(k * 997))])
    assert not err and f == 1, (err, f)
    ts.append(s)
cifras["coste_fijo_ms"] = {"mediana": mediana(ts), "min": round(min(ts) * 1000, 1), "max": round(max(ts) * 1000, 1)}
print("   mediana %s ms (min %s, max %s)" % (mediana(ts), cifras["coste_fijo_ms"]["min"], cifras["coste_fijo_ms"]["max"]))

# ② la tabla entera, ancha
titulo("② la tabla entera, cinco columnas")
s, b, f, rss, err, salida_entera = leer(ANCHA)
assert not err, err
cifras["entera"] = {"s": round(s, 2), "MB": round(b / 1e6, 1), "filas": f, "rss_MB": round(rss / 1024, 1)}
print("   %s filas · %.2f s · %.1f MB de texto · el conector llega a %.0f MB de memoria" % (f, s, b / 1e6, rss / 1024))

# ③ un filtro empujado
titulo("③ filtro eq empujado")
s, b, f, _, err, _ = leer(ANCHA, [("pais", "eq", "ES")])
assert not err, err
cifras["filtro_sin_indice"] = {"s": round(s, 2), "MB": round(b / 1e6, 2), "filas": f}
print("   pais = 'ES' (sin indice): %s filas · %.2f s · %.2f MB" % (f, s, b / 1e6))

# ④ sin limit: diez filas cuestan la tabla entera
titulo("④ sin `limit`")
cifras["sin_limit"] = {
    "para_10_filas_hoy_s": cifras["entera"]["s"],
    "para_10_filas_hoy_MB": cifras["entera"]["MB"],
    "con_limit_seria": "una peticion de coste fijo (~%s ms)" % cifras["coste_fijo_ms"]["mediana"],
}
print("   hoy: %.2f s y %.1f MB para quedarse con diez · con `limit`: ~%s ms" % (
    cifras["entera"]["s"], cifras["entera"]["MB"], cifras["coste_fijo_ms"]["mediana"]))

# ⑤ estrecha frente a ancha
titulo("⑤ una columna frente a cinco, tabla entera")
s, b, f, _, err, _ = leer(ESTRECHA)
assert not err, err
cifras["estrecha"] = {"s": round(s, 2), "MB": round(b / 1e6, 1)}
print("   id solo: %.2f s · %.1f MB (ancha: %.2f s · %.1f MB)" % (s, b / 1e6, cifras["entera"]["s"], cifras["entera"]["MB"]))

# ⑥ texto frente a Arrow, el mismo resultado
titulo("⑥ texto frente a Arrow IPC")
try:
    import pyarrow as pa
    import pyarrow.ipc as ipc
    t = time.perf_counter()
    filas = [json.loads(l) for l in salida_entera.splitlines() if l.strip()]
    parse = time.perf_counter() - t
    tabla = pa.Table.from_pylist(filas)
    sink = pa.BufferOutputStream()
    with ipc.new_stream(sink, tabla.schema) as w:
        w.write_table(tabla)
    arrow = sink.getvalue().size
    cifras["texto_vs_arrow"] = {"texto_MB": cifras["entera"]["MB"], "arrow_MB": round(arrow / 1e6, 1),
                                "parsear_el_texto_s": round(parse, 2), "tipos_en_arrow": str(tabla.schema)}
    print("   texto %.1f MB → Arrow %.1f MB · parsear el texto: %.2f s" % (cifras["entera"]["MB"], arrow / 1e6, parse))
    print("   (y en texto todo es cadena: %s)" % ", ".join("%s:%s" % (f.name, f.type) for f in tabla.schema))
except ImportError:
    print("   sin pyarrow: no se mide")

# ⑦ concurrencia
titulo("⑦ concurrencia contra max_connections")
for n in (10, 50, 120):
    t = time.perf_counter()
    with cf.ThreadPoolExecutor(n) as ex:
        rs = list(ex.map(lambda k: leer(ANCHA, [("id", "eq", str(k + 1))]), range(n)))
    s = time.perf_counter() - t
    malos = [r[4] for r in rs if r[4]]
    cifras["concurrencia_%d" % n] = {"s": round(s, 2), "fallos": len(malos),
                                     "un_fallo": (malos[0][:160] if malos else "")}
    print("   %3d a la vez: %.2f s · %d fallan%s" % (n, s, len(malos), (" · " + malos[0][:120]) if malos else ""))

with open(SALIDA, "w", encoding="utf-8") as f:
    json.dump(cifras, f, ensure_ascii=False, indent=2)
print("\ncifras en", SALIDA)
