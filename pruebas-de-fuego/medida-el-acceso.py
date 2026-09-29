"""0047 · M1 · El inventario de preguntas y de eventos.

Cada ruta que monta `ore-serve`, con lo que hace, para saber qué le preguntaría a `ore-acceso`
(`puede`) y qué le diría (`hizo`). Estático: lee el código, sin cluster ni red.

    python pruebas-de-fuego/medida-el-acceso.py [informe.md | --json]

Lo que mide (0047, § M1):

1. los brazos del enrutador de `rutas.rs`: método, camino y la función que atiende;
2. por función, lo que toca: el árbol (`escribiendo_en`), la celda (`escribiendo`), la forja, un
   Job, el custodio, un puesto;
3. la clase de cada ruta: lectura, árbol, celda, propuesta/rama, puesto, u otra;
4. la potestad candidata (`recurso:verbo`) y cuántas distintas salen;
5. si la ruta necesita el recurso para decidir (lleva un nombre en el camino);
6. el evento que emitiría (`hizo`), si ya lo cubre un commit (`escribiendo_en` y `escribiendo`
   acaban en uno; la forja deja el suyo en propuestas y ramas) y cuántas rutas que escriben no
   dejan hoy ningún rastro;
7. las potestades y los roles que ya existen (`iam/migraciones`);
8. lo que el enrutador monta y `rutas::mapa` no anuncia al arrancar.

Su límite: mira la función que atiende y sólo un nivel; lo que se decida más abajo sale como
«otra». Se dice en el informe.
"""

import collections
import json
import pathlib
import re
import sys

RAIZ = pathlib.Path(__file__).resolve().parent.parent
SERVE = RAIZ / "crates" / "ore-serve" / "src"
RUTAS = SERVE / "rutas.rs"
MIGRACIONES = RAIZ / "iam" / "migraciones"

METODO = r'"(?:GET|POST|PUT|DELETE|PATCH)"'
BRAZO = re.compile(r"\(\s*((?:" + METODO + r"\s*\|?\s*)+),\s*\[([^\]]*)\]\s*\)")
FN = re.compile(r"^(\s*)(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)

# Lo que una función toca, por lo que llama. Un nivel.
MARCAS = [
    ("arbol", re.compile(r"escribiendo_en\(")),
    ("celda", re.compile(r"escribiendo\(")),
    ("forja", re.compile(r"\bforja\b|Api::|\.fusionar|\.abrir_pr")),
    ("job", re.compile(r"lanzar|\bJob\b|trabajo::")),
    ("cola", re.compile(r"encolar|\bcola::|self\.cola\b")),
    ("cofre", re.compile(r"cofre")),
    ("puesto", re.compile(r"puesto")),
]

SINGULAR = {
    "fuentes": "fuente", "paquetes": "paquete", "vistas": "vista", "propuestas": "propuesta",
    "ramas": "rama", "puestos": "puesto", "documentos": "documento", "modelos": "modelo",
    "proyectos": "proyecto", "repositorios": "repositorio", "funciones": "funcion",
    "conceptos": "concepto", "perfiles": "perfil", "datasets": "dataset", "decisiones": "decision",
    "copias": "copia", "tablas": "tabla", "schemas": "schema", "assets": "asset",
}
POR_METODO = {"GET": "leer", "POST": "crear", "PUT": "cambiar", "DELETE": "borrar", "PATCH": "cambiar"}
SIN_PREGUNTA = {"salud", "version"}


def indice_de_funciones():
    """nombre -> cuerpos (puede haber varias con el mismo nombre en ficheros distintos)."""
    cuerpos = collections.defaultdict(list)
    for f in sorted(SERVE.glob("*.rs")):
        t = f.read_text(encoding="utf-8")
        ms = list(FN.finditer(t))
        for i, m in enumerate(ms):
            sangria = len(m.group(1).replace("\n", ""))
            fin = len(t)
            for n in ms[i + 1:]:
                if len(n.group(1).replace("\n", "")) <= sangria:
                    fin = n.start()
                    break
            cuerpos[m.group(2)].append(t[m.start():fin])
    return cuerpos


def segmentos(patron):
    """`"fuentes", n, "catalogar"` -> [("fuentes", True), ("n", False), ("catalogar", True)]."""
    out = []
    for s in [x.strip() for x in patron.split(",") if x.strip()]:
        if s.startswith('"'):
            out.append((s.strip('"'), True))
        elif ".." in s:
            out.append(("..", False))
        else:
            out.append((s, False))
    return out


def camino(segs):
    return "/" + "/".join(s if lit else ("{..}" if s == ".." else "{}") for s, lit in segs)


def normaliza(c):
    return re.sub(r"\{[^}]*\}", "{}", c).replace("{}..", "{..}")


def brazos(texto, fin):
    ms = [m for m in BRAZO.finditer(texto, 0, fin)]
    out = []
    for i, m in enumerate(ms):
        flecha = texto.find("=>", m.end())
        if flecha < 0 or flecha > fin:
            continue
        # Un brazo con alternativas (`(...) | (...) =>`) comparte el cuerpo: el cuerpo empieza en
        # la flecha y acaba en el primer brazo que empiece DESPUÉS de ella.
        entre = texto[m.end():flecha]
        guarda = re.search(r"\bif\b([^\n{]*)", entre)
        siguiente = next((n.start() for n in ms[i + 1:] if n.start() > flecha), fin)
        cuerpo = texto[flecha:siguiente][:1500]
        metodos = re.findall(r'"([A-Z]+)"', m.group(1))
        out.append({
            "linea": texto.count("\n", 0, m.start()) + 1,
            "metodos": metodos,
            "segs": segmentos(m.group(2)),
            "guarda": guarda.group(1).strip() if guarda else "",
            "cuerpo": cuerpo,
        })
    return out


def llamadas(cuerpo, indice):
    nombres = set(re.findall(r"self\.(\w+)\(", cuerpo))
    nombres |= set(re.findall(r"leyendo_en\(\s*\w+\s*,\s*(\w+)", cuerpo))
    nombres |= {n for n in re.findall(r"\b(\w+)\(", cuerpo) if n in indice}
    return sorted(n for n in nombres if n in indice and n not in {"leyendo_en", "ok", "error"})


def clase(b, marcas):
    s0 = b["segs"][0][0] if b["segs"] else ""
    if s0 in ("propuestas", "ramas"):
        return "propuesta/rama"
    if s0 == "puestos":
        return "puesto"
    if "arbol" in marcas:
        return "arbol"
    if "celda" in marcas:
        return "celda"
    if all(m == "GET" for m in b["metodos"]):
        return "lectura"
    return "otra"


def candidata(b, metodo):
    lits = [s for s, lit in b["segs"] if lit]
    if not lits:
        return "?"
    recurso = SINGULAR.get(lits[0], lits[0].rstrip("s"))
    verbo = lits[-1] if len(lits) > 1 and metodo in ("POST", "PUT", "DELETE") else POR_METODO[metodo]
    return f"{recurso}:{verbo}"


def rastro(f):
    """El rastro que deja hoy una ruta: `commit` (el árbol o la celda), `cola` (un commit en la
    cola de trabajos, con la persona de autora; idempotente: repetir no deja otro), `forja` (la
    PR, el merge, la rama), o nada. Leer el árbol de la forja no es dejar rastro. Las lecturas no
    escriben: su rastro es opcional (0047 H12)."""
    if f["metodo"] == "GET":
        return "lectura"
    if "arbol" in f["marcas"] or "celda" in f["marcas"]:
        return "commit"
    if "cola" in f["marcas"]:
        return "cola"
    if f["clase"] == "propuesta/rama":
        return "forja"
    return "ninguno"


def potestades_existentes():
    todas, por_rol = set(), collections.defaultdict(set)
    for f in sorted(MIGRACIONES.glob("*.sql")):
        t = f.read_text(encoding="utf-8")
        for rol, p in re.findall(r"\(\s*'([A-Z]+)'\s*,\s*'([a-z_]+:[a-z_-]+)'", t):
            por_rol[rol].add(p)
        todas |= set(re.findall(r"'([a-z_]+:[a-z_-]+)'", t))
    return todas, por_rol


def anunciadas(texto, desde):
    fin = texto.find("\npub fn ruta_de", desde)
    region = texto[desde:fin]
    return {(m, normaliza(c)) for m, c in re.findall(r'\(\s*"([A-Z]+)",\s*"(/[^"]*)"', region)}


def main():
    texto = RUTAS.read_text(encoding="utf-8")
    inicio_mapa = texto.find("pub fn mapa(")
    indice = indice_de_funciones()
    filas = []
    for b in brazos(texto, inicio_mapa):
        if b["segs"] and b["segs"][0][0] in SIN_PREGUNTA:
            continue
        fns = llamadas(b["cuerpo"], indice)
        marcas = sorted({n for f in fns for c in indice[f] for n, rx in MARCAS if rx.search(c)}
                        | {n for n, rx in MARCAS if rx.search(b["cuerpo"])})
        k = clase(b, marcas)
        con_recurso = any(not lit for _, lit in b["segs"])
        for metodo in b["metodos"]:
            filas.append({**b, "metodo": metodo, "camino": camino(b["segs"]), "fns": fns,
                          "marcas": marcas, "clase": k, "con_recurso": con_recurso,
                          "candidata": candidata(b, metodo)})

    for f in filas:
        f["rastro"] = rastro(f)
    existentes, por_rol = potestades_existentes()
    montadas = {(f["metodo"], normaliza(f["camino"])) for f in filas}
    sin_anunciar = sorted(montadas - anunciadas(texto, inicio_mapa))

    por_clase = collections.Counter(f["clase"] for f in filas)
    candidatas = collections.Counter(f["candidata"] for f in filas)
    lectura = sum(1 for f in filas if f["metodo"] == "GET")

    o = []
    por_rastro = collections.Counter(f["rastro"] for f in filas)
    o.append("# 0047 · M1 · El inventario de preguntas y de eventos\n")
    o.append(f"Rutas (método × camino, sin `salud` ni `version`): **{len(filas)}** · "
             f"lectura (`GET`): **{lectura}** · escritura: **{len(filas) - lectura}** · "
             f"con recurso en el camino: **{sum(f['con_recurso'] for f in filas)}**\n")
    o.append("## Por clase\n\n| clase | rutas |\n|---|---|")
    o += [f"| {k} | {n} |" for k, n in por_clase.most_common()]
    o.append("\n## El rastro que dejan hoy (`hizo`)\n\n| rastro | rutas |\n|---|---|")
    o += [f"| {k} | {n} |" for k, n in por_rastro.most_common()]
    o.append("\nLas que escriben y no dejan ninguno:\n")
    o += [f"- `{f['metodo']} {f['camino']}` ({f['clase']}; {' '.join(f['marcas']) or 'sin marcas'})"
          for f in filas if f["rastro"] == "ninguno"]
    o.append(f"\n## Potestades candidatas: {len(candidatas)} distintas\n")
    o.append("| candidata | rutas | ya existe |\n|---|---|---|")
    o += [f"| `{c}` | {n} | {'sí' if c in existentes else ''} |" for c, n in sorted(candidatas.items())]
    o.append("\n## Las que ya existen (`iam/migraciones`)\n")
    o.append(f"{len(existentes)} potestades: " + ", ".join(f"`{p}`" for p in sorted(existentes)))
    o.append("\n| rol | potestades |\n|---|---|")
    o += [f"| {r} | {len(ps)} |" for r, ps in sorted(por_rol.items())]
    o.append(f"\n## Montadas y no anunciadas por `rutas::mapa`: {len(sin_anunciar)}\n")
    o += [f"- `{m} {c}`" for m, c in sin_anunciar]
    o.append("\n## Rutas\n\n| línea | método | camino | clase | toca | rastro | candidata | atiende |\n"
             "|---|---|---|---|---|---|---|---|")
    for f in sorted(filas, key=lambda f: (f["clase"], f["camino"], f["metodo"])):
        guarda = f" (si {f['guarda']})" if f["guarda"] else ""
        o.append(f"| {f['linea']} | {f['metodo']} | `{f['camino']}`{guarda} | {f['clase']} | "
                 f"{' '.join(f['marcas'])} | {f['rastro']} | `{f['candidata']}` | "
                 f"{' '.join(f['fns'][:4])} |")
    o.append("\n**Límite:** un nivel de llamadas; la clase «otra» se mira a mano. La candidata es "
             "una propuesta de nombre, no el catálogo.")
    informe = "\n".join(o) + "\n"

    # `--json`: la tabla de rutas, para M2 (`medida-el-salto.sh` clasifica con ella lo que cuenta).
    if sys.argv[1:2] == ["--json"]:
        sys.stdout.write(json.dumps([{"metodo": f["metodo"], "camino": normaliza(f["camino"]),
                                      "clase": f["clase"], "rastro": f["rastro"]} for f in filas],
                                    ensure_ascii=False) + "\n")
        return
    if len(sys.argv) > 1:
        pathlib.Path(sys.argv[1]).write_text(informe, encoding="utf-8", newline="\n")
    sys.stdout.reconfigure(encoding="utf-8")
    print(informe if len(sys.argv) == 1 else "\n".join(o[:24]))


if __name__ == "__main__":
    main()
