"""Mide, contra el laboratorio, lo que Phoenix hace con ROW_TIMESTAMP (0071 M7, ficha 3.16).

    python -I -X utf8 sonda_rowts.py [http://127.0.0.1:8765/]

No cambia nada de ORE: crea ore_x0.eventos_* y lo cuenta. phoenixdb importa gssapi aunque no se use Kerberos;
en Windows no hay KfW, y se le pone un doble.
"""
import sys
import time
import types

sys.modules.setdefault("gssapi", types.ModuleType("gssapi"))
_m = types.ModuleType("requests_gssapi")
_m.HTTPSPNEGOAuth = object
sys.modules.setdefault("requests_gssapi", _m)
import phoenixdb  # noqa: E402

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8765/"
conn = phoenixdb.connect(URL, autocommit=True)
cur = conn.cursor()


def sql(texto, mostrar=True, params=None):
    t0 = time.time()
    try:
        cur.execute(texto, params) if params else cur.execute(texto)
        filas = cur.fetchall() if cur.description else []
        dt = time.time() - t0
        if mostrar:
            print(f"  [{dt:6.2f}s] {texto[:110]}")
            for f in filas[:8]:
                print("      ", f)
            if len(filas) > 8:
                print(f"       ... {len(filas)} filas")
        return filas
    except Exception as e:  # noqa: BLE001
        print(f"  [ERROR] {texto[:90]}\n      {str(e)[:260]}")
        return None


def titulo(t):
    print(f"\n== {t}")


for t in ("ts", "seg", "date", "big", "sal", "sin"):
    sql(f"DROP TABLE IF EXISTS ore_x0.eventos_{t}", mostrar=False)

titulo("1. declarar la columna (TIMESTAMP, DATE, BIGINT; primera y no primera de la clave; con sal)")
for nombre, ddl in [
    ("ts_primera", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_ts (ts TIMESTAMP NOT NULL, id BIGINT NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (ts ROW_TIMESTAMP, id))"),
    ("ts_segunda", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_seg (cliente BIGINT NOT NULL, ts TIMESTAMP NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (cliente, ts ROW_TIMESTAMP))"),
    ("date", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_date (d DATE NOT NULL, id BIGINT NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (d ROW_TIMESTAMP, id))"),
    ("bigint", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_big (t BIGINT NOT NULL, id BIGINT NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (t ROW_TIMESTAMP, id))"),
    ("con_sal", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_sal (ts TIMESTAMP NOT NULL, id BIGINT NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (ts ROW_TIMESTAMP, id)) SALT_BUCKETS = 4"),
    ("sin", "CREATE TABLE IF NOT EXISTS ore_x0.eventos_sin (id BIGINT NOT NULL PRIMARY KEY, ts TIMESTAMP, v VARCHAR)"),
]:
    print(" ", nombre)
    sql(ddl, mostrar=False) is None and print("   -> no se pudo")

titulo("2. ¿lo cuenta el catalogo? (SYSTEM.CATALOG y las columnas JDBC)")
sql("SELECT TABLE_NAME, COLUMN_NAME, IS_ROW_TIMESTAMP, KEY_SEQ FROM SYSTEM.CATALOG WHERE TABLE_SCHEM = 'ORE_X0' AND TABLE_NAME LIKE 'EVENTOS_%' AND COLUMN_NAME IS NOT NULL ORDER BY TABLE_NAME, ORDINAL_POSITION")

titulo("3. escribir: en orden, atrasadas, futuras, y actualizar")
ahora_ms = int(time.time() * 1000)
for i in range(5):
    sql(f"UPSERT INTO ore_x0.eventos_ts VALUES (TO_TIMESTAMP('2026-10-20 10:00:0{i}'), {i}, 'a{i}')", mostrar=False)
sql("SELECT ts, id, v FROM ore_x0.eventos_ts ORDER BY ts")
print(" atrasada: ts muy anterior, escrita DESPUES")
sql("UPSERT INTO ore_x0.eventos_ts VALUES (TO_TIMESTAMP('2026-10-19 09:00:00'), 100, 'tarde')")
print(" futura: ts dentro de un dia")
sql("UPSERT INTO ore_x0.eventos_ts VALUES (TO_TIMESTAMP('2026-10-22 09:00:00'), 101, 'futura')")
print(" ts nulo")
sql("UPSERT INTO ore_x0.eventos_ts (id, v) VALUES (102, 'sin ts')")
print(" actualizar una fila existente (misma clave = mismo ts)")
sql("UPSERT INTO ore_x0.eventos_ts VALUES (TO_TIMESTAMP('2026-10-20 10:00:02'), 2, 'ACTUALIZADA')")
sql("SELECT ts, id, v FROM ore_x0.eventos_ts ORDER BY ts")
print(" borrar")
sql("DELETE FROM ore_x0.eventos_ts WHERE id = 3")
sql("SELECT COUNT(*) FROM ore_x0.eventos_ts")

titulo("4. PHOENIX_ROW_TIMESTAMP() en la tabla con ROW_TIMESTAMP y en la que no la tiene")
sql("SELECT id, v, PHOENIX_ROW_TIMESTAMP() FROM ore_x0.eventos_ts ORDER BY id")
sql("SELECT id, v, CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP) FROM ore_x0.eventos_ts ORDER BY id")
sql("UPSERT INTO ore_x0.eventos_sin VALUES (1, TO_TIMESTAMP('2020-01-01 00:00:00'), 'x')", mostrar=False)
time.sleep(1.2)
sql("UPSERT INTO ore_x0.eventos_sin VALUES (2, TO_TIMESTAMP('2020-01-01 00:00:00'), 'y')", mostrar=False)
sql("SELECT id, CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP), ts FROM ore_x0.eventos_sin ORDER BY id")
time.sleep(1.2)
print(" actualizar la fila 1 de la tabla sin ROW_TIMESTAMP: ¿se mueve la marca?")
sql("UPSERT INTO ore_x0.eventos_sin VALUES (1, TO_TIMESTAMP('2020-01-01 00:00:00'), 'x2')", mostrar=False)
sql("SELECT id, v, CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP) FROM ore_x0.eventos_sin ORDER BY id")
print(" MAX en agregado")
sql("SELECT MAX(CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP)) FROM ore_x0.eventos_sin")
sql("SELECT MAX(PHOENIX_ROW_TIMESTAMP()) FROM ore_x0.eventos_sin")

titulo("4b. el filtro por PHOENIX_ROW_TIMESTAMP() ¿distingue segundos y milisegundos?")
marca = sql("SELECT CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP) FROM ore_x0.eventos_sin WHERE id = 1", mostrar=False)[0][0]
print("  marca de la fila 1:", marca)
import datetime as _dt
for delta in (-1.0, -0.001, 0.0, 0.001):
    corte = marca + _dt.timedelta(seconds=delta)
    txt = corte.strftime("%Y-%m-%d %H:%M:%S.") + f"{corte.microsecond // 1000:03d}"
    r = sql(f"SELECT id FROM ore_x0.eventos_sin WHERE PHOENIX_ROW_TIMESTAMP() > TO_TIMESTAMP('{txt}') ORDER BY id", mostrar=False)
    print(f"  > {txt}  ->", [x[0] for x in (r or [])])
    r = sql(f"SELECT id FROM ore_x0.eventos_sin WHERE CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP) > TO_TIMESTAMP('{txt}') ORDER BY id", mostrar=False)
    print(f"  CAST > {txt} ->", [x[0] for x in (r or [])])
sql("SELECT CAST(PHOENIX_ROW_TIMESTAMP() AS TIMESTAMP) FROM ore_x0.eventos_sin ORDER BY PHOENIX_ROW_TIMESTAMP() DESC LIMIT 1")

titulo("5. planes y coste (tabla con ROW_TIMESTAMP, ~10^6 filas)")
sql("SELECT COUNT(*) FROM ore_x0.eventos_ts", mostrar=False)
print("  cargando 10^6 filas: 1000 con ts repartidos en 100 dias y se duplica con UPSERT SELECT (ts + 1 s, id + N)...")
t0 = time.time()
for i in range(1000):
    sql(f"UPSERT INTO ore_x0.eventos_ts VALUES (TO_TIMESTAMP('2026-07-01 00:00:00') + {i * 0.1 / 1.0}, {10000 + i}, 'b{i}')", mostrar=False)
N = 1000
while N < 1_000_000:
    sql(f"UPSERT INTO ore_x0.eventos_ts SELECT ts + 1.0/86400, id + {N}, v FROM ore_x0.eventos_ts WHERE id >= 10000 AND id < {10000 + N}", mostrar=False)
    N *= 2
print(f"  cargadas en {time.time()-t0:.0f}s")
sql("SELECT COUNT(*), MIN(ts), MAX(ts) FROM ore_x0.eventos_ts")
for q in [
    "EXPLAIN SELECT MAX(ts) FROM ore_x0.eventos_ts",
    "EXPLAIN SELECT ts FROM ore_x0.eventos_ts ORDER BY ts DESC LIMIT 1",
    "EXPLAIN SELECT * FROM ore_x0.eventos_ts WHERE ts > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "EXPLAIN SELECT * FROM ore_x0.eventos_ts WHERE ts > TO_TIMESTAMP('2026-10-19 00:00:00') AND ts <= TO_TIMESTAMP('2026-10-20 12:00:00')",
    "EXPLAIN SELECT * FROM ore_x0.eventos_ts WHERE id > 5000",
    "EXPLAIN SELECT * FROM ore_x0.eventos_seg WHERE ts > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "EXPLAIN SELECT * FROM ore_x0.eventos_sal WHERE ts > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "EXPLAIN SELECT * FROM ore_x0.eventos_sin WHERE PHOENIX_ROW_TIMESTAMP() > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "EXPLAIN SELECT * FROM ore_x0.eventos_ts WHERE PHOENIX_ROW_TIMESTAMP() > TO_TIMESTAMP('2026-10-19 00:00:00')",
]:
    r = sql(q, mostrar=False)
    print("  ", q[8:120]); [print("        ", x[0]) for x in (r or [])]
for q in [
    "SELECT MAX(ts) FROM ore_x0.eventos_ts",
    "SELECT ts FROM ore_x0.eventos_ts ORDER BY ts DESC LIMIT 1",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts WHERE ts > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts WHERE ts > TO_TIMESTAMP('2026-10-20 10:00:00')",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts WHERE id > 100000",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts WHERE PHOENIX_ROW_TIMESTAMP() > TO_TIMESTAMP('2026-10-19 00:00:00')",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts WHERE ts > TO_TIMESTAMP('2026-08-01 00:00:00') AND ts <= TO_TIMESTAMP('2026-08-02 00:00:00')",
    "SELECT COUNT(*) FROM ore_x0.eventos_ts",
]:
    sql(q)

titulo("6. CurrentSCN como instantanea")
scn = int(time.time() * 1000)
time.sleep(1.2)
sql("UPSERT INTO ore_x0.eventos_sin VALUES (3, TO_TIMESTAMP('2020-01-01'), 'nueva')", mostrar=False)
c2 = phoenixdb.connect(URL, autocommit=True, CurrentSCN=str(scn))
k2 = c2.cursor()
k2.execute("SELECT id, v FROM ore_x0.eventos_sin ORDER BY id")
print("  tabla sin ROW_TIMESTAMP, leida en el SCN anterior a la fila 3:", k2.fetchall())
k2.execute("SELECT COUNT(*) FROM ore_x0.eventos_ts")
print("  tabla ROW_TIMESTAMP (cada fila tiene su ts como marca), en el SCN de ahora:", k2.fetchall())
print("  (fin)")
