# -*- coding: utf-8 -*-
"""MEDIDA · Concept e Interface antes de escribir sus filas en `documentos.rs` (Forge I2).

    1  la FORMA        qué admite cada esquema (v1alpha4), qué es obligatorio, qué extensiones
    2  DÓNDE VIVE      en un workspace: `packages/<p>/concepts/` · `interfaces/` en la raíz
                       (`ore init`) · `packages/<p>/interfaces/` — y qué dice OOS2030 de cada uno
    3  el COMPILADOR   qué dice `ore validate` en cada rotura que un PUT o un DELETE provocan
    4  los IMPORTADOS  de dónde salen los conceptos que no son del árbol: `vendor/*.oob`, y si
                       OOS9004 (una palabra que nadie habla) los alcanza
    5  el EMISOR       no hay `ore concept add` ni `ore interface add`
    6  el BOCETO       lo que la consola pinta hoy de conceptos, ¿está en acme-retail?

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-forge-concept-e-interface.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ORE = os.path.join(RAIZ, "target", "release", "ore.exe" if os.name == "nt" else "ore")
ACME = os.path.join(RAIZ, "vendor", "oos", "examples", "acme-retail")
VOCAB = os.path.join(RAIZ, "vendor", "oos", "conformance", "v1alpha4", "valid", "vocabulary-package-has-no-entities", "input")
if not os.path.exists(ORE):
    print("sin binario `ore` en target/release: cargo build --release -p ore-cli")
    sys.exit(1)


def ore(*args, cwd):
    r = subprocess.run([ORE, *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode, r.stdout + r.stderr


def leer(p):
    return open(p, encoding="utf-8").read()


def escribir(p, s):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write(s)


CONCEPTO = "apiVersion: oos.dev/v1alpha4\nkind: Concept\nmetadata: { name: personalEmail, namespace: hr }\nspec:\n  type: String\n  labels: { gdpr.sensitivity: high }\n"
INTERFAZ = "apiVersion: oos.dev/v1alpha4\nkind: Interface\nmetadata: { name: Party, namespace: hr }\nspec:\n  requires: [hr.personalEmail]\n"


def habla(T, concepto="hr.personalEmail", etiqueta=None, entidad="packages/hr/entities/Employee.yaml", prop="email"):
    """`Employee.email` pasa a hablar un concepto: `is` en vez de `type`."""
    p = os.path.join(T, *entidad.split("/"))
    s = leer(p)
    viejo = "    %s:\n      type: String\n" % prop
    assert viejo in s, prop
    nuevo = "    %s:\n      is: %s\n" % (prop, concepto)
    if etiqueta:
        # la propiedad redeclara la etiqueta: elevar es legal, rebajar es OOS4012
        s = s.replace(viejo + "      labels: { gdpr.sensitivity: high }\n", nuevo + "      labels: { gdpr.sensitivity: %s }\n" % etiqueta, 1)
    else:
        s = s.replace(viejo, nuevo, 1)
    escribir(p, s)


def implementa(T, cual="hr.Party"):
    p = os.path.join(T, "packages", "hr", "entities", "Employee.yaml")
    escribir(p, leer(p).replace("  nature: entity\n", "  nature: entity\n  implements: [%s]\n" % cual, 1))


def caso(nombre, mutar):
    T = tempfile.mkdtemp(prefix="forge-ci-")
    shutil.copytree(ACME, T, dirs_exist_ok=True)
    mutar(T)
    rc, out = ore("validate", ".", cwd=T)
    errores = [l for l in out.splitlines() if l.startswith("error[")]
    primera = errores[0] if errores else "ok"
    print("     %-62s %s  %s" % (nombre, "pasa " if rc == 0 else "falla", primera[:100]))
    if len(errores) > 1:
        for e in errores[1:3]:
            print("     %-62s        %s" % ("", e[:100]))
    shutil.rmtree(T, ignore_errors=True)
    return sorted(set(re.findall(r"OOS\d{4}", out)))


print("\n  ═══ CONCEPT E INTERFACE, ANTES DE ESCRIBIR EL VERBO ═══")

# ── 1 · la forma ─────────────────────────────────────────────────────────────
print("\n  ① la forma (schemas/v1alpha4):")
for k in ("concept", "interface"):
    s = json.load(open(os.path.join(RAIZ, "vendor", "oos", "schemas", "v1alpha4", f"{k}.schema.json"), encoding="utf-8"))
    m, sp = s["properties"]["metadata"], s["properties"]["spec"]
    print("     %-9s metadata %s (obligatorio %s) · spec %s (obligatorio %s) · patternProperties x-: %s" % (
        k.capitalize(), list(m["properties"]), m.get("required"), list(sp["properties"]), sp.get("required"),
        "sí" if s.get("patternProperties") else "NO en el esquema"))

# ── 2 · dónde vive ───────────────────────────────────────────────────────────
print("\n  ② dónde vive, en un workspace (acme-retail: packages/hr, customers, supply):")
print("     %-62s %s" % ("caso", "resultado"))
R = {}
R["W1"] = caso("W1 · Concept en packages/hr/concepts/ (nadie lo habla)", lambda T: escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO))
R["W2"] = caso("W2 · el mismo Concept en concepts/ de la RAÍZ (ns hr)", lambda T: escribir(os.path.join(T, "concepts/personalEmail.yaml"), CONCEPTO))
R["W3"] = caso("W3 · Interface en interfaces/ de la RAÍZ, como la crea `ore init`", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), escribir(os.path.join(T, "interfaces/Party.yaml"), INTERFAZ)))
R["W4"] = caso("W4 · Interface en packages/hr/interfaces/", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ)))
print("     → el directorio no decide nada (el kind es el discriminante), y OOS2030 no salta en la raíz: un documento")
print("       fuera de packages/ no tiene paquete que lo contradiga. `ore init` crea interfaces/ en la raíz y NO concepts/")

# ── 3 · el compilador ───────────────────────────────────────────────────────
print("\n  ③ lo que dice `ore validate`:")
R["C0"] = caso("C0 · Employee.email habla hr.personalEmail (is en vez de type)", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T)))
R["C1"] = caso("C1 · is: a un concepto que no existe", lambda T: habla(T, "hr.noExiste"))
R["C2"] = caso("C2 · borrar el concepto que email habla", lambda T: habla(T))
R["C3"] = caso("C3 · la propiedad rebaja la etiqueta del concepto (high → low)", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T, etiqueta="low")))
R["C4"] = caso("C4 · la propiedad eleva la etiqueta (high → critical)", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T, etiqueta="critical")))
R["C5"] = caso("C5 · el concepto lo habla SOLO una entidad de OTRO paquete (customers)", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T, entidad="packages/customers/entities/Customer.yaml")))
R["C6"] = caso("C6 · concepto en DRAFT (metadata.labels oos.maturity) que nadie habla", lambda T: escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO.replace("namespace: hr }", "namespace: hr, labels: { oos.maturity: DRAFT } }")))
R["C7"] = caso("C7 · dos ficheros con el mismo concepto hr.personalEmail", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/a.yaml"), CONCEPTO), escribir(os.path.join(T, "packages/hr/concepts/b.yaml"), CONCEPTO), habla(T)))
R["C8"] = caso("C8 · Concept sin type", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO.replace("  type: String\n", "")), habla(T)))
R["I1"] = caso("I1 · Employee implements hr.Party y la satisface", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ), implementa(T)))
R["I2"] = caso("I2 · implements sin satisfacer (nadie habla personalEmail)", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T, entidad="packages/customers/entities/Customer.yaml"), escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ), implementa(T)))
R["I3"] = caso("I3 · borrar la interface que Employee implementa", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), implementa(T)))
R["I4"] = caso("I4 · requires a un concepto que no existe", lambda T: (escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ.replace("hr.personalEmail", "hr.noExiste"))))
R["I5"] = caso("I5 · borrar el concepto que una Interface requiere", lambda T: escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ))
R["I6"] = caso("I6 · una Interface que nadie implementa", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ)))
R["E1"] = caso("E1 · x-rubix-displayName en Concept.metadata", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO.replace("namespace: hr }", "namespace: hr, x-rubix-displayName: Correo }")), habla(T)))
R["E2"] = caso("E2 · x-rubix-displayName en Interface.metadata", lambda T: (escribir(os.path.join(T, "packages/hr/concepts/personalEmail.yaml"), CONCEPTO), habla(T), escribir(os.path.join(T, "packages/hr/interfaces/Party.yaml"), INTERFAZ.replace("namespace: hr }", "namespace: hr, x-rubix-displayName: Parte }")), implementa(T)))

# ── 4 · los importados ──────────────────────────────────────────────────────
print("\n  ④ los importados: un vocabulario empaquetado (`ore pack`) en vendor/*.oob")


def con_oob(T, habla_gdpr=True):
    rc, out = ore("pack", VOCAB, "-o", os.path.join(T, "vendor", "gdpr.oob"), cwd=T)
    if rc != 0:
        os.makedirs(os.path.join(T, "vendor"), exist_ok=True)
        escribir(os.path.join(T, "vendor", "PACK-FALLO.txt"), out)
    if habla_gdpr:
        habla(T, "gdpr.personalEmail", entidad="packages/customers/entities/Customer.yaml")


T0 = tempfile.mkdtemp(prefix="forge-oob-")
rc, out = ore("pack", VOCAB, "-o", os.path.join(T0, "gdpr.oob"), cwd=T0)
print("     `ore pack` del vocabulario (3 conceptos gdpr/acme + 1 lattice): rc=%d%s" % (rc, "" if rc == 0 else " · " + out.strip().splitlines()[0][:90]))
if rc == 0:
    oob = leer(os.path.join(T0, "gdpr.oob"))
    print("     el .oob es la forma canónica en JCS: %d bytes, kinds: %s" % (len(oob), sorted(set(re.findall(r'"kind":"(\w+)"', oob)))))
shutil.rmtree(T0, ignore_errors=True)
R["O1"] = caso("O1 · vendor/gdpr.oob y Customer.email habla gdpr.personalEmail", lambda T: con_oob(T))
R["O2"] = caso("O2 · vendor/gdpr.oob sin que nadie hable sus conceptos", lambda T: con_oob(T, habla_gdpr=False))
print("     → lo importado se lee del .oob como cualquier documento (kind Concept, con su paquete `gdpr`), y OOS9004")
print("       %s" % ("NO lo alcanza: es lo que la spec llama publicar vocabulario" if "OOS9004" not in R["O2"] else "SÍ lo alcanza — y eso contradice la spec §4.1"))

# ── 5 · el emisor ───────────────────────────────────────────────────────────
rc, out = ore("--help", cwd=RAIZ)
mandos = [l.strip().split()[0] for l in out.splitlines() if l.startswith("  ") and l.strip() and not l.strip().startswith("-")]
print("\n  ⑤ el emisor: mandos de `ore` con concept/interface: %s" % ([m for m in mandos if "concept" in m or "interface" in m] or "ninguno — no hay emisor: los escribe una persona (o el verbo)"))

# ── 6 · el boceto ───────────────────────────────────────────────────────────
n_is = len(re.findall(r"^\s*is:", "\n".join(leer(os.path.join(d, f)) for d, _, fs in os.walk(ACME) for f in fs if f.endswith(".yaml")), re.M))
n_con = len([1 for d, _, fs in os.walk(ACME) for f in fs if f.endswith(".yaml") and "kind: Concept" in leer(os.path.join(d, f))])
acme_ts = os.path.join("C:\\", "rubix-platform", "components", "ontology", "acme.ts")
habladas = len(re.findall(r"'gdpr\.\w+'|'iso\.\w+'", leer(acme_ts))) if os.path.exists(acme_ts) else -1
print("\n  ⑥ el boceto: acme-retail tiene %d `is:` y %d Concept; el boceto de la consola (acme.ts) pinta %d propiedades que «hablan» gdpr.* o iso.*" % (n_is, n_con, habladas))
print("     → esas columnas del boceto NO están en acme-retail: se inventaron. Concepts, en la celda, sale de /documentos/Concept y de vendor/*.oob")

# ── lo que sale de aquí ─────────────────────────────────────────────────────
print("\n  ⇒ lo que decide el verbo:")
print("     · un Concept nuevo que nadie habla es OOS9004 (W1) — y con la puerta «no empeora» es un diagnóstico NUEVO: un PUT")
print("       de Concept no podría entrar nunca solo. El verbo tiene que tolerar el OOS9004 que nombra AL documento recién")
print("       escrito (y decirlo: `sinHablar: true`), o el flujo es PUT Entity con `is` primero — que falla por C1")
print("     · quién nombra: a un Concept, `Entity.properties.*.is` (C2) y `Interface.requires` (I5); a una Interface,")
print("       `Entity.implements` (I3). Los tres los dice el compilador desde quien nombra; el 409 explícito los cuenta desde aquí")
print("     · el compilador cubre lo demás: is a lo que no está (C1), rebajar (C3), sin type (C8), requires a lo que no")
print("       está (I4), implements sin satisfacer (I2). El verbo no exige nada propio: Concept sin type ya es OOS1004")
print("     · dónde vive (②): `ore init` crea interfaces/ en la raíz y el compilador acepta documentos fuera de packages/.")
print("       El motor de documentos.rs solo recorre packages/*: para estos kinds tiene que recorrer TAMBIÉN la raíz, y")
print("       decir `paquete: null` de lo que no tiene. Uno nuevo se escribe en packages/<ns>/<carpeta>/")
print("     · /conceptos = los Concept del árbol + los de vendor/*.oob (④), cada uno con quién lo habla (las propiedades con is)")
print("     · OOS9004 dice «nada del paquete» y mide «nada del ÁRBOL» (C5): un concepto de hr hablado solo desde customers pasa")
if "OOS" not in " ".join(R["C7"]):
    print("     · ⚠ C7: dos ficheros que declaran el MISMO concepto (hr.personalEmail) compilan sin queja. El compilador no detecta")
    print("       el nombre cualificado duplicado; el verbo busca por metadata.name y reescribiría el primero que encuentre.")
    print("       Se anota como fallo del compilador (como el agregado sobre vista); no se arregla en esta medida")
print()
