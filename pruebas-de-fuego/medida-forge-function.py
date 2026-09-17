#!/usr/bin/env python3
"""
MEDIDA: Function antes de su fila en `documentos.rs` (Ontology Forge, I2).

    1  la FORMA        v1alpha2 → v1alpha8 (sin datasourceRef) → v1alpha9 (runtime: model)
    2  el COMPILADOR   lo que exige escribir una función sobre acme-retail: integridad,
                       endosos, derivadas, una fuente, el Model que nombra, quién la nombra
    3  el MODEL        lo que /modelos escribe y cómo se resuelve `modelo/<n>` sin gateway
    4  el emisor       ¿hay `ore function …`?
    5  el boceto       lo que la consola pinta hoy de Functions

Todo sobre copias limpias de acme-retail en un directorio temporal: ni un fichero del
repositorio se toca, y cada caso parte del árbol de referencia entero.
Uso: PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-forge-function.py
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


# ── las piezas ───────────────────────────────────────────────────────────────
RETICULO = "apiVersion: oos.dev/v1alpha2\nkind: Lattice\nmetadata: { name: assurance, namespace: acme }\nspec:\n  axis: integrity\n  levels: [untrusted, inferred, reviewed, attested]\n"
MODELO = "apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata: { name: v2-lite }\nspec:\n  profile: l4/qwen-7b\n  tier: shared\n  task: chat\n"


def funcion(api="v1alpha8", runtime="wasm", writes="hr.Employee.grade", endosos="    - endorser: attested\n      attestation: attestations/f.intoto.jsonl\n", extra="", meta="", ns="hr", efectos=None):
    ef = efectos if efectos is not None else "    - writes: %s\n      to: IC2\n" % writes
    cuerpo = "apiVersion: oos.dev/%s\nkind: Function\nmetadata: { name: promover, namespace: %s%s }\nspec:\n  runtime: %s\n" % (api, ns, meta, runtime)
    if runtime == "wasm":
        cuerpo += "  entrypoint: dist/promover.wasm\n"
    else:
        cuerpo += "  model: modelo/v2-lite\n  prompt: \"Propón el grado siguiente.\"\n"
    cuerpo += extra
    cuerpo += "  effects:\n" + ef
    if endosos:
        cuerpo += "  endorsements:\n" + endosos
    return cuerpo


def reticulo(T):
    escribir(os.path.join(T, "lattices", "assurance.yaml"), RETICULO)


def etiqueta(T, prop="grade", nivel="reviewed", entidad="packages/hr/entities/Employee.yaml"):
    """La propiedad destino declara integridad: `labels: { acme.assurance: <nivel> }`."""
    p = os.path.join(T, *entidad.split("/"))
    s = leer(p)
    m = re.search(r"^    %s:\n((?:      .*\n)+)" % prop, s, re.M)
    assert m, prop
    bloque = m.group(1)
    if "labels:" in bloque:
        nuevo = re.sub(r"labels: \{ ?", "labels: { acme.assurance: %s, " % nivel, bloque, 1)
    else:
        nuevo = "      labels: { acme.assurance: %s }\n" % nivel + bloque
    escribir(p, s.replace(bloque, nuevo, 1))


def f_en(T, texto, ruta="packages/hr/functions/promover.yaml"):
    escribir(os.path.join(T, *ruta.split("/")), texto)


def base(T, **kw):
    reticulo(T); etiqueta(T); f_en(T, funcion(**kw))


def caso(nombre, mutar):
    T = tempfile.mkdtemp(prefix="forge-fn-")
    shutil.copytree(ACME, T, dirs_exist_ok=True)
    mutar(T)
    rc, out = ore("validate", ".", cwd=T)
    errores = [l for l in out.splitlines() if l.startswith("error[")]
    primera = errores[0] if errores else "ok"
    print("     %-66s %s  %s" % (nombre, "pasa " if rc == 0 else "falla", primera[:110]))
    for e in errores[1:3]:
        print("     %-66s        %s" % ("", e[:110]))
    shutil.rmtree(T, ignore_errors=True)
    return sorted(set(re.findall(r"OOS\d{4}", out)))


print("\n  ═══ FUNCTION, ANTES DE ESCRIBIR EL VERBO ═══")

# ── 1 · la forma ─────────────────────────────────────────────────────────────
print("\n  ① la forma (schemas):")
for v in ("v1alpha2", "v1alpha9"):
    s = json.load(open(os.path.join(RAIZ, "vendor", "oos", "schemas", v, "function.schema.json"), encoding="utf-8"))
    m, sp = s["properties"]["metadata"], s["properties"]["spec"]
    ef = sp["properties"]["effects"]["items"]
    print("     %-9s metadata %s · spec %s (obligatorio %s)" % (v, list(m["properties"]), list(sp["properties"]), sp.get("required")))
    print("     %-9s effects[] %s (obligatorio %s) · runtime %s" % ("", list(ef["properties"]), ef.get("required"), sp["properties"]["runtime"].get("enum")))
print("     v1alpha8: el compilador retira `datasourceRef` del efecto (effect.rs: OOS1005 bajo ≥ v1alpha8; el destino se deriva por backedBy)")
print("     → tres apiVersion con tres formas; Forge escribe v1alpha9 (la única con `runtime: model`) y el compilador decide")

# ── 2 · el compilador ────────────────────────────────────────────────────────
print("\n  ② el compilador sobre acme-retail (hr.Employee.grade sale de la vista empleados ← tabla workday_worker):")
print("     %-66s %s" % ("caso", "resultado"))
R = {}
R["F0"] = caso("F0 · función wasm sobre hr.Employee.grade, tal cual acme-retail", lambda T: f_en(T, funcion()))
R["F1"] = caso("F1 · + retículo integrity (acme.assurance) + grade: reviewed + attested", lambda T: base(T))
R["F2"] = caso("F2 · F1 sin endosos (I(f) = untrusted < reviewed)", lambda T: base(T, endosos=""))
R["F3"] = caso("F3 · F1 con humanApproval SOLO condicional (when)", lambda T: base(T, endosos="    - endorser: humanApproval\n      when: \"input.grade == 'M3'\"\n"))
R["F3b"] = caso("F3b · F1 con humanApproval incondicional", lambda T: base(T, endosos="    - endorser: humanApproval\n"))
R["F4"] = caso("F4 · endosante fuera del vocabulario (teamReview)", lambda T: base(T, endosos="    - endorser: teamReview\n"))
R["F5"] = caso("F5 · writes hr.Employee.totalCompensation (derivedFrom)", lambda T: (reticulo(T), etiqueta(T, "totalCompensation"), f_en(T, funcion(writes="hr.Employee.totalCompensation"))))
R["F6"] = caso("F6 · efectos sobre hr.Employee.grade Y customers.Customer.email (dos fuentes)", lambda T: (base(T, efectos="    - writes: hr.Employee.grade\n    - writes: customers.Customer.email\n"), etiqueta(T, "email", entidad="packages/customers/entities/Customer.yaml")))
R["F7"] = caso("F7 · datasourceRef en el efecto bajo v1alpha8", lambda T: base(T, efectos="    - writes: hr.Employee.grade\n      datasourceRef: hr_workday\n"))
R["F8"] = caso("F8 · writes hr.Employee.noExiste", lambda T: base(T, writes="hr.Employee.noExiste"))
R["F8b"] = caso("F8b · writes hr.NoExiste.grade (la entidad no está)", lambda T: base(T, writes="hr.NoExiste.grade"))
R["F9"] = caso("F9 · sin effects", lambda T: base(T, efectos=""))
R["F10"] = caso("F10 · F1 con grade: attested (exige el techo) y endoso attested", lambda T: (reticulo(T), etiqueta(T, nivel="attested"), f_en(T, funcion())))
R["F11"] = caso("F11 · F1 leyendo baseSalary: inferred (input) → ¿arrastre OOS7001?", lambda T: (base(T), etiqueta(T, "baseSalary", "inferred"), f_en(T, funcion(extra="  input:\n    baseSalary: { type: Money }\n"))))
print("     — runtime: model —")
R["M0"] = caso("M0 · runtime: model bajo v1alpha8 (model/prompt no son claves de v1alpha8)", lambda T: base(T, runtime="model"))
R["M1"] = caso("M1 · runtime: model bajo v1alpha9 SIN Model en el árbol", lambda T: base(T, api="v1alpha9", runtime="model"))
R["M2"] = caso("M2 · M1 + modelos/v2-lite.yaml (kind: Model escrito a mano)", lambda T: (base(T, api="v1alpha9", runtime="model"), escribir(os.path.join(T, "modelos", "v2-lite.yaml"), MODELO)))
R["M3"] = caso("M3 · el Model solo, sin función que lo invoque", lambda T: escribir(os.path.join(T, "modelos", "v2-lite.yaml"), MODELO))
R["M4"] = caso("M4 · wasm bajo v1alpha9 (la fila puede escribir siempre v1alpha9)", lambda T: base(T, api="v1alpha9"))
R["M5"] = caso("M5 · runtime: model con entrypoint además", lambda T: (base(T, api="v1alpha9", runtime="model", extra="  entrypoint: dist/x.wasm\n"), escribir(os.path.join(T, "modelos", "v2-lite.yaml"), MODELO)))
print("     — quién la nombra, y dónde vive —")
DUTY = "apiVersion: oos.dev/v1alpha4\nkind: Ruleset\nmetadata: { name: promociones, namespace: hr }\nspec:\n  owner: team:people-data\n  targets:\n    - entity: hr.Employee\n  duties:\n    - call: hr.promover\n"
R["D1"] = caso("D1 · un Ruleset con duties.call: hr.promover (F1 presente)", lambda T: (base(T), escribir(os.path.join(T, "rulesets", "promociones.yaml"), DUTY)))
R["D2"] = caso("D2 · el mismo duty sin la función (retirarla)", lambda T: (reticulo(T), etiqueta(T), escribir(os.path.join(T, "rulesets", "promociones.yaml"), DUTY)))
R["D3"] = caso("D3 · retirar el Model que M2 invoca", lambda T: base(T, api="v1alpha9", runtime="model"))
R["W1"] = caso("W1 · la función en functions/ de la RAÍZ", lambda T: (reticulo(T), etiqueta(T), f_en(T, funcion(), "functions/promover.yaml")))
R["W2"] = caso("W2 · la función en packages/customers/ con namespace hr (OOS2030)", lambda T: (reticulo(T), etiqueta(T), f_en(T, funcion(), "packages/customers/functions/promover.yaml")))
R["E1"] = caso("E1 · x-rubix-displayName en Function.metadata", lambda T: base(T, meta=", x-rubix-displayName: Promover"))
R["E2"] = caso("E2 · metadata.labels en Function (la ausencia es normativa)", lambda T: base(T, meta=", labels: { oos.maturity: DRAFT }"))
R["E3"] = caso("E3 · dos ficheros con la misma Function (OOS2035)", lambda T: (base(T), f_en(T, funcion(), "packages/hr/functions/otra.yaml")))

# ── 2b · el camino completo: cuántos documentos hasta que una función compila ──
# `hr.empleados` no se puede materializar a propósito (OOS4011: acme-retail no declara
# `materialization.payload`), así que el destino es supply.Shipment.status ← envios ← shipment_v2
print("\n  ②b el camino completo, sobre supply.Shipment.status (envios ← shipment_v2 en erp_snowflake), acumulando:")
print("     %-66s %s" % ("paso", "códigos que quedan"))


def codigos(T):
    rc, out = ore("validate", ".", cwd=T)
    return sorted(set(re.findall(r"OOS\d{4}", out))), out


def paso(nombre, T):
    c, out = codigos(T)
    print("     %-66s %s" % (nombre, ", ".join(c) or "compila"))
    return c, out


FS = "apiVersion: oos.dev/v1alpha9\nkind: Function\nmetadata: { name: cerrar, namespace: supply }\nspec:\n  runtime: wasm\n  entrypoint: dist/cerrar.wasm\n  effects:\n    - writes: supply.Shipment.status\n      to: DELIVERED\n  endorsements:\n    - endorser: attested\n      attestation: attestations/cerrar.intoto.jsonl\n"
T = tempfile.mkdtemp(prefix="forge-fn-camino-")
shutil.copytree(ACME, T, dirs_exist_ok=True)
S = {}
f_en(T, FS, "packages/supply/functions/cerrar.yaml")
S[0], _ = paso("S0 · la función sola", T)
reticulo(T)
S[1], _ = paso("S1 · + lattices/assurance.yaml (eje integrity)", T)
rc, cedar = ore("export", ".", "--format", "cedarschema", cwd=T)
print("     %-66s %s" % ("S2 · `ore export --format cedarschema` con el árbol así:", "rc=%d — %s" % (rc, (cedar.strip().splitlines() or [""])[0][:70])))
if rc == 0:
    escribir(os.path.join(T, "policies", "acme.cedarschema"), cedar)
S[2], _ = paso("S2 · + esquema Cedar regenerado (si pudo)", T)
etiqueta(T, "status", "reviewed", "packages/supply/entities/Shipment.yaml")
S[3], _ = paso("S3 · + status: { acme.assurance: reviewed }", T)
tabla = os.path.join(T, "packages", "supply", "tables", "snowflake.yaml")
st = leer(tabla)
col = re.search(r"^  columns:\n\s+\"?([A-Za-z_.]+)\"?:", st, re.M).group(1)
escribir(tabla, st.replace("    witness: snapshot\n", "    witness: snapshot\n    key: [%s]\n" % col, 1))
S[4], _ = paso("S4 · + shipment_v2 changes.key: [%s]" % col, T)
vista = os.path.join(T, "packages", "supply", "views", "envios.yaml")
escribir(vista, leer(vista).replace("  from: { table: shipment_v2 }\n", "  from: { table: shipment_v2 }\n  materialized: { datasource: erp_snowflake, table: cache.envios }\n", 1))
S[5], salida5 = paso("S5 · + envios materialized (erp_snowflake / cache.envios)", T)
cond = os.path.join(T, "conduits.yaml")
sc = leer(cond)
escribir(cond, sc.replace("  conduits:\n", "  conduits:\n    materialization.payload:\n      gdpr.sensitivity: high\n      acme.residency: eu_only\n      oos.maturity: STABLE\n      acme.assurance: attested\n", 1))
S[6], salida6 = paso("S6 · + conducto materialization.payload autorizado, CON acme.assurance (OOS4002 si no)", T)
rc, cedar = ore("export", ".", "--format", "cedarschema", cwd=T)
print("     %-66s %s" % ("S7 · `ore export --format cedarschema` ahora:", "rc=%d" % rc))
if rc == 0:
    escribir(os.path.join(T, "policies", "acme.cedarschema"), cedar)
S[7], salida6 = paso("S7 · + esquema Cedar regenerado en policies/acme.cedarschema", T)
if S[7]:
    for l in [l for l in salida6.splitlines() if l.startswith("error[")][:4]:
        print("     %-66s   %s" % ("", l[:110]))
BASE_OK = T  # el árbol en el que la función compila (o lo que quede)
print("     → 7 escrituras para UNA función: Lattice, Entity (etiqueta), Table (key), View (copia), ConduitPolicy, la función, y el esquema Cedar regenerado AL FINAL (export exige el árbol válido)")

# sobre ese árbol, las reglas de integridad por fin corren
print("\n  ②c las reglas de integridad, sobre el árbol de S6:")
print("     %-66s %s" % ("caso", "resultado"))


def sobre_base(nombre, mutar):
    T2 = tempfile.mkdtemp(prefix="forge-fn-int-")
    shutil.copytree(BASE_OK, T2, dirs_exist_ok=True)
    mutar(T2)
    rc, out = ore("validate", ".", cwd=T2)
    errores = [l for l in out.splitlines() if l.startswith("error[")]
    print("     %-66s %s  %s" % (nombre, "pasa " if rc == 0 else "falla", (errores[0] if errores else "ok")[:110]))
    for e in errores[1:2]:
        print("     %-66s        %s" % ("", e[:110]))
    shutil.rmtree(T2, ignore_errors=True)
    return sorted(set(re.findall(r"OOS\d{4}", out)))


FN = "packages/supply/functions/cerrar.yaml"


def fs(**kw):
    kw.setdefault("api", "v1alpha9"); kw.setdefault("writes", "supply.Shipment.status"); kw.setdefault("ns", "supply")
    return funcion(**kw).replace("to: IC2", "to: DELIVERED").replace("name: promover", "name: cerrar")


I = {}
I["sin"] = sobre_base("sin endosos (I(f) = untrusted; status exige reviewed)", lambda T: f_en(T, fs(endosos=""), FN))
I["when"] = sobre_base("humanApproval sólo condicional (when)", lambda T: f_en(T, fs(endosos="    - endorser: humanApproval\n      when: \"input.late == true\"\n"), FN))
I["human"] = sobre_base("humanApproval incondicional", lambda T: f_en(T, fs(endosos="    - endorser: humanApproval\n"), FN))
I["quorum"] = sobre_base("humanApproval con quorum: 2", lambda T: f_en(T, fs(endosos="    - endorser: humanApproval\n      quorum: 2\n"), FN))
I["team"] = sobre_base("endosante teamReview", lambda T: f_en(T, fs(endosos="    - endorser: teamReview\n"), FN))
I["deriv"] = sobre_base("writes supply.Shipment.delayDays (derivedFrom)", lambda T: (etiqueta(T, "delayDays", "reviewed", "packages/supply/entities/Shipment.yaml"), f_en(T, fs(writes="supply.Shipment.delayDays"), FN)))
I["dos"] = sobre_base("efectos sobre supply.Shipment.status Y hr.Employee.grade (dos fuentes)", lambda T: (etiqueta(T), f_en(T, fs(efectos="    - writes: supply.Shipment.status\n    - writes: hr.Employee.grade\n"), FN)))
I["ref"] = sobre_base("writes supply.Shipment.noExiste", lambda T: f_en(T, fs(writes="supply.Shipment.noExiste"), FN))
I["dsref"] = sobre_base("datasourceRef en el efecto (v1alpha9)", lambda T: f_en(T, fs(efectos="    - writes: supply.Shipment.status\n      datasourceRef: erp_snowflake\n"), FN))
I["techo"] = sobre_base("status: attested (el techo) con endoso attested", lambda T: (escribir(os.path.join(T, "packages/supply/entities/Shipment.yaml"), leer(os.path.join(T, "packages/supply/entities/Shipment.yaml")).replace("acme.assurance: reviewed", "acme.assurance: attested")), f_en(T, fs(), FN)))
I["techo_h"] = sobre_base("status: attested con humanApproval (¿llega al techo?)", lambda T: (escribir(os.path.join(T, "packages/supply/entities/Shipment.yaml"), leer(os.path.join(T, "packages/supply/entities/Shipment.yaml")).replace("acme.assurance: reviewed", "acme.assurance: attested")), f_en(T, fs(endosos="    - endorser: humanApproval\n"), FN)))
I["lee_in"] = sobre_base("lee quantity: inferred como `input` (¿arrastra?)", lambda T: (etiqueta(T, "quantity", "inferred", "packages/supply/entities/Shipment.yaml"), f_en(T, fs(extra="  input:\n    quantity: { type: Integer }\n"), FN)))
I["lee"] = sobre_base("lee quantity: inferred en una precondición target.quantity → OOS7001", lambda T: (etiqueta(T, "quantity", "inferred", "packages/supply/entities/Shipment.yaml"), f_en(T, fs(extra="  preconditions:\n    - id: hay\n      expr: \"target.quantity > 0\"\n"), FN)))
I["ref_ent"] = sobre_base("writes supply.NoExiste.status (la entidad no está)", lambda T: f_en(T, fs(writes="supply.NoExiste.status"), FN))
I["sin_model"] = sobre_base("runtime: model SIN el Model (sobre el árbol que compila)", lambda T: f_en(T, fs(runtime="model"), FN))
I["model"] = sobre_base("runtime: model + modelos/v2-lite.yaml", lambda T: (f_en(T, fs(runtime="model"), FN), escribir(os.path.join(T, "modelos", "v2-lite.yaml"), MODELO)))
I["sin_fn"] = sobre_base("retirar la función (queda la copia, la clave, el retículo)", lambda T: os.remove(os.path.join(T, *FN.split("/"))))
I["sin_ent"] = sobre_base("retirar la ENTIDAD Shipment con la función puesta", lambda T: os.remove(os.path.join(T, "packages/supply/entities/Shipment.yaml")))
I["duty"] = sobre_base("un Ruleset con duties.call: supply.cerrar", lambda T: escribir(os.path.join(T, "rulesets", "cierres.yaml"), "apiVersion: oos.dev/v1alpha4\nkind: Ruleset\nmetadata: { name: cierres, namespace: supply }\nspec:\n  owner: team:supply-chain\n  targets:\n    - entity: supply.Shipment\n  duties:\n    - call: supply.cerrar\n"))
I["duty_sin"] = sobre_base("el mismo Ruleset y la función retirada", lambda T: (escribir(os.path.join(T, "rulesets", "cierres.yaml"), "apiVersion: oos.dev/v1alpha4\nkind: Ruleset\nmetadata: { name: cierres, namespace: supply }\nspec:\n  owner: team:supply-chain\n  targets:\n    - entity: supply.Shipment\n  duties:\n    - call: supply.cerrar\n"), os.remove(os.path.join(T, *FN.split("/")))))
I["x"] = sobre_base("x-rubix-displayName en metadata", lambda T: f_en(T, fs(meta=", x-rubix-displayName: Cerrar envío"), FN))
shutil.rmtree(BASE_OK, ignore_errors=True)

# ── 3 · el Model y /modelos ──────────────────────────────────────────────────
print("\n  ③ el Model que una función nombra:")
src = leer(os.path.join(RAIZ, "crates", "ore-serve", "src", "modelos.rs"))
print("     · /modelos (modelos.rs): POST escribe modelos/<n>.yaml y SUSCRIBE en el gateway en el mismo acto; sin gateway, 502 y el árbol intacto")
print("     · DELETE /modelos/<n>: %s" % ("409 si el árbol no compila sin él (una Function lo nombra)" if "Retira primero la Function" in src else "?"))
print("     · el verbo comprueba el perfil contra perfiles.json (la lista certificada): un Model escrito a mano por PUT saltaría eso")
print("     → la fila Function no escribe Models: los nombra. `modelo/<n>` resuelve contra /modelos (0027), y M1/M2 dicen qué pasa sin él")

# ── 4 · el emisor ────────────────────────────────────────────────────────────
rc, out = ore("--help", cwd=RAIZ)
mandos = [l.strip().split()[0] for l in out.splitlines() if l.startswith("  ") and l.strip() and not l.strip().startswith("-")]
print("\n  ④ el emisor: mandos de `ore` con function/propuesta: %s" % ([m for m in mandos if "func" in m or "propon" in m or "propuesta" in m] or "ninguno — la escribe una persona (o el verbo)"))
print("     · lo que sí existe: `Propuesta` (propuesta.rs) se coteja contra effects[].writes — la superficie— y la clave de la entidad")

# ── 5 · el boceto ────────────────────────────────────────────────────────────
n_fn = len([1 for d, _, fs in os.walk(ACME) for f in fs if f.endswith(".yaml") and "kind: Function" in leer(os.path.join(d, f))])
n_lat = [f for d, _, fs in os.walk(ACME) for f in fs if f.endswith(".yaml") and "axis: integrity" in leer(os.path.join(d, f))]
print("\n  ⑤ el boceto: acme-retail tiene %d Function y %d retículo de eje integrity (%s)" % (n_fn, len(n_lat), ", ".join(n_lat) or "ninguno"))
print("     → la consola (Functions.tsx) ya dice «no declara funciones» y enseña la forma con scm.approvePurchaseOrder")

# ── lo que sale de aquí ───────────────────────────────────────────────────────
print("\n  ⇒ lo que decide la fila:")
print("     · F0: en acme-retail una función NO PUEDE entrar sola: %s. Antes que la integridad (OOS7xxx) salta que la" % (", ".join(R["F0"]) or "pasa"))
print("       ontología escribiría por una vista VIRTUAL (OOS2025) cuya raíz no declara `changes.key` (OOS2024). Una función es la copia:")
print("       exige View.materialized, Table.changes.key, el conducto materialization.payload autorizado — y en hr.empleados eso está")
print("       cerrado a propósito (acme-retail no declara ese conducto: OOS4011). Sobre supply son SIETE escrituras (②b): Lattice de eje")
print("       integrity, la etiqueta en la propiedad, Table.changes.key, View.materialized, el conducto (con el retículo nuevo, OOS4002 si")
print("       no), la función, y el esquema Cedar regenerado AL FINAL — `ore export --format cedarschema` exige el árbol válido, así que")
print("       no se puede regenerar hasta que todo lo demás esté (S2 vs S7). Con la puerta «no empeora», cada PUT entra si no añade nada")
print("       nuevo: el orden que entra es lattice (deja OOS2013) → conducto → etiqueta → key → materialized → función → acción `cedarschema`")
print("     · ②c: la integridad SE COMPUTA de los endosos: sin ninguno, untrusted (OOS7002); `when` no cierra; humanApproval incondicional")
print("       llega al TECHO (attested) igual que attested; teamReview OOS7004; derivada OOS4008 (antes de OOS7006); dos fuentes OOS2024/25")
print("       de la otra vista; datasourceRef OOS1005 en v1alpha9; OOS7001 arrastra por PRECONDICIONES (`target.x`), NO por `input`.")
print("       El verbo no exige nada propio: lo dice todo el compilador, con código")
print("     · ⚠ HUECO del compilador: `effects[].writes` a una propiedad o a una ENTIDAD que no existe COMPILA (②c: `supply.Shipment.noExiste`")
print("       y `supply.NoExiste.status` pasan; retirar la entidad con la función puesta pasa). La spec (02-function §8) dice OOS2005.")
print("       Como OOS2035: nadie resuelve `writes` (propiedad() devuelve None y `continue`). Se anota; no se arregla en esta medida")
print("     · M0/M1/M2: `runtime: model` sólo bajo v1alpha9 (bajo v1alpha8 es OOS1005); `modelo/<n>` es OOS2005 sin el Model, y un Model")
print("       a mano lo resuelve — pero el que vale es el de POST /modelos (perfil certificado + suscripción). La fila escribe v1alpha9")
print("       (wasm también compila así, M4) y NO escribe Models. `entrypoint` con `runtime: model` es OOS1004 (M5)")
print("     · D: quién nombra a una Function: Ruleset.duties[].call (OOS2001 si se retira). Quién nombra una Function: la propiedad")
print("       (writes), el Model (model), la política (authorization). DELETE Function → 409 si un Ruleset la llama; DELETE Model ya es")
print("       409 en /modelos; y DELETE Entity debería ser 409 si una función la escribe — hoy ni el compilador lo ve (el hueco)")
print("     · E: `x-rubix-displayName` pasa; `metadata.labels` es OOS1005 (normativo: la integridad no se declara sobre uno mismo);")
print("       dos ficheros con la misma función, OOS2035; en la raíz (functions/) compila igual")
print("     · acme-retail: 0 Function, 0 retículo integrity, 0 conducto payload. La celda no tendrá funciones hasta que tenga copia (P1)")
print()
