"""R0·3 · El disco de derrame propio de un puesto (docs/ejecuciones.md §8.10).

En `/derrame` va montado el volumen que se mide (un disco efímero por pod de una clase, o el
`emptyDir` del nodo). Escribe y lee 4 GB con O_DIRECT (sin la caché de páginas, que en un pod
cuenta contra su memoria) y corre una agrupación que derrama (40 M filas, DuckDB con 1 GB).
El tiempo de conectar el disco al abrir el pod lo mide el lanzador.
"""
import json
import mmap
import os
import shutil
import time

import duckdb

D = os.environ.get("DERRAME", "/derrame")
BLOQUE = 4 << 20
TOTAL = int(os.environ.get("GB", "4")) << 30


def linea(**d):
    print(json.dumps(d, ensure_ascii=False), flush=True)


def directo():
    f = os.path.join(D, "prueba.bin")
    buf = mmap.mmap(-1, BLOQUE)
    buf.write(os.urandom(BLOQUE))
    fd = os.open(f, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_DIRECT, 0o600)
    t0 = time.time()
    for _ in range(TOTAL // BLOQUE):
        os.write(fd, buf)
    os.fsync(fd)
    os.close(fd)
    escribir = TOTAL / 2**20 / (time.time() - t0)
    fd = os.open(f, os.O_RDONLY | os.O_DIRECT)
    t0 = time.time()
    while os.readv(fd, [buf]) > 0:
        pass
    os.close(fd)
    leer = TOTAL / 2**20 / (time.time() - t0)
    os.remove(f)
    return round(escribir), round(leer)


def derrame():
    tmp = os.path.join(D, "duckdb")
    shutil.rmtree(tmp, ignore_errors=True)
    c = duckdb.connect()
    c.execute("set memory_limit = '1GB'")
    c.execute("set threads to 2")
    c.execute("set temp_directory = '%s'" % tmp)
    c.execute("set max_temp_directory_size = '300GB'")
    c.execute("set preserve_insertion_order = false")
    t0 = time.time()
    c.execute("""copy (select (hash(range) %% 20000000)::bigint k, sum(random()) sv, count(*) n
                 from range(40000000) group by k) to '%s' (format parquet)""" % os.path.join(D, "salida.parquet"))
    return round(time.time() - t0, 1)


def main():
    st = os.statvfs(D)
    linea(evento="inicio", volumen=os.environ.get("VOLUMEN"), libre_gb=round(st.f_bavail * st.f_frsize / 2**30))
    e, l = directo()
    linea(evento="disco", volumen=os.environ.get("VOLUMEN"), escribir_mb_s=e, leer_mb_s=l, gb=TOTAL >> 30)
    linea(evento="derrame", volumen=os.environ.get("VOLUMEN"), s=derrame())
    linea(evento="fin")


if __name__ == "__main__":
    main()
