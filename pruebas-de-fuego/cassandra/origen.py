"""Le pregunta al Cassandra directamente, sin pasar por ORE: la verdad contra la que se compara.

    python origen.py semilla   <url-admin> <rol> <clave>   → carga `ore_m6` (y `ore_m6_ajeno`), crea el rol de solo lectura
    python origen.py cuenta    <url> <cliente> <desde ISO> → cuántos pedidos de ese cliente desde esa fecha
    python origen.py meses     <url> <cliente> <desde ISO> → `año|mes|pedidos|total` de ese cliente
    python origen.py lecturas  <url>                       → `filas|suma de sensor` de la tabla grande
    python origen.py no_pagados <url>                      → cuántos pedidos NO tienen estado `pagado` (los nulos no cuentan)
    python origen.py sin_nota  <url>                       → cuántos pedidos tienen la nota nula
    python origen.py escribir  <url>                       → intenta INSERTAR; `denegada` si el rol no escribe
    python origen.py ajeno     <url>                       → intenta LEER `ore_m6_ajeno.secreta`; `denegada` si el rol no llega

Es lo que hace creíble una prueba de fuego: una copia que "casa con el origen" lo dice un cliente
que no es el del conector (aquí, el driver de DataStax para Python).

Requisitos:  pip install cassandra-driver    (y, con Python >= 3.12, `pyasyncore pyasynchat`: el driver
usa `asyncore`, que ya no viene en la biblioteca estándar, y es el reactor que habla TLS).
"""
import datetime
import decimal
import os
import ssl
import sys
import uuid
from urllib.parse import parse_qs, unquote, urlparse

from cassandra.auth import PlainTextAuthProvider
from cassandra.cluster import Cluster
from cassandra.concurrent import execute_concurrent_with_args

# El reactor `asyncore` (que en Python >= 3.12 trae el paquete `pyasyncore`) sí aplica el contexto TLS; el de `asyncio`
# de este driver no lo hace (medido: con él llegan registros TLS donde espera tramas CQL). Sin `pyasyncore`, el de
# `asyncio` basta para todo lo que no es TLS.
try:
    from cassandra.io.asyncorereactor import AsyncoreConnection as Conexion
except Exception:  # pragma: no cover
    from cassandra.io.asyncioreactor import AsyncioConnection as Conexion
from cassandra.policies import AddressTranslator

KEYSPACE = "ore_m6"
AJENO = "ore_m6_ajeno"
# Súbelo al cambiar la semilla: la rehace.
VERSION = 3

CLIENTES = 40
PEDIDOS_POR_CLIENTE = 600
LECTURAS = 150_000
ESTADOS = ["abierto", "pagado", "anulado"]
INICIO = datetime.datetime(2025, 1, 1, tzinfo=datetime.timezone.utc)


class SoloContacto(AddressTranslator):
    """Tras Docker, NAT o un balanceador el nodo anuncia una dirección que no se alcanza: todo va al contacto."""

    def __init__(self, destino: str):
        self.destino = destino

    def translate(self, addr):
        return self.destino


def conectar(url: str) -> Cluster:
    u = urlparse(url)
    q = parse_qs(u.query)
    host = (u.hostname or "127.0.0.1")
    # Con `tls=true`, el cliente verifica el certificado: confía en el almacén del sistema y en `SSL_CERT_FILE`.
    contexto = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE")) if q.get("tls") == ["true"] else None
    c = Cluster(
        [host],
        ssl_context=contexto,
        port=u.port or 9042,
        auth_provider=PlainTextAuthProvider(unquote(u.username), unquote(u.password)) if u.username else None,
        connection_class=Conexion,
        address_translator=SoloContacto(host) if q.get("contacto") == ["solo"] else None,
        protocol_version=5,
        connect_timeout=20,
    )
    return c


def fecha(iso: str) -> datetime.datetime:
    return datetime.datetime.fromisoformat(iso.replace("Z", "+00:00"))


# ── la semilla ──────────────────────────────────────────────────────────────

def pedido(c: int, k: int):
    """Un pedido determinista: el cliente `c`, el `k`-ésimo. Cada 17 horas desde INICIO."""
    f = INICIO + datetime.timedelta(hours=17 * k)
    importe = decimal.Decimal((c * 7919 + k * 104729) % 100000) / 100
    estado = ESTADOS[(c + k) % 3]
    nota = None if k % 5 == 0 else f"n{c}-{k}"
    return c, f, uuid.uuid1(), importe, estado, nota


def semilla(url: str, rol: str, clave: str) -> None:
    cl = conectar(url)
    s = cl.connect()
    s.execute(f"CREATE KEYSPACE IF NOT EXISTS {KEYSPACE} WITH replication = {{'class': 'SimpleStrategy', 'replication_factor': 1}}")
    s.execute(f"CREATE TABLE IF NOT EXISTS {KEYSPACE}.marcas (k text PRIMARY KEY, v int)")
    hecha = s.execute(f"SELECT v FROM {KEYSPACE}.marcas WHERE k = 'semilla'").one()
    if not (hecha and hecha.v == VERSION):
        for t in ("pedidos", "lecturas", "mezcla", "estaticas", "contadores"):
            s.execute(f"DROP TABLE IF EXISTS {KEYSPACE}.{t}")
        s.execute(
            f"CREATE TABLE {KEYSPACE}.pedidos (cliente int, fecha timestamp, id timeuuid, importe decimal, "
            f"estado text, nota text, PRIMARY KEY ((cliente), fecha, id)) WITH CLUSTERING ORDER BY (fecha ASC, id ASC)"
        )
        s.execute(f"CREATE TABLE {KEYSPACE}.lecturas (id bigint PRIMARY KEY, sensor int, valor double, instante timestamp)")
        s.execute("CREATE TYPE IF NOT EXISTS %s.direccion (calle text, num int)" % KEYSPACE)
        s.execute(
            f"CREATE TABLE {KEYSPACE}.mezcla (id int PRIMARY KEY, bin blob, etiquetas list<text>, extras map<text,int>, "
            f"dir frozen<direccion>, grande varint, dur duration, ip inet, u uuid)"
        )
        s.execute(f"CREATE TABLE {KEYSPACE}.estaticas (p int, c int, s text STATIC, v text, PRIMARY KEY (p, c))")
        s.execute(f"CREATE TABLE {KEYSPACE}.contadores (k text PRIMARY KEY, c counter)")

        ins = s.prepare(f"INSERT INTO {KEYSPACE}.pedidos (cliente, fecha, id, importe, estado, nota) VALUES (?, ?, ?, ?, ?, ?)")
        execute_concurrent_with_args(
            s, ins, [pedido(c, k) for c in range(1, CLIENTES + 1) for k in range(PEDIDOS_POR_CLIENTE)], concurrency=128
        )
        ins = s.prepare(f"INSERT INTO {KEYSPACE}.lecturas (id, sensor, valor, instante) VALUES (?, ?, ?, ?)")
        execute_concurrent_with_args(
            s,
            ins,
            [(i, i % 50, (i % 1000) / 8.0, INICIO + datetime.timedelta(seconds=i)) for i in range(1, LECTURAS + 1)],
            concurrency=128,
        )
        s.execute(
            f"INSERT INTO {KEYSPACE}.mezcla (id, bin, etiquetas, extras, dir, grande, dur, ip, u) VALUES "
            f"(1, 0xCAFE, ['a','b'], {{'k': 1}}, {{calle: 'Sol', num: 2}}, 123456789012345678901234567890, 1y2mo3d, '10.0.0.1', "
            f"11111111-1111-1111-1111-111111111111)"
        )
        s.execute(f"INSERT INTO {KEYSPACE}.estaticas (p, c, s, v) VALUES (1, 1, 'est', 'v1')")
        s.execute(f"INSERT INTO {KEYSPACE}.estaticas (p, c, s, v) VALUES (1, 2, 'est', 'v2')")
        # Una partición que solo tiene la columna estática: no es una fila de la tabla.
        s.execute(f"INSERT INTO {KEYSPACE}.estaticas (p, s) VALUES (2, 'solo estatica')")
        s.execute(f"UPDATE {KEYSPACE}.contadores SET c = c + 5 WHERE k = 'a'")
        s.execute(f"INSERT INTO {KEYSPACE}.marcas (k, v) VALUES ('semilla', {VERSION})")

    # Lo que el rol NO debe ver: un keyspace al que no se le concede nada.
    s.execute(f"CREATE KEYSPACE IF NOT EXISTS {AJENO} WITH replication = {{'class': 'SimpleStrategy', 'replication_factor': 1}}")
    s.execute(f"CREATE TABLE IF NOT EXISTS {AJENO}.secreta (id int PRIMARY KEY, v text)")
    s.execute(f"INSERT INTO {AJENO}.secreta (id, v) VALUES (1, 'no se ve')")

    # El rol de solo lectura: SELECT sobre UN keyspace, y nada más.
    s.execute(f"CREATE ROLE IF NOT EXISTS {rol} WITH PASSWORD = '{clave}' AND LOGIN = true")
    s.execute(f"GRANT SELECT ON KEYSPACE {KEYSPACE} TO {rol}")
    cl.shutdown()
    print("semilla lista")


# ── las preguntas ───────────────────────────────────────────────────────────

def pedidos_de(s, cliente: int, desde: datetime.datetime):
    return s.execute(
        f"SELECT fecha, importe FROM {KEYSPACE}.pedidos WHERE cliente = %s AND fecha >= %s", (cliente, desde)
    )


def main() -> int:
    orden, url = sys.argv[1], sys.argv[2]
    if orden == "semilla":
        semilla(url, sys.argv[3], sys.argv[4])
        return 0
    cl = conectar(url)
    s = cl.connect()
    try:
        if orden == "cuenta":
            print(sum(1 for _ in pedidos_de(s, int(sys.argv[3]), fecha(sys.argv[4]))))
        elif orden == "meses":
            acum: dict[tuple[int, int], list] = {}
            for r in pedidos_de(s, int(sys.argv[3]), fecha(sys.argv[4])):
                f = r.fecha.replace(tzinfo=datetime.timezone.utc) if r.fecha.tzinfo is None else r.fecha
                a = acum.setdefault((f.year, f.month), [0, decimal.Decimal(0)])
                a[0] += 1
                a[1] += r.importe
            for (anio, mes), (n, total) in sorted(acum.items()):
                print(f"{anio}|{mes}|{n}|{total:.2f}")
        elif orden == "lecturas":
            n = suma = 0
            for r in s.execute(f"SELECT sensor FROM {KEYSPACE}.lecturas", timeout=120):
                n += 1
                suma += r.sensor
            print(f"{n}|{suma}")
        elif orden == "no_pagados":
            # SQL: un nulo no cumple `<>`. En esta semilla `estado` nunca es nulo, pero la regla es la misma.
            print(sum(1 for r in s.execute(f"SELECT estado FROM {KEYSPACE}.pedidos", timeout=120) if r.estado is not None and r.estado != "pagado"))
        elif orden == "sin_nota":
            print(sum(1 for r in s.execute(f"SELECT nota FROM {KEYSPACE}.pedidos", timeout=120) if r.nota is None))
        elif orden == "escribir":
            from cassandra import Unauthorized

            try:
                s.execute(f"INSERT INTO {KEYSPACE}.marcas (k, v) VALUES ('no_debe_existir', 1)")
            except Unauthorized:
                print("denegada")
                return 0
            print("ESCRIBIÓ")
            return 1
        elif orden == "ajeno":
            from cassandra import Unauthorized

            try:
                s.execute(f"SELECT * FROM {AJENO}.secreta")
            except Unauthorized:
                print("denegada")
                return 0
            print("LEYÓ")
            return 1
        else:
            print(f"orden desconocida: {orden}", file=sys.stderr)
            return 2
    finally:
        cl.shutdown()
    return 0


if __name__ == "__main__":
    sys.exit(main())
