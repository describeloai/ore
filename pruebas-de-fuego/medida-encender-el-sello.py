# -*- coding: utf-8 -*-
"""Que pasa al encender el sello del indice, y de que esta hecho el corpus.

Tres preguntas, y las tres se contestan contando:

  A. QUIEN GANA SUJETO  cuantos repos tienen aristas, y en que version
  B. LA INVARIANTE      «no cambia un solo resultado de v1alpha1 a v1alpha7».
                        Encender esto la pone a prueba, asi que se mide ANTES
  C. EL PRIMER GOLPE    `OOS4011`: nadie declara el conducto. Nadie.
  D. EL SEGUNDO         `OOS4002`, calculado por el motor sobre cada arista
  E. QUE ES ACME        gramatica de verdad sobre origenes inalcanzables
  F. QUE SI SE PUEDE    los dos drivers que existen, y que haria falta
"""
import collections
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
ORE = RAIZ / "target/debug/ore"


def repos(raiz):
    """Cada raiz de repositorio: donde hay un `ontology.config.yaml`."""
    return sorted(p.parent for p in raiz.rglob("ontology.config.yaml"))


def docs(r):
    for f in sorted(r.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            k = re.search(r"^kind:\s*(\w+)", d, re.M)
            v = re.search(r"^apiVersion:\s*oos\.dev/(\S+)", d, re.M)
            if k:
                yield k.group(1), (v.group(1) if v else "?"), d, f


def meta(d, clave):
    """`metadata` en bloque o en linea — las dos formas viven en el corpus."""
    m = re.search(r"^  %s:\s*(\S+)\s*$" % clave, d, re.M)
    if m:
        return m.group(1)
    m = re.search(r"^metadata:\s*\{([^}]*)\}", d, re.M)
    if m:
        m2 = re.search(r"(?:^|[,\s])%s:\s*([^,}\s]+)" % clave, m.group(1))
        if m2:
            return m2.group(1)
    return "?"


def aristas_de(r):
    """La misma regla que `ore_core::aristas`, leida de la gramatica.

    Entidad con `backedBy` —el camino nuevo—, clave SIMPLE, y relaciones con
    `via` simple. Lo compuesto se descarta, igual que en el nucleo.
    """
    out = []
    for kind, ver, d, f in docs(r):
        if kind != "Entity" or "backedBy:" not in d:
            continue
        pk = re.search(r"^  primaryKey:\s*\[([^\]]*)\]", d, re.M)
        if not pk or "," in pk.group(1):
            continue
        qn = "%s.%s" % (meta(d, "namespace"), meta(d, "name"))
        assert "?" not in qn, "metadata ilegible en %s" % f
        cuerpo = d.split("relations:", 1)[1] if "relations:" in d else ""
        for rel, via in re.findall(r"^    (\w+):[\s\S]*?^      via:\s*\[([^\]]*)\]",
                                   cuerpo, re.M):
            if "," not in via:
                out.append((qn, rel, ver))
    return out


print("== encender el sello del indice ==")
TODOS = repos(OOS) + repos(RAIZ / "casos")

# -- A - QUIEN GANA SUJETO ---------------------------------------------------
print()
print("A - QUIEN GANA SUJETO: repos con aristas por el camino NUEVO")
con_aristas = {}
for r in TODOS:
    a = aristas_de(r)
    if a:
        con_aristas[r] = a
print("   repositorios en el arbol            : %3d" % len(TODOS))
print("   ...con aristas por `backedBy`+`via` : %3d" % len(con_aristas))
for r, a in con_aristas.items():
    vs = sorted({v for _, _, v in a})
    print("     %-46s %d aristas · %s"
          % (r.relative_to(RAIZ).as_posix(), len(a), ", ".join(vs)))

# -- B - LA INVARIANTE -------------------------------------------------------
print()
print("B - LA INVARIANTE: «no cambia un resultado de v1alpha1 a v1alpha7»")
por_version = collections.Counter()
for a in con_aristas.values():
    for _, _, v in a:
        por_version[v] += 1
for v, n in sorted(por_version.items()):
    marca = "  <-- OJO: es <= v1alpha7" if v < "v1alpha8" else ""
    print("     %-12s %3d aristas%s" % (v, n, marca))
viejas = sum(n for v, n in por_version.items() if v < "v1alpha8")
print()
if viejas:
    print("   -> %d aristas de borradores viejos ganarian sujeto. Hay que" % viejas)
    print("      ACOTAR POR VERSION, como `OOS2022`/`OOS2028`: la regla mira")
    print("      solo documentos que declaran v1alpha8+.")
else:
    print("   -> CERO. Todas las aristas del camino nuevo son v1alpha8, asi que")
    print("      encenderlo no puede cambiar un resultado viejo: los repos")
    print("      v1alpha1-7 llegan por `Binding`, y ese sello ya corre y sigue")
    print("      igual. La invariante no corre peligro por si sola.")

# -- C - EL PRIMER GOLPE -----------------------------------------------------
print()
print("C - EL PRIMER GOLPE: `OOS4011` — quien declara el conducto")
declaran = 0
for r in TODOS:
    for _, _, d, _ in docs(r):
        if "materialization.topology" in d:
            declaran += 1
            break
print("   repos que declaran `materialization.topology` : %d de %d"
      % (declaran, len(TODOS)))
print("   repos con aristas que lo declaran             : %d de %d"
      % (sum(1 for r in con_aristas
             if any("materialization.topology" in d for _, _, d, _ in docs(r))),
         len(con_aristas)))
print()
print("   -> el primer efecto de encenderlo NO es una etiqueta que no cabe: es")
print("      que el conducto no este declarado, y omitirlo es cerrarlo (P4).")
print("      Y el reparto tiene gracia: el que lo declara es `casos/jerarquia`,")
print("      escrito a mano para el paradigma nuevo. El que NO lo declara es")
print("      `acme-retail`, que declara `materialization.index` —un nombre que")
print("      no conoce ni la spec ni el motor—. No es que falte la costumbre:")
print("      es que el ejemplo insignia tiene una errata que nadie podia ver.")

# -- D - EL SEGUNDO GOLPE ----------------------------------------------------
print()
print("D - EL SEGUNDO GOLPE: `OOS4002`, calculado por el motor")


def simular(r, aristas):
    """Materializa las dos columnas de cada arista con la autorizacion del
    indice, y deja que el motor calcule el sello."""
    tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
    shutil.copytree(r, tmp)
    pol = next((p for p in tmp.rglob("*.yaml")
                if "kind: ConduitPolicy" in p.read_text(encoding="utf-8",
                                                        errors="replace")), None)
    if pol is None:
        shutil.rmtree(tmp.parent, ignore_errors=True)
        return None
    t = pol.read_text(encoding="utf-8")
    t = re.sub(r"^  conduits:", "  conduits:\n    materialization.payload:\n"
               "      gdpr.sensitivity: medium\n      acme.residency:   eu_only\n"
               "      oos.maturity:     REVIEWED\n", t, count=1, flags=re.M)
    pol.write_text(t, encoding="utf-8")
    hechas, canarios = [], []
    for qn, rel, _ in aristas:
        e = next((d for k, _, d, _ in docs(tmp)
                  if k == "Entity" and meta(d, "name") == qn.split(".")[1]), None)
        assert e is not None, "no se encuentra %s" % qn
        pk = re.search(r"^  primaryKey:\s*\[([^\]]*)\]", e, re.M).group(1).strip()
        bb = re.search(r"^  backedBy:\s*(\S+)", e, re.M).group(1)
        via = re.search(r"^    %s:[\s\S]*?^      via:\s*\[([^\]]*)\]" % rel,
                        e.split("relations:", 1)[1], re.M).group(1).strip()
        ns = qn.split(".")[0]
        destino = next(p for p in tmp.rglob("views/%s.yaml" % bb)).parent
        (destino / ("arista-%s-%s.yaml" % (qn.split(".")[1], rel))).write_text(
            "apiVersion: oos.dev/v1alpha8\nkind: View\n"
            "metadata: { name: arista-%s-%s, namespace: %s }\nspec:\n"
            "  owner: team:x\n  from: { view: %s }\n  fields:\n    %s: %s\n    %s: %s\n"
            # Con `key`: la copia de aristas esta identificada por la clave de
            # origen, igual que el `oretopo` de verdad. Sin ella salta `OOS2023`
            # —fechada por columna y sin con que deduplicar— y eso seria un
            # fallo del arnes, no del sujeto.
            "  materialized: { datasource: %s, table: \"oretopo.x\", key: [%s] }\n"
            % (qn.split(".")[1].lower(), rel, ns, bb, pk, pk, via, via,
               DATASOURCE[r], pk), encoding="utf-8")
        hechas.append("%s.%s" % (qn, rel))

        # EL CANARIO. Un cero solo significa «pasa» si el sello estaba mirando.
        # Esta vista gemela lleva una propiedad CLASIFICADA de la misma entidad
        # y tiene que ser RECHAZADA. Si no lo es, el sello no corrio, y entonces
        # el cero de al lado no vale nada.
        #
        # Una suite que solo mira lo que se rechaza no demuestra nada, y una que
        # solo mira lo que pasa, tampoco.
        clasificada = re.search(r"^    (\w+):\s*\n(?:      .*\n)*?      labels:",
                                e, re.M)
        if clasificada:
            (destino / ("canario-%s-%s.yaml" % (qn.split(".")[1], rel))).write_text(
                "apiVersion: oos.dev/v1alpha8\nkind: View\n"
                "metadata: { name: canario-%s-%s, namespace: %s }\nspec:\n"
                "  owner: team:x\n  from: { view: %s }\n"
                "  fields:\n    %s: %s\n    %s: %s\n"
                "  materialized: { datasource: %s, table: \"oretopo.c\", key: [%s] }\n"
                % (qn.split(".")[1].lower(), rel, ns, bb, pk, pk,
                   clasificada.group(1), clasificada.group(1),
                   DATASOURCE[r], pk), encoding="utf-8")
            canarios.append("canario-%s-%s" % (qn.split(".")[1].lower(), rel))

    s = subprocess.run([str(ORE), "validate", str(tmp)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    shutil.rmtree(tmp.parent, ignore_errors=True)
    return hechas, canarios, s.stdout + s.stderr


# El datasource al que apunta la copia simulada. Las dos formas otra vez —
# `- name: erp` y `- { name: erp, ... }`— y sin esto sale un `OOS2004` que el
# guardia de abajo caza, pero mejor no fabricarlo.
DATASOURCE = {}
for r in con_aristas:
    cfg = (r / "ontology.config.yaml").read_text(encoding="utf-8", errors="replace")
    m = (re.search(r"^  - name:\s*(\S+)", cfg, re.M)
         or re.search(r"^  - \{[^}]*?name:\s*([^,}\s]+)", cfg, re.M))
    assert m, "sin datasource legible en %s" % r
    DATASOURCE[r] = m.group(1)

for r, a in con_aristas.items():
    res = simular(r, a)
    if res is None:
        print("   %-40s sin politica de conductos — no se puede simular"
              % r.name)
        continue
    hechas, canarios, salida = res
    fallos = re.findall(r"error\[OOS4002\]: `([\w.-]+)\.(\w+)` lleva `([\w.:]+)`",
                        salida)
    print("   %s · %d aristas simuladas" % (r.name, len(hechas)))
    vistas_malas = sorted({q for q, _, _ in fallos})
    for q in vistas_malas:
        ets = sorted({e for qq, _, e in fallos if qq == q})
        print("     RECHAZA %-26s %s" % (q, ", ".join(ets)))
    cazados = {c for c in canarios if any(c in q for q, _, _ in fallos)}
    if canarios and len(cazados) < len(canarios):
        print("     ARNES INVALIDO: el sello no caza %d de %d canarios — este"
              % (len(canarios) - len(cazados), len(canarios)))
        print("     repo NO esta medido, sea cual sea el numero de arriba")
    elif canarios:
        print("     canarios cazados: %d de %d — el sello estaba mirando"
              % (len(cazados), len(canarios)))
    vistas_malas = [q for q in vistas_malas if "canario-" not in q]
    print("     -> %d de %d aristas no pasarian"
          % (len(vistas_malas), len(hechas)))
    # Y el guardia: «ninguna falla» y «lo rompi yo» se parecen demasiado. Si el
    # arnes ha dejado el repo invalido por otra cosa, el cero no vale nada.
    otros = sorted(set(re.findall(r"error\[(OOS(?!4002)\d+)\]", salida)))
    if otros:
        print("     OJO el arnes dejo otros errores: %s — el cero no es un cero"
              % ", ".join(otros))

print()
print("   -> `acme-retail` esta medido y el canario lo respalda: 2 de 4.")
print("      `casos/jerarquia` NO esta medido, y el motivo es mejor que el")
print("      numero: su tabla declara `{ mode: append, witness: field }`, y de")
print("      eso NO SE PUEDE MANTENER COPIA NINGUNA (`OOS2023`). Ahi el sello")
print("      del indice no llega a ser la pregunta — hay una pared antes.")
print()
print("      Y eso descubre algo que el sello por si solo no contesta: el")
print("      indice de topologia tiene marca de agua PROPIA y vive fuera del")
print("      circuito delta, asi que puede que `OOS2023` no sea suyo. Se")
print("      simula con `spec.materialized`, y eso le presta reglas de")
print("      mantenimiento que la copia de aristas quiza no tenga. Queda")
print("      dicho: es el limite de este arnes, no un resultado.")

# -- E - QUE ES ACME ---------------------------------------------------------
print()
print("E - QUE ES ACME: gramatica de verdad sobre origenes inalcanzables")
tipos = collections.Counter()
for r in TODOS:
    cfg = (r / "ontology.config.yaml").read_text(encoding="utf-8", errors="replace")
    for t in re.findall(r"^    type:\s*(\S+)", cfg, re.M):
        tipos[t] += 1
print("   tipos de datasource declarados en todo el arbol:")
for t, n in tipos.most_common():
    print("     %-14s %2d" % (t, n))
drivers = sorted(p.name.replace("ore-read-", "")
                 for p in (RAIZ / "crates").iterdir()
                 if p.name.startswith("ore-read-"))
print("   drivers que el motor tiene          : %s" % ", ".join(drivers))
sin = [t for t in tipos if t not in drivers]
print("   tipos SIN driver                    : %s" % ", ".join(sorted(sin)))
print()
print("   Y las tablas de acme son gramatica legitima, no relleno: nombres de")
print("   columna enteros y opacos, `physicalType` donde importa, y las dos")
print("   caras —`reads` con `fullScan: forbidden` y `requiredFilters`, que es")
print("   lo que una API con cuota de verdad impone—.")
perf = collections.Counter()
for r in TODOS:
    for k, _, d, _ in docs(r):
        if k == "Table":
            perf["con `profile`" if "profile:" in d else "sin `profile`"] += 1
for k, v in sorted(perf.items()):
    print("     tablas %-16s %3d" % (k, v))
print("   perfiles de conector en el arbol    : %d"
      % len(list(OOS.rglob("connectors/*"))))
print()
print("   -> son PREGUNTAS Y RESPUESTAS SOBRE HECHOS, con la forma correcta y")
print("      sin nadie al otro lado: `oos.dev/connectors/workday@^3.2` es una")
print("      dependencia a un registro que no existe. Lo que se ejercita es la")
print("      GRAMATICA y el COMPILADOR —etiquetas, conductos, planes, sellos—,")
print("      que es todo lo que se decide sin abrir una conexion. Lo que NO se")
print("      ejercita es leer una fila.")

# -- F - QUE SI SE PUEDE -----------------------------------------------------
print()
print("F - QUE SI SE PODRIA HACER DE VERDAD, hoy")
print("   `ore-read-postgres` y `ore-read-jsonl` existen y sirven planes. Un")
print("   repo contra un postgres real —o contra ficheros— recorre el ciclo")
print("   entero: `discover` espeja columnas y caras, `view` planifica,")
print("   `materialize` copia, y el sello del indice tendria por fin una copia")
print("   que MIRAR de verdad.")
print()
print("   BigQuery no tiene driver. Anadirlo es un crate `ore-read-bigquery`")
print("   que traduce el mismo fragmento de plan a su dialecto — el protocolo")
print("   ya esta separado justo para eso (ADR 0008), y `ore-driver` existe")
print("   porque al escribir el SEGUNDO driver quedo claro que el contrato no")
print("   podia vivir dentro del primero.")
