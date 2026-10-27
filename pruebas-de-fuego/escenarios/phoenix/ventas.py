"""Escenarios de cliente · Phoenix (0071): tres clientes con tres formas de usar Phoenix.

    python -I -X utf8 ventas.py [http://IP:8765/]

  A. VENTAS      un esquema de tienda: pedidos con ROW_TIMESTAMP (el incremental), lineas con clave compuesta, un
                 indice secundario cubierto, una vista y una tabla de clientes con DECIMAL/DATE/ARRAY.
  B. SaaS        una tabla MULTITENANT con tres inquilinos, cada uno con su vista (TENANT_ID en la conexion).
  C. LEGADO      una tabla SALTED en el esquema por defecto, nombres en minusculas citados, sin tipos finos.

phoenixdb importa gssapi aunque no se use Kerberos; en Windows no hay KfW, y se le pone un doble.
"""
import random
import sys
import types

sys.modules.setdefault("gssapi", types.ModuleType("gssapi"))
_m = types.ModuleType("requests_gssapi")
_m.HTTPSPNEGOAuth = object
sys.modules.setdefault("requests_gssapi", _m)
import phoenixdb  # noqa: E402

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8765/"
R = random.Random(42)
ESTADOS = ["nuevo", "pagado", "enviado", "entregado", "devuelto"]
CIUDADES = ["Madrid", "Barcelona", "Valencia", "Sevilla", "Bilbao"]


def conectar(**props):
    return phoenixdb.connect(URL, autocommit=True, **props)


def lotes(cur, sentencia, filas, n=500):
    for i in range(0, len(filas), n):
        cur.executemany(sentencia, filas[i:i + n])


c = conectar()
cur = c.cursor()

# ── A · VENTAS ───────────────────────────────────────────────────────────────
cur.execute("""CREATE TABLE IF NOT EXISTS ventas.clientes (
    id BIGINT NOT NULL PRIMARY KEY, nombre VARCHAR, email VARCHAR, ciudad VARCHAR,
    alta DATE, saldo DECIMAL(12,2), etiquetas VARCHAR ARRAY)""")
cur.execute("""CREATE TABLE IF NOT EXISTS ventas.pedidos (
    creado TIMESTAMP NOT NULL, id BIGINT NOT NULL, cliente_id BIGINT, estado VARCHAR,
    total DECIMAL(12,2), envio_gratis BOOLEAN, nota VARCHAR,
    CONSTRAINT pk PRIMARY KEY (creado ROW_TIMESTAMP, id)) IMMUTABLE_ROWS = true""")
cur.execute("""CREATE TABLE IF NOT EXISTS ventas.lineas (
    pedido_id BIGINT NOT NULL, linea INTEGER NOT NULL, sku VARCHAR, cantidad SMALLINT, precio DECIMAL(10,2),
    CONSTRAINT pk PRIMARY KEY (pedido_id, linea))""")
cur.execute("CREATE INDEX IF NOT EXISTS idx_pedidos_cliente ON ventas.pedidos (cliente_id) INCLUDE (total, estado)")
cur.execute("CREATE VIEW IF NOT EXISTS ventas.pedidos_pagados AS SELECT * FROM ventas.pedidos WHERE estado = 'pagado'")

cur.execute("SELECT COUNT(*) FROM ventas.clientes")
if cur.fetchone()[0] == 0:
    clientes = [(i, f"Cliente {i}", f"c{i}@ejemplo.es", CIUDADES[i % 5], f"2025-{1 + i % 12:02d}-{1 + i % 28:02d}",
                 round(R.uniform(-50, 900), 2), [f"seg{i % 4}", "web"]) for i in range(1, 2001)]
    lotes(cur, "UPSERT INTO ventas.clientes VALUES (?, ?, ?, ?, TO_DATE(?), ?, ?)", clientes)
    pedidos, lineas = [], []
    for i in range(1, 15001):
        dia = 1 + i % 28
        pedidos.append((f"2026-09-{dia:02d} {i % 24:02d}:{i % 60:02d}:00", i, 1 + R.randrange(2000), R.choice(ESTADOS),
                        round(R.uniform(5, 800), 2), i % 3 == 0, None if i % 5 else "regalo"))
        for n in range(1 + i % 4):
            lineas.append((i, n + 1, f"SKU-{R.randrange(300):04d}", 1 + R.randrange(5), round(R.uniform(1, 200), 2)))
    lotes(cur, "UPSERT INTO ventas.pedidos VALUES (TO_TIMESTAMP(?), ?, ?, ?, ?, ?, ?)", pedidos)
    lotes(cur, "UPSERT INTO ventas.lineas VALUES (?, ?, ?, ?, ?)", lineas)

# ── B · SaaS multitenant ─────────────────────────────────────────────────────
cur.execute("""CREATE TABLE IF NOT EXISTS saas.eventos (
    inquilino VARCHAR NOT NULL, id BIGINT NOT NULL, usuario VARCHAR, accion VARCHAR, ts TIMESTAMP,
    CONSTRAINT pk PRIMARY KEY (inquilino, id)) MULTI_TENANT = true""")
cur.execute("SELECT COUNT(*) FROM saas.eventos")
if cur.fetchone()[0] == 0:
    ev = [(t, i, f"u{i % 50}", R.choice(["login", "compra", "baja", "consulta"]), f"2026-10-{1 + i % 20:02d} 10:00:00")
          for t in ("acme", "globex", "initech") for i in range(1, 1501)]
    lotes(cur, "UPSERT INTO saas.eventos VALUES (?, ?, ?, ?, TO_TIMESTAMP(?))", ev)
for t in ("acme", "globex", "initech"):
    ct = conectar(TenantId=t)
    ct.cursor().execute(f"CREATE VIEW IF NOT EXISTS saas.eventos_{t} AS SELECT * FROM saas.eventos")
    ct.close()

# ── C · LEGADO ───────────────────────────────────────────────────────────────
cur.execute("""CREATE TABLE IF NOT EXISTS "Facturas" (
    "num" VARCHAR NOT NULL PRIMARY KEY, "Importe" DOUBLE, "pagada" BOOLEAN, "emitida" TIMESTAMP)
    SALT_BUCKETS = 4""")
cur.execute('SELECT COUNT(*) FROM "Facturas"')
if cur.fetchone()[0] == 0:
    f = [(f"F{i:06d}", round(R.uniform(10, 5000), 2), i % 2 == 0, f"2025-{1 + i % 12:02d}-{1 + i % 28:02d} 09:00:00")
         for i in range(1, 3001)]
    lotes(cur, 'UPSERT INTO "Facturas" VALUES (?, ?, ?, TO_TIMESTAMP(?))', f)

for t in ("ventas.clientes", "ventas.pedidos", "ventas.lineas", "saas.eventos", '"Facturas"'):
    cur.execute(f"SELECT COUNT(*) FROM {t}")
    print(t, cur.fetchone()[0])
