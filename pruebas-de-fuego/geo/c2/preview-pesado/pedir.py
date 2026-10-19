"""ORE 0068 C2·3 pieza 5 · El conductor del laboratorio del Preview Heavyweight, dentro del banco (`docker exec`) y
como ana (lo llama `medir.sh`):
    pedir.py sesion                 → abre (o da) la sesión de Python de ana: su id
    pedir.py lanzar <caso> <sesion> → POST /puestos/{sesion}/preview: una línea JSON con lo que contesta
    pedir.py ver <caso> <trabajo>   → GET /puestos/{trabajo}/celdas/1: OK|FALLA y lo que dice
    pedir.py lago                   → lo que el lago y el árbol dicen al final (nada escrito)
"""
import json
import re
import sys
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8080"
ANA = {"x-ore-sujeto": "persona:ana", "content-type": "application/json"}
ARBOL = "/tmp/banco/arbol"


def pedir(metodo, ruta, cuerpo=None):
    q = urllib.request.Request(BASE + ruta, data=None if cuerpo is None else json.dumps(cuerpo).encode(),
                               method=metodo, headers=ANA)
    try:
        with urllib.request.urlopen(q, timeout=60) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


PY = "packages/geo/pesadopy/transforms/anillos.py"
JAVA = "packages/geo/pesadojava/transforms/geo/AnillosJava.java"
SQL = "packages/geo/pesado/transforms/conteo.sql"


def en_el_editor(ruta, cambio):
    """El fichero del árbol (del árbol del banco), con el cambio del editor sin guardar."""
    texto = open(ARBOL + "/" + ruta, encoding="utf-8").read()
    for a, b in cambio:
        assert a in texto, (ruta, a)
        texto = texto.replace(a, b)
    return texto


# caso → (fichero, transform, cambio del editor, qué se espera)
CASOS = {
    # el texto del editor, no el del árbol: 80 m y una columna más
    "py": (PY, "anillos", [("ST_Buffer(zona, 50)", "ST_Buffer(zona, 80)"),
                           ('"id", ', "\"id\", \"'editor' AS de\", ")], "ok"),
    "java": (JAVA, "anillos", [('z.selectExpr("id", ', 'z.selectExpr("id", "\'editor\' AS de", ')], "ok"),
    "py-fuera": (PY, "fuera", [], "runtime"),
    "java-techo": (JAVA, "anillos", [('write("geo.anillos_java"', 'write("geo.otra"')], "runtime"),
    "java-no-compila": (JAVA, "anillos", [("Dataset<Row> z", "Datset<Row> z")], "syntax"),
    "sql": (SQL, "1", [], 422),
}


def main():
    que = sys.argv[1]
    if que == "sesion":
        c, r = pedir("POST", "/puestos", {"lenguaje": "python"})
        print(r.get("id") if c in (200, 201, 202) else "ERROR %s %s" % (c, r))
    elif que == "lanzar":
        fichero, t, cambio, _ = CASOS[sys.argv[2]]
        c, r = pedir("POST", "/puestos/%s/preview" % sys.argv[3],
                     {"fichero": fichero, "contenido": en_el_editor(fichero, cambio), "transform": t})
        print(json.dumps({"codigo": c, **{k: r.get(k) for k in ("trabajo", "celda", "entorno", "estado", "error", "motivo")}}))
    elif que == "ver":
        caso, trabajo = sys.argv[2], sys.argv[3]
        espera = CASOS[caso][3]
        c, r = pedir("GET", "/puestos/%s/celdas/1" % trabajo)
        ficha = r.get("preview") or {}
        visto = ((r.get("salida") or {}).get("informe") or {}).get("preview") or {}
        err = ficha.get("error") or {}
        if espera == "ok":
            cols = {x["name"]: x for x in visto.get("columnas") or []}
            geo = cols.get("anillo", {})
            fila = (visto.get("filas") or [[None]])[0]
            bien = (ficha.get("estado") == "succeeded" and visto.get("total") == 4 and "de" in cols
                    and geo.get("type") == "Geometry<Polygon, EPSG:25830>" and geo.get("iceberg") == "binary"
                    and isinstance(fila[list(cols).index("anillo")], str)
                    and fila[list(cols).index("anillo")].startswith("0103000000"))  # WKB en hex de un Polygon (01 §5)
            dicho = "total %s · columnas %s · %s" % (visto.get("total"), [(k, v["type"]) for k, v in cols.items()],
                                                     str(fila[list(cols).index("anillo")])[:40] if "anillo" in cols else "")
        else:
            bien = ficha.get("estado") == "failed" and err.get("tipo") == espera
            dicho = "%s · %s" % ({k: v for k, v in err.items() if k != "mensaje"}, (err.get("mensaje") or "")[:200])
        print("%s · %s · %s · %s" % ("OK   " if bien else "FALLA", caso, ficha.get("estado"), dicho))
    elif que == "lago":
        for t in ("anillos", "anillos_java", "otra"):
            c, r = pedir("GET", "/v1/geo/namespaces/default/tables/" + t)
            print("%s · geo.%s · %s" % ("OK   " if c == 404 else "FALLA", t, "no existe" if c == 404 else "¡%s!" % c))
        import subprocess

        log = subprocess.run(["git", "-C", ARBOL, "log", "--format=%s", "-30"], capture_output=True, text=True).stdout
        trabajos = [x for x in log.splitlines() if re.match(r"^Trabajo trabajo-", x)]
        print("%s · el árbol · %s" % ("OK   " if not trabajos else "FALLA",
                                      "ningún informe de trabajo" if not trabajos else trabajos))


main()
