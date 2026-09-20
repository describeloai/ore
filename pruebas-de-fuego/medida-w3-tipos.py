#!/usr/bin/env python3
"""
MEDIDA · W3.5 · el contrato de tipos: qué hay que cubrir (20 de septiembre)

Antes de ESCRIBIR el contrato de tipos (ORE ↔ Arrow/Parquet ↔ Python/TS/Java),
qué tipos existen de verdad en los tres sitios donde un tipo nace:

  §1  EL VOCABULARIO   lo que OOS deja declarar: `scalarType` de una propiedad
                       de Entity (v1alpha1 basic.schema.json §3.1) y el
                       `physicalType` de una columna de Table (ODCS, opaco)
  §2  LAS COPIAS       lo que hay en los buckets: cada artefacto ORECOPY1 de
                       demo y victor, su Parquet, y el tipo Arrow de cada
                       columna — el censo de tipos que `over()` tiene que
                       devolver HOY, con cuántas columnas tiene cada uno
  §3  LOS LENGUAJES    para cada tipo Arrow del censo (y los difíciles de
                       `medida-w3-leer.py`), en qué tipo NATIVO puede vivir
                       en pyarrow/pandas, DuckDB node-api (valores tipados) y
                       DuckDB JDBC/Arrow Java — consultado, no medido

Uso:
  python pruebas-de-fuego/medida-w3-tipos.py [--inquilinos demo,victor]

Lee los buckets con `gcloud storage` (sesión propia); baja los artefactos a un
temporal y lo borra. Nada de pago, nada en el clúster.

Nota (W3.6a, ese mismo día por la tarde): desde que la copia es un dataset
(0031 §10), lo que la pasada deja en el bucket es una tabla Iceberg bajo
`ore/v2/copias/<p>_<v>/`, con el puntero en `copias/<p>_<v>.json`. §2 sigue
leyendo los sobres `ORECOPY1` de `ore/v1/` que queden —el censo que motivó el
contrato—; el de las tablas nuevas lo da `iceberg_scan` desde el puesto.
"""
import collections
import datetime as dt
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROYECTO = "project-8853a180-450d-47be-b83"


def fila(k, v, nota=""):
    print("  %-30s %-44s %s" % (k, v, nota))


def sh(*args):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), capture_output=True)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


def vocabulario():
    print("§1 · el vocabulario de OOS")
    b = json.load(open(os.path.join(RAIZ, "vendor/oos/schemas/v1alpha1/type/basic.schema.json"), encoding="utf-8"))
    ramas = b["$defs"]["scalarType"]["oneOf"]
    fila("Entity.property.type", ", ".join(ramas[0]["enum"]), "v1alpha1 §3.1")
    for r in ramas[1:]:
        fila("", r.get("pattern", "")[:44], (r.get("description") or "").split("\n")[0][:60])
    t = json.load(open(os.path.join(RAIZ, "vendor/oos/schemas/v1alpha8/table.schema.json"), encoding="utf-8"))
    pt = t["properties"]["spec"]["properties"]["columns"]["additionalProperties"]["properties"]["physicalType"]
    fila("Table.columns.<c>.physicalType", "cadena libre, %s" % pt["description"].split(".")[0][:36], "opcional: un origen sin tipos tiene columnas sin tipo")
    fila("View.fields", "sin tipo: el de la columna de abajo", "o un agregado count/sum/min/max/avg")


def censo(inquilinos):
    import pyarrow.parquet as pq

    print()
    print("§2 · las copias: el tipo Arrow de cada columna, en los buckets de %s" % ", ".join(inquilinos))
    tipos = collections.Counter()
    por_copia = []
    tmp = tempfile.mkdtemp(prefix="ore-tipos-")
    try:
        for inq in inquilinos:
            bucket = "%s-t-%s-copia" % (PROYECTO, inq)
            c, out, err = sh("gcloud", "storage", "ls", "gs://%s/ore/v1/" % bucket)
            objetos = [l.strip() for l in out.splitlines() if l.strip() and "/plan/" not in l and not l.endswith("/")]
            fila("%s · artefactos" % inq, str(len(objetos)))
            for o in objetos:
                destino = os.path.join(tmp, o.rsplit("/", 1)[1])
                sh("gcloud", "storage", "cp", o, destino, "--quiet")
                try:
                    crudo = open(destino, "rb").read()
                except OSError:
                    continue
                if crudo[:8] != b"ORECOPY1":
                    fila("  %s" % o[-16:], "no es ORECOPY1")
                    continue
                n = int.from_bytes(crudo[8:12], "little")
                cab = json.loads(crudo[12:12 + n])
                esquema = pq.read_schema(io.BytesIO(crudo[12 + n:]))
                meta = pq.read_metadata(io.BytesIO(crudo[12 + n:]))
                cols = [(f.name, str(f.type)) for f in esquema]
                for _, t in cols:
                    tipos[t] += 1
                por_copia.append((inq, cab.get("conducto", "?"), meta.num_rows, cols))
                os.remove(destino)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    for inq, conducto, filas, cols in sorted(por_copia):
        resumen = collections.Counter(t for _, t in cols)
        fila("  %s · %s" % (inq, conducto[:24]), "%d filas · %d col" % (filas, len(cols)), " · ".join("%s×%d" % (t, k) for t, k in resumen.most_common()))
    print()
    fila("tipos Arrow distintos", str(len(tipos)), "en %d copias" % len(por_copia))
    for t, k in tipos.most_common():
        fila("  %s" % t, "%d columnas" % k)
    return tipos


# Lo que cada camino de lectura da para cada tipo Arrow — consultado en las
# librerías (pyarrow 24 / pandas 3, @duckdb/node-api 1.5 valores tipados y su
# conversor JSON, DuckDB JDBC 1.5 getObject y Arrow Java 18). «exacto» = el
# valor no pierde; «cadena» = exacto pero como texto; «pierde» = se degrada.
LENGUAJES = [
    # tipo arrow            pyarrow        pandas(default)          pandas(ArrowDtype)  node: valor tipado        node: JSON            java: JDBC getObject        java: Arrow
    ("int8/16/32",          "exacto",      "exacto; con nulos float", "exacto",          "number",                 "number",             "Integer/Short/Byte",       "exacto"),
    ("int64",               "exacto",      "exacto; con nulos float", "exacto",          "bigint",                 "cadena",             "Long",                     "exacto"),
    ("uint64",              "exacto",      "float (pierde)",          "exacto",          "bigint",                 "cadena",             "String (pierde tipo)",     "exacto"),
    ("double / float",      "exacto",      "exacto (NaN → NaN)",      "exacto",          "number (NaN, ±Infinity)", "\"NaN\"/\"Infinity\"", "Double (NaN ok)",          "exacto"),
    ("bool",                "exacto",      "object si nulos",         "exacto",          "boolean",                "boolean",            "Boolean",                  "exacto"),
    ("string / large_string", "exacto",    "object (str)",            "exacto",          "string",                 "string",             "String",                   "exacto"),
    ("dictionary<string>",  "exacto",      "category",                "category",        "string",                 "string",             "String",                   "exacto (dict)"),
    ("binary",              "exacto",      "object (bytes)",          "exacto",          "DuckDBBlobValue",        "\"\\x00\\x01\" (cadena)", "Blob (DuckDBBlobResult)", "exacto (byte[])"),
    ("date32",              "exacto",      "object (date)",           "exacto",          "DuckDBDateValue",        "\"YYYY-MM-DD\"",     "java.sql.Date",            "exacto"),
    ("timestamp[us]",       "exacto",      "datetime64[us]",          "exacto",          "DuckDBTimestampValue",   "cadena sin zona",    "Timestamp (local!)",       "exacto"),
    ("timestamp[us, tz]",   "exacto",      "datetime64[us, tz]",      "exacto",          "DuckDBTimestampTZValue", "cadena con desfase de la sesión", "Timestamp (pierde zona)", "exacto (Long + tz)"),
    ("timestamp[ns]",       "exacto",      "datetime64[ns]",          "exacto",          "DuckDBTimestampNanosecondsValue", "cadena",    "1970 (BUG del mapeo)",     "exacto"),
    ("time64[us]",          "exacto",      "object (time)",           "exacto",          "DuckDBTimeValue",        "\"HH:MM:SS.ffffff\"", "00:00 (BUG del mapeo)",    "exacto (Long)"),
    ("decimal128(p≤18)",    "exacto",      "object (Decimal)",        "exacto",          "DuckDBDecimalValue (exacto)", "number (pierde >15 dígitos)", "BigDecimal (exacto)", "exacto (BigDecimal)"),
    ("decimal128(p>18)",    "exacto",      "object (Decimal)",        "exacto",          "DuckDBDecimalValue (exacto)", "number (pierde)", "BigDecimal (exacto); llano→double (pierde)", "exacto"),
    ("list<T>",             "exacto",      "object (ndarray)",        "exacto",          "DuckDBListValue",        "array",              "DuckDBArray → toString",   "exacto (ListVector)"),
    ("struct",              "exacto",      "object (dict)",           "exacto",          "DuckDBStructValue",      "object",             "DuckDBStruct → toString",  "exacto (StructVector)"),
    ("map<K,V>",            "exacto",      "object (lista de pares)", "exacto",          "DuckDBMapValue",         "[{key,value}]",      "{} (BUG del mapeo)",       "exacto (MapVector)"),
    ("null",                "exacto",      "object",                  "exacto",          "null",                   "null",               "null",                     "exacto"),
]


def lenguajes():
    print()
    print("§3 · dónde puede vivir cada tipo en cada camino (consultado en las librerías; «pierde» = se degrada)")
    print("  %-22s %-9s %-24s %-11s %-30s %-30s %-34s %s" % ("arrow", "pyarrow", "pandas por defecto", "ArrowDtype", "node valor tipado", "node JSON", "java JDBC getObject", "java Arrow"))
    for f in LENGUAJES:
        print("  %-22s %-9s %-24s %-11s %-30s %-30s %-34s %s" % f)


def main():
    inquilinos = ["demo", "victor"]
    if "--inquilinos" in sys.argv:
        inquilinos = sys.argv[sys.argv.index("--inquilinos") + 1].split(",")
    print("MEDIDA · W3.5 · el contrato de tipos · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    vocabulario()
    censo(inquilinos)
    lenguajes()


if __name__ == "__main__":
    main()
