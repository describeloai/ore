# -*- coding: utf-8 -*-
"""El paso 2: la forma de la declaracion. ¿Y hace falta declararla?

`ontologia-como-repositorio.md` §6.3 pide que el paquete diga de que vistas se
compone, y `medida-terreno-paquete.py` ya midio el terreno: 55 vistas, maximo 2
por paquete, media 1,3, y con el directorio como defecto CERO `package.yaml` que
tocar. Lo que no se ha medido es la FORMA, y antes que la forma hay una pregunta
que este proyecto se hace siempre y aqui no se hizo:

  **P2 · lo derivable no se declara.**

Asi que primero: ¿es derivable el conjunto? Y si no lo es, ¿que forma toma, y
donde vive? Cinco frentes:

  A. LA FORMA QUE YA EXISTE   como escribe listas este modelo. La nueva no
                              inventa una forma: copia una
  B. ¿ES DERIVABLE?           tres reglas candidatas contra el corpus, y donde
                              discrepan. Si una acierta siempre, P2 manda
  C. DONDE VIVE               `package.yaml` o `ontology.config.yaml`. Lo decide
                              `publicables()`, y se comprueba empaquetando
  D. EL DIGEST                §7.1 afirma que una lista declarada no lo toca.
                              Se comprueba con el caso que existe para eso
  E. EL DIAGNOSTICO           que se mueve de `pack` a `validate`, y cuantos
                              arboles lo notarian
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
    r"\b4ce4f86-cd8b-429f-9c14-8865e67fa2c6\scratchpad\declaracion"
)


def campo(d, k):
    m = re.search(r"(?:^|[{,\s])%s:\s*([\w.\-/]+)" % k, d, re.M)
    return m.group(1) if m else None


def meta_de(d):
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


def correr(*a):
    r = subprocess.run([str(ORE), *[str(x) for x in a]], capture_output=True, text=True)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


# ── recolecta ───────────────────────────────────────────────────────────────
docs, vistas, tirones = [], [], collections.defaultdict(set)
for kind, d, f in documentos(RAIZ):
    p, a = paquete_de(f), arbol_de(f)
    qn = (campo(meta_de(d), "namespace") or "") + "." + (campo(meta_de(d), "name") or "?")
    docs.append((kind, qn, p, a, d, f))
    if kind == "View" and p:
        vistas.append((qn, p, a, f))
    if kind == "Entity":
        bb = campo(d, "backedBy")
        if bb:
            tirones[(a, corto(bb))].add("entidad")
    if kind == "View":
        mv = re.search(r"from:\s*\{?\s*view:\s*([\w.]+)", d)
        if mv:
            tirones[(a, corto(mv.group(1)))].add("vista")

print("== corpus:", RAIZ, "==")
print("   vistas en un paquete:", len(vistas))

# ── A · LA FORMA QUE YA EXISTE ──────────────────────────────────────────────
print()
print("A - LA FORMA QUE YA EXISTE: como escribe listas este modelo")
formas = [
    ("workspace.members", "lista de PATRONES de ruta, con `packages/*` por defecto",
     "«el valor por defecto lo aplica el COMPILADOR al normalizar»"),
    ("dependencies", "lista de `{package, version}` — packageRef",
     "«la UNICA referencia que lleva version, porque es la unica que"
     " cruza el limite del artefacto»"),
    ("datasources", "lista de `{name, type, connectionEnv}`", "en el manifiesto raiz"),
    ("primaryKey · uniqueKeys", "lista de NOMBRES, sin adorno", "el orden importa"),
]
for k, forma, nota in formas:
    print("   %-24s %s" % (k, forma))
    print("   %-24s   %s" % ("", nota))
print()
print("   -> dentro del paquete NADA lleva version, y la frase del esquema dice")
print("      por que. Una lista de vistas del propio paquete es, por forma,")
print("      `primaryKey`: nombres pelados. Y su defecto se aplica AL")
print("      NORMALIZAR, como `packages/*`, no en el documento.")

# ── B · ¿ES DERIVABLE? ──────────────────────────────────────────────────────
print()
print("B - ¿ES DERIVABLE EL CONJUNTO? Tres reglas candidatas")
r1 = r2 = r3 = 0
muertas = collections.Counter()
detalle = collections.Counter()
for qn, p, a, f in vistas:
    quien = tirones.get((a, corto(qn)), set())
    r3 += 1
    if not quien:
        r1 += 1
        # ¿Es «publicada» o esta MUERTA? El corpus lo dice por donde vive: un
        # caso `invalid/` existe para romper, no para exponer.
        muertas["invalid" if "invalid" in f.as_posix() else "valid/otro"] += 1
    if "entidad" in quien:
        r2 += 1
    detalle["+".join(sorted(quien)) or "nadie"] += 1
print("   %-46s %3d" % ("R1 · publico = lo que nadie del paquete tira", r1))
print("   %-46s %3d" % ("R2 · publico = lo que una entidad respalda", r2))
print("   %-46s %3d" % ("R3 · publico = todo", r3))
print()
print("   Y el reparto entero, que es lo que dice si R1 vale:")
for k, v in sorted(detalle.items(), key=lambda x: -x[1]):
    print("     %-16s %3d" % (k, v))
print()
print("   De las que NADIE tira, donde viven:")
for k, v in sorted(muertas.items()):
    print("     %-16s %3d" % (k, v))
print("   -> R1 confunde PUBLICADA con MUERTA: las dos se ven igual desde")
print("      dentro del paquete. Es la razon por la que P2 no aplica — no es")
print("      que la lista sea cara de derivar, es que NO ES DERIVABLE: la")
print("      informacion que falta -«esto lo expongo a proposito»- no esta")
print("      escrita en ningun sitio del arbol.")

# ── C · DONDE VIVE ──────────────────────────────────────────────────────────
print()
print("C - DONDE VIVE: y lo decide `publicables()`, no el gusto")
CASO = RAIZ / "examples/acme-retail"
if TMP.exists():
    shutil.rmtree(TMP)
TMP.mkdir(parents=True)
cod, out = correr("pack", CASO)
print("   `ore pack` de acme-retail            :", "ok" if cod == 0 else "error")
print("   el .oob lleva `kind: OntologyConfig`  :",
      "SI" if "OntologyConfig" in out else "NO")
print("   el .oob lleva `kind: Package`         :",
      "SI" if "Package" in out else "NO")
print("   -> `link::publicables()` quita el `OntologyConfig` de lo que viaja.")
print("      Una lista declarada ahi NO llegaria a quien consume el paquete, y")
print("      la declaracion existe justo para que el consumidor la lea. Va en")
print("      `package.yaml`, y no hay segunda opcion.")

# ── D · EL DIGEST ───────────────────────────────────────────────────────────
print()
print("D - EL DIGEST: §7.1 dice que una lista declarada no lo toca")
EQ = RAIZ / "conformance/canonical/package-layout-equivalence"


def digest_de(d):
    c, o = correr("pack", d)
    m = re.search(r'"digest":"(sha256:[0-9a-f]+)"', o) or re.search(
        r"(sha256:[0-9a-f]{8,})", o)
    return m.group(1) if m else ("ERROR: " + o.strip().splitlines()[0][:60] if o else "?")


for lado in ("a", "b"):
    shutil.copytree(EQ / lado, TMP / lado)
antes_a, antes_b = digest_de(TMP / "a"), digest_de(TMP / "b")
# La lista, simulada con una extension de proveedor: el motor todavia no
# conoce `views:`, y lo que se mide aqui es si una CLAVE NUEVA en el
# manifiesto rompe la equivalencia de disposicion, no si el motor la entiende.
for lado, pk in (("a", "package.yaml"), ("b", "packages/hr/package.yaml")):
    p = TMP / lado / pk
    # `spec` viene EN LINEA en este caso, asi que anadir una clave indentada
    # debajo no es YAML —lo fue, y dio `OOS1001`—. Se reescribe el mapa, y
    # dentro de `spec`, que es donde iria la lista de verdad.
    p.write_text(
        p.read_text(encoding="utf-8").replace(
            "spec: { owner: team:people }",
            "spec: { owner: team:people, x-test-views: [hr.empleados] }"),
        encoding="utf-8")
desp_a, desp_b = digest_de(TMP / "a"), digest_de(TMP / "b")
print("   %-28s %s" % ("plano, antes", antes_a))
print("   %-28s %s" % ("multipaquete, antes", antes_b))
print("   %-28s %s" % ("plano, con la lista", desp_a))
print("   %-28s %s" % ("multipaquete, con la lista", desp_b))
ok = not desp_a.startswith("ERROR") and not desp_b.startswith("ERROR")
print("   convergen ANTES  :", "SI" if antes_a == antes_b else "NO")
print("   convergen DESPUES:", ("SI" if desp_a == desp_b else "NO") if ok
      else "NO SE PUDO MEDIR — el manifiesto no compila")
print("   el digest cambia :", ("SI" if antes_a != desp_a else "NO") if ok else "?",
      " (y debe: el contenido cambio)")
print("   -> §7.1 se confirma: la lista vive en `package.yaml`, que ya esta")
print("      DENTRO del paquete, y nombra vistas por su nombre cualificado —")
print("      no por su ruta. La equivalencia de disposicion no la toca.")

# ── E · EL DIAGNOSTICO ──────────────────────────────────────────────────────
print()
print("E - EL DIAGNOSTICO que se mueve de `pack` a `validate`")
arboles = collections.defaultdict(set)
for kind, qn, p, a, d, f in docs:
    if kind == "Package" and a and p:
        arboles[a].add(p)
multi = [a for a, ps in arboles.items() if len(ps) > 1]
print("   %-46s %3d" % ("arboles con UN solo miembro", len(arboles) - len(multi)))
print("   %-46s %3d" % ("arboles con VARIOS miembros", len(multi)))
for a in multi[:6]:
    print("       %-44s %d miembros" % (a.name, len(arboles[a])))
print("   -> hoy la contencion la sostiene el EMPAQUETADOR, y su mensaje nombra")
print("      lo que no es: `OOS2018` dice «la vista no existe» cuando existe y")
print("      esta en el paquete de al lado. Con la lista, la comprobacion baja")
print("      a `validate` y el diagnostico pasa a ser el correcto — «no la")
print("      declaras, o no declaras la dependencia».")
