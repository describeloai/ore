# -*- coding: utf-8 -*-
"""El terreno de la entidad: sus DOS superficies, y cual de las dos falta.

`medida-superficie-publica.py` dejo tres filas decididas por el recuento —el
sustrato no cruza nunca (0 de 70), el significado si (2 de 108, y las dos son
un vocabulario compartido)— y una abierta: **la entidad**.

Esta abierta porque es la unica que esta en los DOS ejes a la vez:

  superficie de CONSUMO      quien pide datos la ve — `export --format graphql`
  superficie de REFERENCIA   sobre que puede construir OTRO PAQUETE

La vista solo estaba en el segundo. La entidad esta en los dos, y hay que medir
si a los dos les falta lo mismo. Cinco frentes:

  A. EL SUJETO       cuantas entidades y como se reparten por paquete
  B. QUIEN LA NOMBRA por clase de referencia, con la frontera del paquete
  C. QUIEN NO        las referencias por CLASIFICACION, que no acoplan a un
                     nombre. Es el contraste que dice de que tamano es B
  D. LA DE CONSUMO   ya existe y ya tiene privado: se mide CORRIENDO el emisor
  E. EL PRECIO       que costaria un defecto privado en la de referencia
"""
import collections
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
RAIZ = pathlib.Path(r"C:\ORE\vendor\oos")
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\superficie-entidad"
)


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
    d = f.parent
    for _ in range(8):
        if (d / "ontology.config.yaml").exists():
            return d
        d = d.parent
    return None


def corto(x):
    return x.rsplit(".", 1)[-1]


def entidad_de(ref):
    """`hr.Employee.baseSalary` -> `hr.Employee`. Un `derivedFrom` y un
    `writes` nombran una PROPIEDAD, y de quien se acopla uno es de su
    entidad."""
    partes = ref.split(".")
    return ".".join(partes[:-1]) if len(partes) > 2 else ref


# ── recolecta ───────────────────────────────────────────────────────────────
indice, docs = {}, []
for kind, d, f in documentos(RAIZ):
    p, a = paquete_de(f), arbol_de(f)
    qn = (campo(metadatos(d), "namespace") or "") + "." + (campo(metadatos(d), "name") or "?")
    docs.append((kind, qn, p, a, d, f))
    if p is not None:
        indice[(a, kind, qn)] = p
        indice.setdefault((a, kind, corto(qn)), p)


def dueno(arbol, ref):
    return indice.get((arbol, "Entity", ref)) or indice.get((arbol, "Entity", corto(ref)))


refs = collections.defaultdict(list)     # clase -> [(paquete, arbol, destino)]
nombrada = collections.defaultdict(set)  # (arbol, corto) -> {clases}
por_clasificacion = collections.Counter()

for kind, qn, p, a, d, f in docs:
    if p is None:
        continue

    def anota(clase, ref):
        refs[clase].append((p, a, ref))
        nombrada[(a, corto(ref))].add(clase)

    if kind == "Entity":
        for m in re.finditer(r"(?:^|[{,\s])target:\s*([\w.]+)", d, re.M):
            anota("relations.target", m.group(1))
        for m in re.finditer(r"derivedFrom:\s*\[([^\]]*)\]", d):
            for x in re.split(r"[,\s]+", m.group(1).strip()):
                if x and x.count(".") >= 2:
                    anota("derivedFrom", entidad_de(x))
    elif kind == "Binding":
        t = campo(d, "targetEntity")
        if t:
            anota("Binding.targetEntity", t)
    elif kind == "Function":
        for m in re.finditer(r"(?:^|[{,\s])writes:\s*([\w.]+)", d, re.M):
            anota("effects.writes", entidad_de(m.group(1)))
    elif kind == "Resolution":
        e = campo(d, "entity")
        if e:
            anota("Resolution.entity", e)
    elif kind == "Ruleset":
        por_clasificacion["Ruleset.targets (por etiqueta)"] += len(
            re.findall(r"atLeast:", d)
        )

for f in sorted(RAIZ.rglob("*.cedar")):
    t = f.read_text(encoding="utf-8", errors="replace")
    por_clasificacion['Cedar `Label::"..."`'] += len(re.findall(r'Label::"', t))
    # La excepcion, y hay que decirla: `resource == Property::"hr.Employee.taxId"`
    # SI acopla a un nombre. Son las unicas de todo el corpus que lo hacen.
    por_clasificacion['Cedar `resource == Property::"..."`'] += len(
        re.findall(r'resource\s*==\s*Property::', t)
    )

entidades = [(qn, p, a) for k, qn, p, a, _, _ in docs if k == "Entity" and p]

print("== corpus:", RAIZ, "==")
print("   entidades en un paquete:", len(entidades))

# ── A · EL SUJETO ───────────────────────────────────────────────────────────
print()
print("A - EL SUJETO: como se reparten por paquete")
porpaq = collections.Counter(p for _, p, _ in entidades)
forma = collections.Counter(porpaq.values())
for n in sorted(forma):
    print("   %2d entidad(es)  %4d paquetes  %s" % (n, forma[n], "#" * min(40, forma[n])))
print("   %-40s %.1f" % ("media del paquete que tiene alguna",
                         len(entidades) / max(1, len(porpaq))))

# ── B · QUIEN LA NOMBRA ─────────────────────────────────────────────────────
print()
print("B - QUIEN NOMBRA A UNA ENTIDAD, y desde donde")
print("   %-24s %6s %7s %7s %7s" % ("clase", "total", "dentro", "CRUZAN", "en .oob"))
tot = dentro_t = cruz_t = fuera_t = 0
for clase, lista in sorted(refs.items()):
    dentro = cruzan = fuera = 0
    for p, a, r in lista:
        d = dueno(a, r)
        if d is None:
            fuera += 1
        elif d == p:
            dentro += 1
        else:
            cruzan += 1
    print("   %-24s %6d %7d %7d %7d" % (clase, len(lista), dentro, cruzan, fuera))
    for p, a, r in lista:
        d = dueno(a, r)
        if d is not None and d != p:
            print("        %s -> %s   (%s => %s)" % (p.name, r, p.name, d.name))
    tot += len(lista); dentro_t += dentro; cruz_t += cruzan; fuera_t += fuera
print("   %-24s %6s %7s %7s %7s" % ("-" * 24, "", "", "", ""))
print("   %-24s %6d %7d %7d %7d" % ("TOTAL", tot, dentro_t, cruz_t, fuera_t))

# ¿Y a cuantas entidades no las nombra nadie?
nadie = sum(1 for qn, _, a in entidades if not nombrada.get((a, corto(qn))))
print()
print("   %-46s %3d  de %d" % ("entidades que NINGUN documento nombra", nadie, len(entidades)))
print("   %-46s %3d" % ("  a las que si nombra alguien", len(entidades) - nadie))
print("   -> a diferencia de la vista, aqui «que no la nombre nadie» NO la hace")
print("      candidata a nada: una entidad no existe para que otra la use, sino")
print("      para que la pidan. Su consumidor no la NOMBRA en un documento.")

# ── C · QUIEN NO LA NOMBRA ──────────────────────────────────────────────────
print()
print("C - LO QUE SE ACOPLA A UNA ENTIDAD SIN NOMBRARLA")
for k, v in sorted(por_clasificacion.items()):
    print("   %-40s %4d" % (k, v))
print("   -> el gobierno apunta a la CLASIFICACION, no a la identidad. Un")
print("      `Ruleset` dice `atLeast: { gdpr.sensitivity: high }` y una politica")
print("      Cedar dice `Label::\"...\"`. 66 de 68 no acoplan a un nombre, asi")
print("      que no entran en la cuenta de B ni les afecta un defecto privado.")
print("      Las 2 que si — `resource == Property::\"hr.Employee.taxId\"` — son la")
print("      excepcion, y viven en el paquete de la entidad que nombran.")

# ── D · LA SUPERFICIE DE CONSUMO ────────────────────────────────────────────
print()
print("D - LA DE CONSUMO YA EXISTE, Y YA TIENE PRIVADO. Se mide corriendola.")
CASO = RAIZ / "conformance/v1alpha5/emit/ceiling-prunes-the-classified/input"


def correr(*a):
    r = subprocess.run([str(ORE), *[str(x) for x in a]], capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


if TMP.exists():
    shutil.rmtree(TMP)
shutil.copytree(CASO, TMP)
# Un gemelo exacto de la entidad del caso, en DRAFT, sobre otro objeto fisico.
t = (TMP / "entities/Customer.yaml").read_text(encoding="utf-8")
(TMP / "entities/Prospect.yaml").write_text(
    t.replace("name: Customer", "name: Prospect").replace("oos.maturity: STABLE",
                                                          "oos.maturity: DRAFT"),
    encoding="utf-8")
b = next((TMP / "bindings").glob("*.yaml")).read_text(encoding="utf-8")
(TMP / "bindings/prospect.yaml").write_text(
    b.replace("hr.Customer", "hr.Prospect").replace("name: crm", "name: crm2")
     .replace("public.tb_customer", "public.tb_prospect"),
    encoding="utf-8")

cod, sdl = correr("export", TMP, "--format", "graphql")
print("   techo del conducto  :", "contextSurface = gdpr.sensitivity:medium · oos.maturity:REVIEWED")
print("   entidades del caso  : hr.Customer (STABLE) · hr.Prospect (DRAFT)")
print("   tipos en el SDL     :", ", ".join(re.findall(r"^type (\w+)", sdl, re.M)) or "(ninguno)")
print("   -> la DRAFT no sale prohibida ni marcada: SALE AUSENTE. El privado de")
print("      la superficie de consumo ya esta, y es la CLASIFICACION.")

conductos = [d for k, _, _, _, d, _ in docs if k == "ConduitPolicy"]
con_cs = sum(1 for d in conductos if "contextSurface" in d)
print()
print("   %-46s %3d" % ("ConduitPolicy en el corpus", len(conductos)))
print("   %-46s %3d" % ("  que autorizan `contextSurface`", con_cs))
print("   -> y sin el no se emite NADA: un conducto no listado esta en el FONDO,")
print("      que no admite nada. Hoy la superficie de consumo esta cerrada en")
print("      %d de %d paquetes con politica, y eso ya es «privado por defecto»"
      % (len(conductos) - con_cs, len(conductos)))
print("      — pero para todo el paquete a la vez, no por entidad.")

# ── E · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("E - EL PRECIO de un defecto privado en la superficie de REFERENCIA")
print("   %-46s %3d" % ("referencias a entidades que cruzan hoy", cruz_t))
print()
print("   Y la pregunta que la medida NO contesta, dicha entera: una entidad")
print("   privada seguiria emitiendose a GraphQL, porque son DOS EJES. Privado")
print("   ahi significaria «otro paquete no puede construir sobre esto», no")
print("   «nadie puede pedirlo». Confundirlos daria un `access:` que apaga el")
print("   producto en vez de acotar el acoplamiento.")
