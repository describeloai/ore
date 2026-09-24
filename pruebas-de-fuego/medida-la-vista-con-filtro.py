"""UNA VIEW CON FILTRO, LEIDA DESDE UN PUESTO · ¿se aplica el `where` y los `fields`?

Salio al medir el espectro de quitar la regex (`medida-la-regex-fuera.py`):
`datos_de` resuelve una View a su DATASET RAIZ (`vistas::raiz_de_lectura`) y
los tres SDK hacen `create view p.v as select * from <ese dataset>`. Leido en
el codigo, nadie aplica el `where` ni los `fields` de la View. Antes de
tratarlo como un hueco mas se mide en vivo: una View con `where: {pais: ES}`
y `fields: {id, pais}` sobre `hr.ventas` (una de cada cuatro filas es ES), y
el SDK de Python de verdad leyendola por `over()` y por `sql()`.

    python pruebas-de-fuego/medida-la-vista-con-filtro.py [--filas 20000]

El banco es el de `medida-el-catalogo-como-resolutor.py` (ore-serve de verdad,
S3 de mentira, puesto reclamado), importado tal cual. No toca el cluster.
"""
import importlib.util
import os
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
if "--filas" not in sys.argv:
    sys.argv += ["--filas", "20000"]
sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
m = importlib.util.module_from_spec(sp)
sp.loader.exec_module(m)

VISTA = """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: ventasES, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  where: { pais: ES }
  fields: { id: id, pais: pais }
"""


def main():
    print("=== una View con filtro, leida desde un puesto")
    b = m.Banco()
    try:
        b.levantar()
        m.escribir(b.A + "/packages/hr/views/ventasES.yaml", VISTA)
        os.environ.update(ORE_SERVE=b.directo, PUESTO=m.PUESTO, ORE_ALMACEN="dir:" + b.tmp)
        sys.path.insert(0, m.RAIZ + "/puesto/python")
        import ore

        ore.puesto.servidor, ore.puesto.id = b.directo, m.PUESTO
        ore.puesto._cabeceras = dict(m.AGENTE)
        todas = ore.over("hr.ventas", como="arrow")
        es = sum(1 for p in todas.column("pais").to_pylist() if p == "ES")
        print()
        print("  hr.ventas: %d filas, columnas %s; de ellas pais=ES: %d" % (todas.num_rows, todas.column_names, es))
        print("  la View hr.ventasES declara where {pais: ES} y fields {id, pais}")
        print("  lo que TENDRIA que dar: %d filas, columnas ['id', 'pais']" % es)
        v = ore.over("hr.ventasES", como="arrow")
        print("  over('hr.ventasES')           → %d filas, columnas %s" % (v.num_rows, v.column_names))
        s = ore.sql("select count(*) as n from hr.ventasES", como="arrow")
        print("  sql('select count(*) … ES')   → %s" % s.column("n").to_pylist())
        mal = v.num_rows != es or v.column_names != ["id", "pais"]
        print()
        if mal:
            print("  ⛔ LA VIEW NO SE APLICA: desde un puesto, leer `hr.ventasES` es leer el dataset")
            print("    entero —todas las filas y todas las columnas, tambien las que la View no")
            print("    expone—. `datos` da el puntero de la raiz y el SDK hace `select *` sobre el.")
        else:
            print("  ✓ la View se aplica (filas y columnas)")
    finally:
        b.cerrar()
    return 0


if __name__ == "__main__":
    sys.exit(main())
