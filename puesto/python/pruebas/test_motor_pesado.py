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


class ElSqlPesado(unittest.TestCase):
    """2b · `sql()` en Heavyweight: la consulta que da ore-serve, sobre las vistas de `over()`."""

    def setUp(self):
        self.sesion = mock.MagicMock()
        self.sesion.sql.side_effect = lambda q: ("corrida", q)
        p = mock.patch.object(pesado, "sesion", return_value=self.sesion)
        p.start()
        self.addCleanup(p.stop)

    def test_registra_las_vistas_con_over_y_corre_la_reescrita(self):
        respuesta = {"spark": {"consulta": "select * from `ore__geo__default__lugares` l",
                               "vistas": [["`ore__geo__default__lugares`", "geo.lugares"]]}}
        df = mock.MagicMock()
        with mock.patch.object(ore.session, "id", "trabajo-ana-1"), \
                mock.patch.object(ore.session, "pedir", return_value=(200, respuesta)) as pedir, \
                mock.patch.object(pesado, "over", return_value=df) as over:
            r = pesado.sql("select * from geo.lugares l")
        pedir.assert_called_once_with("POST", "/puestos/trabajo-ana-1/sql",
                                      {"texto": "select * from geo.lugares l", "para": "spark"})
        over.assert_called_once_with("geo.lugares")
        df.createOrReplaceTempView.assert_called_once_with("ore__geo__default__lugares")
        self.assertEqual(r, ("corrida", "select * from `ore__geo__default__lugares` l"))

    def test_sin_puesto_o_sin_nombres_es_spark_tal_cual(self):
        with mock.patch.object(ore.session, "id", ""), mock.patch.object(ore.session, "pedir") as pedir:
            self.assertEqual(pesado.sql("select 1 from geo.t"), ("corrida", "select 1 from geo.t"))
        with mock.patch.object(ore.session, "id", "p"), mock.patch.object(ore.session, "pedir") as pedir2:
            self.assertEqual(pesado.sql("select 1"), ("corrida", "select 1"))
        pedir.assert_not_called()
        pedir2.assert_not_called()

    def test_lo_que_ore_serve_niega_se_dice(self):
        with mock.patch.object(ore.session, "id", "p"), \
                mock.patch.object(ore.session, "pedir", return_value=(422, {"error": "la consulta no se analiza"})):
            with self.assertRaisesRegex(RuntimeError, "no se analiza"):
                pesado.sql("select from where geo.t")


class ElInsertPesado(unittest.TestCase):
    """2b · `insert into` en Heavyweight: las columnas sin nombre y los tipos, de la tabla."""

    def test_los_ayudantes_de_la_celda_despachan_al_pesado(self):
        with mock.patch.dict(os.environ, {"ORE_MOTOR": "heavyweight"}), \
                mock.patch.object(pesado, "por_posicion", return_value="pp") as pp, \
                mock.patch.object(pesado, "como_la_tabla", return_value="ct") as ct:
            self.assertEqual(ore._por_posicion("df", "geo.t", [1]), "pp")
            self.assertEqual(ore._como_la_tabla("df", "geo.t"), "ct")
        pp.assert_called_once_with("df", "geo.t", [1])
        ct.assert_called_once_with("df", "geo.t")

    def test_una_columna_sin_nombre_toma_la_de_su_posicion(self):
        class Df:
            columns = ["id", "(0.5)"]

            def toDF(self, *c):
                return list(c)

        md = {"current-schema-id": 0, "schemas": [{"schema-id": 0, "fields": [
            {"name": "id", "type": "long"}, {"name": "peso", "type": "double"}]}]}
        with mock.patch.object(pesado, "_metadatos", return_value=(200, md)):
            self.assertEqual(pesado.por_posicion(Df(), "geo.t", [1]), ["id", "peso"])
        with mock.patch.object(pesado, "_metadatos", return_value=(404, {})):
            with self.assertRaisesRegex(RuntimeError, "does not exist yet"):
                pesado.por_posicion(Df(), "geo.t", [1])

    def test_el_tipo_de_spark_de_cada_primitivo(self):
        self.assertEqual(pesado._tipo_spark("long"), "bigint")
        self.assertEqual(pesado._tipo_spark("timestamp"), "timestamp_ntz")
        self.assertEqual(pesado._tipo_spark("timestamptz"), "timestamp")
        self.assertEqual(pesado._tipo_spark("decimal(10, 2)"), "decimal(10, 2)")
        self.assertIsNone(pesado._tipo_spark({"type": "struct"}))


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
