#!/usr/bin/env python3
"""LA PRUEBA DE LA ORG DE MENTIRA (ADR 0083 paso 5): que reproduce lo que X0 midió.

Una org de mentira que deriva de la de verdad es peor que no tenerla: da verde a un conector que contra
Salesforce falla. Aquí se fijan, sin red ni cargo, las rarezas medidas (ADR 0083 §3) con las que está escrita:

    python3 pruebas-de-fuego/prueba-salesforce-de-mentira.py
"""
import importlib.util
import os
import sys
import unittest
from decimal import Decimal

AQUI = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("mentira", os.path.join(AQUI, "salesforce-de-mentira.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def sembrar():
    m.Estado.objetos.clear()
    campos = [
        {"name": "Industry", "type": "picklist", "label": "Industry", "nillable": True, "filterable": True, "sortable": True},
        {"name": "N", "type": "int", "label": "N", "nillable": True, "filterable": True, "sortable": True},
        {"name": "T", "type": "datetime", "label": "T", "nillable": True, "filterable": True, "sortable": True},
        {"name": "Largo", "type": "textarea", "label": "Largo", "nillable": True, "filterable": False, "sortable": False},
    ]
    o = m.Objeto("Cuenta__c", campos)
    for nombre, ind, n, t in [
        ("Ana", "Banking", "5", "2026-10-31T20:39:17Z"),
        ("ana", "banking", "7", "2026-10-31T20:39:17.500Z"),
        ("Luis", None, None, None),
        ("", "Energy", "5", "2000-02-29T12:30:00Z"),
    ]:
        h = m.Manejador.alta  # el método no necesita servidor
        h(None, o, {"Name": nombre, "Industry": ind, "N": n, "T": t})
    m.Estado.objetos["cuenta__c"] = o
    return o


def ids(soql, todos=False):
    q, idx = m.ejecutar(soql, todos)
    return [q.obj.fila(i)["Name"] for i in idx]


class Rarezas(unittest.TestCase):
    def setUp(self):
        sembrar()

    def test_igual_y_like_no_distinguen_mayusculas_B2(self):
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE Name = 'ANA'"), ["Ana", "ana"])
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE Name LIKE 'a%'"), ["Ana", "ana"])
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE Industry IN ('BANKING')"), ["Ana", "ana"])

    def test_distinto_incluye_los_nulos_B3(self):
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE N != 5"), ["ana", "Luis"])
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE N != 5 AND N != NULL"), ["ana"])

    def test_el_orden_pone_los_nulos_primero_en_los_dos_sentidos_B5(self):
        self.assertEqual(ids("SELECT Name FROM Cuenta__c ORDER BY N ASC")[0], "Luis")
        self.assertEqual(ids("SELECT Name FROM Cuenta__c ORDER BY N DESC")[0], "Luis")
        self.assertEqual(ids("SELECT Name FROM Cuenta__c ORDER BY N ASC NULLS LAST")[-1], "Luis")
        self.assertEqual(ids("SELECT Name FROM Cuenta__c ORDER BY N DESC NULLS LAST")[-1], "Luis")

    def test_not_solo_entre_parentesis_y_and_or_no_se_mezclan(self):
        with self.assertRaises(m.Falla) as e:
            ids("SELECT Name FROM Cuenta__c WHERE NOT N = 5")
        self.assertEqual(e.exception.codigo, "MALFORMED_QUERY")
        # Un nulo no cumple `N = 5`, así que sí cumple su negación.
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE NOT (N = 5)"), ["ana", "Luis"])
        with self.assertRaises(m.Falla):
            ids("SELECT Name FROM Cuenta__c WHERE N = 5 AND Name = 'a' OR N = 7")
        self.assertEqual(len(ids("SELECT Name FROM Cuenta__c WHERE (N = 5 AND Name = 'ana') OR N = 7")), 2)

    def test_un_campo_no_filtrable_o_no_ordenable_es_un_error_de_verdad(self):
        for q in ("SELECT Name FROM Cuenta__c WHERE Largo = 'x'", "SELECT Name FROM Cuenta__c ORDER BY Largo"):
            with self.assertRaises(m.Falla) as e:
                ids(q)
            self.assertEqual(e.exception.codigo, "INVALID_FIELD")
            # El mensaje real empieza por un salto de línea.
            self.assertTrue(e.exception.mensaje.startswith("\n"))

    def test_un_datetime_guarda_segundos_I1(self):
        o = m.Estado.objetos["cuenta__c"]
        # `…17.500Z` se guardó como `…17.000`.
        self.assertEqual(m.texto_dt(o.filas[1]["T"], "+0000"), "2026-10-31T20:39:17.000+0000")

    def test_un_literal_ignora_la_fraccion_de_segundo_I2(self):
        # Las dos Ana (…17.000 y …17.500 guardado como …17.000) casan con `= …17.999Z`.
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE T = 2026-10-31T20:39:17.999Z"), ["Ana", "ana"])
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE T > 2026-10-31T20:39:17.500Z"), [])
        self.assertEqual(ids("SELECT Name FROM Cuenta__c WHERE T < 2026-10-31T20:39:17.999Z"), ["n-4"])  # la de 2000 (su Name '' es nulo y toma el de por defecto)
        # …y los microsegundos no son error.
        self.assertEqual(len(ids("SELECT Name FROM Cuenta__c WHERE T >= 2026-10-31T20:39:17.999999Z")), 2)

    def test_la_papelera_y_queryall(self):
        o = m.Estado.objetos["cuenta__c"]
        o.filas[2]["IsDeleted"] = True
        self.assertNotIn("Luis", ids("SELECT Name FROM Cuenta__c"))
        self.assertIn("Luis", ids("SELECT Name FROM Cuenta__c", todos=True))

    def test_lo_que_una_org_rechaza_al_guardar_falla(self):
        c = {"name": "Importe", "type": "currency", "precision": 12, "scale": 2}
        self.assertEqual(m.guardar(c, "9999999999.99"), Decimal("9999999999.99"))
        with self.assertRaises(m.Falla):
            m.guardar(c, "99999999999.99")  # no cabe en (12, 2)
        with self.assertRaises(m.Falla):
            m.guardar({"name": "I", "type": "int"}, "2147483648")
        self.assertIsNone(m.guardar({"name": "S", "type": "string", "length": 5}, ""))  # '' es nulo
        with self.assertRaises(m.Falla):
            m.guardar({"name": "S", "type": "string", "length": 3}, "largo")


class Formatos(unittest.TestCase):
    def test_numeros_como_los_escribe_java(self):
        self.assertEqual(m.java_double(20.0), "20.0")
        self.assertEqual(m.java_double(-1e10), "-1.0E10")
        self.assertEqual(m.java_double(0.5), "0.5")
        self.assertEqual(m.java_double(1.25e-5), "1.25E-5")
        self.assertEqual(m.decimal_texto(Decimal("100.00")), "100.0")
        self.assertEqual(m.decimal_texto(Decimal("-2.50")), "-2.5")
        self.assertEqual(m.decimal_texto(Decimal("0.00")), "0.0")

    def test_bulk_escribe_el_nulo_entrecomillado_G1(self):
        self.assertEqual(m.celda_csv(None), '""')
        self.assertEqual(m.celda_csv('a,"b"'), '"a,""b"""')
        self.assertEqual(m.celda_csv("x\ny"), '"x\ny"')
        self.assertEqual(m.celda_csv(""), "")

    def test_las_fechas_rest_y_bulk(self):
        ms = m.ms_de("2026-10-31T20:39:17Z")
        self.assertEqual(m.texto_dt(ms, "+0000"), "2026-10-31T20:39:17.000+0000")
        self.assertEqual(m.texto_dt(ms, "Z"), "2026-10-31T20:39:17.000Z")
        self.assertEqual(m.ms_de("2026-11-01T00:00:00+02:00"), m.ms_de("2026-10-31T22:00:00Z"))
        self.assertEqual(m.texto_http(0), "Thu, 01 Jan 1970 00:00:00 GMT")


if __name__ == "__main__":
    unittest.main(verbosity=1)
