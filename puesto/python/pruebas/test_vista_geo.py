"""El contrato de una vista con geometría (ORE 0066 E1·1, paso 1).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

`create_view` describe la consulta con DuckDB sobre tablas vacías con los tipos
del árbol. Una columna `Geometry<…, CRS>` es `GEOMETRY('CRS')`, el catálogo
carga `spatial`, y lo que sale vuelve a OOS con su CRS: `ST_Buffer` lo
conserva, `ST_Area` es `Float`, una geometría sin CRS se niega y mezclar dos se
niega. El subtipo de lo que pasa tal cual lo pone `ore-serve` (`columnasGeo`).
Sin `spatial` (fuera de la imagen, sin red), se salta.
"""

import re
import unittest
from unittest import mock

import ore
from ore import lsp_sql

try:
    import os

    import duckdb

    _c = duckdb.connect()
    # Donde las deja la imagen del puesto (con el directorio por defecto, ahí no
    # carga y esto se saltaba en silencio: 0068 C1·1 pieza 0).
    if os.path.isdir(ore.EXTENSIONES):
        _c.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
    _c.execute("load spatial")
    SPATIAL = True
except Exception:
    SPATIAL = False

INDICE = {"items": {"a": {"kind": "Dataset", "paquete": "geo", "schema": "default", "name": "lugares",
                          "expone": [{"name": "id", "type": "Integer"}, {"name": "nombre", "type": "String"},
                                     {"name": "geom", "type": "Geometry<Point, EPSG:4326>"},
                                     {"name": "utm", "type": "Geometry<Point, EPSG:25830>"},
                                     {"name": "geog", "type": "Geography<Point>"}]}}}


class TiposDeDuckdb(unittest.TestCase):
    def test_de_oos_a_duckdb(self):
        self.assertEqual(lsp_sql.tipo_duckdb("Geometry<Point, EPSG:4326>"), "GEOMETRY('EPSG:4326')")
        self.assertEqual(lsp_sql.tipo_duckdb("Geometry<LineString Z, EPSG:25830>"), "GEOMETRY('EPSG:25830')")
        self.assertEqual(lsp_sql.tipo_duckdb("Geometry"), "GEOMETRY('OGC:CRS84')")
        self.assertEqual(lsp_sql.tipo_duckdb("Geography<Point>"), "GEOMETRY('OGC:CRS84')")
        self.assertEqual(lsp_sql.tipo_duckdb("String"), "VARCHAR")

    def test_de_duckdb_a_oos(self):
        self.assertEqual(ore._oos_de_duckdb("GEOMETRY('EPSG:25830')"), "Geometry<Any, EPSG:25830>")
        self.assertEqual(ore._oos_de_duckdb("GEOMETRY('OGC:CRS84')"), "Geometry")
        self.assertEqual(ore._oos_de_duckdb("UBIGINT"), "Integer")

    def test_el_tipo_exacto_no_cambia_el_contrato(self):
        self.assertTrue(ore._mismo_tipo("Geometry<Point, EPSG:4326>", "Geometry<Any, EPSG:4326>"))
        self.assertTrue(ore._mismo_tipo("Geography<Point>", "Geometry"))
        self.assertFalse(ore._mismo_tipo("Geometry<Point, EPSG:4326>", "Geometry<Any, EPSG:25830>"))
        self.assertFalse(ore._mismo_tipo("Geometry<Point, EPSG:4326>", "String"))


@unittest.skipIf(not SPATIAL, "sin spatial")
class ContratoDeLaVista(unittest.TestCase):
    def crear(self, sql, columnas_geo=None):
        puestos = []

        def pedir(metodo, ruta, cuerpo=None, **_):
            if metodo == "GET" and ruta == "/assets":
                return 200, INDICE
            if metodo == "PUT":
                puestos.append(cuerpo["yaml"])
                return 201, {"columnasGeo": columnas_geo or {}}
            return 404, None

        with mock.patch.object(ore.session, "pedir", side_effect=pedir), \
                mock.patch.object(ore.session, "id", None, create=True):
            r = ore.create_view("geo.zonas", sql)
        return r, (puestos[0] if puestos else "")

    def test_lo_que_pasa_tal_cual_y_lo_derivado(self):
        r, yaml = self.crear("select id, geom, ST_Buffer(utm, 100) as zona, ST_Area(utm) as a, "
                             "h3_latlng_to_cell(40.4, -3.7, 9) as celda from geo.lugares",
                             columnas_geo={"geom": "Geometry<Point, EPSG:4326>"})
        self.assertEqual(r["columns"], {"id": "Integer", "geom": "Geometry<Point, EPSG:4326>",
                                        "zona": "Geometry<Any, EPSG:25830>", "a": "Float", "celda": "Integer"})
        # Lo que el SDK escribe: el CRS de DuckDB, y la versión que trae los tipos geo.
        self.assertIn("apiVersion: oos.dev/v1alpha32", yaml)
        self.assertIn('geom: { type: "Geometry<Any, EPSG:4326>" }', yaml)

    def test_geography_va_en_crs84(self):
        r, _ = self.crear("select geog from geo.lugares")
        self.assertEqual(r["columns"], {"geog": "Geometry"})

    def test_sin_crs_se_dice_como_ponerlo(self):
        with self.assertRaisesRegex(ValueError, "no CRS.*ST_SetCRS"):
            self.crear("select ST_Point(1, 2) as p from geo.lugares")
        r, _ = self.crear("select ST_SetCRS(ST_Point(1, 2), 'EPSG:4326') as p from geo.lugares")
        self.assertEqual(r["columns"], {"p": "Geometry<Any, EPSG:4326>"})

    def test_dos_crs_no_se_mezclan(self):
        with self.assertRaisesRegex(ValueError, "cannot be described.*coordinate reference systems"):
            self.crear("select ST_Intersects(geom, utm) as x from geo.lugares")

    def test_sin_geo_es_como_siempre(self):
        r, yaml = self.crear("select id, upper(nombre) as n from geo.lugares")
        self.assertEqual(r["columns"], {"id": "Integer", "n": "String"})
        self.assertIn("apiVersion: oos.dev/v1alpha14", yaml)
        self.assertIsNone(re.search(r"Geometry", yaml))


@unittest.skipIf(not SPATIAL, "sin spatial")
class ElEditor(unittest.TestCase):
    """0068 C1·1 pieza 0: los diagnósticos del editor con geometría, con el
    catálogo cargando `spatial` de donde lo deja la imagen."""

    def diagnosticos(self, sql):
        return [d.get("message", "") for d in lsp_sql.diagnosticar(sql, lsp_sql.Catalogo(INDICE))]

    def test_bien_no_dice_nada(self):
        self.assertEqual(self.diagnosticos("select ST_Buffer(utm, 100) as z from geo.default.lugares"), [])

    def test_mezclar_crs_se_subraya(self):
        d = self.diagnosticos("select ST_Intersects(geom, utm) as x from geo.default.lugares")
        # Dicho como ORE, en una línea y con su remedio (pieza 4).
        self.assertEqual(d, ["`ST_Intersects` gets geometries in two CRS, `EPSG:4326` and `EPSG:25830`, and ORE "
                             "never reprojects on its own: reproject one with "
                             "`ST_Transform(g, 'EPSG:4326', 'EPSG:25830')`"])

    def test_completa_las_funciones_geo(self):
        texto = "select st_bu"
        items, _ = lsp_sql.completar_lista(texto, len(texto), lsp_sql.Catalogo(INDICE))
        buffer = [x for x in items if x[0] == "ST_Buffer"]
        self.assertEqual(len(buffer), 1, items[:5])
        self.assertEqual(buffer[0][1], 3)  # Function
        self.assertIn("GEOMETRY", buffer[0][2])
        texto = "select h3_latlng"
        items, _ = lsp_sql.completar_lista(texto, len(texto), lsp_sql.Catalogo(INDICE))
        self.assertIn("h3_latlng_to_cell", [x[0] for x in items])

    def test_sin_st_no_ofrece_funciones(self):
        texto = "select geo"
        items, _ = lsp_sql.completar_lista(texto, len(texto), lsp_sql.Catalogo(INDICE))
        self.assertFalse([x for x in items if x[1] == 3])

    def test_explica_una_funcion_geo(self):
        texto = "select ST_Buffer(utm, 100) from geo.default.lugares"
        h = lsp_sql.explicar(texto, 0, 10, lsp_sql.Catalogo(INDICE))
        self.assertTrue(h.startswith("**ST_Buffer**"), h)
        self.assertIn("ST_Buffer(", h)

    def test_st_sin_tabla_geo_carga_spatial(self):
        self.assertEqual(self.diagnosticos("select ST_AsText(ST_Point(1, 2)) as p from geo.default.lugares"), [])


if __name__ == "__main__":
    unittest.main()
