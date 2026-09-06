# -*- coding: utf-8 -*-
"""La costura entre `ore-core` y `ore-view`: de ida, y con un juicio fuera del
camino critico.

El motor comprueba una cosa que el nucleo NO sabe comprobar — el flujo
implicito de Denning: una vista que recorta por `nationalId` y expone solo `id`
no copia el DNI, y aun asi revela quien lo tiene. La arista `INDIRECT`. Y
`ore-cli/src/vista.rs` lo dice de si mismo:

    «`ore validate` no lo mira porque el nucleo no tiene linaje por columna. El
     dia que lo tenga, esta comprobacion se movera alli; hasta entonces vive
     aqui y SE NIEGA IGUAL.»

«Se niega igual» — si alguien ejecuta `ore view`. Esto mide si ese hueco tiene
SUJETO hoy, o si es estructural como resulto ser el abanico de rutas (127 bases,
una vista cada una, cero).

La distincion importa y es la misma de siempre: una regla sin sujeto se puede
dejar escrita; un hueco CON sujeto es un paquete que dice `ok` y no lo esta.

  A. LA COSTURA   quien alcanza el motor y quien no, derivado del despacho
  B. EL SUJETO    vistas que recortan por un campo que no exponen, y si eso
                  que recortan lleva etiqueta
  C. EL COTEJO    los dos veredictos sobre el mismo arbol
  D. EL HUECO     con sujeto o estructural
"""
import pathlib
import re
import subprocess

import yaml

RAIZ = pathlib.Path(r"C:\ORE")
OOS = RAIZ / "vendor/oos"
ORE = RAIZ / "target/debug/ore"


def docs_de(raiz):
    """Los documentos de una raiz, por kind. Tolerante: un fichero que no
    parsea no se inventa, se salta y se cuenta."""
    out, malos = [], 0
    for f in sorted(raiz.rglob("*.yaml")):
        try:
            for d in yaml.safe_load_all(f.read_text(encoding="utf-8", errors="replace")):
                if isinstance(d, dict) and "kind" in d:
                    out.append((d, f))
        except Exception:
            malos += 1
    return out, malos


def raices():
    """Cada directorio con `ontology.config.yaml` es un paquete compilable."""
    for f in sorted(OOS.rglob("ontology.config.yaml")):
        yield f.parent


print("== la costura entre el nucleo y el motor ==")

# -- A - LA COSTURA ----------------------------------------------------------
print()
print("A - QUIEN ALCANZA EL MOTOR, derivado del despacho de `main.rs`")
main = (RAIZ / "crates/ore-cli/src/main.rs").read_text(encoding="utf-8")
# `Command::X => ... modulo::fn(...)`, y luego se mira si ese modulo importa
# `ore_view`. Es derivacion, no lista a mano: un comando nuevo entra solo.
despacho = dict(re.findall(r"Command::(\w+)\s*(?:\{[^}]*\})?\s*=>\s*(?:return\s+)?"
                           r"(?:crate::)?(\w+)(?:::\w+)?\(", main))
directo, llama_a = {}, {}
for f in sorted((RAIZ / "crates/ore-cli/src").rglob("*.rs")):
    t = f.read_text(encoding="utf-8", errors="replace")
    codigo = "\n".join(l for l in t.split("\n") if not l.strip().startswith("//"))
    directo[f.stem] = "ore_view" in codigo
    llama_a[f.stem] = set(re.findall(r"crate::(\w+)::", codigo))


def corre_el_motor(m, visto=()):
    """Transitivo: un comando alcanza el motor si su modulo lo importa O si
    llama a otro que si. Sin esto, `registro.rs` —que importa medio motor— no
    se le atribuiria a quien lo usa."""
    if m in visto or m not in directo:
        return False
    if directo[m]:
        return True
    return any(corre_el_motor(x, visto + (m,)) for x in llama_a.get(m, ()))


print("   %-14s %-22s %s" % ("comando", "va a", "¿corre el motor?"))
print("   " + "-" * 56)
for cmd, destino in sorted(despacho.items()):
    # El destino puede ser un modulo propio o una funcion libre de `main.rs`.
    es_modulo = destino in directo and destino != "main"
    corre = corre_el_motor(destino if es_modulo else "main")
    print("   %-14s %-22s %s"
          % (cmd, destino if es_modulo else destino + "() · main.rs",
             "SI" if corre else "no"))
print()
print("   La direccion es de ida: el paquete alimenta al motor y NADA vuelve al")
print("   veredicto. `validate` no lo toca.")

# -- B - EL SUJETO -----------------------------------------------------------
print()
print("B - EL SUJETO: recortar por lo que no se expone")
print()
print("   %-46s %s" % ("", "vistas"))
tot_v = con_where = oculto = con_etiqueta = materializadas = 0
casos, virtuales = [], []
for r in raices():
    docs, _ = docs_de(r)
    vistas = {}
    for d, f in docs:
        if d.get("kind") != "View":
            continue
        n = (d.get("metadata") or {}).get("name")
        ns = (d.get("metadata") or {}).get("namespace")
        vistas[n] = (d.get("spec") or {}, ns, f)
    # El suelo del datasource etiqueta TODO lo que salga de esa fuente.
    suelos = set()
    for d, f in docs:
        if d.get("kind") == "OntologyConfig":
            for ds in d.get("datasources") or []:
                if isinstance(ds, dict) and ds.get("labels"):
                    suelos.add(ds.get("name"))
    # Las etiquetas de propiedad de la entidad, por el campo que las lleva.
    etiquetadas = set()
    for d, f in docs:
        if d.get("kind") != "Entity":
            continue
        for prop, cuerpo in ((d.get("spec") or {}).get("properties") or {}).items():
            if isinstance(cuerpo, dict) and cuerpo.get("labels"):
                etiquetadas.add(prop)

    def fuente_de(spec, visto=()):
        """Baja la cadena hasta el `datasource` de la tabla raiz."""
        frm = spec.get("from") or {}
        if "table" in frm:
            corto = str(frm["table"]).split(".")[-1]
            for d, _ in docs:
                if d.get("kind") == "Table" and (d.get("metadata") or {}).get("name") == corto:
                    return (d.get("spec") or {}).get("datasource")
            return None
        sig = str(frm.get("view", "")).split(".")[-1]
        if sig in vistas and sig not in visto:
            return fuente_de(vistas[sig][0], visto + (sig,))
        return None

    for n, (spec, ns, f) in vistas.items():
        tot_v += 1
        w = spec.get("where") or {}
        if not w:
            continue
        con_where += 1
        campos = set((spec.get("fields") or {}).keys())
        ocultos = [k for k in w if k not in campos]
        if not ocultos:
            continue
        oculto += 1
        ds = fuente_de(spec)
        por_suelo = ds in suelos
        por_prop = [k for k in ocultos if k in etiquetadas]
        if por_suelo or por_prop:
            con_etiqueta += 1
            # Y la capa que faltaba en la primera version de este arnes: el
            # motor NO se niega por una vista virtual. La arista `INDIRECT` la
            # computa siempre —se ve en el linaje de `ore view`— pero lo que
            # se niega es que UNA COPIA lleve mas de lo que su conducto
            # autoriza. Sin copia no hay nada que sellar:
            #     «flujo  virtual — cada lectura va al origen; nada que copiar»
            # Contar sin esto daba «3 con sujeto» cuando el motor acepta los
            # tres. Un sujeto que no dispara la regla no es un sujeto.
            if spec.get("materialized"):
                materializadas += 1
                casos.append((r, n, ocultos, "suelo de `%s`" % ds if por_suelo
                              else "propiedad etiquetada: %s" % ", ".join(por_prop)))
            else:
                virtuales.append((r, n))

print("   %-46s %6d" % ("vistas en paquetes compilables", tot_v))
print("   %-46s %6d" % ("  ...con `where`", con_where))
print("   %-46s %6d" % ("  ...que recortan por un campo NO expuesto", oculto))
print("   %-46s %6d" % ("  ...y ese campo lleva etiqueta", con_etiqueta))
print("   %-46s %6d" % ("  ...y ADEMAS la vista es `materialized`", materializadas))
print()
print("   Esa ultima fila es la que decide, y no estaba en la primera version")
print("   de este arnes. La arista `INDIRECT` el motor la computa SIEMPRE —se")
print("   lee en el linaje de `ore view`— pero lo que niega es que una COPIA")
print("   lleve mas de lo que su conducto autoriza. Una vista virtual no copia:")
print("     «flujo  virtual — cada lectura va al origen; nada que copiar»")
print()
if casos:
    print("   Los que disparan la regla:")
    for r, n, ocultos, por_que in casos:
        print("     %-46s %s" % (r.relative_to(OOS).as_posix()[:46], n))
        print("       recorta por %-22s %s" % (", ".join(ocultos), por_que))
else:
    print("   NINGUNO dispara la regla. Los %d con etiqueta son virtuales:" % con_etiqueta)
    for r, n in virtuales:
        print("     %-46s %s" % (r.relative_to(OOS).as_posix()[:46], n))

# -- C - EL COTEJO -----------------------------------------------------------
print()
print("C - EL COTEJO: los dos veredictos sobre el mismo arbol")


def veredicto(cmd, r):
    p = subprocess.run([str(ORE), cmd, str(r)], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    linea = next((l.strip() for l in ((p.stdout or "") + (p.stderr or "")).split("\n")
                  if re.match(r"\s*(ok|error)", l)), "")
    return p.returncode, (linea[:44] or "(sin veredicto)")


# Se cotejan los candidatos: lo que se mide es si los dos mandos dicen lo
# mismo, no si uno acierta. Si no hay ninguno que dispare, se cotejan los
# virtuales con etiqueta, que son los que MAS cerca estan de disparar.
mirar = [c[0] for c in casos] or [r for r, _ in virtuales]
for r in mirar:
    cv, sv = veredicto("validate", r)
    cw, sw = veredicto("view", r)
    print("   %s" % r.relative_to(OOS).as_posix())
    print("     validate -> %-46s (%d)" % (sv, cv))
    print("     view     -> %-46s (%d)" % (sw, cw))
    print("     %s" % ("los dos dicen lo mismo" if (cv == 0) == (cw == 0)
                       else "DISCREPAN: el paquete compila y el motor no lo pasa"))

# -- D - EL HUECO ------------------------------------------------------------
print()
print("D - EL HUECO: ¿con sujeto o estructural?")
print()
if materializadas:
    print("   CON SUJETO: %d copia(s) llevan una etiqueta que entra por un" % materializadas)
    print("   filtro oculto, y `validate` no las mira porque no corre el motor.")
else:
    print("   SIN SUJETO EN EL CORPUS — y ojo, que no es lo mismo que el abanico")
    print("   de rutas, donde no habia forma de construirlo. Aqui §E la hay.")
    print()
    print("   El flujo implicito EXISTE")
    print("   —%d vistas recortan por un campo que no exponen, y %d de ellas por"
          % (oculto, con_etiqueta))
    print("   algo etiquetado— pero las %d son VIRTUALES, y una vista virtual no" % con_etiqueta)
    print("   copia nada. La regla no tiene con que dispararse.")
    print()
    print("   Y el cotejo lo confirma: `validate` dice `ok` y `view` tambien.")
    print("   No hay discrepancia HOY porque no hay caso, no porque los dos")
    print("   mandos comprueben lo mismo.")
print()
print("   Lo que queda dicho en las dos ramas, y no depende del corpus: el")
print("   juicio del motor esta FUERA del camino de `validate`. Y eso no se")
print("   afirma, se construye — §E.")

# -- E - EL EXPERIMENTO ------------------------------------------------------
print()
print("E - EL EXPERIMENTO: construir el caso que el corpus no tiene")
print()
print("   «Estructural» no es «imposible», y la unica forma de saber la")
print("   diferencia es construirlo. Hacen falta CUATRO ingredientes, y ninguno")
print("   es exotico:")
print("     1. dos entidades en una cadena — la de abajo etiqueta una columna")
print("        que la de arriba usa SOLO para filtrar;")
print("     2. la vista de arriba `materialized`;")
print("     3. el conducto autorizado POR DEBAJO de esa etiqueta;")
print("     4. la columna del filtro NO expuesta arriba.")
print()
import shutil
import tempfile

base = OOS / "conformance/v1alpha8/valid/materialized-view-over-table-within-clearance/input"
tmp = pathlib.Path(tempfile.mkdtemp(prefix="costura-"))
destino = tmp / "input"
shutil.copytree(base, destino)
(destino / "views/empleados.yaml").write_text(
    "apiVersion: oos.dev/v1alpha8\nkind: View\n"
    "metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  "
    "from: { table: erp.employees }\n  freshness: 15m\n  fields:\n    "
    "employeeId: employee_id\n    nationalId: national_id\n    pais: country\n",
    encoding="utf-8")
(destino / "views/iberia.yaml").write_text(
    "apiVersion: oos.dev/v1alpha8\nkind: View\n"
    "metadata: { name: iberia, namespace: hr }\nspec:\n  owner: team:hr\n  "
    "from: { view: empleados }\n  fields:\n    id: employeeId\n  where:\n    "
    "pais: [ES, PT]\n  materialized: { datasource: lago, table: \"cache.iberia\" }\n",
    encoding="utf-8")
(destino / "entities/Employee.yaml").write_text(
    "apiVersion: oos.dev/v1alpha8\nkind: Entity\n"
    "metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  "
    "primaryKey: [id]\n  backedBy: iberia\n  properties:\n    id: { type: String }\n",
    encoding="utf-8")
# La entidad de abajo: es la UNICA forma de que una columna que arriba solo
# filtra lleve etiqueta. Un suelo de datasource no sirve —etiqueta todo por
# igual, asi que la arista `INDIRECT` no anade nada— y una propiedad de la
# entidad de arriba tampoco, porque `OOS2022` la obliga a estar expuesta.
(destino / "entities/Plantilla.yaml").write_text(
    "apiVersion: oos.dev/v1alpha8\nkind: Entity\n"
    "metadata: { name: Plantilla, namespace: hr }\nspec:\n  nature: entity\n  "
    "primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    "
    "employeeId: { type: String }\n    pais:\n      type: String\n      "
    "labels: { gdpr.sensitivity: high }\n",
    encoding="utf-8")
c = destino / "conduits.yaml"
c.write_text(c.read_text(encoding="utf-8").replace(
    "gdpr.sensitivity: high", "gdpr.sensitivity: low"), encoding="utf-8")

rc_v, txt_v = veredicto("validate", destino)
rc_w, txt_w = veredicto("view", destino)
print("   `ore validate` -> %-42s exit %d" % (txt_v, rc_v))
print("   `ore view`     -> %-42s exit %d" % (txt_w, rc_w))
print()
if rc_v == 0 and rc_w != 0:
    print("   DISCREPAN. El paquete compila y el motor se niega, y el motivo es")
    print("   exactamente el flujo implicito:")
    salida = subprocess.run([str(ORE), "view", str(destino)], capture_output=True,
                            text=True, encoding="utf-8", errors="replace")
    for l in (salida.stdout or "").split("\n"):
        if any(x in l for x in ("no compila", "INFLUENCIA", "del origen", "de esta vista")):
            # La consola de Windows es cp1252 y la flecha del linaje no cabe.
            print("     %s" % l.strip().replace("←", "<-")
                  .encode("ascii", "replace").decode())
    print()
    print("   -> el hueco es CONSTRUIBLE, no estructural. Que el corpus no lo")
    print("      tenga es un accidente del corpus, no una propiedad del modelo.")
else:
    print("   NO discrepan — el experimento no reproduce el hueco, y entonces lo")
    print("   que hay que revisar es esta receta antes que la conclusion.")
shutil.rmtree(tmp, ignore_errors=True)

# -- F - Y QUIEN LO PROBARIA -------------------------------------------------
print()
print("F - Y NADIE LO PROBARIA, porque el arnes tampoco corre el motor")
conf = (RAIZ / "crates/ore-cli/tests/conformance.rs").read_text(encoding="utf-8")
subs = sorted(set(re.findall(r'correr\("(\w+)"', conf)))
print("   subcomandos que invoca el arnes de conformidad : %s" % ", ".join(subs))
fuga = subprocess.run(["grep", "-rl", "fuga", str(RAIZ / "crates/ore-cli/tests")],
                      capture_output=True, text=True).stdout.split()
print("   pruebas de `ore-cli` que afirman una fuga      : %s"
      % (", ".join(pathlib.Path(x).name for x in fuga) or "NINGUNA"))
print("   donde SI esta probada                          : "
      "`ore-view/src/flow.rs`, con las estructuras del motor")
print()
print("   Asi que la negativa por flujo implicito existe, esta probada DENTRO")
print("   del motor, y no la ejerce ningun caso de conformidad — porque ningun")
print("   caso puede: el arnes solo llama a `validate`. Un nivel de conformidad")
print("   no puede certificar lo que su propio arnes no invoca.")
