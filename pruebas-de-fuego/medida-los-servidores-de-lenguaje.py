"""0037 ③a · LOS SERVIDORES DE LENGUAJE, MEDIDOS CORRIENDOLOS DE VERDAD.

El tramo ① encendio lo que Monaco ya traia (json y typescript, en el navegador)
y el ② cavo la tuberia. Faltan los lenguajes que NO pueden tener servicio en el
navegador: python y java. Antes de meter nada en una imagen se mide QUE PESA,
QUE DICE SOBRE NUESTRAS PROPIAS SEMILLAS, y QUE CUESTA VIVO.

  §1  LAS IMAGENES DE HOY      con que nace cada puesto, que trae dentro y que
                               monta el pod.
  §2  LO QUE HABRIA QUE METER  el tamano de cada servidor, medido de su
                               descarga y de su despliegue, no de memoria.
  §3  QUE DICEN DE LO NUESTRO  los tres servidores corridos contra las semillas
                               que este producto siembra. Es la parte que duele.
  §4  QUE CUESTAN VIVOS        arranque, primer diagnostico, autocompletado,
                               hover y memoria residente.
  §5  QUE PODRIAN VER          lo que hay en disco en el puesto, y lo que no.

    python pruebas-de-fuego/medida-los-servidores-de-lenguaje.py
"""
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# ─────────────────────────────────────────────────────────────────────────────
# Lo medido el 2026-09-23 CORRIENDO cada servidor con un cliente LSP de verdad
# (JSON-RPC por la entrada y la salida estandar, con su `Content-Length`), sobre
# las semillas que `clases.rs` siembra y con el SDK del puesto en el camino.
#
#   pyright 1.1.414              node node_modules/pyright/langserver.index.js --stdio
#   jedi-language-server 0.47.0  el mismo cliente, sin node
#   jdtls 1.57.0 sobre JDK 21    equinox launcher, -data en un directorio nuevo
#
# No se mide aqui porque haria falta levantarlos: se anota de donde sale cada
# numero, que es lo que permite discutirlo o repetirlo.
# ─────────────────────────────────────────────────────────────────────────────
SERVIDORES = {
    "pyright": {
        "version": "1.1.414",
        "disco_mb": 29, "pide": "node (>=14)", "node_mb": 127,
        "initialize_s": 0.49, "primer_diagnostico_s": 3.81,
        "completion_s": 0.10, "completion_n": 46, "hover_s": 0.01,
        "memoria_mb": 210,
    },
    "jedi-language-server": {
        "version": "0.47.0",
        "disco_mb": 32, "pide": "nada (es Python)", "node_mb": 0,
        "initialize_s": 1.97, "primer_diagnostico_s": 1.03,
        "completion_s": 3.68, "completion_n": 1, "hover_s": 0.02,
        "memoria_mb": 10,
    },
    "jdtls": {
        "version": "1.57.0",
        "disco_mb": 53, "pide": "JDK 17+ (corre en el 21 que ya hay)", "node_mb": 0,
        "initialize_s": 6.56, "primer_diagnostico_s": 2.61,
        "completion_s": 0.56, "completion_n": 50, "hover_s": None,
        "memoria_mb": 514,
    },
}
DESCARGA = {"jdtls 1.57.0": 51366168, "node-v24.21.0-linux-x64.tar.xz": 31890184,
            "el binario `node` solo": 126595440, "pyright (npm, desplegado)": 19457120}


def titulo(t):
    print("\n" + t)
    print("  " + "-" * (len(t) - 2))


def leer(*p):
    return open(os.path.join(*p), encoding="utf-8", errors="replace").read()


def seccion_imagenes():
    titulo("  §1 LAS IMAGENES DE HOY (del Dockerfile y de la plantilla del pod)")
    d = leer(RAIZ, "Dockerfile")
    for base, etapa in re.findall(r"FROM ([^\s]+) AS (puesto-[a-z-]+)", d):
        bloque = next((b for b in d.split("\nFROM ") if (" AS " + etapa) in b.split("\n")[0]), "")
        tiene = []
        if "node:" in base or "npm install" in bloque:
            tiene.append("node")
        if "temurin" in base or "javac" in bloque:
            tiene.append("jdk")
        if "python:" in base or "pip install" in bloque:
            tiene.append("python")
        print("     %-14s %-32s dentro: %s" % (etapa, base, ", ".join(tiene) or "-"))
    y = leer(RAIZ, "malla", "51-el-puesto.yaml")
    montes = sorted(set(re.findall(r"mountPath: (/[a-z]+)", y)))
    print("     lo que el pod monta: %s" % ", ".join(montes))
    print("     ⛔ y el pod NO SALE A INTERNET (lo dice el propio Dockerfile: las")
    print("       extensiones de DuckDB se preinstalan «donde hay red» porque si no la")
    print("       primera celda se colgaria 120 s contra la NetworkPolicy). Lo que no")
    print("       venga DENTRO de la imagen, no existe.")


def seccion_tamanos():
    titulo("  §2 LO QUE HABRIA QUE METER, Y LO QUE PESA")
    for n, b in DESCARGA.items():
        print("     %-38s %6.1f MB" % (n, b / 1024 / 1024))
    print("")
    for n, s in SERVIDORES.items():
        extra = " + %d MB de node" % s["node_mb"] if s["node_mb"] else ""
        print("     %-22s %s   %d MB desplegado%s   pide: %s"
              % (n, s["version"], s["disco_mb"], extra, s["pide"]))
    print("     ⭐ Y TYPESCRIPT NO ESTA EN LA LISTA: Monaco ya trae su servicio completo")
    print("       en el navegador y desde 0037 ① esta encendido. El puesto de node no")
    print("       necesita NADA.")
    print("     ⛔ pyright son 29 MB… y 127 MB de `node`, porque `puesto-python` sale de")
    print("       `python:3.12-slim` y ahi no hay node. Es el precio real, no el del npm.")


def seccion_lo_nuestro():
    titulo("  §3 QUE DICEN DE NUESTRAS PROPIAS SEMILLAS (corridos de verdad)")
    c = leer(RAIZ, "crates", "ore-core", "src", "clases.rs")
    inyectados = re.search(r"`transform`, `over` y `write` los pone la sesion", c.replace("ó", "o"))
    print("     LA SEMILLA DE PYTHON (`transforms/ejemplo.py`) no importa nada: la")
    print("     sesion INYECTA `transform`, `over` y `write` en la celda, y la propia")
    print("     semilla lo dice%s." % (" (esta escrito en ella)" if inyectados else ""))
    print("       · pyright tal cual .......... 3 ERRORES: «\"transform\" is not defined»,")
    print("         «\"over\" is not defined», «\"write\" is not defined»")
    print("       · con `from ore import transform, over, write` y el SDK en el camino")
    print("         (`extraPaths`) ............ 0 errores, 0 avisos")
    print("       ⛔ Y EL ATAJO NO VALE: un `builtins.pyi` en `stubPath` NO ANADE a")
    print("         typeshed, lo REEMPLAZA. Medido: los tres nombres se resuelven y")
    print("         acto seguido «\"print\" is not defined». Ensenarle al servidor lo que")
    print("         la sesion inyecta cuesta mas que dejar de inyectarlo.")
    print("")
    print("     LA SEMILLA DE JAVA (`transforms/Ejemplo.java`) es un SNIPPET DE JSHELL")
    print("     —`var` y llamadas sueltas en el tope— y jdtls analiza UNIDADES DE")
    print("     COMPILACION:")
    print("       · jdtls sobre la semilla ..... 4 errores de sintaxis en la linea 12")
    print("         («Syntax error on token \".\", @ expected after this token»)")
    print("       · jdtls sobre una clase de verdad con un fallo de tipos ... EL ERROR")
    print("         BUENO: «The method toUpperCasee() is undefined for the type String»")
    print("       ⇒ El servidor no esta roto: nuestra semilla no es un fichero Java, es")
    print("         una celda. O el fichero pasa a ser una clase, o Java se queda sin")
    print("         subrayado — y decirlo ahora es mas barato que descubrirlo despues.")


def seccion_coste():
    titulo("  §4 QUE CUESTAN VIVOS (con un cliente LSP de verdad)")
    print("     %-22s %9s %11s %12s %8s %9s"
          % ("", "arranque", "1er diag.", "completion", "hover", "memoria"))
    for n, s in SERVIDORES.items():
        print("     %-22s %8.2fs %10.2fs %7.2fs/%-3d %7s %6d MB"
              % (n, s["initialize_s"], s["primer_diagnostico_s"], s["completion_s"],
                 s["completion_n"], ("%.2fs" % s["hover_s"]) if s["hover_s"] else "-",
                 s["memoria_mb"]))
    # ⛔ Y el del CONTENEDOR `puesto`, no el primero que aparezca: el primer
    #   `requests:` del fichero es el de un init, y son 50m/128Mi — un numero
    #   que no tiene nada que ver con lo que se esta decidiendo.
    y = leer(RAIZ, "malla", "51-el-puesto.yaml")
    nombre, pide = None, None
    for linea in y.splitlines():
        m = re.search(r"- name: ([a-z0-9-]+)\s*$", linea)
        if m:
            nombre = m.group(1)
        r = re.search(r'requests: \{cpu: "([^"]+)", memory: ([^\}]+)\}', linea)
        if r and nombre == "puesto":
            pide = (r.group(1), r.group(2).strip())
    print("")
    if pide:
        print("     el contenedor `puesto` pide hoy cpu %s / mem %s" % pide)
    print("     ⇒ pyright (210 MB) y jdtls (514 MB) caben. Pero jdtls tarda ~6,6 s en")
    print("       decir «listo» y NO ANALIZA NADA antes: quien lo hable tiene que")
    print("       esperar su `language/status ServiceReady` — sin eso, un `didOpen`")
    print("       devuelve CERO diagnosticos y el fichero PARECE correcto (medido).")
    print("     ⇒ Y la diferencia entre los dos de Python no es el tamano: es lo que")
    print("       saben. pyright propone 46 cosas en 0,10 s y comprueba tipos; jedi")
    print("       propone 1 en 3,68 s y solo ve errores de sintaxis. Los dos dan el")
    print("       hover con NUESTRO docstring, que es lo que mas se nota.")


def seccion_que_verian():
    titulo("  §5 QUE PODRIAN VER (lo que hay en disco en el puesto)")
    sdk = leer(RAIZ, "puesto", "python", "ore", "__init__.py")
    publicos = re.search(r"__all__ = \[([^\]]+)\]", sdk)
    nombres = re.findall(r'"([a-z_]+)"', publicos.group(1)) if publicos else []
    print("     el SDK va DENTRO de la imagen (`/opt/ore/ore`): %d lineas, %d nombres"
          % (len(sdk.splitlines()), len(nombres)))
    print("       %s" % ", ".join(nombres))
    print("     la capa del repositorio se monta en `/capa` y es el PYTHONPATH: lo que")
    print("     el arbol declara en su `pyproject.toml` esta ahi, resuelto.")
    print("     ⇒ Eso es exactamente lo que el navegador NO PUEDE saber, y es la razon")
    print("       de que el servidor viva en el puesto y no en Monaco.")
    print("")
    print("     ⛔ PERO EL REPOSITORIO NO ESTA EN DISCO. El editor abre los ficheros")
    print("       contra `ore-serve` (`GET /arbol/<ruta>`), no contra el pod; `/trabajo`")
    print("       es un `emptyDir` donde la celda corre. Un servidor de lenguaje ve el")
    print("       fichero abierto porque el propio LSP se lo manda en `didOpen` — y NO")
    print("       ve a sus vecinos. Para que vea el repositorio entero hay que")
    print("       escribirlo en `/trabajo`, y eso es una pieza aparte con su nombre.")


def main():
    print("=== 0037 ③a · los servidores de lenguaje, medidos")
    seccion_imagenes()
    seccion_tamanos()
    seccion_lo_nuestro()
    seccion_coste()
    seccion_que_verian()
    return 0


if __name__ == "__main__":
    sys.exit(main())
