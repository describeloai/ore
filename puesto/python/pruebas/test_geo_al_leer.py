"""La geometría al leer (ORE 0067 G2·3·2).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

En la tabla Iceberg la geometría es `binary` (G2·0), y DuckDB la daría como
BLOB. Con el `geo` que trae el puntero de `datos` (los tipos del árbol), la
fuente la lee como `GEOMETRY` con su CRS: las `ST_*` funcionan, Arrow sale con
la marca `geoarrow.wkb` y pandas da un GeoDataFrame con su CRS. Sin geopandas o
sin `spatial` (fuera de la imagen, sin red), se salta.
"""

import os
import tempfile
import unittest

try:
    import geopandas
    import pyarrow as pa
    import pyarrow.parquet as pq
    import shapely
except ImportError:
    geopandas = None

import ore


@unittest.skipIf(geopandas is None, "sin geopandas")
class GeoAlLeer(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.f = os.path.join(self.dir, "municipios.parquet")
        wkb = [shapely.to_wkb(shapely.box(440000, 4474000, 441000, 4475000)), None]
        pq.write_table(pa.table({"id": ["a", "b"], "geom": pa.array(wkb, pa.binary())}), self.f)
        self.r = {"geo": {"geom": {"type": "Geometry<MultiPolygon, EPSG:25830>", "crs": "EPSG:25830"}}}

    def test_sin_geo_es_el_blob_de_siempre(self):
        fuente = ore._con_geo("read_parquet('%s')" % self.f.replace("\\", "/"), {})
        t = ore._duckdb().sql("select * from %s" % fuente).fetch_arrow_table()
        self.assertEqual(str(t.schema.field("geom").type), "binary")

    def test_con_geo_es_geometria_con_su_crs(self):
        try:
            fuente = ore._con_geo("read_parquet('%s')" % self.f.replace("\\", "/"), self.r)
        except Exception as e:  # sin `spatial` ni red para instalarla
            self.skipTest("sin spatial: %s" % e)
        con = ore._duckdb()
        area = con.sql("select ST_Area(geom) from %s where id = 'a'" % fuente).fetchone()[0]
        self.assertEqual(area, 1_000_000)
        t = con.sql("select * from %s" % fuente).fetch_arrow_table()
        md = t.schema.field("geom").metadata
        self.assertEqual(md[b"ARROW:extension:name"], b"geoarrow.wkb")
        g = ore._como(t, "pandas")
        self.assertIsInstance(g, geopandas.GeoDataFrame)
        self.assertEqual(g.crs.to_epsg(), 25830)
        self.assertEqual(g.geometry.iloc[0].area, 1_000_000)
        self.assertTrue(g.geometry.iloc[1] is None or g.geometry.iloc[1].is_empty)


if __name__ == "__main__":
    unittest.main()


@unittest.skipIf(geopandas is None, "sin geopandas")
class UnaTablaEscritaPorOre(unittest.TestCase):
    """ORE 0067, medido en victor el 2026-10-17: una tabla geo que escribe `write()`
    no tiene tipos en el árbol, y el puntero llega sin `geo`. Sus ficheros son
    GEOMETRY (G2·1) y DuckDB sin `spatial` no los lee como la columna `binary` de
    Iceberg. El SDK toma entonces el `geo` de las propiedades `ore.geo.<col>`.
    La tabla de `cuerpos/geo-v3-ore` la selló el escritor de ORE (los fijos de
    EPSG:25830 de la matriz, `examples/v5-fijos.rs`)."""

    def test_sin_geo_en_el_puntero_lo_toma_de_la_tabla(self):
        import glob
        raiz = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cuerpos", "geo-v3-ore")
        ml = glob.glob(os.path.join(raiz, "metadata", "*.metadata.json"))[0].replace("\\", "/")
        r = {"metadata_location": ml}
        fuente, r = ore._fuente_de_respuesta("p.geo", r)
        self.assertEqual(r["geo"]["geom"]["crs"], "EPSG:25830")
        t = ore._duckdb().sql("select * from %s order by id" % fuente).to_arrow_table()
        self.assertEqual(t.num_rows, 177)
        g = ore._como(t, "pandas")
        self.assertIsInstance(g, geopandas.GeoDataFrame)
        self.assertEqual(g.crs.to_epsg(), 25830)
        self.assertEqual(g.geometry.iloc[0].geom_type, "Point")


@unittest.skipIf(geopandas is None, "sin geopandas")
class SqlConFuncionesGeo(unittest.TestCase):
    """ORE 0067 G3·6: `ST_*` (y `ST_Read`) o `h3_*` en `sql()` cargan `spatial` y
    `h3` aunque la consulta no toque una tabla geo (antes: «st_read is not in
    the catalog»)."""

    def test_st_y_h3_sin_tabla_geo(self):
        con = ore._duckdb()
        try:
            con.execute("load h3")
        except Exception:
            self.skipTest("sin h3 en este entorno")
        r = ore.sql("select ST_AsText(ST_Point(1, 2)) p, h3_latlng_to_cell(40.4, -3.7, 8) c", format="arrow")
        self.assertEqual(r.column("p").to_pylist(), ["POINT (1 2)"])
