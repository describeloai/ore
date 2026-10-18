"""Un Transform SQL con geometría (ORE 0068 C1·1, pieza 3).

    bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh 'test_transform_sql_geo.py'

`create or replace dataset … as select …` es, en el puesto, `write(salida,
sql(consulta, format="arrow"))` (ore-serve escribe la celda). Lo que `sql()` da
lleva la geometría marcada con su CRS, `write()` la esboza `binary` y ore-store la
escribe geo. Un `insert` pasa por `_como_la_tabla`, que conserva la marca. Mezclar
CRS se niega al enlazar, y el cast de otro CRS se dice como ORE. Sin `spatial` o sin
shapely, se salta.
"""

import json
import os
import unittest
from unittest import mock

import ore
from ore import lsp_sql

try:
    import duckdb
    import pyarrow as pa
    import shapely

    _c = duckdb.connect()
    if os.path.isdir(ore.EXTENSIONES):
        _c.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
    _c.execute("load spatial")
    LISTO = True
except Exception:
    LISTO = False

UTM = "ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')"
WGS = "ST_SetCRS(ST_Point(-3.7, 40.4), 'EPSG:4326')"


@unittest.skipIf(not LISTO, "sin spatial o sin shapely")
class TransformSqlGeo(unittest.TestCase):
    def setUp(self):
        ore._con = None

    def test_lo_que_escribe_va_marcado_con_su_crs(self):
        t = ore.sql("select 1 as id, ST_Buffer(%s, 100) as zona" % UTM, format="arrow")
        campo = t.schema.field("zona")
        self.assertTrue(ore._es_geo(campo))
        meta = json.loads(campo.metadata[b"ARROW:extension:metadata"])
        self.assertEqual(meta["crs"]["id"], {"authority": "EPSG", "code": 25830})
        campos = ore._esquema_iceberg(t.schema)["fields"]
        self.assertEqual([(c["name"], c["type"]) for c in campos], [("id", "long"), ("zona", "binary")])
        # Y viaja así en el IPC que recibe ore-store.
        self.assertTrue(ore._es_geo(pa.ipc.open_stream(ore._ipc(t)).read_all().schema.field("zona")))

    def test_un_insert_conserva_la_marca(self):
        t = ore.sql("select 1::INTEGER as id, %s as geom" % UTM, format="arrow")
        tabla = {"metadata": {"current-schema-id": 0, "schemas": [{"schema-id": 0, "fields": [
            {"name": "id", "type": "long"}, {"name": "geom", "type": "binary"}]}]}}
        with mock.patch.object(ore.session, "pedir", return_value=(200, tabla)):
            como = ore._como_la_tabla(t, "geo.default.lugares")
        self.assertEqual(str(como.schema.field("id").type), "int64")  # al tipo de la tabla
        self.assertTrue(ore._es_geo(como.schema.field("geom")))  # y la geometría, con su marca
        self.assertEqual(como.schema.field("geom").metadata, t.schema.field("geom").metadata)

    def test_mezclar_crs_se_niega(self):
        with self.assertRaisesRegex(duckdb.Error, "(?s)two CRS.*ST_Transform.*different coordinate reference systems"):
            ore.sql("select ST_Intersects(%s, %s) as x" % (UTM, WGS), format="arrow")

    def test_el_cast_de_otro_crs_se_dice_como_ore(self):
        with self.assertRaisesRegex(duckdb.Error, r"never reprojects.*ST_Transform\(g, 'EPSG:4326', 'EPSG:25830'\)"):
            ore.sql("select %s::GEOMETRY('EPSG:25830') as g" % WGS, format="arrow")

    def test_dicho_geo_solo_lo_geo(self):
        self.assertIsNone(lsp_sql.dicho_geo("Catalog Error: Table with name x does not exist!"))


if __name__ == "__main__":
    unittest.main()
