"""Escenario de cliente · Cassandra `retail` (0070): un cluster de una tienda, sin autenticacion, detras de un
balanceador (`contacto=solo`). Imprime el CQL que lo siembra:

    python retail.py | docker exec -i cas-legado cqlsh

Cinco tablas con lo que un cliente de verdad tiene: clave de particion + clustering, UDT, colecciones, `tuple`,
`counter`, una serie temporal por (dia, hora) y una tabla vacia.
"""
import random

R = random.Random(42)
ESTADOS = ["nuevo", "pagado", "enviado", "entregado", "devuelto"]
CIUDADES = [("Calle Mayor 1", "Madrid", "28013"), ("Gran Via 22", "Barcelona", "08010"), ("Av. del Puerto 5", "Valencia", "46021")]
print("""CREATE KEYSPACE IF NOT EXISTS retail WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 1};
USE retail;
CREATE TYPE IF NOT EXISTS direccion (calle text, ciudad text, cp text);
CREATE TABLE IF NOT EXISTS pedidos (
  cliente_id int, pedido_id timeuuid, fecha timestamp, total decimal, estado text,
  etiquetas set<text>, lineas list<text>, atributos map<text,int>, envio frozen<direccion>,
  PRIMARY KEY ((cliente_id), pedido_id)) WITH CLUSTERING ORDER BY (pedido_id DESC);
CREATE TABLE IF NOT EXISTS productos (
  sku text PRIMARY KEY, nombre text, precio decimal, stock int, activo boolean, creado date, peso float,
  dimensiones tuple<int,int,int>);
CREATE TABLE IF NOT EXISTS eventos (
  dia date, hora int, id timeuuid, usuario text, accion text, PRIMARY KEY ((dia, hora), id));
CREATE TABLE IF NOT EXISTS visitas (pagina text PRIMARY KEY, n counter);
CREATE TABLE IF NOT EXISTS vacia (id int PRIMARY KEY, x text);""")
for i in range(500):
    print(f"INSERT INTO productos (sku, nombre, precio, stock, activo, creado, peso, dimensiones) VALUES "
          f"('SKU-{i:04d}', 'Producto {i}', {R.randint(100, 99999) / 100:.2f}, {R.randint(0, 500)}, {str(R.random() > .1).lower()}, "
          f"'2026-0{R.randint(1, 9)}-{R.randint(10, 28)}', {R.random() * 20:.2f}, ({R.randint(1, 90)}, {R.randint(1, 90)}, {R.randint(1, 90)}));")
for c in range(200):
    for _ in range(R.randint(10, 60)):
        calle, ciudad, cp = R.choice(CIUDADES)
        dia = R.randint(1, 28)
        lineas = ", ".join(f"'SKU-{R.randint(0, 499):04d} x{R.randint(1, 5)}'" for _ in range(R.randint(1, 4)))
        print(f"INSERT INTO pedidos (cliente_id, pedido_id, fecha, total, estado, etiquetas, lineas, atributos, envio) VALUES "
              f"({c}, now(), '2026-10-{dia:02d} 1{R.randint(0, 9)}:00:00+0000', {R.randint(500, 250000) / 100:.2f}, '{R.choice(ESTADOS)}', "
              f"{{'web', 'oferta'}}, [{lineas}], {{'regalo': {R.randint(0, 1)}, 'prioridad': {R.randint(1, 3)}}}, "
              f"{{calle: '{calle}', ciudad: '{ciudad}', cp: '{cp}'}});")
for dia in ("2026-10-19", "2026-10-20", "2026-10-21"):
    for hora in range(24):
        for _ in range(R.randint(20, 120)):
            print(f"INSERT INTO eventos (dia, hora, id, usuario, accion) VALUES ('{dia}', {hora}, now(), 'u{R.randint(1, 300)}', "
                  f"'{R.choice(['ver', 'carrito', 'comprar', 'buscar'])}');")
for p in ("/", "/ofertas", "/carrito", "/buscar", "/producto"):
    print(f"UPDATE visitas SET n = n + {R.randint(10, 5000)} WHERE pagina = '{p}';")
