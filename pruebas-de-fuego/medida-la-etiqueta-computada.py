# -*- coding: utf-8 -*-
"""La etiqueta computada: la unica que nadie escribio.

El censo de paridad dejo un solo cabo: `OOS4001` no tenia caso en el paradigma
nuevo. Al arreglar el sello resulto que ahora SI salta, y eso cambia la
pregunta: ya no es «como se provoca», es «que es esto y por que merece un
codigo aparte de `OOS4002`». Cinco frentes:

  A. LAS TRES        de donde puede venir la etiqueta de una propiedad
     PROCEDENCIAS
  B. QUE LA HACE     computada = `join` de los origenes de una derivacion. Y
     DISTINTA        el corolario duro: NO SE PUEDE ESCRIBIR
  C. POR QUE UN      `OOS4001` y `OOS4002` son la misma violacion. Lo que las
     CODIGO APARTE   separa es el REMEDIO
  D. EL CORPUS       cuantas derivadas hay y cuantas suben de verdad
  E. LO QUE FALTA    el caso, ahora que es alcanzable
"""
import collections
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"
ORE = RAIZ / "target/debug/ore"


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def docs(*raices):
    for r in raices:
        for f in sorted(r.rglob("*.yaml")):
            t = f.read_text(encoding="utf-8", errors="replace")
            for d in re.split(r"^---\s*$", t, flags=re.M):
                k = re.search(r"^kind:\s*(\w+)", d, re.M)
                if k:
                    yield k.group(1), d, f


print("== la etiqueta computada ==")

# -- A - LAS TRES PROCEDENCIAS -----------------------------------------------
print()
print("A - LAS TRES PROCEDENCIAS de la etiqueta de una propiedad")
print("   `flow::Origin`, en sus propias palabras:")
print()
print("     Declared   escrita en la propia propiedad")
print("     Inherited  heredada de la entidad, o del datasource")
print("     Computed   COMPUTADA por el compilador propagando `join` desde los")
print("                origenes de una derivacion. NADIE LA ESCRIBIO EN")
print("                NINGUNA PARTE")
print()
print("   Y la procedencia no es decorativa: viaja con el nivel —`Labels` es")
print("   `reticulo -> (nivel, de donde salio)`— porque el diagnostico tiene")
print("   que decir a quien reclamar.")

# -- B - QUE LA HACE DISTINTA ------------------------------------------------
print()
print("B - QUE LA HACE DISTINTA: no se puede escribir")
print("   `04-flow` §3.1, normativo:")
print("     «La propagacion es COMPUTADA, NUNCA DECLARADA (principio P2).»")
print("       etiqueta(d) = join( etiqueta(s1), ..., etiqueta(sn) )")
print("     «Una implementacion NO DEBE permitir que una propiedad derivada")
print("      declare una etiqueta distinta de la computada.»")
print()
print("   Y tiene codigo propio para el intento:")
print("     `OOS4008` · propiedad derivada que declara etiqueta en vez de")
print("                 computarla")
print()
print("   -> o sea que es la unica etiqueta del sistema que NO ESTA EN NINGUN")
print("      FICHERO. `high` en una propiedad se lee; `critical` en una derivada")
print("      hay que calcularlo, y escribirlo esta PROHIBIDO. Por eso es la que")
print("      mas facilmente se pierde: no hay linea que se quede huerfana si el")
print("      calculo deja de llegar a alguna parte.")
print()
print("   Es exactamente lo que paso: el sello no subia por la cadena, y de las")
print("   tres procedencias la que quedaba sin sujeto era esta.")

# -- C - POR QUE UN CODIGO APARTE --------------------------------------------
print()
print("C - POR QUE UN CODIGO APARTE, si la violacion es la misma")
flow = (RAIZ / "crates/ore-core/src/flow.rs").read_text(encoding="utf-8")
n = len(re.findall(r"Origin::Computed => \(Code::Oos4001", flow))
print("   los dos salen del MISMO sitio, eligiendo por procedencia (%d veces):" % n)
print("     Computed  -> OOS4001 · «computada por join»")
print("     Declared  -> OOS4002 · «declarada»")
print("     Inherited -> OOS4002 · «heredada»")
print()
print("   La regla violada es una sola —`L no debe alcanzar C salvo L ⊑ C`— asi")
print("   que dos codigos solo se justifican si el REMEDIO es distinto. Lo es:")
print()
print("     con `OOS4002` el nivel esta ESCRITO en algun sitio. Se quita el")
print("       campo de la vista, se eleva el conducto, o se relaja el suelo —y")
print("       las tres son una linea que alguien puede ir a buscar")
print("     con `OOS4001` no hay linea. El nivel salio de un `join`, asi que")
print("       para bajarlo hay que ir a los ORIGENES de la derivacion, que son")
print("       otras propiedades y pueden estar en otra entidad")
print()
print("   -> es el mismo criterio con el que `OOS2024` y `OOS2025` son dos: «las")
print("      dos condiciones tienen remedios distintos, y por eso son dos")
print("      codigos». Aqui ademas el mensaje tiene que decir POR QUE el nivel")
print("      es ese, porque el autor no lo escribio y no lo reconoce.")

# -- D - EL CORPUS -----------------------------------------------------------
print()
print("D - EL CORPUS: cuantas derivadas hay, y cuantas SUBEN de verdad")
derivadas = []
for k, d, f in docs(OOS, RAIZ / "casos"):
    if k != "Entity":
        continue
    for m in re.finditer(r"^    (\w+):\s*\n(?:      .*\n)*?      derivedFrom:\s*\[([^\]]*)\]",
                         d, re.M):
        derivadas.append((f.name, m.group(1),
                          [x.strip() for x in m.group(2).split(",")]))
print("   %-46s %3d" % ("propiedades con `derivedFrom`", len(derivadas)))
por_fichero = collections.Counter(f for f, _, _ in derivadas)
for f, n2 in por_fichero.most_common(5):
    print("     %-30s %d" % (f, n2))
print()
print("   Una derivada solo produce `Computed` si el `join` de sus origenes")
print("   SUBE por encima de lo que ya tenia heredado. Si el suelo del")
print("   datasource ya es igual o mayor, la etiqueta se queda `Inherited` y")
print("   `OOS4001` no puede saltar aunque la derivacion exista.")

# -- E - LO QUE FALTA --------------------------------------------------------
print()
print("E - LO QUE FALTA: el caso, y ahora es alcanzable")
tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
(tmp / "packages/hr/views/comp.yaml").write_text(
    "apiVersion: oos.dev/v1alpha8\nkind: View\n"
    "metadata: { name: comp, namespace: hr }\nspec:\n  owner: team:hr\n  "
    "from: { view: empleados }\n  fields:\n    employeeId: employeeId\n    "
    "totalCompensation: totalCompensation\n"
    '  materialized: { datasource: hr_workday, table: "cache.comp", '
    "key: [employeeId] }\n", encoding="utf-8")
p = tmp / "conduits.yaml"
p.write_text(p.read_text(encoding="utf-8").replace(
    "    materialization.topology:",
    "    materialization.payload:\n      gdpr.sensitivity: high\n"
    "      acme.residency:   eu_only\n      oos.maturity:     REVIEWED\n\n"
    "    materialization.topology:", 1), encoding="utf-8")
salida = correr("validate", tmp)
shutil.rmtree(tmp.parent, ignore_errors=True)
m = re.search(r"error\[(OOS400\d)\]: `([\w.]+)` lleva `([\w.:]+)` \(([^)]+)\)", salida)
if m:
    print("   se copia `totalCompensation` —derivada de dos `critical`— por un")
    print("   conducto de `high`:")
    print("     %s  %s  lleva %s (%s)" % m.groups())
print()
print("   -> y antes del arreglo del sello esto decia `high (heredada)`, o sea")
print("      que `OOS4001` NO ERA ALCANZABLE en el paradigma nuevo. No le")
print("      faltaba un caso: le faltaba poder saltar. El censo de paridad lo")
print("      senalo como el unico ciego, y el motivo era ese.")
