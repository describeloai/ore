# -*- coding: utf-8 -*-
"""El alcance de darle madurez a la vista, en todo su espectro.

La vista es la unidad -`docs/ontologia-como-repositorio.md`- y hoy no puede
decir en que estado esta: `document.rs::metadata_keys` le admite solo
`name · namespace · description`, asi que no lleva `oos.maturity`. No puede
declararse DRAFT ni deprecarse. Y `ore discover` ya sufre el defecto: su ayuda
dice que propone «entidades y vistas en DRAFT» y solo marca las entidades
-`inductor.rs:1559` contra `inductor.rs:1896`-.

El precedente esta resuelto en el arbol, con el mismo disparador. `document.rs`
sobre `Concept`: «la primera version de este `match` se lo nego por miedo a la
duplicacion. ERA UN ERROR, y lo destapo `confidence`: un concepto acunado por
inferencia tiene que poder declararse DRAFT».

Esto NO decide. Mide el alcance, en seis frentes:

  A. EL SUJETO      cuantas vistas y tablas, y que defecto heredaria cada una
  B. LA PUERTA      el brazo compartido de `metadata_keys`: quien mas esta ahi
                    y por que razon, para que partirlo no arrastre a nadie
  C. EL FLUJO       lo caro. `metadata.labels` de una entidad NO clasifica solo
                    el documento: `flow.rs::propagar` la hereda a TODAS sus
                    propiedades como `Origin::Inherited`. Se mide corriendo el
                    compilador, no leyendolo
  D. EL PRECIO      cuantas vistas materializadas y cuantos conductos habria
                    que tocar si la etiqueta de la vista fluyera igual
  E. LA REGLA       «la lectura no puede ser mas madura que la pregunta»:
                    cuantos pares entidad/vista la violarian hoy
  F. LA SUPERFICIE  quien tendria que ensenarla, y quien hoy no puede

LO QUE SE HIZO DESPUES DE MEDIR, para que el guion siga diciendo la verdad al
volver a correrlo: la vista admite `metadata.labels` con `oos.maturity` como
unica clave -`02-view` §4.1-, la tabla NO, y `ore view` ensena el estado. B pasa
de 10 kinds en el brazo a 9. Lo que sigue SIN decidir es C: la etiqueta de la
vista no fluye, y por eso D sigue costando 0.
"""
import collections
import pathlib
import re
import shutil
import subprocess
import sys

ORE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else r"C:\ORE\target\debug\ore.exe")
RAIZ = pathlib.Path(r"C:\ORE\vendor\oos")
CRATES = pathlib.Path(r"C:\ORE\crates")
TMP = pathlib.Path(
    r"C:\Users\PC\AppData\Local\Temp\claude\C--ORE"
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\madurez-medida"
)

# `oos.maturity` se DERIVA de `status` de ODCS -`package.schema.json`: «el nivel
# del reticulo oos.maturity se deriva de este valor y sirve de valor por defecto
# para las entidades del paquete que no declaren el suyo»-.
DERIVA = {
    "proposed": "DRAFT", "draft": "DRAFT", "active": "STABLE",
    "deprecated": "DEPRECATED", "retired": "DEPRECATED",
}
# Orden ASCENDENTE POR RESTRICTIVIDAD, de `flow.rs::maturity()`. STABLE es el
# fondo: lo que se puede servir a cualquiera. DEPRECATED es el techo.
ORDEN = ["STABLE", "REVIEWED", "DRAFT", "DEPRECATED"]


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


def madurez_declarada(d):
    m = re.search(r"oos\.maturity:\s*(\w+)", metadatos(d))
    return m.group(1) if m else None


# ── recolecta ───────────────────────────────────────────────────────────────
kinds = collections.Counter()
vistas, tablas, entidades, conductos = [], [], [], []
estado_de = {}

for kind, d, f in documentos(RAIZ):
    kinds[kind] += 1
    p = paquete_de(f)
    if kind == "Package" and p is not None:
        estado_de[p] = campo(metadatos(d), "status")
    qn = (campo(metadatos(d), "namespace") or "") + "." + (campo(metadatos(d), "name") or "?")
    if kind == "View":
        vistas.append((p, qn, "materialized:" in d, f))
    elif kind == "Table":
        tablas.append((p, qn, f))
    elif kind == "Entity":
        entidades.append((p, qn, campo(d, "backedBy"), madurez_declarada(d), f))
    elif kind == "ConduitPolicy":
        conductos.append((p, d, f))

print("== corpus:", RAIZ, "==")
print("   paquetes:", len(estado_de), " vistas:", len(vistas), " tablas:", len(tablas))

# ── A · EL SUJETO ───────────────────────────────────────────────────────────
print()
print("A - EL SUJETO: cuantas, y que defecto heredaria cada una de su paquete")
def defecto(p):
    return DERIVA.get(estado_de.get(p) or "", "sin paquete")

for etiqueta, cosas in (("vistas", vistas), ("tablas", tablas)):
    c = collections.Counter(defecto(x[0]) for x in cosas)
    print("   %-8s %3d   %s" % (etiqueta, len(cosas),
                                " · ".join("%s %d" % (k, v) for k, v in sorted(c.items()))))
print("   -> con el defecto derivado de `status`, lo que hay que ESCRIBIR es 0:")
print("      una vista solo declara madurez cuando difiere de la de su paquete,")
print("      que es exactamente como funciona hoy la entidad.")

# ── B · LA PUERTA ───────────────────────────────────────────────────────────
print()
print("B - LA PUERTA: el brazo compartido de `document.rs::metadata_keys`")
doc = (CRATES / "ore-core/src/document.rs").read_text(encoding="utf-8", errors="replace")
# Sin cuantificador anidado: se corta por el brazo y se retrocede hasta el `=>`
# anterior, que es donde empieza este. Un `(A|B)+` sobre 1145 lineas es
# retroceso catastrofico -lo fue-.
MARCA = '=> &["name", "namespace", "description"]'
antes = doc.split(MARCA)[0] if MARCA in doc else ""
brazo = antes.rsplit("=>", 1)[-1] if antes else ""
compaņeros = re.findall(r"Kind::(\w+)", brazo)
print("   comparten el brazo:", len(compaņeros))
for k in compaņeros:
    print("     %-14s %4d documentos en el corpus" % (k, kinds.get(k, 0)))
print("   y las razones NO son una sola -`document.rs` las escribe aparte-:")
print("     Function/Resolution  su integridad SE COMPUTA; una etiqueta seria")
print("                          una afirmacion sobre uno mismo")
print("     Ruleset/Lattice/...  NO PORTAN DATO, luego no tienen clasificacion")
print("     Table                NO LLEVA SIGNIFICADO, y es un HECHO: nadie")
print("                          acuerda un hecho por etapas")
print("   -> sacar a View no toco el argumento de los otros nueve.")

# ── C · EL FLUJO ────────────────────────────────────────────────────────────
print()
print("C - EL FLUJO: que hace hoy `metadata.labels` en una ENTIDAD")
print("   `flow.rs::propagar` -«Heredadas de la entidad: lo cierto del conjunto")
print("   se declara una vez»- las mete en TODAS sus propiedades como")
print("   `Origin::Inherited`. O sea: NO clasifica solo el documento.")
CASO = RAIZ / "conformance/v1alpha8/valid/materialized-view-over-table-within-clearance/input"
if TMP.exists():
    shutil.rmtree(TMP)
shutil.copytree(CASO, TMP)


def correr(*a):
    r = subprocess.run([str(ORE), *[str(x) for x in a]], capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


c0, s0 = correr("validate", TMP)
e = TMP / "entities/Employee.yaml"
e.write_text(
    e.read_text(encoding="utf-8").replace(
        "metadata: { name: Employee, namespace: hr }",
        "metadata: { name: Employee, namespace: hr, labels: { oos.maturity: DRAFT } }",
    ),
    encoding="utf-8",
)
c1, s1 = correr("validate", TMP)
print("   caso de partida                      :", "ok" if c0 == 0 else "error")
print("   la entidad se declara DRAFT          :",
      "ok" if c1 == 0 else ",".join(sorted(set(re.findall(r"OOS\d{4}", s1)))))
for ln in [l for l in s1.splitlines() if "error[" in l][:1]:
    print("     ", ln.strip()[:96])
print("   -> un conducto que NO nombra un reticulo autoriza su FONDO, y el")
print("      fondo de `oos.maturity` es STABLE. Por eso un DRAFT no se copia.")
print("      Es coherente: un borrador no debe aterrizar en produccion.")

# ── D · EL PRECIO ───────────────────────────────────────────────────────────
print()
print("D - EL PRECIO si la etiqueta de la VISTA fluyera igual que la de la entidad")
mat = [v for v in vistas if v[2]]
print("   %-46s %3d" % ("vistas con `materialized`", len(mat)))
con_pay = [c for c in conductos if "materialization.payload" in c[1]]
con_mad = [c for c in con_pay if "oos.maturity" in c[1]]
print("   %-46s %3d" % ("conductos que declaran `materialization.payload`", len(con_pay)))
print("   %-46s %3d" % ("  ...y que ya nombran `oos.maturity`", len(con_mad)))
print("   -> una vista DRAFT con `materialized` dejaria de compilar salvo que su")
print("      conducto eleve `oos.maturity`. Y `discover` NO propone `materialized`")
print("      -«son decisiones de operacion con coste»-, asi que lo inducido no lo")
print("      toca: el precio lo paga quien materializa un borrador a proposito.")

# ── E · LA REGLA ────────────────────────────────────────────────────────────
print()
print("E - LA REGLA que aparece: la LECTURA no puede ser mas madura que la PREGUNTA")
qn_vista = {}
for p, qn, _, _ in vistas:
    qn_vista[(p, qn)] = defecto(p)
    qn_vista[(p, qn.rsplit(".", 1)[-1])] = defecto(p)
pares, violan, sin_vista = 0, [], 0
for p, qn, bb, mad, f in entidades:
    if not bb:
        continue
    dv = qn_vista.get((p, bb)) or qn_vista.get((p, bb.rsplit(".", 1)[-1]))
    if dv is None:
        sin_vista += 1
        continue
    pares += 1
    me = mad or defecto(p)
    if me in ORDEN and dv in ORDEN and ORDEN.index(me) < ORDEN.index(dv):
        violan.append((qn, me, bb, dv))
print("   %-46s %3d" % ("pares entidad->vista cotejables", pares))
print("   %-46s %3d" % ("  la vista esta en otro paquete o no resuelve", sin_vista))
print("   %-46s %3d" % ("  la entidad seria MAS madura que su vista", len(violan)))
for qn, me, bb, dv in violan[:6]:
    print("     %-24s %-10s -> %-16s %s" % (qn, me, bb, dv))
print("   (con el defecto derivado del paquete, entidad y vista arrancan iguales:")
print("    la regla solo muerde cuando alguien declara una de las dos a mano)")
a_mano = collections.Counter(m for _, _, _, m, _ in entidades if m)
print("   entidades que declaran madurez A MANO hoy: %d de %d   %s"
      % (sum(a_mano.values()), len(entidades),
         " · ".join("%s %d" % kv for kv in sorted(a_mano.items()))))
print("   -> es el lado que YA usa el mecanismo. La vista no tiene ni eso.")

# ── F · LA SUPERFICIE ───────────────────────────────────────────────────────
print()
print("F - LA SUPERFICIE: quien tendria que ensenarla")
cli = (CRATES / "ore-cli/src").rglob("*.rs")
menciones = {f.name: len(re.findall(r"maturity", f.read_text(encoding='utf-8', errors='replace')))
             for f in cli}
for n, c in sorted(menciones.items(), key=lambda x: -x[1]):
    if c:
        print("     %-18s %2d menciones de `maturity`" % (n, c))
main = (CRATES / "ore-cli/src/main.rs").read_text(encoding="utf-8", errors="replace")
prom = re.search(r"Promote \{([^}]*)\}", main)
print("     ore promote        toma", (prom.group(1).strip() if prom else "?"),
      "- NO puede direccionar una vista")
print("     ore view           imprime cada vista y no dice su estado")
print("     dev (MCP)          `vocabulario.rs` EXCLUYE `oos.maturity` de los")
print("                        reticulos de la superficie de contexto")

print()
print("VEREDICTO")
print("   La iteracion NO es «anadir un campo». Son tres decisiones:")
print("   1. partir el brazo de `metadata_keys` para View y Table  -> barato,")
print("      y no toca el argumento de los otros ocho kinds;")
print("   2. decidir si la vista admite CUALQUIER reticulo o SOLO `oos.maturity`")
print("      -> lo primero reabre «dos sitios dicen que es una columna»;")
print("         lo segundo no, porque la madurez es del DOCUMENTO;")
print("   3. decidir si esa etiqueta FLUYE como la de la entidad -> es lo que")
print("      da el valor (un borrador no se copia a produccion) y es lo unico")
print("      con precio.")
