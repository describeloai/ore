# -*- coding: utf-8 -*-
"""Las etiquetas no tienen dueno. ¿Importa, y donde exactamente?

`ConduitPolicy` DEBE declarar `owner`, y el motor dice por que al negarse a
compilar sin el:

    «elevar la autorizacion de un conducto es LA decision de seguridad de este
     modelo, y UN TECHO DEL QUE NADIE RESPONDE ES EL HUECO QUE ESTE CAMPO
     CIERRA.»

Quien escribe `nationalId: critical` decide de la misma familia un piso mas
abajo, y `Entity` no tiene `owner` ni lo admite. La pregunta es si eso es un
hueco de verdad o ya esta tapado por otro sitio. Seis frentes:

  A. DONDE SE ESCRIBEN  las tres superficies que fijan una clasificacion
  B. LO QUE YA CUBRE    `OOS8001`: pasarse de etiqueta SI arrastra un dueno
  C. EL HUECO           quedarse corto no. Y se demuestra en una linea
  D. LA ASIMETRIA       el techo tiene dueno; el suelo, no
  E. QUIEN DECIDE QUE   el reticulo fija a que nivel empieza a exigirse
     SE EXIGE           gobierno, y tampoco responde nadie
  F. VEREDICTO
"""
import collections
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
EJEMPLO = OOS / "examples/acme-retail"
ORE = RAIZ / "target/debug/ore"


def correr(*args):
    s = subprocess.run([str(ORE), *map(str, args)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    return s.stdout + s.stderr


def docs(raiz):
    for f in sorted(raiz.rglob("*.yaml")):
        txt = f.read_text(encoding="utf-8", errors="replace")
        for d in re.split(r"^---\s*$", txt, flags=re.M):
            k = re.search(r"^kind:\s*(\w+)", d, re.M)
            if k:
                yield k.group(1), d, f


TODOS = list(docs(OOS)) + list(docs(RAIZ / "casos"))
print("== las etiquetas sin dueno ==")

# -- A - DONDE SE ESCRIBEN ---------------------------------------------------
print()
print("A - LAS TRES SUPERFICIES QUE FIJAN UNA CLASIFICACION")
donde = collections.Counter()
for k, d, _ in TODOS:
    if re.search(r"^\s+labels:", d, re.M) or re.search(r"[{,]\s*labels:", d):
        donde[k] += 1
for k, v in donde.most_common():
    print("   %-18s %3d documentos con `labels`" % (k, v))
print()
print("   Y son tres decisiones distintas, no una:")
print("     1. la ETIQUETA de una propiedad      `Entity`")
print("     2. el SUELO de un datasource         `OntologyConfig`")
print("        —todo lo que sale de ahi es al menos eso—")
print("     3. a que nivel EMPIEZA a exigirse    `Lattice.requiresGovernance`")
print("        gobierno")

# -- B - LO QUE YA CUBRE -----------------------------------------------------
print()
print("B - LO QUE YA ESTA CUBIERTO: pasarse de etiqueta arrastra un dueno")
rep = correr("report", EJEMPLO)
m = re.search(r"(\d+) propiedad\(es\) exigen gobierno", rep)
duenos = sorted(set(re.findall(r"\((team:[\w-]+)\)", rep)))
print("   `ore report` sobre el ejemplo: %s propiedades exigen gobierno"
      % (m.group(1) if m else "?"))
print("   y responden: %s" % ", ".join(duenos))
print()
print("   `OOS8001` no compila si una propiedad exige gobierno y ninguna regla")
print("   la cubre. Asi que una etiqueta ALTA no puede quedarse sin dueno: la")
print("   exigencia arrastra una regla, y la regla DEBE declarar `owner`.")
print("   -> en esta direccion no hay hueco. Es un mecanismo, y funciona.")

# -- C - EL HUECO ------------------------------------------------------------
print()
print("C - EL HUECO: quedarse corto. Y cabe en UNA LINEA")
print("   Se baja el suelo de `hr_workday` de `high` a `low` — una palabra en")
print("   `ontology.config.yaml`— y se vuelve a preguntar.")


def gobernadas(arbol):
    r = correr("report", arbol)
    mm = re.search(r"(\d+) propiedad\(es\) exigen gobierno", r)
    return int(mm.group(1)) if mm else None


tmp = pathlib.Path(tempfile.mkdtemp()) / "r"
shutil.copytree(EJEMPLO, tmp)
antes = gobernadas(EJEMPLO)
cfg = tmp / "ontology.config.yaml"
cfg.write_text(cfg.read_text(encoding="utf-8").replace(
    "gdpr.sensitivity: high      # suelo", "gdpr.sensitivity: low       # suelo", 1),
    encoding="utf-8")
despues = gobernadas(tmp)
valida = "error[" not in correr("validate", tmp)
print()
print("   propiedades que exigen gobierno : %s  ->  %s" % (antes, despues))
print("   `ore validate`                  : %s"
      % ("ok · SIN ERRORES" if valida else "falla"))
print()
print("   %d propiedades dejan de exigir gobierno, y el repositorio compila."
      % (antes - despues))
print("   Nadie ha tenido que aprobar nada: `OntologyConfig` no admite `owner`.")

# Y el sello que se acaba de encender, con la misma linea.
pol = tmp / "conduits.yaml"
pol.write_text(re.sub(r"^      gdpr\.sensitivity: high$",
                      "      gdpr.sensitivity: low", pol.read_text(encoding="utf-8"),
                      flags=re.M), encoding="utf-8")
sigue = "error[" not in correr("validate", tmp)
shutil.rmtree(tmp.parent, ignore_errors=True)
print()
print("   Y con la MISMA linea, el sello del indice se puede devolver a `low`")
print("   y sigue compilando: %s" % ("SI" if sigue else "no"))
print("   -> o sea que bajar el suelo no solo desgobierna propiedades: afloja")
print("      el conducto que se acaba de sellar, y por el mismo sitio.")

# -- D - LA ASIMETRIA --------------------------------------------------------
print()
print("D - LA ASIMETRIA: el techo tiene dueno, el suelo no")
doc_rs = (RAIZ / "crates/ore-core/src/document.rs").read_text(encoding="utf-8")
admite = [k for k in ("Package", "View", "Entity", "Table", "ConduitPolicy",
                      "Ruleset", "Function", "RequestPolicy", "Lattice",
                      "OntologyConfig")
          if re.search(r"Kind::%s => &\[[^\]]*\"owner\"" % k, doc_rs, re.S)]
print("   admiten `owner` : %s" % ", ".join(admite))
print("   NO lo admiten   : %s" % ", ".join(
    k for k in ("Entity", "Table", "Lattice", "OntologyConfig") if k not in admite))
print()
print("   %-34s %s" % ("el TECHO — `ConduitPolicy`", "DEBE declarar `owner`"))
print("   %-34s %s" % ("el SUELO — `datasources[].labels`", "no tiene ni campo"))
print()
print("   Y el techo es el mas visible de los dos: vive en un fichero de")
print("   seguridad con CODEOWNERS, y elevarlo se revisa. Bajar el suelo es una")
print("   palabra en la configuracion, y desclasifica en cascada TODO lo que")
print("   hereda de esa fuente. El campo esta puesto en el sitio menos")
print("   peligroso de los dos.")

# -- E - QUIEN DECIDE QUE SE EXIGE -------------------------------------------
print()
print("E - Y UN TERCERO: quien decide a que nivel empieza a exigirse gobierno")
ret = [f for _, d, f in TODOS if "requiresGovernance" in d]
print("   reticulos con `requiresGovernance` : %d" % len(ret))
print("   `Lattice` admite `owner`           : %s"
      % ("si" if "Lattice" in admite else "NO"))
print("   -> el reticulo decide que `high` exija `authorization`. Subir ese")
print("      piso a `critical` desactiva `OOS8001` para todo un nivel, y")
print("      tampoco responde nadie. Es el mismo hueco una capa mas arriba.")

# -- F - VEREDICTO -----------------------------------------------------------
print()
print("F - VEREDICTO")
print("   El hueco existe, y NO es el que dije. No es «las etiquetas no tienen")
print("   dueno»: pasarse de etiqueta ya arrastra uno, y eso funciona.")
print()
print("   Es que LO QUE FIJA EL MINIMO no responde ante nadie. Tres campos,")
print("   ninguno con `owner`, y los tres bajan gobierno en cascada:")
print()
print("     `datasources[].labels`        el suelo de una fuente")
print("     `Lattice.requiresGovernance`  desde que nivel se exige")
print("     una propiedad sin etiqueta    hereda el suelo, y nada mas")
print()
print("   La forma es exactamente la de `OOS4011` y la del sello del indice: la")
print("   omision y lo bajo no dan sintoma. Y la casa ya tiene la respuesta")
print("   escrita para el techo —«un techo del que nadie responde es el hueco")
print("   que este campo cierra»—; falta aplicarla al suelo.")
print()
print("   Lo mas barato que lo cerraria: `owner` en `OntologyConfig`, que es")
print("   donde vive el suelo, y en `Lattice`. No hace falta que `Entity` lo")
print("   tenga — su etiqueta alta ya arrastra una regla con dueno, y su")
print("   etiqueta baja esta acotada por el suelo (`OOS4012`), que es justo el")
print("   campo que se quedaria sin cubrir.")
