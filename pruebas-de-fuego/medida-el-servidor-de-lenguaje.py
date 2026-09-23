"""0037 ⓪ · EL SERVIDOR DE LENGUAJE, MEDIDO ANTES DE DECIDIR NADA.

La pregunta viene de poner el mismo `.java` en VS Code y en nuestro editor: el
nuestro colorea menos, y no autocompleta, ni dice que algo no compila mientras
escribes. Antes de prometer un «VS Code integrado» se mide qué hay, qué falta y
qué costaría, para los CINCO lenguajes que este producto sirve — java, ts,
python, sql y json.

  §1  LO QUE YA ESTÁ EN EL NAVEGADOR   qué servicios de lenguaje trae Monaco
                                       instalado (corren en un worker, sin
                                       servidor ninguno) y si la consola los
                                       tiene cableados.
  §2  EL CANAL DE HOY                  cómo habla la consola con el puesto, y
                                       CUÁNTO TARDA una ida y vuelta de verdad
                                       (de la última prueba de fuego, con
                                       agentes reales). Un LSP manda cientos
                                       de mensajes por minuto mientras se
                                       teclea: o el canal aguanta, o no hay
                                       integración que valga.
  §3  LAS IMÁGENES                     con qué nace cada puesto y si hay algún
                                       servidor de lenguaje dentro.
  §4  EL SITIO DONDE CORRERÍA          lo que el pod del puesto pide hoy
                                       (CPU/RAM) frente a lo que cada servidor
                                       necesita.
  §5  SQL, EL CASO RARO                lo que YA sabemos del esquema (el índice
                                       del árbol): cuántos datasets y cuántas
                                       columnas podría ofrecer un autocompletado
                                       nuestro, sin LSP de nadie.

    python pruebas-de-fuego/medida-el-servidor-de-lenguaje.py [--arbol <dir>]
"""
import json
import os
import re
import statistics
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
CONSOLA = "C:/rubix-platform"

# Lo que cada servidor pide de memoria, de su propia documentación. No se mide
# aquí (haría falta levantarlos): se anota de dónde sale y se contrasta con §4.
SERVIDORES = [
    ("java", "eclipse.jdt.ls", "~700 MB-1 GB RSS con un proyecto mediano; arranque 5-20 s (indexa)"),
    ("typescript", "typescript-language-server / tsserver", "~200-400 MB; arranque 1-3 s"),
    ("python", "pyright (nodo) o python-lsp-server", "~150-400 MB; arranque 1-2 s"),
    ("sql", "sqls / sql-language-server", "~50 MB; NECESITA el esquema, y el esquema es nuestro (§5)"),
    ("json", "vscode-json-languageservice", "en el navegador, sin proceso (§1)"),
]


def titulo(t):
    print("\n" + t)
    print("  " + "─" * (len(t) - 2))


def seccion_navegador():
    titulo("  §1 LO QUE YA ESTÁ EN EL NAVEGADOR (Monaco, sin servidor)")
    d = os.path.join(CONSOLA, "node_modules", "monaco-editor", "esm", "vs", "language")
    hay = sorted(x for x in os.listdir(d)) if os.path.isdir(d) else []
    print("     servicios de lenguaje instalados: %s" % (", ".join(hay) or "ninguno"))
    print("     (`common` es la base; los demás son servicios COMPLETOS en un worker:")
    print("      diagnósticos, autocompletado, hover y formato, sin proceso aparte)")
    # ¿Los tiene cableados la consola? Monaco los arranca solo si hay worker.
    cableado = []
    for raiz, _, fs in os.walk(os.path.join(CONSOLA, "components")):
        for f in fs:
            if not f.endswith((".ts", ".tsx")):
                continue
            t = open(os.path.join(raiz, f), encoding="utf-8", errors="replace").read()
            for clave in ("MonacoEnvironment", "getWorker", "typescriptDefaults", "jsonDefaults"):
                if clave in t:
                    cableado.append("%s → %s" % (os.path.relpath(os.path.join(raiz, f), CONSOLA).replace("\\", "/"), clave))
    print("     cableado en la consola: %s" % (", ".join(cableado) if cableado else "NADA"))
    print("     ⇒ ts/js y json tienen servicio completo YA COMPRADO y sin encender;")
    print("       java, python y sql no lo tienen ni pueden tenerlo en el navegador")


def seccion_canal():
    titulo("  §2 EL CANAL DE HOY (consola → ore-serve → agente del puesto)")
    agente = open(os.path.join(RAIZ, "puesto", "python", "agente.py"), encoding="utf-8", errors="replace").read()
    espera = re.search(r"plazo=(\d+)", agente)
    unaCadaVez = "pendiente" in agente and "celdas" in agente
    print("     transporte: la consola manda UNA CELDA (`POST /puestos/{id}/celdas`); el")
    print("     agente la recoge con una espera larga (`GET /puestos/{id}/pendiente`,")
    print("     plazo=%s s) y devuelve el resultado por otra llamada." % (espera.group(1) if espera else "?"))
    print("     una cada vez: %s · quien pregunta es el AGENTE, no el servidor" % ("sí" if unaCadaVez else "no"))
    print("     sin canal bidireccional: %s" % ("sí (ni websocket ni SSE en el agente)"
                                                if "websocket" not in agente.lower() else "no"))
    print("")
    print("     ⛔ No se mide aquí la ida y vuelta: haría falta un puesto vivo, y lo que el")
    print("       agente anota en su registro (`celda N · tipo · NN ms`) es lo que TARDA EN")
    print("       EJECUTAR, no lo que tarda en llegar. Decir un número de red que no se ha")
    print("       medido sería inventarlo.")
    print("     ⇒ Lo que sí se puede afirmar por la forma: un LSP manda decenas de mensajes")
    print("       por SEGUNDO mientras se teclea (didChange, completion, hover, signature) y")
    print("       espera respuesta en decenas de ms. Un canal de UNA celda cada vez, con el")
    print("       agente preguntando cada %s s, NO sirve tal cual: hace falta un conducto" % (espera.group(1) if espera else "?"))
    print("       bidireccional y permanente (websocket) de la consola al puesto.")


def seccion_imagenes():
    titulo("  §3 LAS IMÁGENES DEL PUESTO (con qué nace cada entorno)")
    texto = open(os.path.join(RAIZ, "Dockerfile"), encoding="utf-8", errors="replace").read()
    etapas = re.findall(r"FROM ([^\s]+) AS (puesto-[a-z-]+)", texto)
    bloques = re.split(r"\nFROM ", texto)
    for base, etapa in etapas:
        b = next((x for x in bloques if (" AS " + etapa) in x.split("\n")[0]), "")
        paquetes = re.findall(r"(?:pip install[^\n]*|apt-get install[^\n]*|npm (?:install|i)[^\n]*)", b)
        lsp = [p for p in paquetes if re.search(r"pyright|pylsp|python-lsp|jdtls|jdt|typescript-language|sqls|language-server", p)]
        print("     %-14s base %-28s servidor de lenguaje dentro: %s"
              % (etapa, base, ", ".join(lsp) if lsp else "NINGUNO"))


def seccion_sitio():
    titulo("  §4 DÓNDE CORRERÍA (lo que el pod del puesto pide hoy)")
    y = open(os.path.join(RAIZ, "malla", "51-el-puesto.yaml"), encoding="utf-8", errors="replace").read()
    nombre = None
    for linea in y.splitlines():
        m = re.search(r"- name: ([a-z0-9-]+)\s*$", linea)
        if m:
            nombre = m.group(1)
        r = re.search(r"requests: \{cpu: \"([^\"]+)\", memory: ([^\}]+)\}", linea)
        if r and nombre:
            print("     %-22s requests cpu %-6s mem %s" % (nombre, r.group(1), r.group(2).strip()))
        l = re.search(r"limits:\s+\{cpu: \"([^\"]+)\", memory: ([^\}]+)\}", linea)
        if l and nombre:
            print("     %-22s limits   cpu %-6s mem %s" % ("", l.group(1), l.group(2).strip()))
    print("\n     lo que pide cada servidor (de su documentación, no medido aquí):")
    for leng, cual, coste in SERVIDORES:
        print("       %-11s %-38s %s" % (leng, cual, coste))


def seccion_sql(arbol):
    titulo("  §5 SQL: LO QUE YA SABEMOS DEL ESQUEMA (y nadie más sabe)")
    if not arbol or not os.path.isdir(arbol):
        print("     (sin árbol: pásame --arbol <dir> para contar sobre uno de verdad)")
        return
    datasets = columnas = tablas = vistas = 0
    nombres = []
    for raiz, _, fs in os.walk(os.path.join(arbol, "packages")):
        for f in fs:
            if not f.endswith(".yaml"):
                continue
            t = open(os.path.join(raiz, f), encoding="utf-8", errors="replace").read()
            kind = next((l.split(":", 1)[1].strip() for l in t.splitlines() if l.startswith("kind:")), "")
            if kind not in ("Dataset", "Table", "View"):
                continue
            datasets += kind == "Dataset"
            tablas += kind == "Table"
            vistas += kind == "View"
            n = re.search(r"name:\s*([A-Za-z0-9_]+)", t)
            if n:
                nombres.append(n.group(1))
            columnas += len(re.findall(r"^\s{4}\w+:", t, re.M))
    print("     en este árbol: %d Dataset · %d Table · %d View — %d referencias que un"
          % (datasets, tablas, vistas, datasets + tablas + vistas))
    print("     `select … from ` podría ofrecer, con ~%d columnas detrás" % columnas)
    print("     ⇒ ningún LSP de SQL del mundo sabe esto: el esquema no está en una base,")
    print("       está en NUESTRO índice. Aquí el «servidor de lenguaje» somos nosotros.")


def main(argv):
    arbol = argv[argv.index("--arbol") + 1] if "--arbol" in argv else None
    print("═══ 0037 ⓪ · el servidor de lenguaje, medido")
    seccion_navegador()
    seccion_canal()
    seccion_imagenes()
    seccion_sitio()
    seccion_sql(arbol)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
