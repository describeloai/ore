# -*- coding: utf-8 -*-
"""¿Hasta dónde llega una consola encima de esto, hoy?

La pregunta no es «¿se puede hacer una web?» —siempre se puede— sino **qué
parte de la consola sería fachada sobre algo que ya contesta, y qué parte
tendría que inventar la respuesta**. Lo segundo es lo caro, y es lo que hay
que saber antes de dibujar una pantalla.

  A. LA FORMA DEL ARBOL     que hay, cuanto es y quien depende de quien
  B. QUIEN TOCA LA RED      la frontera, que es lo que decide el despliegue
  C. LA SUPERFICIE          que verbo contesta en JSON y cual solo en prosa
  D. LAS TRES CAPAS         semantico, kinetico y dinamico, pieza a pieza
  E. EL FLUJO DE LA CONSOLA paso a paso, y donde se rompe
"""
import pathlib
import re
import subprocess
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def loc(d):
    return sum(len(texto(f).splitlines()) for f in d.rglob("*.rs"))


print("== la consola encima de esto, medido ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LA FORMA DEL ARBOL")
print()
crates = {}
for c in sorted(CRATES.iterdir()):
    if not (c / "Cargo.toml").is_file():
        continue
    t = texto(c / "Cargo.toml")
    # Las dependencias internas: las que empiezan por `ore-` en columna 0.
    deps = sorted({m.group(1) for m in re.finditer(r"^(ore-[a-z0-9-]+)\s*=", t, re.M)})
    externas = sorted({m.group(1) for m in re.finditer(r"^([a-z][a-z0-9_-]*)\s*=", t, re.M)
                       if not m.group(1).startswith("ore-")})
    crates[c.name] = (loc(c / "src"), deps, externas)

total = sum(v[0] for v in crates.values())
print("   %-20s %7s  %s" % ("crate", "loc", "depende de"))
print("   " + "-" * 74)
for n, (l, deps, _) in crates.items():
    print("   %-20s %7d  %s" % (n, l, " · ".join(deps) if deps else "—"))
print("   %-20s %7d" % ("TOTAL", total))
print()
pesados = crates["ore-core"][0] + crates["ore-view"][0]
parrafo("Catorce crates, y el reparto no es casual: `ore-core` y `ore-view` son "
        "el %d%% del arbol y **ninguno abre una conexion**. Todo lo que toca "
        "el mundo esta en piezas pequenas y separadas." % (100 * pesados // total))

# -- B -------------------------------------------------------------------------
print()
print("B - QUIEN TOCA LA RED, Y QUIEN NO")
print()
RED = ("reqwest", "tokio", "hyper", "postgres", "tokio-postgres", "aws-sdk", "rustls",
       "native-tls", "ureq")
print("   %-20s %-10s %s" % ("crate", "toca red", "por que / con que"))
print("   " + "-" * 74)
hermeticos = []
for n, (_, _, ext) in crates.items():
    toca = sorted(set(ext) & set(RED))
    if not toca:
        hermeticos.append(n)
    print("   %-20s %-10s %s" % (n, "SI" if toca else "no", " · ".join(toca) if toca else ""))
print()
parrafo("%d de %d crates son HERMETICOS: contestan desde el arbol de ficheros y "
        "nada mas. Eso es lo que hace que una consola pueda tener casi toda su "
        "logica en un proceso sin credenciales, y empujar lo unico que las "
        "necesita a un subproceso con la URL entrando por stdin."
        % (len(hermeticos), len(crates)))

# -- C -------------------------------------------------------------------------
print()
print("C - LA SUPERFICIE: QUE CONTESTA EN MAQUINA Y QUE EN PROSA")
print()
main = texto(RAIZ / "crates/ore-cli/src/main.rs")
i0 = main.find("enum Command")
cuerpo = main[i0:main.find("\n}", i0)]
verbos = []
for m in re.finditer(r'^\s*(?:#\[command\(name = "([a-z-]+)"[^\]]*\)\]\s*)?([A-Z][A-Za-z]*)\s*(\{|\(|,)',
                     cuerpo, re.M):
    verbos.append(m.group(1) or re.sub(r"(?<!^)(?=[A-Z])", "-", m.group(2)).lower())
verbos = sorted(set(verbos))
sin = re.search(r"SIN_IMPLEMENTAR: \[&str; \d+\] = \[(.*?)\]", main)
no_hacen = re.findall(r'"([a-z-]+)"', sin.group(1)) if sin else []

# Un verbo «contesta en maquina» si su ruta emite JSON. Se busca el emisor, no
# la palabra: `println!("{}", x.json())` o un `Json::obj`.
def emite_json(modulo):
    t = texto(RAIZ / "crates/ore-cli/src" / (modulo + ".rs"))
    return bool(re.search(r"\.json\(\)|Json::obj|jcs\(\)", t))

MODULO = {
    "view": "vista", "diff": None, "report": "informe", "verify": "verificar",
    "materialize": "materializar", "discover": "inductor", "review": "revision",
    "source": "lector", "package": "paquete", "drift-detect": "deriva",
    "cache": "cache", "lock": "candado", "pack": "empaquetar", "init": "inicio",
    "export": None, "validate": None, "compile": None, "dev": None,
}
print("   %-14s %-12s %s" % ("verbo", "estado", "contesta"))
print("   " + "-" * 66)
for v in verbos:
    if v in no_hacen:
        estado, resp = "declarado", "nada — `SIN_IMPLEMENTAR`"
    else:
        estado = "construido"
        mod = MODULO.get(v)
        if mod and emite_json(mod):
            resp = "JSON"
        elif v in ("diff", "report", "verify"):
            resp = "JSON"
        elif v in ("export", "compile", "pack", "lock"):
            resp = "un artefacto en disco"
        else:
            resp = "prosa"
    print("   %-14s %-12s %s" % (v, estado, resp))
print()
parrafo("Esta es la fila que decide el coste de la consola: **lo que contesta "
        "en prosa hay que volver a contestarlo en JSON**. No es reescribir la "
        "logica —la logica esta y es hermetica— es que la salida se compuso "
        "para un terminal y una pantalla necesita la estructura.")

# -- D -------------------------------------------------------------------------
print()
print("D - LAS TRES CAPAS, PIEZA A PIEZA")
print()


def hay(patron, *rutas):
    return any(re.search(patron, texto(RAIZ / r)) for r in rutas)


CAPAS = [
    ("SEMANTICO", [
        ("el hecho — `Table` con sus dos caras", "crates/ore-core/src/document.rs",
         r'Kind::Table', True),
        ("la pregunta — `View`, con agrupacion", "crates/ore-core/src/vistas.rs",
         r"pub fn agrupacion", True),
        ("el significado — `Entity`, `Concept`, `Interface`", "crates/ore-core/src/document.rs",
         r"Kind::Concept", True),
        ("las relaciones — `relations` y `via`", "crates/ore-core/src/aristas.rs",
         r"via|relations", True),
        ("la identidad entre fuentes — `Resolution`", "crates/ore-core/src/document.rs",
         r"Kind::Resolution", True),
        ("el linaje por columna, al compilar", "crates/ore-view/src/lineage.rs",
         r"pub fn linaje", True),
        ("el gobierno del flujo — `OOS4xxx`", "crates/ore-core/src/flow.rs",
         r"Oos4001", True),
    ]),
    ("KINETICO", [
        ("la superficie del efecto — `Function.effects`", "crates/ore-core/src/effect.rs",
         r"effects", True),
        ("la regla de integridad — `OOS7xxx`", "crates/ore-core/src/effect.rs",
         r"Oos7002", True),
        ("la `Propuesta` y sus cinco identidades", "crates/ore-core/src/propuesta.rs",
         r"pub struct Propuesta|pub fn digest", True),
        ("verificar una propuesta — `ore verify`", "crates/ore-cli/src/verificar.rs",
         r"pub fn ", True),
        ("QUIEN INVOCA la funcion", "crates/ore-cli/src/main.rs",
         r"Command::Invoke", False),
        ("QUIEN APLICA la propuesta", "crates/ore-cli/src/main.rs",
         r"Command::Apply", False),
        ("un driver que ESCRIBA", "crates/ore-driver/src/lib.rs",
         r"fn escribir_filas|verbo escribir", False),
    ]),
    ("DINAMICO", [
        ("el IR del plan, con identidad", "crates/ore-view/src/plan.rs",
         r"pub fn digest", True),
        ("el compilador de deltas", "crates/ore-view/src/delta_compiler.rs",
         r"pub fn motivos", True),
        ("el analizador de refresco", "crates/ore-view/src/refresh_analyzer.rs",
         r"pub fn analizar", True),
        ("el modelo de coste", "crates/ore-view/src/cost_model.rs",
         r"pub fn decidir", True),
        ("el estado parcial y la upquery", "crates/ore-view/src/state_store.rs",
         r"upquery|pub fn ", True),
        ("el mantenedor incremental", "crates/ore-maintain/src/lib.rs",
         r"pub fn ", True),
        ("EJECUTAR el residuo", "crates/ore-cli/src/main.rs",
         r"fn ejecutar_residuo|Command::Query", False),
        ("SERVIR una consulta — `ore serve`", "crates/ore-cli/src/main.rs",
         r"fn servir", False),
    ]),
]
for capa, piezas in CAPAS:
    hechas = sum(1 for _, r, p, _ in piezas if hay(p, r))
    print("   %s — %d de %d" % (capa, hechas, len(piezas)))
    for q, r, p, esperado in piezas:
        esta = hay(p, r)
        marca = "si " if esta else "NO "
        aviso = "" if esta == esperado else "   <-- cambio desde la ultima medida"
        print("     %s %s%s" % (marca, q, aviso))
    print()

# -- E -------------------------------------------------------------------------
print()
print("E - EL FLUJO DE LA CONSOLA, PASO A PASO")
print()
PASOS = [
    ("conectar un origen", "`ore source add` + `ore source check`", "HECHO",
     "la URL viaja por stdin, nunca por argv ni por el documento"),
    ("ver que tiene", "`ore source explore` · `ore source catalog`", "HECHO",
     "y el catalogo se puede capturar a fichero, con procedencia real"),
    ("elegir tablas como hechos", "`ore discover --source`", "HECHO",
     "induce `Table` + `View` + `Entity` por objeto, en DRAFT"),
    ("resolver lo que no se pudo inducir", "`ore review`", "HECHO",
     "un formulario por clase, y `--answers` para automatizarlo"),
    ("modelar vistas sobre los hechos", "editar YAML", "A MEDIAS",
     "`ore view add` induce una vista, pero MODELAR es editar el documento: no "
     "hay verbo que cambie `fields`, `where` o `groupBy`"),
    ("gobernar", "`ore validate` · `ore view`", "HECHO",
     "y es lo mas fuerte que hay: el linaje por columna y el flujo implicito se "
     "comprueban AL COMPILAR, sin abrir nada"),
    ("agrupar en paquetes", "`ore package new/move/split/merge`", "HECHO", ""),
    ("versionar y publicar", "`ore diff` · `ore lock` · `ore pack`", "HECHO",
     "con los cuatro ejes y el bump exigido"),
    ("ver la deriva del origen", "`ore drift-detect`", "HECHO",
     "0 sin deriva, 2 con deriva — el contrato de Terraform"),
    ("poblar una copia", "`ore materialize`", "HECHO", ""),
    ("CONSULTAR", "—", "FALTA", "`ore serve` esta declarado y no hace nada"),
    ("EJECUTAR una funcion", "—", "FALTA", "nadie invoca y nadie aplica"),
]
print("   %-32s %-10s %s" % ("paso", "estado", "con que"))
print("   " + "-" * 76)
for q, con, e, _ in PASOS:
    print("   %-32s %-10s %s" % (q, e, con))
print()
for q, _, e, por in PASOS:
    if por and e != "HECHO":
        print("   · %s — %s" % (q, e))
        parrafo(por, "       ")
        print()
