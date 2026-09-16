# -*- coding: utf-8 -*-
"""MEDIDA · Ontology Forge contra `ore-serve`: qué sección es real y qué verbo le falta.

Forge (consola, Data → Ontology Forge) no lee ficheros: lee `ore-serve`. Antes de
cada iteración de Forge se mide esto, y la iteración es exactamente la diferencia:

    1  lo que `ore-serve` SIRVE     las rutas de `crates/ore-serve/src/rutas.rs` (estático)
    2  lo que CONTESTA en una celda  las de lectura, contra `https://<celda>.ore.paladio.io`
                                     con el token de agente de la celda (como los Jobs)
    3  lo que cada sección NECESITA  la tabla SECCIONES de abajo, verbo a verbo
    4  la forma de lo que llega      `/esquema`: ¿emite labels? ¿relations? — lo que Entities
                                     y Links pueden pintar sin inventar
    5  las extensiones               `x-rubix-displayName` / `x-rubix-titleKey` contra
                                     `ore validate` (docs/forge.md §4): pasan, y son opacas

Sale una tabla por sección —real · parcial · falta— y el orden de las iteraciones.

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-forge-contra-serve.py [celda]
"""
import json
import os
import re
import shutil
import subprocess
import sys

CELDA = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else "victor"
NS = "t-" + CELDA
PROYECTO = "project-8853a180-450d-47be-b83"
IDP = "https://login.paladio.io/realms/rubix"
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORE = os.path.join(RAIZ, "target", "release", "ore.exe" if os.name == "nt" else "ore")

# ── 3 · lo que cada sección necesita ─────────────────────────────────────────
# (método, ruta) tal como se escribirían en `rutas.rs`. Las de `/documentos`,
# `/derivados` y `/acciones` son las tres familias de docs/forge.md §5.
#
# `/documentos/<Kind>` y no `/documentos?kind=<Kind>`: la puerta descarta la cadena de
# consulta a propósito (`http.rs`: ningún dato entra por la URL), y el kind va LITERAL en
# `rutas.rs` —`["documentos", "Entity"]`— para que lo que no se sirve no cuente como
# servido. `rutas_servidas` conserva un segmento con comillas tal cual y uno sin ellas
# como `{n}`, así que `documentos/Entity` sólo cuenta cuando `Entity` está escrito.
SECCIONES = [
    ("Explore", [("GET", "paquetes"), ("GET", "paquetes/{n}/esquema"), ("GET", "arbol"), ("GET", "derivados/diagnosticos")]),
    ("Entities", [("GET", "paquetes/{n}/esquema"), ("GET", "documentos/Entity"), ("PUT", "documentos/Entity/{ns}/{n}"), ("DELETE", "documentos/Entity/{ns}/{n}")]),
    ("Concepts", [("GET", "conceptos"), ("PUT", "documentos/Concept/{ns}/{n}")]),
    ("Links", [("GET", "documentos/Entity"), ("PUT", "documentos/Entity/{ns}/{n}"), ("GET", "derivados/topologia")]),
    ("Interfaces", [("GET", "documentos/Interface"), ("PUT", "documentos/Interface/{ns}/{n}")]),
    ("Views", [("GET", "documentos/View"), ("GET", "documentos/Table"), ("PUT", "documentos/View/{ns}/{n}")]),
    ("Functions", [("GET", "modelos"), ("POST", "modelos"), ("DELETE", "modelos/{n}"), ("GET", "documentos/Function"), ("PUT", "documentos/Function/{ns}/{n}")]),
    ("Policies", [("GET", "documentos/Ruleset"), ("GET", "documentos/ConduitPolicy"), ("GET", "documentos/Lattice"), ("GET", "documentos/RequestPolicy"), ("GET", "politicas")]),
    ("Ontology config", [("GET", "paquetes"), ("POST", "paquetes"), ("GET", "fuentes"), ("POST", "fuentes"), ("GET", "paquetes/{n}/decisiones"), ("POST", "paquetes/{n}/decisiones"), ("GET", "dependencias"), ("POST", "acciones/validar"), ("POST", "acciones/diff")]),
]


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


# ── 1 · lo que ore-serve sirve, leído del código ──────────────────────────────
def kinds_servidos():
    """La tabla `KINDS` de documentos.rs: los kinds que `/documentos/{kind}` resuelve. Lo que
    no está ahí contesta 404, así que no cuenta como servido."""
    src = open(os.path.join(RAIZ, "crates", "ore-serve", "src", "documentos.rs"), encoding="utf-8").read()
    tabla = src[src.index("const KINDS"):]
    tabla = tabla[: tabla.index("];")]
    return re.findall(r'nombre:\s*"(\w+)"', tabla)


def rutas_servidas():
    src = open(os.path.join(RAIZ, "crates", "ore-serve", "src", "rutas.rs"), encoding="utf-8").read()
    kinds = kinds_servidos()
    rutas = set()
    for m in re.finditer(r'\("(GET|POST|PUT|DELETE|PATCH)",\s*\[([^\]]*)\]', src):
        partes = [p.strip() for p in m.group(2).split(",") if p.strip()]
        ruta = "/".join(p.strip('"') if p.startswith('"') else "{n}" for p in partes)
        # `/documentos/{kind}` es genérica en rutas.rs y se resuelve contra la tabla: una
        # ruta por kind servido, y ninguna por los que no están.
        if ruta.startswith("documentos/{n}"):
            for k in kinds:
                rutas.add((m.group(1), ruta.replace("documentos/{n}", "documentos/" + k, 1)))
        else:
            rutas.add((m.group(1), ruta))
    return rutas


def normaliza(ruta):
    return re.sub(r"\{[a-z]+\}", "{n}", ruta.split("?")[0])


# ── 2 · lo que contesta la celda ─────────────────────────────────────────────
def token_del_agente():
    cli = sh("gcloud secrets versions access latest --secret=%s-agente-cliente --project=%s" % (NS, PROYECTO))
    sec = sh("gcloud secrets versions access latest --secret=%s-agente-secreto --project=%s" % (NS, PROYECTO))
    if not cli or not sec:
        return None
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token",
                        "-d", "grant_type=client_credentials", "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec],
                       capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)["access_token"]
    except (ValueError, KeyError):
        return None


def pide(tok, ruta):
    r = subprocess.run(["curl", "-s", "-m", "15", "-o", "-", "-w", "\n%{http_code}", "-H", "authorization: Bearer " + tok,
                        "https://%s.ore.paladio.io/%s" % (CELDA, ruta)], capture_output=True, text=True, encoding="utf-8", errors="replace")
    cuerpo, _, cod = r.stdout.rpartition("\n")
    try:
        return cod, json.loads(cuerpo)
    except ValueError:
        return cod, cuerpo[:200]


# ── 5 · las extensiones contra ore validate ──────────────────────────────────
def extensiones():
    src = os.path.join(RAIZ, "vendor", "oos", "examples", "acme-retail")
    dst = os.path.join(os.environ.get("TEMP", "/tmp"), "forge-ext")
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(src, dst)
    ent = os.path.join(dst, "packages", "hr", "entities", "Employee.yaml")
    base = open(ent, encoding="utf-8").read()

    def valida(texto):
        with open(ent, "w", encoding="utf-8", newline="") as f:
            f.write(texto)
        r = subprocess.run([ORE, "validate", dst], capture_output=True, text=True, encoding="utf-8", errors="replace")
        codigos = sorted(set(re.findall(r"OOS\d{4}", r.stdout + r.stderr)))
        return r.returncode, codigos

    cab = "metadata:\n  name: Employee\n"
    return [
        ("metadata.displayName sin prefijo", valida(base.replace(cab, cab + "  displayName: Empleado\n", 1))),
        ("metadata.x-rubix-displayName", valida(base.replace(cab, cab + "  x-rubix-displayName: Empleado\n", 1))),
        ("spec.titleKey sin prefijo", valida(base.replace("spec:\n  nature: entity\n", "spec:\n  nature: entity\n  titleKey: fullName\n", 1))),
        ("metadata.x-rubix-titleKey: noExiste", valida(base.replace(cab, cab + "  x-rubix-titleKey: noExiste\n", 1))),
    ]


print()
print("  ═══ FORGE CONTRA ORE-SERVE · %s ═══" % CELDA)

servidas = rutas_servidas()
print("\n  ① rutas en rutas.rs: %d" % len(servidas))
for m, r in sorted(servidas):
    print("     %-6s /%s" % (m, r))

tok = token_del_agente()
vivo = {}
if tok:
    for ruta in ["salud", "version", "paquetes", "fuentes", "modelos", "documentos/Entity"]:
        vivo[ruta] = pide(tok, ruta)
    paquetes = vivo["paquetes"][1].get("packages", []) if isinstance(vivo["paquetes"][1], dict) else []
    for p in paquetes:
        vivo["paquetes/%s/esquema" % p["name"]] = pide(tok, "paquetes/%s/esquema" % p["name"])
        vivo["paquetes/%s/decisiones" % p["name"]] = pide(tok, "paquetes/%s/decisiones" % p["name"])
    print("\n  ② lo que contesta %s.ore.paladio.io con el token de ore-agente-%s:" % (CELDA, CELDA))
    for ruta, (cod, cuerpo) in vivo.items():
        resumen = ""
        if isinstance(cuerpo, dict):
            claves = list(cuerpo.keys())[:6]
            n = next((len(v) for v in cuerpo.values() if isinstance(v, list)), None)
            resumen = "%s%s" % (", ".join(claves), (" · %d elementos" % n) if n is not None else "")
        print("     %-34s %s  %s" % ("/" + ruta, cod, resumen))
else:
    print("\n  ② sin token de agente (gcloud/IdP): se mide sólo lo estático")

# ── 4 · la forma de lo que llega: /documentos/Entity (I1) o, si la celda todavía
#        corre un ore-serve sin él, /esquema ─────────────────────────────────────
docs = vivo.get("documentos/Entity", ("", None))
hay_documentos = isinstance(docs[1], dict) and "documentos" in docs[1]
if hay_documentos:
    ents = docs[1]["documentos"]
    con_labels = sum(1 for e in ents if e.get("metadata", {}).get("labels") or any(p.get("labels") for p in e.get("spec", {}).get("properties", {}).values()))
    con_rel = sum(1 for e in ents if e.get("spec", {}).get("relations"))
    campos = sorted({k for e in ents for k in e.keys()})
    print("\n  ④ /documentos/Entity: %d entidades · campos por entidad: %s" % (len(ents), ", ".join(campos)))
    print("     con labels: %d · con relations: %d  → %s" % (con_labels, con_rel,
          "Entities puede pintar sensibilidad y Links puede pintar aristas" if con_labels and con_rel else
          "el verbo llega entero; lo que no hay son labels/relations EN ESTE ARBOL (un discover no las induce)"))
esquemas = [v for k, v in vivo.items() if k.endswith("/esquema") and isinstance(v[1], dict)]
if esquemas and not hay_documentos:
    ents = [e for _, c in esquemas for e in c.get("entities", [])]
    con_labels = sum(1 for e in ents if "labels" in json.dumps(e.get("metadata", {})) or "labels" in json.dumps(e.get("properties", [])))
    con_rel = sum(1 for e in ents if "relations" in e or "relations" in json.dumps(e.get("spec", {})))
    campos = sorted({k for e in ents for k in e.keys()})
    print("\n  ④ /esquema: %d entidades · campos por entidad: %s" % (len(ents), ", ".join(campos)))
    print("     con labels: %d · con relations: %d  → %s" % (con_labels, con_rel,
          "Entities puede pintar sensibilidad y Links puede pintar aristas" if con_labels and con_rel else
          "Entities sin sensibilidad y Links sin aristas hasta que /esquema (o /documentos) las emita"))

# ── 3 · la tabla por sección ─────────────────────────────────────────────────
print("\n  ③ por sección:")
plan = []
for nombre, verbos in SECCIONES:
    hay = [(m, r) for m, r in verbos if (m, normaliza(r)) in servidas]
    faltan = [(m, r) for m, r in verbos if (m, normaliza(r)) not in servidas]
    estado = "real" if not faltan else ("parcial" if hay else "falta")
    plan.append((nombre, estado, hay, faltan))
    print("     %-16s %-8s tiene %d/%d" % (nombre, estado, len(hay), len(verbos)))
    for m, r in faltan:
        print("        falta  %-6s /%s" % (m, r))

# ── 5 · extensiones ──────────────────────────────────────────────────────────
if os.path.exists(ORE):
    print("\n  ⑤ displayName / titleKey contra ore validate (acme-retail):")
    for tag, (rc, codigos) in extensiones():
        print("     %-40s %s %s" % (tag, "pasa " if rc == 0 else "falla", " ".join(codigos)))
else:
    print("\n  ⑤ sin binario `ore` en target/release: no se miden las extensiones")

# ── el orden de las iteraciones ──────────────────────────────────────────────
print("\n  ⇒ iteraciones, en orden (cada una cierra una fila de ③):")
familias = {}
for nombre, estado, hay, faltan in plan:
    for m, r in faltan:
        fam = r.split("/")[0].split("?")[0]
        familias.setdefault(fam, set()).add(nombre)
for i, (fam, secs) in enumerate(sorted(familias.items(), key=lambda x: -len(x[1])), 1):
    print("     I%d  /%-12s desbloquea %s" % (i, fam, ", ".join(sorted(secs))))
print()
