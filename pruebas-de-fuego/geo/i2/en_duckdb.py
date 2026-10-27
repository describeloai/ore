"""ORE 0072 I2·2 · el SQL del índice, ejecutado en DuckDB de verdad (Linux, `spatial`), sobre una fuente pequeña
hecha a medida: puntos, una línea de ~50 km, una franja hueca, un cuadrado, una geometría nula y un borrado por
posición, en dos ficheros. El SQL lo escribe Rust (`indice_sql::pruebas::escribe_el_sql_de_ejemplo`, con
ORE_I2_SQL) en `sql/`, con las claves Hilbert que da Rust para unos puntos.

  docker run --rm -v <i2>/sql:/sql -v <i2>:/i2 python:3.12-slim sh -c "pip -q install duckdb && python -I /i2/en_duckdb.py"

Comprueba: I-1 (filas y suma de ids de la fuente = las del índice), las columnas y sus ids de campo, lo partido
(trozos 0..n−1, uno 0 por fila), la nula dentro, el orden Hilbert en el fichero, el id de una posición y que la
clave de DuckDB sea la de Rust. Emite «I2|…» y sale con 1 si algo falla.
"""
import glob
import json
import os
import sys

import duckdb

os.makedirs("/datos", exist_ok=True)
os.makedirs("/trabajo/indice", exist_ok=True)
fallos = []


def di(que, ok, **kw):
    print("I2|%s|%s|%s" % (que, "ok" if ok else "FALLA", json.dumps(kw, default=str)), flush=True)
    if not ok:
        fallos.append(que)


con = duckdb.connect()
con.execute("INSTALL spatial; LOAD spatial;")
print("duckdb", duckdb.__version__, flush=True)

# ── la fuente: dos ficheros (4326) y un borrado por posición ────────────────────────────────────
con.execute("""
COPY (SELECT * FROM (VALUES
  (ST_AsWKB(ST_Point(-73.98, 40.75)), 10.5, 1),
  (ST_AsWKB(ST_Point(-73.97, 40.76)), 11.0, 2),   -- borrada
  (ST_AsWKB(ST_Point(-73.99, 40.74)), 12.0, 3),
  (ST_AsWKB(ST_Point(-74.00, 40.73)), 13.0, 4),
  (ST_AsWKB(ST_GeomFromText('LINESTRING(-74.25 40.55, -73.75 40.85)')), 50.0, 5),
  (NULL, 0.0, 6)
) t(geom, tarifa, cuando)) TO '/datos/a.parquet' (FORMAT parquet);
COPY (SELECT * FROM (VALUES
  (ST_AsWKB(ST_Point(-73.95, 40.78)), 7.0, 7),
  (ST_AsWKB(ST_Point(-73.94, 40.79)), 8.0, 8),
  (ST_AsWKB(ST_Point(-73.93, 40.80)), 9.0, 9),
  (ST_AsWKB(ST_GeomFromText('POLYGON((-74.20 40.60, -74.19 40.60, -73.80 40.84, -73.81 40.84, -74.20 40.60))')), 1.0, 10),
  (ST_AsWKB(ST_GeomFromText('POLYGON((-73.96 40.70, -73.95 40.70, -73.95 40.71, -73.96 40.71, -73.96 40.70))')), 2.0, 11)
) t(geom, tarifa, cuando)) TO '/datos/b.parquet' (FORMAT parquet);
COPY (SELECT 'gs://cubo/t/viajes/data/a.parquet' AS file_path, 1::BIGINT AS pos) TO '/datos/borrados.parquet' (FORMAT parquet);
""")

rust = json.load(open("/sql/rust.json"))
sql = lambda n: open("/sql/" + n).read()

# 1 · la vista de la fuente
con.execute(sql("1-lectura.sql"))

# 2 · extensión y cuenta
xmin, ymin, xmax, ymax, filas_f, suma_f = con.execute(sql("2-extension.sql")).fetchone()
esperados = [i for i in (0, 2, 3, 4, 5)] + [(1 << 40) + i for i in range(5)]
di("cuenta_de_la_fuente", (filas_f, suma_f) == (len(esperados), sum(esperados) % 2 ** 64),
   filas=filas_f, suma=suma_f, esperadas=len(esperados))
r = rust["rejilla"]
di("extension_dentro_de_la_rejilla", r[0] <= xmin and r[1] <= ymin and xmax <= r[2] and ymax <= r[3],
   extension=[xmin, ymin, xmax, ymax], rejilla=r)

# 3 · el índice
con.execute(sql("3-indice.sql"))
ficheros = sorted(glob.glob("/trabajo/indice/*.parquet"))
di("escrito", len(ficheros) >= 1, ficheros=[os.path.basename(f) for f in ficheros])

# 4 · cuenta del índice: I-1
filas_i, suma_i = con.execute(sql("4-cuenta.sql")).fetchone()
di("I-1", (filas_i, suma_i) == (filas_f, suma_f), indice=[filas_i, suma_i], fuente=[filas_f, suma_f])

# las columnas y sus ids de campo
f0 = ficheros[0]
esq = con.execute(f"SELECT name, field_id FROM parquet_schema('{f0}') WHERE name <> 'duckdb_schema'").fetchall()
nombres = [n for n, _ in esq]
di("columnas", nombres[:7] == ["_ore_id", "_ore_fichero", "_ore_fila", "_ore_clave", "_ore_trozo", "geom", "geom_bbox"]
   and "tarifa" in nombres and "cuando" in nombres, columnas=esq)
di("ids_de_campo", all(i is not None for _, i in esq), sin_id=[n for n, i in esq if i is None])

t = f"read_parquet('/trabajo/indice/*.parquet', file_row_number = true, filename = true)"
# lo partido: la línea (ordinal 0, fila 4) y la franja (ordinal 1, fila 3); el cuadrado y los puntos, no
trozos = dict(con.execute(f"SELECT _ore_fichero || '/' || _ore_fila, list(_ore_trozo ORDER BY _ore_trozo) FROM {t} GROUP BY 1").fetchall())
linea, franja, cuadrado = trozos.get("0/4", []), trozos.get("1/3", []), trozos.get("1/4", [])
di("se_parte_la_linea_larga", len(linea) > 1 and linea == list(range(len(linea))), trozos=len(linea))
di("se_parte_la_franja_hueca", len(franja) > 1 and franja == list(range(len(franja))), trozos=len(franja))
di("no_se_parte_el_cuadrado", cuadrado == [0], trozos=cuadrado)
di("la_borrada_no_esta", "0/1" not in trozos, filas=sorted(trozos))
nula = con.execute(f"SELECT geom IS NULL, geom_bbox IS NULL, _ore_trozo FROM {t} WHERE _ore_fichero = 0 AND _ore_fila = 5").fetchall()
di("la_nula_esta_y_sin_caja", nula == [(True, True, 0)], nula=nula)
# las cajas de los trozos caben en una celda del suelo (z13 ≈ 4 892 m, más el borde)
ancho = con.execute(f"SELECT max(geom_bbox.xmax - geom_bbox.xmin), max(geom_bbox.ymax - geom_bbox.ymin) FROM {t} WHERE _ore_fichero || '/' || _ore_fila IN ('0/4', '1/3')").fetchone()
di("trozos_de_una_celda", max(ancho) <= 4892.0, ancho=ancho)

# el id de una posición: el de Rust
id12 = con.execute(f"SELECT _ore_id FROM {t} WHERE _ore_fichero = 1 AND _ore_fila = 2").fetchone()[0]
di("id_de_la_posicion", id12 == rust["id_1_2"], duckdb=id12, rust=rust["id_1_2"])

# el orden: la clave Hilbert de cada fila, no decreciente en el orden del fichero (nulas al final)
caja = "ST_MakeBox2D(ST_Point(%r, %r), ST_Point(%r, %r))" % tuple(r)
claves = [k for (k,) in con.execute(f"""
  SELECT CASE WHEN geom_bbox IS NULL THEN NULL ELSE
    ST_Hilbert(ST_Point(greatest(least((geom_bbox.xmin + geom_bbox.xmax) / 2, {r[2]!r}), {r[0]!r}),
                        greatest(least((geom_bbox.ymin + geom_bbox.ymax) / 2, {r[3]!r}), {r[1]!r})), {caja}) END
  FROM {t} ORDER BY filename, file_row_number""").fetchall()]
llenas = [k for k in claves if k is not None]
di("orden_hilbert", llenas == sorted(llenas) and claves[len(llenas):] == [None] * (len(claves) - len(llenas)),
   claves=claves)

# la clave de DuckDB frente a la de Rust (Rejilla::clave, 16 bits)
for x, y, k in rust["claves"]:
    d = con.execute(f"SELECT ST_Hilbert(ST_Point({x!r}, {y!r}), {caja})").fetchone()[0]
    di("hilbert_rust_igual_a_duckdb", d == k, punto=[x, y], duckdb=d, rust=k)

# la profundidad de lo escrito, con la vara de I0·d
prof, grupos = con.execute(sql("5-profundidad.sql")).fetchone()
di("profundidad", prof is not None and 1 <= prof <= grupos, profundidad=prof, grupos=grupos)

print("I2|resumen|%s|%s" % ("ok" if not fallos else "FALLA", json.dumps(fallos)), flush=True)
sys.exit(1 if fallos else 0)
