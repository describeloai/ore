"""El motor Heavyweight del SDK (ORE 0068 C2·3), lo que no necesita Spark.

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Lo que corre contra Spark y el `/v1` de un ore-serve de verdad está en
`pruebas-de-fuego/geo/c2/arnes-spark/` (`medir.sh sdk.py`, 15 casos). Aquí: que el
motor lo dice el arnés y sólo él (`ORE_MOTOR`), que los mismos nombres despachan a
Spark con él y no sin él, el reparto de memoria del pod y el SRID de cada CRS.
"""

import os
import unittest
from unittest import mock

import ore
from ore import spark as pesado


class ElMotor(unittest.TestCase):
    def test_sin_ore_motor_es_el_ligero(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("ORE_MOTOR", None)
            self.assertFalse(pesado.activo())
        with mock.patch.dict(os.environ, {"ORE_MOTOR": "heavyweight"}):
            self.assertTrue(pesado.activo())

    def test_los_mismos_nombres_despachan_al_pesado(self):
        with mock.patch.dict(os.environ, {"ORE_MOTOR": "heavyweight"}), \
                mock.patch.object(pesado, "over", return_value="df") as over, \
                mock.patch.object(pesado, "sql", return_value="sdf") as sql, \
                mock.patch.object(pesado, "write", return_value={"rows": 1}) as write:
            self.assertEqual(ore.over("geo.lugares"), "df")
            self.assertEqual(ore.sql("select 1"), "sdf")
            self.assertEqual(ore.write("geo.zonas", "df"), {"rows": 1})
        over.assert_called_once_with("geo.lugares")
        sql.assert_called_once_with("select 1")
        write.assert_called_once_with("geo.zonas", "df", "overwrite")

    def test_lo_que_el_pesado_no_hace_se_dice_antes_de_spark(self):
        with mock.patch.dict(os.environ, {"ORE_MOTOR": "heavyweight"}):
            with self.assertRaises(NotImplementedError):
                ore.write("geo.zonas", "df", mode="upsert", key=["id"])

    def test_el_techo_va_antes_que_el_motor(self):
        with mock.patch.dict(os.environ, {"ORE_MOTOR": "heavyweight"}), \
                mock.patch.object(ore, "_transform", ore._Transform("z", ["geo.a"], "geo.zonas")), \
                mock.patch.object(pesado, "write") as write:
            with self.assertRaises(PermissionError):
                ore.write("geo.otra", "df")
        write.assert_not_called()


class LaMemoria(unittest.TestCase):
    """Medido (pieza 0): la JVM ocupa el heap + ~0,8 GiB; Python, ~130 MB."""

    def test_el_heap_sale_de_lo_que_mide_el_pod(self):
        for mb, heap in (("4096", "2048m"), ("49152", "38092m"), ("1024", "1024m")):
            with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": mb}):
                self.assertEqual(pesado._memoria_del_driver(), heap, mb)
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": ""}):
            self.assertEqual(pesado._memoria_del_driver(), "2g")


class ElSrid(unittest.TestCase):
    def test_de_cada_crs(self):
        self.assertEqual(pesado._srid("EPSG:25830"), 25830)
        # Sedona no conoce OGC:CRS84, y su 4326 ya es lon/lat (C2·2).
        self.assertEqual(pesado._srid("OGC:CRS84"), 4326)
        self.assertIsNone(pesado._srid("ESRI:102100"))
        self.assertIsNone(pesado._srid(None))


if __name__ == "__main__":
    unittest.main()
