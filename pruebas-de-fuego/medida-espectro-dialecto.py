# -*- coding: utf-8 -*-
"""El espectro de la migracion al dialecto declarado.

`medida-el-dialecto-declarado` dijo que la forma esta escrita dos veces y que
el dialecto son cuatro ejes que son datos. Esto mide lo que cuesta moverlo, y
lo primero que sale es que la forma obvia —«un binario `ore-read-sql` y un
manifiesto por familia»— NO SE PUEDE, y no por gusto: por el cierre de
dependencias que este arbol ya vigila.

  A. QUE SE MUEVE   linea a linea: cuanto de los dos traductores es la MISMA
                    forma y cuanto es dialecto
  B. LA PIEDRA      por que no puede ser un binario, medido sobre `Cargo.lock`
  C. LA FORMA       lo que si puede ser, y que preserva
  D. LO QUE ROMPE   pruebas, identidades de crate y el guardian
  E. EL ESPECTRO    los tramos, en orden, y que compra cada uno
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
PG = RAIZ / "crates/ore-read-postgres/src/sql.rs"
BQ = RAIZ / "crates/ore-read-bigquery/src/sql.rs"


def lineas(f):
    """Sin pruebas, sin comentarios, sin blancos, normalizadas."""
    t = f.read_text(encoding="utf-8", errors="replace").split("#[cfg(test)]")[0]
    out = []
    for l in t.split("\n"):
        s = l.strip()
        if not s or s.startswith(("//", "///", "//!")):
            continue
        out.append(re.sub(r"\s+", " ", s))
    return out


print("== el espectro del dialecto declarado ==")

# -- A - QUE SE MUEVE --------------------------------------------------------
print()
print("A - QUE SE MUEVE, linea a linea")
a, b = lineas(PG), lineas(BQ)
# La FORMA es lo que los dos escriben igual. Es una medida cruda —una linea
# identica en dos ficheros escritos por separado no es casualidad— y sirve para
# lo unico que hace falta: acotar por abajo cuanto esta duplicado.
#
# Y sin contar la puntuacion: una llave de cierre coincide en cualquier par de
# ficheros de Rust y no es evidencia de nada. La primera version las conto y de
# 26 lineas «identicas» once eran `}`, lo cual infla justo la cifra que este
# arnes existe para acotar.
comun = [l for l in a if l in b and len(l.strip("{}();, ")) > 6]
print("   %-42s %4d" % ("`ore-read-postgres/sql.rs`", len(a)))
print("   %-42s %4d" % ("`ore-read-bigquery/sql.rs`", len(b)))
print("   %-42s %4d" % ("lineas IDENTICAS en los dos", len(comun)))
print()
print("   Las identicas, que son el esqueleto:")
for l in comun:
    print("     %s" % l[:66])
print()
print("   Y esa cifra es un SUELO, no la duplicacion real: no cuenta las que")
print("   dicen lo mismo con otra letra —montar la condicion del filtro, unir")
print("   con AND, decidir si hay WHERE—, que en (B) del arnes anterior salieron")
print("   iguales en los cinco hitos.")

# -- B - LA PIEDRA -----------------------------------------------------------
print()
print("B - LA PIEDRA: por que NO puede ser un binario")
lock = (RAIZ / "Cargo.lock").read_text(encoding="utf-8")
aristas = {}
for bloque in lock.split("[[package]]")[1:]:
    n = re.search(r'^name = "(.+)"', bloque, re.M)
    if not n:
        continue
    deps = re.search(r"^dependencies = \[(.*?)\]", bloque, re.M | re.S)
    aristas[n.group(1)] = re.findall(r'"([^" ]+)', deps.group(1)) if deps else []
miembros = {d.name for d in (RAIZ / "crates").iterdir() if d.is_dir()}


def cierre(r):
    vistos, pila = set(), [r]
    while pila:
        x = pila.pop()
        for d in aristas.get(x, []):
            if d not in vistos:
                vistos.add(d)
                pila.append(d)
    return vistos - miembros


pg, bq, js = (cierre("ore-read-postgres"), cierre("ore-read-bigquery"),
              cierre("ore-read-jsonl"))
print("   cierre de `ore-read-postgres` : %3d crates de fuera" % len(pg))
print("   cierre de `ore-read-bigquery` : %3d" % len(bq))
print("   cierre de `ore-read-jsonl`    : %3d" % len(js))
print("   cierre de `ore-cli`           : %3d" % len(cierre("ore-cli")))
print()
print("   un solo binario que los junte : %3d" % len(pg | bq | js))
print("     ...y de esos, SOLO de Postgres: %3d" % len(pg - (bq | js)))
print()
print("   -> juntar los tres en un `ore-read-sql` embarcaria %d crates —`tokio`,"
      % len(pg - (bq | js)))
print("      `native-tls`, `openssl`, FFI de plataforma— a un lector de BigQuery")
print("      que delega en `bq` y no abre un socket, y a uno de ficheros que")
print("      solo lee un fichero. Y `tests/dependencias.rs` existe justo para")
print("      que eso no pase sin que nadie lo note:")
print("        «un binario sin codigo de red no puede hacer una llamada; uno con")
print("         una pila TLS enlazada que promete no usarla tiene una politica».")
print()
print("   El transporte NO es dialecto. Es lo unico del driver que de verdad es")
print("   suyo, y es lo que pesa.")

# -- C - LA FORMA CORRECTA ---------------------------------------------------
print()
print("C - LA FORMA: una BIBLIOTECA, no un binario")
print()
print("   `ore-driver` ya reparte asi: el protocolo se comparte y la traduccion")
print("   es del driver. Lo que este espectro anade es que LA FORMA tambien se")
print("   comparte, y solo el dialecto y el transporte se quedan fuera.")
print()
print("   %-24s %s" % ("hoy", "despues"))
print("   " + "-" * 66)
FILAS = [
    ("ore-driver", "ore-driver — el protocolo, igual"),
    ("(no existe)", "ore-sql — LA FORMA: monta el SELECT/WHERE"),
    ("pg: sql.rs + transporte", "pg: dialecto (datos) + transporte"),
    ("bq: sql.rs + `bq`", "bq: dialecto (datos) + `bq`"),
]
for x, y in FILAS:
    print("   %-24s %s" % (x, y))
print()
print("   Y el manifiesto no tiene por que ser un FICHERO que se lee en tiempo")
print("   de ejecucion. Siendo una biblioteca, un dialecto es una constante del")
print("   crate que lo usa: sigue siendo DATO —se lee de un vistazo, se compara")
print("   con el de al lado, no hay logica— y no anade un analizador ni un")
print("   fallo nuevo por fichero ausente. Un fichero solo hace falta el dia que")
print("   alguien de fuera quiera anadir un dialecto sin recompilar, y ese dia")
print("   es una decision aparte.")

# -- D - LO QUE ROMPE --------------------------------------------------------
print()
print("D - LO QUE ROMPE")


def pruebas(f):
    t = f.read_text(encoding="utf-8", errors="replace")
    return len(re.findall(r"#\[test\]", t))


tot = 0
for c in ("ore-read-postgres", "ore-read-bigquery"):
    n = sum(pruebas(x) for x in (RAIZ / "crates" / c / "src").rglob("*.rs"))
    tot += n
    print("   %-30s %2d pruebas" % (c, n))
print("   %-30s %2d" % ("juntas", tot))
print()
print("   Y no se pierden: cambian de sujeto. Las que afirman LA FORMA —«el SQL")
print("   no pide una columna que no este en la proyeccion», «ningun valor se")
print("   interpola», «el rango sale como dos condiciones»— pasan a `ore-sql` y")
print("   se escriben UNA VEZ, que es exactamente lo que el CDK de Airbyte")
print("   compra. Las que afirman el DIALECTO —como se cita, como se marca un")
print("   parametro— se quedan, y son las cortas.")
print()
propios = {"ore-core", "ore-cli", "ore-driver", "ore-read-jsonl", "ore-read-postgres"}
print("   EL GUARDIAN, y una grieta que salio al medir esto:")
print("     `dependencias.rs` excluye del recuento una lista de miembros ESCRITA")
print("     A MANO, y tiene %d de los %d que hay. No listados: %s"
      % (len(propios), len(miembros), ", ".join(sorted(miembros - propios))))
print("     Por eso `CIERRE` vale 34 y un recuento que excluya TODOS los")
print("     miembros da %d: la diferencia son miembros contados como si fueran"
      % len(cierre("ore-cli")))
print("     de fuera. No es un agujero —el guardian sigue saltando si el arbol")
print("     crece— pero es una lista a mano que envejece, y un `ore-sql` nuevo")
print("     entraria en ella sin que nada obligue a anadirlo.")

# -- E - EL ESPECTRO ---------------------------------------------------------
print()
print("E - EL ESPECTRO, en orden")
print()
print("   1 · `ore-sql`, la forma, CON LOS DOS DIALECTOS YA DENTRO.")
print("       Nace con Postgres y BigQuery como constantes, porque una forma")
print("       extraida de un solo caso es el caso con otro nombre. Aqui hay dos")
print("       escritos por separado, que es la unica prueba de que la forma es")
print("       forma. Las pruebas de forma se mudan enteras.")
print()
print("   2 · LOS DOS DRIVERS ENCIMA. Cada uno se queda con su transporte y su")
print("       constante de dialecto. `sql.rs` desaparece de los dos.")
print("       Criterio de listo: las %d pruebas siguen verdes, repartidas." % tot)
print()
print("   3 · LAS TRES BANDERAS que no son texto: si el dialecto exige tipar")
print("       parametros, cual es el `fullScan` por defecto y de donde sale la")
print("       cara D. Van al dialecto como datos, no como codigo.")
print()
print("   4 · LA TERCERA FAMILIA, que es la que cobra la apuesta. Escribir")
print("       —MySQL, Snowflake, DuckDB— y ver si de verdad es una constante y")
print("       un transporte, o si aparece un quinto eje. Si aparece, la forma")
print("       estaba extraida de dos casos y no de la clase.")
print()
print("   5 · EL GUARDIAN, corregido: que la lista de miembros se derive del")
print("       directorio en vez de escribirse. Es de (D) y no del dialecto, pero")
print("       lo destapo esto y se paga barato.")
print()
print("   Lo que NO entra en este espectro, y conviene decirlo: el catalogo. La")
print("   consulta de catalogo de cada familia es larga, especifica y ya vive")
print("   donde tiene que vivir. Meterla en la forma seria juntar dos cosas que")
print("   solo comparten el nombre «SQL».")
