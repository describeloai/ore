# -*- coding: utf-8 -*-
"""El arsenal: de que fuentes se pueden sacar tablas HOY, y hasta donde.

La intuicion a cotejar es: «el formato de los datos tiene una base solida, pero
los drivers estan por construir». Se mide, porque «por construir» tiene grados
y el reparto no es el que parece — hay una familia que entra y no sale, y otra
que sale y no entra.

El circuito son TRES fases, y una familia solo esta completa si cubre las tres:

    (1) CATALOGO  que objetos hay, con que columnas y que caras   -> `discover`
    (2) INDUCIR   catalogo -> Table + View + Entity en DRAFT      -> puro, sin red
    (3) LEER      filas, para poblar una copia                    -> `materialize`

  A. EL REPARTO   quien hace cada fase, derivado del codigo
  B. LA MATRIZ    familia x verbo, derivada de cada driver
  C. EL AGUJERO   que familia cubre que, y donde se corta
  D. EL EXPERIMENTO  inducir con y sin sondeo, que es la diferencia real entre
                  Postgres y BigQuery, y que dice `validate` de cada uno
  E. EL SALTO     que hay que construir, en orden de lo que desbloquea
"""
import json
import pathlib
import re
import shutil
import subprocess
import tempfile

RAIZ = pathlib.Path(r"C:\ORE")
CRATES = RAIZ / "crates"
ORE = RAIZ / "target/debug/ore"


def hay(programa):
    return shutil.which(programa) is not None


print("== el arsenal de fuentes ==")

# -- A - EL REPARTO ----------------------------------------------------------
print()
print("A - EL REPARTO: quien hace cada fase")
lector = (CRATES / "ore-cli/src/lector.rs").read_text(encoding="utf-8")
mat = (CRATES / "ore-cli/src/materializar.rs").read_text(encoding="utf-8")
receta = re.findall(r'"(\w+)" => \w+\(fuente, &url\)', lector)
print("   (1) catalogo  `ore discover --source`")
print("       receta INTERNA para  : %s" % (", ".join(receta) or "(ninguna)"))
print("       para el resto delega : `ore-read-<tipo>` en el PATH, verbo `catalogo`")
print("   (2) inducir   `inductor.rs` — puro: sin red, sin credencial, sin driver")
print("   (3) leer      `ore materialize` -> `ore-read-<tipo>`, verbo `leer`")
tiene_receta_leer = "ore-read-{tipo}" in mat and "bigquery" not in mat
print("       y AQUI no hay receta interna de nada: %s"
      % ("confirmado" if tiene_receta_leer else "revisar"))
print()
print("   El `type` sale del esquema de la URL, asi que anadir una familia NO")
print("   toca `ore`: es poner un binario `ore-read-<tipo>` en el PATH. Eso es")
print("   lo que hace que «los drivers estan por construir» sea una frase con")
print("   sentido — el sitio donde enchufarlos ya existe y esta cerrado.")

# -- B - LA MATRIZ -----------------------------------------------------------
print()
print("B - LA MATRIZ: familia x verbo, derivada de cada driver")


def verbos_de(crate):
    """Los verbos que el `match` del driver acepta, y si alguno se niega."""
    f = CRATES / crate / "src/main.rs"
    if not f.is_file():
        return {}
    t = f.read_text(encoding="utf-8", errors="replace")
    out = {}
    # Los dos drivers no escriben el `match` igual: `jsonl` casa `"leer" =>` y
    # `postgres` casa `Some("leer") =>`. La primera version solo vio el primero
    # y dio a Postgres —el unico driver COMPLETO— tres verbos en «no».
    for v in re.findall(r'(?:Some\()?"(\w+)"\)? =>', t):
        # Un brazo que devuelve `Err("... no sabe ...")` esta declarado y no
        # implementado. Contarlo como verbo seria contar el nombre.
        brazo = re.search(r'"%s" =>\s*(.{0,120})' % v, t, re.S)
        out[v] = not (brazo and "no sabe" in brazo.group(1))
    return out


FAMILIAS = [
    ("bigquery", None, "receta interna · ejecuta `bq`"),
    ("postgres", "ore-read-postgres", "binario, FFI nativo"),
    ("jsonl", "ore-read-jsonl", "binario, cero dependencias"),
]
print("   %-10s %-14s %-9s %-9s %-9s %s"
      % ("familia", "quien", "catalogo", "leer", "testigo", "¿aqui?"))
print("   " + "-" * 74)
matriz = {}
for fam, crate, como in FAMILIAS:
    if crate:
        v = verbos_de(crate)
        binario = RAIZ / ("target/debug/%s.exe" % crate)
        listo = "construye" if binario.is_file() else "sin construir"
    else:
        # La receta de BigQuery solo cubre el catalogo: `materializar` llama a
        # `ore-read-bigquery` y no existe.
        v = {"catalogo": True}
        listo = "`bq` en PATH" if hay("bq") else "falta `bq`"
    matriz[fam] = v
    def c(x):
        return "si" if v.get(x) else ("declarado" if x in v else "no")
    print("   %-10s %-14s %-9s %-9s %-9s %s"
          % (fam, crate or "ore (receta)", c("catalogo"), c("leer"), c("testigo"), listo))
print()
print("   «declarado» = el verbo existe en el `match` y contesta que no sabe.")

# -- C - EL AGUJERO ----------------------------------------------------------
print()
print("C - EL AGUJERO: ninguna familia cubre el circuito entero")
for fam, v in matriz.items():
    tiene = [k for k in ("catalogo", "leer") if v.get(k)]
    falta = [k for k in ("catalogo", "leer") if not v.get(k)]
    print("   %-10s tiene: %-18s falta: %s"
          % (fam, ", ".join(tiene) or "-", ", ".join(falta) or "NADA"))
print()
print("   -> `bigquery` ENTRA Y NO SALE: espeja tablas y no puede poblar una")
print("      copia, porque `materialize` no tiene receta y no hay")
print("      `ore-read-bigquery`.")
print("   -> `jsonl` SALE Y NO ENTRA: lee filas y no sabe decir que hay en un")
print("      directorio. Esta dicho en su codigo, no deducido.")
print("   -> `postgres` es la unica completa, y ademas la unica que SONDEA:")
print("      `wal_level` y `relreplident` deciden `changes.mode`, y eso no es")
print("      metadato de catalogo — es un hecho de la instalacion.")

# -- D - EL EXPERIMENTO ------------------------------------------------------
print()
print("D - EL EXPERIMENTO: que cambia entre sondear y no sondear")
print()
print("   El catalogo de Postgres trae `reads` y `changes`; el de BigQuery no")
print("   —`INFORMATION_SCHEMA` no dice si se puede empujar un predicado ni si")
print("   la tabla retracta—. Se induce el MISMO catalogo con y sin ellos.")

CON = {
    "source": "crm",
    "tables": [{
        "name": "public.clientes", "kind": "table",
        "columns": [{"name": "id", "type": "Integer"},
                    {"name": "email", "type": "String"}],
        "primaryKey": ["id"],
        "reads": {"predicatePushdown": ["eq"], "fullScan": "cheap"},
        "changes": {"mode": "upsert", "key": ["id"], "witness": "log"},
    }],
}
SIN = json.loads(json.dumps(CON))
del SIN["tables"][0]["reads"]
del SIN["tables"][0]["changes"]

for etiqueta, cat in (("CON sondeo (postgres)", CON), ("SIN sondeo (bigquery)", SIN)):
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="arsenal-"))
    (tmp / "cat.json").write_text(json.dumps(cat), encoding="utf-8")
    salida = tmp / "out"
    d = subprocess.run([str(ORE), "discover", "--from", str(tmp / "cat.json"),
                        "--out", str(salida), "--name", "crm"],
                       capture_output=True, text=True, encoding="utf-8",
                       errors="replace")
    print()
    print("   %s" % etiqueta)
    tabla = next(iter(salida.rglob("*/tables/*.yaml")), None) or \
        next(iter(salida.rglob("tables/*.yaml")), None)
    if tabla:
        cuerpo = tabla.read_text(encoding="utf-8", errors="replace")
        # Desde la primera linea del bloque fisico hasta el final. Filtrar
        # linea a linea por `^reads|^changes` solo enseñaba las CABECERAS, que
        # es justo lo que no distingue a los dos catalogos.
        m = re.search(r"^  (?:#|reads|changes).*", cuerpo, re.S | re.M)
        for l in (m.group(0).rstrip().split("\n") if m else ["(sin bloque fisico)"]):
            print("     %s" % l.rstrip()[:72].encode("ascii", "replace").decode())
    else:
        print("     discover no dejo tabla (rc=%d): %s"
              % (d.returncode, (d.stderr or "").strip()[:60]))
    v = subprocess.run([str(ORE), "validate", str(salida)], capture_output=True,
                       text=True, encoding="utf-8", errors="replace")
    codigos = sorted(set(re.findall(r"error\[(OOS\d+)\]", (v.stdout or "") + (v.stderr or ""))))
    print("     `ore validate` -> %s" % (", ".join(codigos) or "ok · sin errores"))
    shutil.rmtree(tmp, ignore_errors=True)

print()
print("   Y conviene recordar que lo inducido NO COMPILA a proposito:")
print("   «una entidad sin clave primaria falla con `OOS2010`, y esta bien que")
print("   falle — inventar la clave seria lo unico peor». Los diagnosticos SON")
print("   la cola de revision. Lo que se compara arriba no es «cual funciona»,")
print("   es QUE se pierde cuando el catalogo no sondea.")

# -- E - EL SALTO ------------------------------------------------------------
print()
print("E - EL SALTO: que construir, por lo que desbloquea cada cosa")
print()
print("   1 · `ore-read-bigquery leer` — el mas barato y el que mas abre.")
print("       El catalogo YA entra por la receta; lo que falta es la fase (3).")
print("       Sin el, BigQuery da tablas y vistas que no se pueden poblar.")
print()
print("   2 · el SONDEO de BigQuery — sin `reads`, el planificador no empuja")
print("       nada y el residuo es el plan entero; sin `changes`, toda tabla")
print("       nace `mode: none`, y una vista sobre ella no se puede mantener.")
print("       Es la diferencia medida en (D), y no es cosmetica.")
print()
print("   3 · `ore-read-jsonl catalogo` — barato y cierra la segunda familia.")
print("       Exige inferir columnas y tipos de los datos, que es justo lo que")
print("       el proyecto evita en otras partes: hay que decidir si se infiere")
print("       o si se le pide un esquema al lado.")
print()
print("   4 · la tercera familia — y aqui lo que se compra no es una fuente:")
print("       es la prueba de que el protocolo aguanta. `jsonl` ya demostro que")
print("       la peticion no es SQL; una tercera demostraria que el sondeo de")
print("       caras tampoco es de Postgres.")
