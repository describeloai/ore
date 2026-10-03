# 0049 B5·3 · la derivación incremental, en vivo. Se pega en una celda de un puesto Python
# (repositorio de transforms), en una rama que no sea main. Tres pasadas, cada una en su celda
# o todas seguidas: la 1.ª calcula los 4 contratos, la 2.ª no calcula nada, y la 3.ª, tras
# meter una copia de un contrato con otra ruta, tampoco (la identidad es el contenido).
import re
import ore

ENTRADA = "s3_standard.nueva_carpeta.contratos_sql"
SALIDA = "s3_standard.nueva_carpeta.paginas_b5"


def paginas(item):
    """Una fila por página: sin librerías, lo que el PDF dice de sí."""
    datos = item.read_bytes()
    n = len(re.findall(rb"/Type\s*/Page(?!s)", datos)) or 1
    for p in range(1, n + 1):
        yield {"anchor": {"kind": "page", "page": p}, "paginas": n, "bytes": len(datos)}


@ore.transform(inputs=[ore.collection(ENTRADA)], output=SALIDA)
def paginas_b5(contratos):
    return contratos.apply(paginas, version="1")


print("① primera:", paginas_b5(ore.collection(ENTRADA)))
print("② otra vez:", paginas_b5(ore.collection(ENTRADA)))

# ③ una copia de un contrato con otra ruta: mismo contenido, misma identidad
primero = next(ore.collection(ENTRADA).items())
with ore.collection(ENTRADA).transaction() as t:
    t.put("copia-b5/" + primero.ref.path.split("/")[-1], primero.read_bytes())
print("③ con la copia:", paginas_b5(ore.collection(ENTRADA)))

t = ore.over(SALIDA, format="arrow")
print("④ la tabla:", t.num_rows, "filas ·", t.column_names)
for f in t.slice(0, 2).to_pylist():
    print("   ", f["_item"]["path"], f["_anchor"]["kind"], f["_anchor"]["page"], f["_status"]["state"],
          f["_derivation"]["fn"], f["_derivation"]["fn_version"], f["paginas"], f["bytes"])
