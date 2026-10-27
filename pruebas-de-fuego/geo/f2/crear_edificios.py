"""ORE 0072 F2·1·5 · Crear `geo.edificios` en victor: los edificios del banco en la comunidad de Madrid
(~1,33 M polígonos, EPSG:4326; lo mismo que leyó F1·3: 80 s de leer, 20 s de escribir).

⛔ Lo corre UNA PERSONA (0052: el dataset es de quien lo crea; a un agente, `ore-serve` le contesta 403).
   En un notebook o un puesto de victor, con tu sesión: pega este fichero en una celda y ejecútala.
   La base `geo` tiene que existir (el `+` de Databases).

Después: Map → Add data → geo › default › edificios, y acercarse a Madrid (las teselas, desde z13).
"""
import ore

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
MADRID = "ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0)"

con = ore._duckdb()
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
# El bucket del banco se lee con el token de Google del puesto.
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (
    ore._token_de_google().replace("'", "''"), BANCO))

edificios = con.execute(f"""
  select row_number() over () id, ST_SetCRS(geom::GEOMETRY, 'EPSG:4326') geom
  from read_parquet('{BANCO}/g22/a_tal_cual.parquet')
  where ST_Intersects(geom, {MADRID})
""").fetch_arrow_table()
print(edificios.num_rows, "edificios")

r = ore.write("geo.edificios", edificios)
print({k: v for k, v in dict(r).items() if k in ("rows", "snapshot", "geo")})
