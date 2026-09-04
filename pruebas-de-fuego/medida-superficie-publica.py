# -*- coding: utf-8 -*-
"""El terreno de la declaracion publico/privado, antes de decidir nada.

`docs/ontologia-como-repositorio.md` §6.3 dice que al paquete le falta DECLARAR
de que vistas se compone. Al medir el terreno de esa declaracion aparecio la
pregunta que la sigue y que nadie ha hecho: **de las que declare, ¿cuales puede
usar otro paquete?** 18 de 55 vistas no las tira nadie, y la medida las llamo
legitimas -«una vista existe para exponer, no para que una entidad la use»-,
pero hoy no hay forma de distinguir una vista EXPUESTA de un peldano intermedio.

Los tres referentes lo resuelven, cada uno a su manera:

  Cognite   el data model LISTA sus views; lo no listado no es del modelo
  Dremio    tres capas por convencion: preparacion -> negocio -> consumidor
  dbt       `access: public | protected | private` en el modelo

Esto NO decide. Mide el terreno, en cinco frentes:

  A. QUIEN TIRA DE CADA VISTA   y desde donde: entidad, otra vista, o nadie
  B. LO MISMO PARA LA TABLA     que es el caso extremo: el puntero fisico de
                                un paquete no es asunto de nadie mas
  C. QUE CRUZA HOY DE VERDAD    el censo entero de referencias por clase, con
                                la frontera del paquete. Es lo que decide cual
                                debe ser el DEFECTO, y no una intuicion
  D. QUIEN YA DECIDE QUE SALE   tres mecanismos existen y ninguno es este.
                                Nombrarlos es lo que evita reinventar uno
  E. EL PRECIO DE CADA DEFECTO  cuantas referencias de hoy romperia cada uno

Lector heredado de `medida-terreno-paquete.py`, con su correccion: se compara
por nombre corto ADEMAS de por el cualificado, porque `backedBy: empleados` y
`backedBy: hr.empleados` son la misma referencia y tirar las cortas mide el
parser en vez del corpus.
"""
import collections
import pathlib
import re
import sys

RAIZ = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\vendor\oos")
CRATES = pathlib.Path(r"C:\ORE\crates")


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-/]+)" % k, d, re.M)
    return m.group(1) if m else None


def metadatos(d):
    if "metadata:" not in d:
        return ""
    return re.split(r"^\s*spec:", d.split("metadata:", 1)[1], maxsplit=1, flags=re.M)[0]


def documentos(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            m = re.search(r"^kind:\s*(\w+)", d, re.M)
            if m:
                yield m.group(1), d, f


def paquete_de(f):
    d = f.parent
    for _ in range(6):
        if (d / "package.yaml").exists():
            return d
        d = d.parent
    return None


def arbol_de(f):
    """El arbol cargable. Dos paquetes de casos distintos no son vecinos, y
    resolver entre ellos inventaria cruces que nadie escribio."""
    d = f.parent
    for _ in range(8):
        if (d / "ontology.config.yaml").exists():
            return d
        d = d.parent
    return None


def corto(x):
    return x.rsplit(".", 1)[-1]


# ── recolecta ───────────────────────────────────────────────────────────────
# indice[(arbol, kind, nombre)] -> paquete   ·  con nombre corto y cualificado
indice = {}
docs = []          # (kind, qn, paquete, arbol, cuerpo, fichero)

for kind, d, f in documentos(RAIZ):
    p, a = paquete_de(f), arbol_de(f)
    qn = (campo(metadatos(d), "namespace") or "") + "." + (campo(metadatos(d), "name") or "?")
    docs.append((kind, qn, p, a, d, f))
    if p is not None:
        indice[(a, kind, qn)] = p
        indice.setdefault((a, kind, corto(qn)), p)


def dueno(arbol, kind, ref):
    """De que paquete es lo que esta referencia nombra, si se sabe."""
    return indice.get((arbol, kind, ref)) or indice.get((arbol, kind, corto(ref)))


# ── las referencias, por clase ──────────────────────────────────────────────
# (clase, kind del destino, [(paquete origen, arbol, destino)])
refs = collections.defaultdict(list)
tirones = collections.defaultdict(list)   # vista destino -> [(clase, paquete origen)]

for kind, qn, p, a, d, f in docs:
    if p is None:
        continue

    def anota(clase, destino_kind, ref):
        refs[(clase, destino_kind)].append((p, a, ref))
        if destino_kind == "View":
            dst = dueno(a, "View", ref)
            tirones[(a, corto(ref))].append((clase, p, dst))

    if kind == "Entity":
        bb = campo(d, "backedBy")
        if bb:
            anota("Entity.backedBy", "View", bb)
        # Ancladas a `^\s+` la primera version perdia las formas EN LINEA
        # —`{ type: String, is: gdpr.email }`—, que son la mayoria del corpus:
        # daba 4 `is` cuando 31 entidades lo usan. Es la misma clase de fallo
        # que ya mordio en `medida-terreno-paquete.py`: medir el parser.
        for m in re.finditer(r"(?:^|[{,\s])is:\s*([\w.]+)", d, re.M):
            anota("Property.is", "Concept", m.group(1))
        impl = re.search(r"implements:\s*\[([^\]]*)\]", d)
        if impl:
            for x in re.split(r"[,\s]+", impl.group(1).strip()):
                if x:
                    anota("Entity.implements", "Interface", x)
        for m in re.finditer(r"(?:^|[{,\s])target:\s*([\w.]+)", d, re.M):
            anota("relations.target", "Entity", m.group(1))
    elif kind == "View":
        mv = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
        if mv:
            anota("View.from.view", "View", mv.group(1))
        mt = re.search(r"from:\s*\{?\s*table:\s*([\w.]+)", d)
        if mt:
            anota("View.from.table", "Table", mt.group(1))
    elif kind == "Interface":
        req = re.search(r"requires:\s*\[([^\]]*)\]", d)
        if req:
            for x in re.split(r"[,\s]+", req.group(1).strip()):
                if x:
                    anota("Interface.requires", "Concept", x)

vistas = [(qn, p, a) for kind, qn, p, a, _, _ in docs if kind == "View" and p]
tablas = [(qn, p, a) for kind, qn, p, a, _, _ in docs if kind == "Table" and p]

print("== corpus:", RAIZ, "==")
print("   vistas:", len(vistas), " tablas:", len(tablas))

# ── A · QUIEN TIRA DE CADA VISTA ────────────────────────────────────────────
print()
print("A - QUIEN TIRA DE CADA VISTA")
clases = collections.Counter()
for qn, p, a in vistas:
    quienes = tirones.get((a, corto(qn)), [])
    if not quienes:
        clases["nadie la tira"] += 1
    elif all(c == "View.from.view" for c, _, _ in quienes):
        clases["solo otra VISTA"] += 1
    elif all(c == "Entity.backedBy" for c, _, _ in quienes):
        clases["solo una ENTIDAD"] += 1
    else:
        clases["las dos cosas"] += 1
for k in ("nadie la tira", "solo otra VISTA", "solo una ENTIDAD", "las dos cosas"):
    print("   %-18s %3d  %s" % (k, clases[k], "#" * clases[k]))
print()
print("   La lectura, y es la que da forma a la declaracion:")
print("     `solo otra VISTA`  es un PELDANO — existe para que otra se apoye;")
print("     `solo una ENTIDAD` es el respaldo de una lectura del propio paquete;")
print("     `nadie la tira`    es la candidata a superficie... o esta muerta,")
print("                        y hoy NADA distingue las dos.")

# ── B · LA TABLA ────────────────────────────────────────────────────────────
print()
print("B - LO MISMO PARA LA TABLA: el caso extremo")
tiradas_t = {corto(r) for _, _, r in refs[("View.from.table", "Table")]}
sin = sum(1 for qn, _, _ in tablas if corto(qn) not in tiradas_t)
print("   %-40s %3d" % ("tablas", len(tablas)))
print("   %-40s %3d" % ("  que alguna vista tira", len(tablas) - sin))
print("   %-40s %3d" % ("  que no tira nadie", sin))
print("   -> el puntero fisico de un paquete no es asunto de otro. Si algun")
print("      dia hay defecto, el de la tabla no admite discusion.")

# ── C · QUE CRUZA HOY ───────────────────────────────────────────────────────
print()
print("C - QUE CRUZA LA FRONTERA DEL PAQUETE, HOY, POR CLASE")
print("   Tres desenlaces, no dos. El tercero es el que importa: un destino que")
print("   no esta en ningun `.yaml` del arbol vive en un `.oob` vendorizado, y")
print("   eso NO es un cruce clandestino — es el mecanismo declarado.")
print()
print("   %-22s %6s %7s %7s %7s" % ("clase", "total", "dentro", "CRUZAN", "en .oob"))
sustrato = ("Entity.backedBy", "View.from.view", "View.from.table")
tot_s = tot_m = cruz_s = cruz_m = oob_s = oob_m = 0
for (clase, dk), lista in sorted(refs.items()):
    dentro = cruzan = fuera = 0
    for p, a, r in lista:
        d = dueno(a, dk, r)
        if d is None:
            fuera += 1
        elif d == p:
            dentro += 1
        else:
            cruzan += 1
    marca = "  <- SUSTRATO" if clase in sustrato else "  <- significado"
    print("   %-22s %6d %7d %7d %7d%s" % (clase, len(lista), dentro, cruzan, fuera, marca))
    # Los que cruzan se NOMBRAN. Un recuento de cruces sin decir cuales no se
    # puede contradecir, y este es el numero del que cuelga el defecto.
    for pp, aa, rr in lista:
        dd = dueno(aa, dk, rr)
        if dd is not None and dd != pp:
            print("        %s -> %s   (%s => %s)" % (pp.name, rr, pp.name, dd.name))
    if clase in sustrato:
        tot_s += len(lista); cruz_s += cruzan; oob_s += fuera
    else:
        tot_m += len(lista); cruz_m += cruzan; oob_m += fuera
print("   %-22s %6s %7s %7s %7s" % ("-" * 22, "", "", "", ""))
print("   %-22s %6d %7d %7d %7d" % ("SUSTRATO", tot_s, tot_s - cruz_s - oob_s, cruz_s, oob_s))
print("   %-22s %6d %7d %7d %7d" % ("significado", tot_m, tot_m - cruz_m - oob_m, cruz_m, oob_m))

# ¿Y los arboles que tienen dependencia declarada, tiran de ella?
con_dep = {a for _, _, _, a, d, _ in docs
           if a and re.search(r"^dependencies:", d, re.M)}
oob = list(RAIZ.rglob("*.oob"))
print()
print("   %-46s %3d" % ("arboles que declaran `dependencies`", len(con_dep)))
print("   %-46s %3d" % ("`.oob` vendorizados en el corpus", len(oob)))

# ── D · QUIEN YA DECIDE QUE SALE ────────────────────────────────────────────
print()
print("D - QUIEN YA DECIDE QUE SALE, y por que ninguno es esto")
gql = (CRATES / "ore-core/src/graphql.rs").read_text(encoding="utf-8", errors="replace")
emite_entidades = "pkg.entities()" in gql
emite_vistas = bool(re.search(r"Kind::View", gql))
print("   %-26s %s" % ("contextSurface", "un CONDUCTO: filtra CAMPOS por su etiqueta"))
print("   %-26s %s" % ("", "efectiva. Es el eje del significado, no el de"))
print("   %-26s %s" % ("", "la pertenencia"))
print("   %-26s %s" % ("link::publicables()", "quita el manifiesto del workspace y el lock"))
print("   %-26s %s" % ("", "de lo que viaja en un `.oob`. Es el eje del"))
print("   %-26s %s" % ("", "artefacto"))
print("   %-26s %s" % ("export --format graphql", "emite entidades: %s · vistas: %s"
                       % ("SI" if emite_entidades else "no", "SI" if emite_vistas else "NO")))
print()
print("   -> la superficie de CONSUMO ya existe y son las entidades: una vista")
print("      no llega jamas a un consumidor de datos. La que no existe es la")
print("      superficie de REFERENCIA — sobre que puede construir OTRO PAQUETE —")
print("      y el enlazado la resuelve plana sobre el arbol entero, sin")
print("      preguntar de que miembro es un documento.")

# ── E · EL PRECIO DE CADA DEFECTO ───────────────────────────────────────────
print()
print("E - EL PRECIO DE CADA DEFECTO, contado sobre lo que hay escrito")
print("   %-46s %3d" % ("referencias al sustrato que cruzan hoy", cruz_s))
print("   %-46s %3d" % ("referencias de significado que cruzan hoy", cruz_m))
print()
print("   privado por defecto  rompe las %d del sustrato" % cruz_s)
print("   publico por defecto  no rompe nada, y no protege nada: el dia que")
print("                        alguien se apoye en un peldano intermedio, ese")
print("                        peldano deja de poder cambiar")
