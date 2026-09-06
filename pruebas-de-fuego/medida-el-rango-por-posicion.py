# -*- coding: utf-8 -*-
"""El rango por posicion: quien lo declara, quien lo pide y quien lo sirve.

El protocolo lo define desde el ADR 0016: `start`/`end` SIN `cursor` significa
que el rango va sobre la posicion del propio origen —un LSN, un snapshot— y no
sobre una columna. Y ningun driver lo sirve.

Antes de escribirlo conviene ver por que, porque hay tres piezas y solo una
esta rota:

  A. QUIEN LO DECLARA   `changes.witness` de cada catalogo
  B. QUIEN LO PIDE      lo que `materializar` sabe poner en una peticion
  C. QUIEN LO SIRVE     lo que cada driver declara saber hacer
  D. EL CRUCE           y cual de las tres es la que falta
"""
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"


def cuerpo(f, nombre):
    t = f.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^(?:pub )?fn %s\b.*?\{" % nombre, t, re.M | re.S)
    if not m:
        return ""
    i, prof = m.end(), 1
    while i < len(t) and prof:
        prof += (t[i] == "{") - (t[i] == "}")
        i += 1
    return t[m.start():i]


print("== el rango por posicion ==")

# -- A - QUIEN LO DECLARA ----------------------------------------------------
print()
print("A - QUIEN DECLARA UN TESTIGO QUE ORDENA")
print()
FUENTES = [
    ("postgres", CRATES / "ore-read-postgres/src/main.rs", "changes"),
    ("bigquery", CRATES / "ore-cli/src/lector.rs", "changes"),
    ("jsonl", CRATES / "ore-read-jsonl/src/catalogo.rs", "de_directorio"),
]
print("   %-10s %s" % ("familia", "testigos que su catalogo puede emitir"))
print("   " + "-" * 60)
declara = {}
for fam, f, fn in FUENTES:
    c = cuerpo(f, fn)
    ws = sorted(set(re.findall(r'"witness", Json::s\("(\w+)"\)|"witness".to_string\(\), Json::s\("(\w+)"\)', c)
                    and [a or b for a, b in re.findall(
                        r'"witness"(?:\.to_string\(\))?, Json::s\("(\w+)"\)|"witness": "(\w+)"', c)]
                    or re.findall(r'"witness", Json::s\("(\w+)"\)', c)))
    if not ws:
        ws = sorted(set(re.findall(r'"witness"[^\n]*?"(\w+)"', c)))
    declara[fam] = ws
    print("   %-10s %s" % (fam, ", ".join(ws) or "(ninguno)"))
print()
print("   `log` y `snapshot` son los dos que NO van sobre una columna. Y no son")
print("   lo mismo:")
print("     log       una POSICION en un flujo de cambios. Ordena, asi que")
print("               «lo que hay entre A y B» tiene sentido.")
print("     snapshot  una IDENTIDAD de version. NO ordena — dos digests no se")
print("               comparan— asi que no hay rango: hay un PIN.")

# -- B - QUIEN LO PIDE -------------------------------------------------------
print()
print("B - QUIEN LO PIDE: lo que `materializar` sabe poner en la peticion")
mat = CRATES / "ore-cli/src/materializar.rs"
# La peticion se arma en `peticion`, no en `leer`: se separo al cerrar esto,
# por lo mismo que `ore-sql` separa la traduccion del transporte —un aserto
# que exigiera lanzar un proceso ajeno no se ejecutaria nunca en la suite—.
leer = cuerpo(mat, "peticion") or cuerpo(mat, "leer")
guarda = re.search(r"if let Some\(\w+\) = desde.*", leer) or re.search(
    r"if let \(Some\(\w+\), Some\(\w+\)\) = \(\w+, \w+\)", leer)
print()
print("   la guarda del rango : %s" % (guarda.group(0) if guarda else "(no encontrada)"))
print("   pone `cursor`       : %s" % ("si" if '"cursor"' in leer else "no"))
print("   pone `start`        : %s" % ("si" if '"start"' in leer else "no"))
print("   pone `end`          : %s" % ("si" if '"end"' in leer else "no"))
print()
sin_columna = "ordena" in leer
print()
print("   pone rango SIN columna : %s" % ("si" if sin_columna else "NO"))
print()
if sin_columna:
    print("   CERRADO. Antes la guarda era `(Some(cursor), Some(desde))`, asi que")
    print("   con `witness: log` —donde el cursor es `None`— `desde` y `hasta` se")
    print("   calculaban, se pasaban y SE TIRABAN: la lectura era entera siempre y")
    print("   ningun driver tuvo nunca la ocasion de negarse ni de servirlo.")
    print()
    print("   Ahora el rango va sobre la columna o sobre la POSICION, y lo que")
    print("   decide cual es si el testigo ORDENA. Y se toma del que el origen")
    print("   acaba de contestar, no del que la tabla declara: cuando discrepan")
    print("   manda el origen.")
else:
    print("   -> el rango SOLO se pone si hay `cursor`, asi que un `witness: log`")
    print("      se relee entero en cada refresco y nadie lo ve.")
com = re.search(r"Cuando la petici[oó]n sepa llevar un rango.*?medida", mat.read_text(encoding="utf-8"), re.S)
if com:
    print("     «%s»" % " ".join(com.group(0).split())[:150])

# -- C - QUIEN LO SIRVE ------------------------------------------------------
print()
print("C - QUIEN LO SIRVE: lo que cada driver declara saber hacer")
print()
print("   %-10s %-28s %s" % ("familia", "rango_servible(cursor, pos)", "sirve posicion"))
print("   " + "-" * 66)
for fam, crate in (("postgres", "ore-read-postgres"), ("bigquery", "ore-read-bigquery"),
                   ("jsonl", "ore-read-jsonl")):
    f = CRATES / crate / "src/main.rs"
    t = f.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"rango_servible\(&p, (\w+), (\w+)\)", t)
    par = "(%s, %s)" % (m.group(1), m.group(2)) if m else "?"
    print("   %-10s %-28s %s" % (fam, par, "SI" if m and m.group(2) == "true" else "no"))

# -- D - EL CRUCE ------------------------------------------------------------
print()
print("D - EL CRUCE, y cual de las tres piezas falta")
print()
print("   %-10s %-18s %-16s %s" % ("familia", "declara", "pide", "sirve"))
print("   " + "-" * 62)
for fam in ("postgres", "bigquery", "jsonl"):
    d = ", ".join(x for x in declara.get(fam, []) if x in ("log", "snapshot")) or "-"
    print("   %-10s %-18s %-16s %s" % (fam, d, "NO (siempre entera)", "no"))
print()
print("   Las tres columnas decian cosas distintas, y de las tres piezas solo")
print("   una estaba rota:")
print()
print("   1 · `snapshot` NO NECESITA RANGO, y ya estaba explotado. Dos digests")
print("       no se ordenan, asi que lo unico que se puede hacer con el es un")
print("       pin: si el testigo no cambio, no hay que copiar. Y eso YA")
print("       funcionaba —el testigo entra en la cabecera, y una cabecera igual")
print("       da el recibo que corta el ciclo en el paso ④ sin leer nada.")
print()
print("   2 · `log` SI ORDENA, y la pieza rota estaba en `materializar` y no en")
print("       los drivers: la peticion no sabia llevar un rango sin columna, asi")
print("       que ningun driver tuvo nunca la ocasion de negarse ni de servirlo.")
print("       CERRADO — §B.")
print()
print("   3 · Y ahora que la peticion lo lleva, la SOBRE-DECLARACION se ve:")
print("       `ore-read-postgres` emite `witness: log` para toda tabla con")
print("       decodificacion logica, y su `leer` hace UN `SELECT` sobre el estado")
print("       presente. Declara un testigo cuyo rango no sirve.")
print()
print("       Y no se \"arregla\" quitando el `log`: a diferencia de `reads` —que")
print("       es lo que el DRIVER empuja— `changes` describe EL OBJETO, y ese")
print("       objeto si tiene un changelog. Lo que falta es que alguien lo lea.")
print("       Mientras tanto `materializar` copia entera y LO DICE, que es la")
print("       diferencia entre un hueco visible y uno que no existia.")
print()
print("   LO QUE FALTA, y por que no se escribe hoy:")
print()
print("     postgres  un rango por LSN exige decodificacion logica, una ranura de")
print("               replicacion y una sesion que dure. Es otro programa, y su")
print("               propio driver ya lo dice.")
print("     jsonl     no puede y esta bien: su testigo es un digest, y dos")
print("               digests no se ordenan. Por eso su modo es `snapshot`.")
print("     bigquery  SI podria, con la funcion de tabla `CHANGES` sobre el")
print("               historial. Y no se escribe porque no se puede EJERCER:")
print("               ninguna tabla del dataset de pruebas tiene el historial")
print("               encendido, y encenderlo es modificar el dataset de otro —lo")
print("               mismo que ya dice la receta del catalogo de su tercera")
print("               fila—. Codigo que no se puede ejercer es justo lo que este")
print("               arbol no escribe.")
