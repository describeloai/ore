# -*- coding: utf-8 -*-
"""¿Y si el nombre gobernable fuera `<tabla>.<columna>`?

La idea, dicha entera: la POLITICA deja de nombrar `hr.Employee.nationalId` y
nombra la columna fisica; la VISTA se queda con el significado de dominio, y
puede agrupar. Suena natural porque separa dos cosas que hoy van juntas — lo
que un dato ES, y como se pregunta por el.

Esto no la juzga: la coteja contra el arbol. Seis frentes, y cada uno
pregunta si la propuesta CUMPLE lo que promete, no si es bonita:

  A. ¿ES INDEPENDIENTE DE RUTA?  que es lo unico que la propuesta compra. Si
     `<tabla>.<columna>` es a su vez una ruta, no compra nada
  B. ¿ALCANZA A TODO LO GOBERNADO?  toda propiedad gobernada tiene que tener
     columna, o el esquema deja huecos sin nombre
  C. LO QUE HAY QUE REESCRIBIR   las politicas, y contra que resuelven
  D. LAS ETIQUETAS               donde viven hoy y a cuantos sitios irian
  E. ¿PUEDE LA VISTA AGRUPAR?    la segunda mitad de la propuesta, contra la
     gramatica que hay
  F. EL VEREDICTO                que problema resuelve, cual no, y cual crea

Y este arnes conto 2 TABLAS DE 87 en su primera pasada, por leer solo el
`name:` en bloque cuando casi todo el corpus escribe `metadata: { name: x }`
en linea. La cifra era coherente consigo misma —«2 tablas, 2 objetos, 1:1»— y
por eso no chirriaba: un censo que se equivoca por abajo da el MISMO veredicto
que uno correcto cuando el veredicto es «1:1». Se deja escrito porque es el
modo de fallo que este arbol persigue.
"""
import collections
import pathlib
import re

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
INSIGNIA = OOS / "examples/acme-retail"


def documentos(*raices):
    for r in raices:
        if not r.is_dir():
            continue
        for f in sorted(r.rglob("*.yaml")):
            t = f.read_text(encoding="utf-8", errors="replace")
            for d in re.split(r"^---\s*$", t, flags=re.M):
                k = re.search(r"^kind:\s*(\w+)", d, re.M)
                if k:
                    yield k.group(1), d, f


def campo(d, clave):
    """El valor de una clave, escrita en bloque o dentro de un `{ ... }`.

    La primera version solo miraba el `name:` en bloque, y `metadata: { name: x }` en
    linea es la forma que usa casi todo el corpus: conto 2 tablas de 87.
    """
    m = re.search(r"(?:^|[\s{,])%s:\s*\"?([^\s,}\"]+)" % clave, d)
    return m.group(1) if m else None


TODOS = list(documentos(OOS, RAIZ / "casos"))
print("== el nombre gobernable: `<tabla>.<columna>` ==")

# -- A - ¿ES INDEPENDIENTE DE RUTA? ------------------------------------------
print()
print("A - ¿`<tabla>.<columna>` es independiente de RUTA?")
print()
print("   Es lo unico que la propuesta compra, asi que si falla aqui no compra")
print("   nada. Y `Table` es un DOCUMENTO: nombra un objeto fisico con")
print("   `datasource` + `object`. Dos documentos pueden nombrar el mismo.")
objetos = collections.defaultdict(set)
for k, d, f in TODOS:
    if k != "Table":
        continue
    n, ds, ob = campo(d, "name"), campo(d, "datasource"), campo(d, "object")
    ns = campo(d, "namespace")
    if n and ds and ob:
        # Cualificado, y con el PAQUETE en la clave: dos paquetes distintos que
        # nombran el mismo objeto no son un abanico, son dos ontologias.
        objetos[(str(f.parent.parent), ds, ob)].add("%s.%s" % (ns or "?", n))
rep = collections.Counter(len(v) for v in objetos.values())
print()
print("   tablas del corpus                     : %d" % sum(len(v) for v in objetos.values()))
print("   objetos fisicos distintos             : %d" % len(objetos))
for k in sorted(rep):
    print("     %d documento(s) por objeto fisico    : %3d" % (k, rep[k]))
for (pk, ds, ob), ns in sorted(objetos.items()):
    if len(ns) > 1:
        print("     -> %s.%s : %s" % (ds, ob, ", ".join(sorted(ns))))

# Y la RUTA de verdad no es la tabla: es la cadena. Una vista sobre otra vista
# anade un eslabon, y son 42 — casi un tercio.
abastece = collections.Counter()
for k, d, f in TODOS:
    if k != "View":
        continue
    if re.search(r"table:\s*[\w.]", d):
        abastece["from: {table}"] += 1
    elif re.search(r"view:\s*[\w.]", d):
        abastece["from: {view} — un eslabon MAS"] += 1
    elif re.search(r"datasource:\s*[\w.]", d):
        abastece["from: {datasource,object} (v1alpha7)"] += 1
print()
print("   Y de que se abastece cada vista, que es donde esta la ruta de verdad:")
for k in sorted(abastece):
    print("     %-38s %4d" % (k, abastece[k]))
print()
print("   Hoy 1:1, y la gramatica NO lo exige: no hay regla que prohiba dos")
print("   `kind: Table` sobre el mismo `datasource`+`object`. O sea que")
print("   `<tabla>.<columna>` es independiente de ruta POR COSTUMBRE, igual que")
print("   la vista lo es hoy —127 bases, una vista cada una—. Lo verdaderamente")
print("   independiente es `<datasource>.<objeto>.<columna>`, que no es un")
print("   nombre del modelo: es el nombre del sistema ajeno, y cambia cuando el")
print("   DBA renombra una columna sin avisarnos.")
print()
print("   -> la propuesta cambia UN nombre de documento por OTRO nombre de")
print("      documento. La independencia de ruta no la da estar mas abajo: la")
print("      da que haya UNA regla que impida el abanico, y esa regla hoy no")
print("      existe en ningun nivel.")

# -- B - ¿ALCANZA A TODO LO GOBERNADO? ---------------------------------------
print()
print("B - ¿ALCANZA A TODO LO GOBERNADO? La propiedad que no tiene columna")
con_col = sin_col = derivadas = 0
ejemplos = []
for k, d, f in TODOS:
    if k != "Entity":
        continue
    tiene_vista = bool(re.search(r"^\s+backedBy:", d, re.M))
    m = re.search(r"^\s+properties:\s*$(.*?)(?=^\s{2}\w+:|\Z)", d, re.M | re.S)
    if not m:
        continue
    for pm in re.finditer(r"^\s{4}(\w+):(.*?)(?=^\s{4}\w+:|\Z)", m.group(1), re.M | re.S):
        nombre, cuerpo = pm.group(1), pm.group(2)
        if "derivedFrom" in cuerpo:
            derivadas += 1
            if len(ejemplos) < 4:
                ejemplos.append("%s.%s" % (f.stem, nombre))
        elif tiene_vista:
            con_col += 1
        else:
            sin_col += 1
print("   %-52s %4d" % ("propiedades que llegan a una columna", con_col))
print("   %-52s %4d" % ("propiedades DERIVADAS: no tienen columna nunca", derivadas))
print("   %-52s %4d" % ("propiedades de una entidad sin vista: tampoco", sin_col))
print("     derivadas, por ejemplo: %s" % ", ".join(ejemplos))
print()
print("   Una derivada no tiene columna POR DEFINICION —`derivedFrom` es la")
print("   unica excepcion escrita a `OOS2022`— y es exactamente donde vive el")
print("   gobierno interesante: su etiqueta es el `join` de sus origenes")
print("   (`OOS4008`). En el buque insignia esa propiedad es")
print("   `totalCompensation`, y aparece en TRES de las cuatro politicas.")
print()
print("   -> y son DOS huecos distintos, que conviene no sumar:")
print("      %4d derivadas   · no tienen columna POR DEFINICION. Es el hueco" % derivadas)
print("                        estructural: existe aunque todo se migre")
print("      %4d de entidad  · sin vista. Es el hueco de la migracion, y se" % sin_col)
print("           sin vista    cierra escribiendo vistas")
print("      La primera cifra es la que juzga la propuesta. Un nombre que no")
print("      alcanza a lo gobernado no es un nombre gobernable, y las %d" % derivadas)
print("      derivadas son justo donde vive el `join` de etiquetas.")

# -- C - LO QUE HAY QUE REESCRIBIR -------------------------------------------
print()
print("C - LAS POLITICAS: que nombran hoy, y contra que resolverian")
refs = collections.Counter()
for f in sorted(INSIGNIA.rglob("*")):
    if f.suffix not in (".cedar", ".yaml") or "entities" in f.parts:
        continue
    t = f.read_text(encoding="utf-8", errors="replace")
    for m in re.finditer(r"\b([a-z_]+)\.([A-Z]\w+)\.(\w+)", t):
        refs["%s.%s.%s" % m.groups()] += 1
print("   referencias `<ns>.<Entidad>.<propiedad>` en el buque insignia:")
for r, n in sorted(refs.items()):
    print("     %-40s x%d" % (r, n))
print()
print("   Cada una tendria que pasar a `<tabla>.<columna>`. Y la traduccion no")
print("   es mecanica en un sentido: `hr.Employee.nationalId` -> la vista")
print("   `hr.empleados` -> su raiz `hr.workday` -> la columna. Tres saltos, y")
print("   el ultimo lo decide `fields`, que alguien puede cambiar sin tocar la")
print("   politica. Hoy el renombre esta gobernado —`moved`/`reserved`—")
print("   precisamente porque el nombre de campo es una superficie publica.")

# -- D - LAS ETIQUETAS -------------------------------------------------------
print()
print("D - LAS ETIQUETAS: donde viven, y a cuantos sitios irian")
en_meta = en_prop = 0
for k, d, f in TODOS:
    if k != "Entity":
        continue
    meta = re.search(r"^metadata:(.*?)(?=^spec:|\Z)", d, re.M | re.S)
    if meta and "labels:" in meta.group(1):
        en_meta += 1
    spec = re.search(r"^spec:(.*)", d, re.M | re.S)
    if spec:
        en_prop += len(re.findall(r"^\s{6}labels:", spec.group(1), re.M))
print("   entidades que etiquetan en `metadata` (hereda toda propiedad): %d" % en_meta)
print("   propiedades que elevan su propia etiqueta                    : %d" % en_prop)
print()
print("   Una etiqueta de entidad se escribe UNA VEZ y la heredan todas sus")
print("   propiedades. Sobre columnas no hay a quien heredar: `Table` no admite")
print("   `labels` —«es el objeto tal cual esta», `document.rs:280`— y si se le")
print("   admitieran, etiquetar `acme.residency: eu_only` pasaria de una linea")
print("   en `Customer` a una por columna.")
print()
print("   Y el argumento de por que la tabla no las lleva no es de comodidad:")
print("     «una tabla es un HECHO, y los cuatro niveles de ese reticulo son")
print("      verbos de acuerdo. NADIE ACUERDA UN HECHO.»")
print("   Poner el nombre gobernable en la tabla obliga a contradecir eso, o a")
print("   dejar las etiquetas donde estan — y entonces el nombre gobernable y")
print("   la etiqueta viven en documentos distintos, que es peor que hoy.")

# -- E - ¿PUEDE LA VISTA AGRUPAR? --------------------------------------------
print()
print("E - «LA VISTA COMO SIGNIFICADO DE DOMINIO, AGRUPABLE»")
doc = (RAIZ / "crates/ore-core/src/document.rs").read_text(encoding="utf-8")
m = re.search(r"Kind::View if version >= ApiVersion::V1Alpha8 => &\[(.*?)\],\n", doc, re.S)
claves = re.findall(r'"(\w+)"', m.group(1)) if m else []
print("   `View`.spec en v1alpha8 : %s" % ", ".join(claves))
ent = re.search(r"Kind::Entity => &\[\n(.*?)\n            \],\n", doc, re.S)
ce = re.findall(r'"(\w+)"', ent.group(1)) if ent else []
print("   `Entity`.spec           : %s" % ", ".join(ce))
print("   lo que la vista NO tiene: %s" % ", ".join(sorted(set(ce) - set(claves))))
fuentes = len(re.findall(r"^\s+(Tabla|Vista|Datasource)\s*[({]",
                         (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8"), re.M))
print()
print("   `from` admite UNA fuente: `{table: X}` o `{view: Y}`. No hay `join`,")
print("   no hay lista. Asi que «agrupable» hoy es exactamente lo que la vista")
print("   NO puede hacer: una vista no cruza dos tablas, y sin `primaryKey` ni")
print("   `relations` tampoco puede decir por donde se cruzarian.")
print()
print("   -> la segunda mitad de la propuesta no es un movimiento de")
print("      significado: es una GRAMATICA NUEVA —join, clave y relaciones en la")
print("      vista— que es justo la mitad de `Entity` que no se mueve sola.")

# -- F - EL VEREDICTO --------------------------------------------------------
print()
print("F - ¿RESUELVE EL PROBLEMA?")
print()
print("   El problema era: si el significado vive en una ruta, no alcanza a los")
print("   demas caminos hacia el mismo hecho. La propuesta lo ataca bajando el")
print("   nombre un piso. Y un piso mas abajo el problema es EL MISMO:")
print()
print("     lo resuelve  · nada, por si solo. `<tabla>.<columna>` es tan ruta")
print("                    como `<vista>.<campo>` mientras no haya una regla")
print("                    que prohiba dos tablas sobre un objeto")
print("     no alcanza   · %d derivadas, que no tienen columna por definicion —" % derivadas)
print("                    y son donde vive el `join` de etiquetas. (Otras %d" % sin_col)
print("                    son de entidades sin vista: ese hueco es de la")
print("                    migracion, no de la propuesta)")
print("     rompe        · las etiquetas dejan de heredarse: %d entidades" % en_meta)
print("                    etiquetan una vez y pasarian a etiquetar por columna")
print("     contradice   · «nadie acuerda un hecho», que es por lo que `Table`")
print("                    no lleva `labels`")
print("     necesita     · join, clave y relaciones en `View`, que no existen")
print()
print("   PERO nombra bien lo que falta, y eso no lo tenia nadie:")
print()
print("     Lo que hace gobernable a un nombre no es su PISO. Es que exista")
print("     UNA regla que garantice que no hay dos nombres para el mismo hecho.")
print("     Hoy esa regla no existe en ningun nivel — ni sobre vistas (127")
print("     bases, una vista cada una POR COSTUMBRE) ni sobre tablas (%d" % len(objetos))
print("     objetos, un documento cada uno POR COSTUMBRE).")
print()
print("     Esa regla se puede escribir HOY, sin mover un solo significado, y")
print("     es la que decide si el peldano 6 es posible: mientras el abanico")
print("     no este prohibido, el significado no puede vivir en una ruta —")
print("     este la ruta arriba o abajo.")
