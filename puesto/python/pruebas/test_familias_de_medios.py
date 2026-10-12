"""OOS v1alpha29 · las familias de medios en el SDK: `create_collection` admite
las familias nuevas, `archive` y `multimodal` (sin `formats`), y escribe la
colección en v1alpha29; las de siempre, en v1alpha19 como antes.
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import ore  # noqa: E402


class Familias(unittest.TestCase):
    def yaml(self, media, formatos):
        return ore._yaml_de_coleccion("lago.archivo.c", media, formatos, None, None, None, None)

    def test_las_de_siempre_siguen_en_v1alpha19(self):
        y = self.yaml("document", ["pdf"])
        self.assertIn("apiVersion: oos.dev/v1alpha19", y)
        self.assertIn("formats: [pdf]", y)

    def test_una_familia_nueva_es_de_v1alpha29(self):
        y = self.yaml("geospatial", ["geojson", "kml"])
        self.assertIn("apiVersion: oos.dev/v1alpha29", y)
        self.assertIn("media: geospatial", y)
        self.assertIn("formats: [geojson, kml]", y)

    def test_una_multimodal_sin_formatos_no_los_escribe(self):
        y = self.yaml("multimodal", [])
        self.assertIn("apiVersion: oos.dev/v1alpha29", y)
        self.assertIn("media: multimodal", y)
        self.assertNotIn("formats", y)

    def test_el_vocabulario(self):
        for m in ("structured", "text", "geospatial", "model3d", "medical", "scientific",
                  "design", "archive", "multimodal"):
            self.assertIn(m, ore.MEDIA)
        self.assertNotIn("binary", ore.MEDIA)

    def test_sin_formatos_solo_una_multimodal(self):
        with self.assertRaises(ValueError) as e:
            ore.create_collection("lago.archivo.c", "image", [])
        self.assertIn("formats", str(e.exception))
        with self.assertRaises(ValueError) as e:
            ore.create_collection("lago.archivo.c", "binary", ["bin"])
        self.assertIn("media", str(e.exception))


if __name__ == "__main__":
    unittest.main()
