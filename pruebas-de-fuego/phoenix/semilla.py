"""Siembra el laboratorio de Phoenix (0071): los casos que el conector tiene que entender.

    python semilla.py [http://127.0.0.1:8765/]

phoenixdb importa gssapi aunque no se use Kerberos; en Windows no hay KfW, y se le pone un doble.
"""
import sys
import types

sys.modules.setdefault("gssapi", types.ModuleType("gssapi"))
_m = types.ModuleType("requests_gssapi")
_m.HTTPSPNEGOAuth = object
sys.modules.setdefault("requests_gssapi", _m)
import phoenixdb  # noqa: E402

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8765/"


def conectar(**props):
    return phoenixdb.connect(URL, autocommit=True, **props)


c = conectar()
cur = c.cursor()


def sql(texto):
    cur.execute(texto)



# ── todos los tipos, clave compuesta ────────────────────────────────────────
sql("""CREATE TABLE IF NOT EXISTS ore_x0.tipos (
    k1 VARCHAR NOT NULL, k2 BIGINT NOT NULL,
    t_tiny TINYINT, t_small SMALLINT, t_int INTEGER, t_big BIGINT,
    t_uint UNSIGNED_INT, t_ulong UNSIGNED_LONG,
    t_float FLOAT, t_double DOUBLE, t_dec DECIMAL(12,3), t_decsin DECIMAL,
    t_bool BOOLEAN, t_var VARCHAR, t_char CHAR(3), t_bin BINARY(4), t_varbin VARBINARY,
    t_date DATE, t_time TIME, t_ts TIMESTAMP, t_uts UNSIGNED_TIMESTAMP,
    t_aint INTEGER ARRAY, t_avar VARCHAR ARRAY,
    CONSTRAINT pk PRIMARY KEY (k1, k2))""")
sql("""UPSERT INTO ore_x0.tipos VALUES ('a', 1,
    -5, 300, -70000, -9000000000, 2000000000, 9000000000000000000,
    1.5, 2.25, 12345.678, 3.14159265358979323846,
    TRUE, 'héllo ñ 日本', 'abc', 'ab', 'bytes',
    TO_DATE('2026-10-20 13:45:59'), TO_TIME('2026-10-20 13:45:59'),
    TO_TIMESTAMP('2026-10-20 13:45:59.123456789'), TO_TIMESTAMP('2026-10-20 13:45:59.5'),
    ARRAY[1,2,3], ARRAY['x','y'])""")
sql("UPSERT INTO ore_x0.tipos (k1, k2) VALUES ('b', 2)")  # todo nulo salvo la clave

# ── pedidos: tabla grande (se dobla en el servidor hasta ~262 000 filas) ─────
sql("""CREATE TABLE IF NOT EXISTS ore_x0.pedidos (
    id BIGINT NOT NULL PRIMARY KEY, cliente VARCHAR, importe DECIMAL(12,2),
    creado TIMESTAMP, estado VARCHAR)""")
# ── M3 · lo que el catálogo tiene que saber distinguir ───────────────────────
# un índice (getTables lo da como INDEX: no es una tabla; uno sobre tabla inmutable, que no pide el codec WAL de
# los índices mutables), una tabla sin esquema y una en minúsculas citada
sql("CREATE TABLE IF NOT EXISTS ore_x0.inmutable (id BIGINT NOT NULL PRIMARY KEY, estado VARCHAR) IMMUTABLE_ROWS = true")
sql("CREATE INDEX IF NOT EXISTS idx_inmutable_estado ON ore_x0.inmutable (estado)")
sql("CREATE TABLE IF NOT EXISTS suelta (id BIGINT NOT NULL PRIMARY KEY, v VARCHAR)")
sql('CREATE TABLE IF NOT EXISTS ore_x0."minusculas" (id BIGINT NOT NULL PRIMARY KEY, "Mixta" VARCHAR)')
sql("UPSERT INTO suelta VALUES (1, 'sin esquema')")
sql("""UPSERT INTO ore_x0."minusculas" VALUES (1, 'citada')""")

cur.execute("SELECT COUNT(*) FROM ore_x0.pedidos")
if cur.fetchone()[0] == 0:
    for i in range(1, 1025):
        cur.execute("UPSERT INTO ore_x0.pedidos VALUES (?, ?, ?, ?, ?)", [
            i, "cli%03d" % (i % 97), i * 1.25, None, ("abierto", "cerrado", "anulado")[i % 3]])
    n = 1024
    for _ in range(8):  # 1024 * 2^8 = 262 144
        sql("UPSERT INTO ore_x0.pedidos SELECT id + %d, cliente, importe, creado, estado FROM ore_x0.pedidos" % n)
        n *= 2

# ── tabla con sal, vista, ROW_TIMESTAMP, multi-tenant, ajeno ────────────────
sql("""CREATE TABLE IF NOT EXISTS ore_x0.salada (id BIGINT NOT NULL PRIMARY KEY, v VARCHAR) SALT_BUCKETS = 4""")
for i in range(1, 41):
    cur.execute("UPSERT INTO ore_x0.salada VALUES (?, ?)", [i, "v%d" % i])

sql("""CREATE VIEW IF NOT EXISTS ore_x0.pedidos_cerrados AS SELECT * FROM ore_x0.pedidos WHERE estado = 'cerrado'""")

sql("""CREATE TABLE IF NOT EXISTS ore_x0.eventos (
    ts TIMESTAMP NOT NULL, id BIGINT NOT NULL, dato VARCHAR,
    CONSTRAINT pk PRIMARY KEY (ts ROW_TIMESTAMP, id))""")
for i in range(1, 21):
    cur.execute("UPSERT INTO ore_x0.eventos VALUES (TO_TIMESTAMP('2026-10-%02d 10:00:00'), ?, ?)" % (i if i <= 28 else 28), [i, "e%d" % i])

sql("""CREATE TABLE IF NOT EXISTS ore_x0.inquilinos (
    tenant_id VARCHAR NOT NULL, id BIGINT NOT NULL, dato VARCHAR,
    CONSTRAINT pk PRIMARY KEY (tenant_id, id)) MULTI_TENANT = true""")
for t in ("t1", "t2"):
    ct = conectar(TenantId=t)
    cc = ct.cursor()
    cc.execute("CREATE VIEW IF NOT EXISTS ore_x0.vista_%s AS SELECT * FROM ore_x0.inquilinos" % t)
    for i in range(1, 4):
        cc.execute("UPSERT INTO ore_x0.vista_%s (id, dato) VALUES (%d, '%s-%d')" % (t, i, t, i))
    ct.close()

sql("CREATE TABLE IF NOT EXISTS ore_x0_ajeno.secretos (id BIGINT NOT NULL PRIMARY KEY, s VARCHAR)")
sql("UPSERT INTO ore_x0_ajeno.secretos VALUES (1, 'no se ve')")

cur.execute("SELECT COUNT(*) FROM ore_x0.pedidos")
print("pedidos:", cur.fetchone()[0])
print("semilla lista")
