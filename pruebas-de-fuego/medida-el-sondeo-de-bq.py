# -*- coding: utf-8 -*-
"""La naturaleza del sondeo de BigQuery: que pregunta, que deriva y donde para.

Antes de sacar el primer arsenal de verdad conviene mirar por dentro la pieza
que lo produce. No es un driver: es una RECETA dentro de `ore` —`lector.rs`—
que ejecuta `bq` y traduce lo que dice.

  A. LA FORMA        una consulta, no N. Que sale de esa decision
  B. LA ITERACION    sobre que se itera de verdad, y en que orden
  C. LO QUE VARIA    que se sondea de cada tabla y que es constante
  D. DONDE PARA      los limites, incluido uno que no esta guardado
  E. EL CONTRASTE    donde vive el hecho, aqui y en Postgres
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
LECTOR = (RAIZ / "crates/ore-cli/src/lector.rs").read_text(encoding="utf-8")


def cuerpo(nombre):
    """El cuerpo de una funcion, por conteo de llaves desde su firma."""
    m = re.search(r"^fn %s\b.*?\{" % nombre, LECTOR, re.M | re.S)
    if not m:
        return ""
    i, prof = m.end(), 1
    while i < len(LECTOR) and prof:
        prof += (LECTOR[i] == "{") - (LECTOR[i] == "}")
        i += 1
    return LECTOR[m.start():i]


print("== el sondeo de BigQuery, por dentro ==")

# -- A - LA FORMA ------------------------------------------------------------
print()
print("A - LA FORMA: una consulta, no N")
q = cuerpo("consulta")
print("   llamadas a `bq` por descubrimiento : 1")
print("   CTEs de esa consulta               : %s"
      % ", ".join(re.findall(r"(\w+) AS \(", q)))
vistas_is = sorted(set(re.findall(r"INFORMATION_SCHEMA\.(\w+)", q)))
print("   vistas de `INFORMATION_SCHEMA`     : %s" % ", ".join(vistas_is))
print("   metadata fuera de ese esquema      : %s"
      % ", ".join(sorted(set(re.findall(r"`\{d\}\.(\w+)`", q)))))
print()
print("   La forma ingenua —listar y describir cada tabla— esta medida y")
print("   descartada: «`bq show` tarda ~10 s por tabla, asi que DOCE TABLAS")
print("   AGOTARON un limite de dos minutos». Las mismas doce, en una sola")
print("   consulta: 13,8 s.")
print()
print("   -> el sondeo no itera sobre tablas. Itera sobre FILAS de un resultado")
print("      plano, y agrupar es de `armar`, ya en local y sin red.")

# -- B - LA ITERACION --------------------------------------------------------
print()
print("B - LA ITERACION: sobre que, y en que orden")
sel = re.search(r"SELECT c\.table_name.*?FROM", q, re.S)
campos = re.findall(r"(?:AS (\w+)|(?:c|t|n|kc|fk|o|fp)\.(\w+))", sel.group(0)) if sel else []
# Sin repetir y en orden: el `SELECT` alias algunas columnas (`fp.description
# AS column_description`), asi que la extraccion las ve dos veces.
vistos, limpios = set(), []
for c in (a or b for a, b in campos):
    if c not in vistos:
        vistos.add(c)
        limpios.append(c)
print("   la fila es (tabla, columna), y trae :")
for i in range(0, len(limpios), 4):
    print("     %s" % ", ".join(limpios[i:i + 4]))
orden = re.search(r"ORDER BY ([^\"\\\\]+)", q)
print("   orden                               : %s" % (orden.group(1).strip() if orden else "?"))
print()
print("   Y ese orden NO es cosmetico — `armar` lo dice: «llegan ordenadas por")
print("   (table_name, ordinal_position), y ese orden se conserva: el orden de")
print("   las columnas es del origen y no nos toca reordenarlo».")
print()
print("   Asi que hay dos bucles y solo uno cuesta red:")
print("     el del servidor : una pasada por el dataset entero")
print("     el de `armar`   : una pasada por las filas, agrupando por tabla")

# -- C - LO QUE VARIA --------------------------------------------------------
print()
print("C - LO QUE VARIA POR TABLA, y lo que es constante")
r = cuerpo("reads")
ops = re.search(r'let operadores = \[([^\]]+)\]', r)
print("   `reads.predicatePushdown` : %s"
      % (ops.group(1).replace('"', '') if ops else "?"))
print("     CONSTANTE, y con motivo escrito: «`reads` describe EL OBJETO, no a")
print("     quien lo consulta: BigQuery es un motor SQL completo y los contesta")
print("     todos».")
print()
print("   Lo que si se sondea son DOS hechos que el servidor afirma:")
print("     require_partition_filter -> `fullScan: forbidden` + `requiredFilters`")
print("     (cualquier otro)         -> `fullScan: expensive`")
print("     enable_change_history    -> `changes: {retract, log}` o `{none, none}`")
print("     table_type != BASE TABLE -> `changes: {none, none}`")
print()
c = cuerpo("changes")
print("   `expensive` frente a `cheap` no es prudencia: BigQuery factura por")
print("   bytes leidos, y `cheap` empujaria al planificador a recorrer la tabla.")
print("   La fila del historial esta SIN MEDIR contra un dataset real y lo dice:")
sin_medir = "no se ha podido medir" in LECTOR or "no se ha medido" in LECTOR
print("     «ninguna tabla de `rubix_demo_ventas` tiene el historial encendido,")
print("      y encenderlo es una modificacion del dataset de otro». (%s)"
      % ("declarado en el codigo" if sin_medir else "REVISAR"))

# -- D - DONDE PARA ----------------------------------------------------------
print()
print("D - DONDE PARA EL SONDEO")
tope = re.search(r"--max_rows=(\d+)", LECTOR)
# La guardia seria comparar el numero de filas devueltas contra el tope. Que
# el tope aparezca no es una guardia — la primera version de este arnes busco
# «max_rows» en la funcion y encontro EL PROPIO FLAG, asi que dijo «si» y se
# contradijo con su propio parrafo tres lineas mas abajo.
bq_fn = cuerpo("bigquery")
guardia = bool(re.search(r"(items\(\)\.len\(\)|filas\.len\(\)|count\(\))\s*[><=]", bq_fn))
print("   tope de filas que se le pide a `bq` : %s" % (tope.group(1) if tope else "(ninguno)"))
print("   comprobacion de si se toco el tope  : %s" % ("si" if guardia else "NO HAY"))
print()
print("   La fila es (tabla, columna), asi que el tope se agota en columnas y")
print("   no en tablas: con %s filas, un dataset de 20 columnas por tabla cabe"
      % (tope.group(1) if tope else "?"))
print("   hasta ~%d tablas, y uno de tablas anchas mucho antes."
      % (int(tope.group(1)) // 20 if tope else 0))
print()
print("   -> Y si se toca, `bq` devuelve las primeras y ya esta. El catalogo")
print("      sale con tablas de menos —y con la ultima A MEDIAS, porque el corte")
print("      es por fila y no por tabla— y NADA lo dice. Es el modo de fallo de")
print("      la casa otra vez: lo que falta se parece a lo que esta bien.")
print()
print("   Un solo alcance por fuente, ademas: la URL es")
print("   `bigquery://<proyecto>/<dataset>`, asi que un proyecto con N datasets")
print("   son N fuentes declaradas. Eso no es un limite del sondeo — es el")
print("   reparto, y `INFORMATION_SCHEMA` de BigQuery es por dataset.")

# -- E - EL CONTRASTE --------------------------------------------------------
print()
print("E - EL CONTRASTE con Postgres: donde vive el hecho")
pg = (RAIZ / "crates/ore-read-postgres/src/main.rs").read_text(encoding="utf-8")
# Solo las del CATALOGO: contar todas las de `main.rs` mezcla los tres verbos
# y compara peras con manzanas, porque la fila de al lado es de descubrimiento.
cat_pg = re.search(r"let url = entrada\.trim\(\);.*?Ok\(armar\(", pg, re.S)
consultas_pg = len(re.findall(r"\.query(?:_one)?\(", cat_pg.group(0) if cat_pg else pg))
print("   %-34s %-22s %s" % ("", "BigQuery", "PostgreSQL"))
print("   %-34s %-22s %s" % ("quien lo hace", "receta dentro de `ore`", "binario fuera"))
print("   %-34s %-22s %s" % ("consultas por descubrimiento", "1", str(consultas_pg)))
print("   %-34s %-22s %s" % ("de donde sale la cara D",
                             "opciones de la tabla", "estado del CLUSTER"))
print("   %-34s %-22s %s" % ("", "(enable_change_history)", "(wal_level, relreplident)"))
print()
print("   Y esa ultima fila es la diferencia de verdad. En Postgres, que una")
print("   tabla emita cambios depende de una opcion del SERVIDOR que ninguna")
print("   tabla declara —por eso el driver avisa por stderr cuando `wal_level`")
print("   no es `logical`: un `changes: none` en cuarenta tablas se parece")
print("   demasiado a un origen que de verdad no cambia—. En BigQuery el hecho")
print("   es de la tabla, y esta en su metadata.")
print()
print("   Dicho de otro modo: los dos sondean, y no sondean lo mismo porque el")
print("   hecho no vive en el mismo sitio.")
