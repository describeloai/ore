"""La copia de una vista con geometría (ORE 0068 C1·1, pieza 2).

    bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh 'test_calcular_geo.py'

`ore materialize --preparar` deja cada dataset en Arrow, y `volcar` marca sus
columnas geo con `geoarrow.wkb` y el CRS de su `ore.geo.<col>`. `calcular` carga
`spatial` antes de registrarlas (sin él, DuckDB las lee sin CRS, en silencio), y lo
que devuelve una consulta con `ST_*` sale marcado con su CRS para `sellar-arrow`.
Sin `spatial` o sin shapely, se salta.
"""

import json
import os
import shutil
import tempfile
import unittest

import ore
from ore import calcular

try:
    import duckdb
    import pyarrow as pa
    import pyarrow.ipc as ipc
    import shapely

    _c = duckdb.connect()
    if os.path.isdir(ore.EXTENSIONES):
        _c.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
    _c.execute("load spatial")
    LISTO = True
except Exception:
    LISTO = False


def _marcada(crs):
    return pa.field("geom", pa.binary(), metadata={b"ARROW:extension:name": b"geoarrow.wkb",
                                                    b"ARROW:extension:metadata": json.dumps({"crs": crs}).encode()})


@unittest.skipIf(not LISTO, "sin spatial o sin shapely")
class LaCopiaConGeometria(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.dir, "entradas"))

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def copia(self, consulta, columnas, entradas):
        rels = {}
        for nombre, tabla in entradas.items():
            rel = "entradas/%s.arrow" % nombre
            with ipc.new_stream(os.path.join(self.dir, rel), tabla.schema) as w:
                w.write_table(tabla)
            rels[nombre] = rel
        with open(os.path.join(self.dir, "peticion.json"), "w", encoding="utf-8") as f:
            json.dump({"consulta": consulta, "columnas": columnas, "entradas": rels,
                       "esquema_de_datasets": "__ore_dataset"}, f)
        n = calcular.calcular(self.dir)
        return n, ipc.open_stream(os.path.join(self.dir, "salida.arrow")).read_all()

    def lugares(self, crs="EPSG:25830"):
        punto = shapely.to_wkb(shapely.Point(440000, 4474000))
        return pa.table({"id": [1], "geom": pa.array([punto], pa.binary())},
                        schema=pa.schema([pa.field("id", pa.int64()), _marcada(crs)]))

    def test_st_buffer_sale_con_su_crs(self):
        n, t = self.copia('select id, ST_Buffer(geom, 100) as zona from __ore_dataset."geo.lugares"',
                          {"id": "Integer", "zona": "Geometry<Any, EPSG:25830>"},
                          {"geo.lugares": self.lugares()})
        self.assertEqual(n, 1)
        campo = t.schema.field("zona")
        self.assertTrue(ore._es_geo(campo))
        meta = json.loads(campo.metadata[b"ARROW:extension:metadata"])
        self.assertEqual(meta["crs"]["id"], {"authority": "EPSG", "code": 25830})
        # Un círculo de 100 m aproximado por segmentos: π·100² con un 1 % de margen.
        self.assertAlmostEqual(shapely.from_wkb(t.column("zona")[0].as_py()).area, 31416, delta=314)

    def test_el_crs_de_la_entrada_casa_con_el_de_la_consulta(self):
        _, t = self.copia("select ST_Intersects(geom, ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')) as x "
                          'from __ore_dataset."geo.lugares"', {"x": "Boolean"}, {"geo.lugares": self.lugares()})
        self.assertTrue(t.column("x")[0].as_py())

    def test_sin_geo_como_siempre(self):
        _, t = self.copia('select id * 2 as d from __ore_dataset."v.n"', {"d": "Integer"},
                          {"v.n": pa.table({"id": [21]})})
        self.assertEqual(t.column("d")[0].as_py(), 42)


if __name__ == "__main__":
    unittest.main()
