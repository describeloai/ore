# -*- coding: utf-8 -*-
"""La medida del cambio: que ve `ore diff` cuando cambia el sustrato.

`docs/ontologia-como-repositorio.md` §4 sostiene que ya somos un repositorio y
lo apoya, entre otras cosas, en que `diff.rs` es el segundo fichero mas grande
de `ore-core`. La tesis entera -la ontologia es el conjunto acordado de
preguntas, versionado, con politica de cambio rompedor- descansa en ese verbo:
`OOS5021` comprueba la version declarada contra los cambios DETECTADOS y
`OOS5022` exige el preaviso del SLA para los cambios DETECTADOS.

La pregunta que faltaba hacer: **si la unidad es la vista, ¿ve `diff` una vista?**

Mutaciones de una sola variable sobre casos v1alpha8 conformes. De cada una se
pregunta dos cosas, en este orden:

  1. ¿lo caza `validate`?   -> entonces no hace falta que lo vea `diff`
  2. ¿lo reporta `diff`?    -> entonces el consumidor se entera

Lo que cae entre las dos -valida en verde y `diff` calla- viaja al consumidor
sin version y sin preaviso. Y el control dice si el arnes mide algo: una
propiedad retirada de la entidad, que `OOS5001` tiene que cazar.

DOS CASOS, y la diferencia importa: `OOS2022` -una propiedad sin campo- solo se
aplica a entidades que declaran v1alpha8, y en el corpus 9 de 23 entidades con
`backedBy` declaran una version anterior. El segundo caso es el bueno; el
primero enseña lo que pasa en las nueve.
"""
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
CONF = pathlib.Path(r"C:\ORE\vendor\oos\conformance\v1alpha8\valid")
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\diff-sustrato"
)

# (nombre, fichero, patron, sustituto)  ·  patron None = borrar el fichero
CASOS = [
    (
        "a-function-writes-through-its-view   (entidad v1alpha1 sobre vista v1alpha8)",
        "a-function-writes-through-its-view",
        [
            ("vista · la copia se muda de sitio", "views/empleados.yaml",
             r'table: "cache\.hr_empleados"', 'table: "cache.hr_empleados_v2"'),
            ("vista · deja de materializarse", "views/empleados.yaml",
             r"\n  materialized:.*", ""),
            ("vista · recorta filas con un `where`", "views/empleados.yaml",
             r"(\n  materialized:)", '\n  where: { status: "activo" }\\1'),
            ("vista · afloja la frescura", "views/empleados.yaml",
             r"(\n  owner: team:rrhh)", "\\1\n  freshness: 24h"),
            ("vista · pierde un campo", "views/empleados.yaml",
             r"\n    estado: status", ""),
            ("vista · desaparece entera", "views/empleados.yaml", None, None),
            ("tabla · apunta a otro objeto fisico", "tables/employees.yaml",
             r'object: "public\.employees"', 'object: "public.employees_v2"'),
            ("tabla · el escaneo pasa a ser caro", "tables/employees.yaml",
             r"fullScan: cheap", "fullScan: expensive"),
            ("tabla · deja de retractar", "tables/employees.yaml",
             r"mode: retract", "mode: append"),
            ("CONTROL · la entidad pierde una propiedad", "entities/Employee.yaml",
             r"\n    estado:\n      type: String\n      labels: \{ acme\.assurance: reviewed \}", ""),
        ],
    ),
    (
        "materialized-view-over-table-within-clearance   (entidad v1alpha8, vista sobre vista)",
        "materialized-view-over-table-within-clearance",
        [
            ("vista raiz · invierte su `where`", "views/empleados.yaml",
             r'deleted: "false"', 'deleted: "true"'),
            ("vista raiz · afloja la frescura", "views/empleados.yaml",
             r"freshness: 15m", "freshness: 24h"),
            ("vista raiz · la copia se muda de sitio", "views/empleados.yaml",
             r'table: "cache\.hr_employees"', 'table: "cache.hr_employees_v2"'),
            ("vista raiz · deja de materializarse", "views/empleados.yaml",
             r"\n  materialized:.*", ""),
            ("vista de arriba · estrecha el `where`", "views/iberia.yaml",
             r"pais: \[ES, PT\]", "pais: [ES]"),
            ("vista de arriba · pierde un campo", "views/iberia.yaml",
             r"\n    dni: nationalId", ""),
            ("tabla · apunta a otro objeto fisico", "tables/employees.yaml",
             r'object: "public\.employees"', 'object: "public.employees_v2"'),
            ("tabla · el escaneo pasa a ser caro", "tables/employees.yaml",
             r"fullScan: cheap", "fullScan: expensive"),
            ("CONTROL · la entidad pierde una propiedad", "entities/Employee.yaml",
             r"\n    dni:\n      type: String\n      labels: \{ gdpr\.sensitivity: high \}", ""),
        ],
    ),
]


def montar(origen, destino, fichero=None, patron=None, sustituto=None):
    if destino.exists():
        shutil.rmtree(destino)
    shutil.copytree(origen, destino)
    if fichero is None:
        return True
    f = destino / fichero
    if patron is None:
        f.unlink()
        return True
    t = f.read_text(encoding="utf-8")
    n = re.sub(patron, sustituto, t, count=1)
    if n == t:
        return False            # la mutacion no mordio: se dice, no se calla
    f.write_text(n, encoding="utf-8")
    return True


def correr(*args):
    r = subprocess.run([str(ORE), *[str(a) for a in args]], capture_output=True, text=True)
    return (r.returncode, (r.stdout or "") + (r.stderr or ""))


print("== binario:", ORE, "==")
mudos_total = 0
for titulo, caso, mutaciones in CASOS:
    origen = CONF / caso / "input"
    antes = TMP / caso / "antes"
    montar(origen, antes)
    cod, salida = correr("validate", antes)

    print()
    print("==", titulo, "==")
    print("   el caso de partida valida:", "SI" if cod == 0 else "NO -> " + salida[:100])
    print()
    print("   %-40s %-16s %s" % ("mutacion", "validate", "`ore diff`"))
    print("   " + "-" * 88)

    for nombre, fichero, patron, sustituto in mutaciones:
        despues = TMP / caso / "despues"
        if not montar(origen, despues, fichero, patron, sustituto):
            print("   %-40s %s" % (nombre, "LA MUTACION NO MORDIO - revisar el patron"))
            continue

        cv, sv = correr("validate", despues)
        codigos_v = sorted(set(re.findall(r"OOS\d{4}", sv)))
        estado_v = "ok" if cv == 0 else (",".join(codigos_v) or "error")

        _, sd = correr("diff", antes, despues)
        # `OOS5021` sale SIEMPRE, incluso comparando un arbol consigo mismo:
        # pide un patch por publicar de nuevo. No dice nada sobre si el cambio
        # se vio, asi que se descuenta -y descontarlo es la mitad de la medida.
        codigos_d = sorted(set(re.findall(r"OOS\d{4}", sd)) - {"OOS5021"})
        dice = ",".join(codigos_d) if codigos_d else "NADA"

        if cv == 0 and not codigos_d:
            mudos_total += 1
            dice += "   <- pasa sin que nadie lo vea"
        print("   %-40s %-16s %s" % (nombre, estado_v, dice))

print()
print("VEREDICTO")
print("   cambios que validan en verde y `diff` no reporta:", mudos_total)
print()
print("   ERA 13 de 19 cuando se escribio este guion, y `diff.rs` no mencionaba")
print("   `View` ni `Table` ni una vez. El ADR 0019 y el peldano que lo sigue le")
print("   devolvieron el sujeto a `OOS5007`, `OOS5019` y `OOS5020` —que eran de")
print("   v1alpha1 y hablaban del binding— y escribieron el par espejo del")
print("   recorte, `OOS5028` y `OOS5029`.")
print()
print("   LO QUE SIGUE MUDO, y se dice para que no se descubra por sorpresa:")
print("     `freshness` que se afloja     un orden de una sola direccion, sin codigo")
print("     `reads`/`changes` que encogen idem — «la fuente admite menos»")
print()
print("   Y lo que YA NO: un campo que desaparece da `OOS5001` desde que la")
print("   vista tiene `moved` —`02-view` §4.2—, que era la valvula que le")
print("   faltaba a la regla.")
