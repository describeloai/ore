"""La geometría al escribir (ORE 0067 G2·3·3).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Un GeoDataFrame va a `write()` como Arrow con la geometría en WKB y la marca
`geoarrow.wkb` (el CRS, en PROJJSON con su `id`): es lo que `ore-store`
(`carga::normalizar`) acepta y lleva al escritor geo. En la tabla Iceberg que se
esboza, la columna es `binary`. Sin geopandas, se salta.
"""

import json
import unittest

try:
    import geopandas
    import pyarrow as pa
    import shapely
except ImportError:
    geopandas = None

import ore


@unittest.skipIf(geopandas is None, "sin geopandas")
class GeoAlEscribir(unittest.TestCase):
    def test_un_geodataframe_va_como_wkb_marcado(self):
        gdf = geopandas.GeoDataFrame(
            {"id": [1, 2]}, geometry=[shapely.Point(440000, 4474000), None], crs="EPSG:25830")
        t = ore._arrow_de(gdf)
        campo = t.schema.field("geometry")
        self.assertTrue(ore._es_geo(campo))
        self.assertFalse(ore._es_geo(t.schema.field("id")))
        meta = json.loads(campo.metadata[b"ARROW:extension:metadata"])
        self.assertEqual(meta["crs"]["id"], {"authority": "EPSG", "code": 25830})
        self.assertEqual(shapely.from_wkb(t.column("geometry")[0].as_py()).x, 440000)
        self.assertIsNone(t.column("geometry")[1].as_py())
        # La marca viaja en el IPC que recibe ore-store.
        de_vuelta = pa.ipc.open_stream(ore._ipc(t)).read_all()
        self.assertTrue(ore._es_geo(de_vuelta.schema.field("geometry")))

    def test_el_esquema_que_esboza_write(self):
        # 0068 C1·1 pieza 0: una `_es_geo(t)` de texto tapó a esta y `write()`
        # fallaba con cualquier tabla. El esquema, con geo y sin geo.
        gdf = geopandas.GeoDataFrame(
            {"id": [1], "nombre": ["a"]}, geometry=[shapely.Point(1, 2)], crs="EPSG:4326")
        campos = ore._esquema_iceberg(ore._arrow_de(gdf).schema)["fields"]
        self.assertEqual([(c["name"], c["type"]) for c in campos],
                         [("id", "long"), ("nombre", "string"), ("geometry", "binary")])


if __name__ == "__main__":
    unittest.main()
