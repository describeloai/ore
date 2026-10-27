"""`Binary` en el SDK de Python (OOS v1alpha35, ORE 0076).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Unos bytes son `Binary`: en DuckDB, `BLOB`; en Iceberg, `binary`; en JSON,
base64. `write()` los esboza (antes decía que el contrato no tenía `binary`) y
`create view` los describe como `Binary`, no como `Opaque`.
"""

import unittest

import pyarrow as pa

import ore
from ore import sql_functions
from ore.contrato import convertir


class BinaryEnDuckDB(unittest.TestCase):
    def test_un_blob_es_binary_y_lo_demas_opaco_sigue_siendolo(self):
        self.assertEqual(ore._oos_de_duckdb("BLOB"), "Binary")
        self.assertEqual(ore._oos_de_duckdb("BLOB[]"), "list<Binary>")
        for t in ("INTERVAL", "BIT", "JSON"):
            self.assertEqual(ore._oos_de_duckdb(t), "Opaque", t)

    def test_una_funcion_sql_con_binary_es_blob(self):
        self.assertEqual(sql_functions.duckdb_type("Binary"), "BLOB")
        self.assertEqual(sql_functions.duckdb_type("list<Binary>"), "BLOB[]")
        self.assertEqual(sql_functions.duckdb_type("Struct<id: Integer, firma: Binary>"),
                         'STRUCT("id" BIGINT, "firma" BLOB)')
        self.assertEqual(sql_functions.duckdb_type("Opaque"), "BLOB")

    def test_describe_de_duckdb_dice_binary(self):
        import duckdb

        c = duckdb.connect()
        c.execute("create table t (firma blob, otra blob[])")
        tipos = {f[0]: f[1] for f in c.execute("describe t").fetchall()}
        self.assertEqual({k: ore._oos_de_duckdb(v) for k, v in tipos.items()},
                         {"firma": "Binary", "otra": "list<Binary>"})


class BinaryAlEscribir(unittest.TestCase):
    def test_las_columnas_de_bytes_se_esbozan_binary(self):
        t = pa.table({
            "id": pa.array([1, 2], pa.int64()),
            "a": pa.array([b"\x00\x01", None], pa.binary()),
            "b": pa.array([b"x", b""], pa.large_binary()),
            "c": pa.array([b"1234", b"abcd"], pa.binary(4)),
            "d": pa.array([[b"x"], None], pa.list_(pa.binary())),
        })
        campos = {f["name"]: f["type"] for f in ore._esquema_iceberg(t.schema)["fields"]}
        self.assertEqual([campos[k] for k in "abc"], ["binary"] * 3)
        self.assertEqual(campos["d"]["type"], "list")
        self.assertEqual(campos["d"]["element"], "binary")

    def test_una_geometria_sigue_siendo_binary_y_se_reconoce_por_su_marca(self):
        sin = pa.field("g", pa.binary())
        con = pa.field("g", pa.binary(), metadata={b"ARROW:extension:name": b"geoarrow.wkb"})
        self.assertFalse(ore._es_geo(sin))
        self.assertTrue(ore._es_geo(con))
        for campo in (sin, con):
            self.assertEqual(ore._esquema_iceberg(pa.schema([campo]))["fields"][0]["type"], "binary")

    def test_binary_vuelve_a_arrow_como_binary(self):
        self.assertEqual(ore._arrow_de_iceberg("binary"), pa.binary())


class BinaryEnElContrato(unittest.TestCase):
    def test_bytes_entra_en_base64_estricto_y_vuelve_igual(self):
        self.assertEqual(convertir("firma", "3q2+7w==", bytes), bytes([0xDE, 0xAD, 0xBE, 0xEF]))
        self.assertEqual(convertir("firma", "", bytes), b"")
        with self.assertRaises(ore.contrato.ContractError):
            convertir("firma", "no es base64!", bytes)


if __name__ == "__main__":
    unittest.main()
