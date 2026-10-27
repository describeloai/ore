"""ORE 0072 I4·2 · el Job del índice siguiendo a una tabla con historia, de punta a punta en local (la imagen
`ore-piramide:local`, con DuckDB, y el binario de la rama): `historia.py` deja en el banco una tabla con cuatro
instantáneas (una base, una carga, un borrado copy-on-write y una retención); el Job corre en cada una, en orden,
contra el mismo destino —la primera construye, las demás deben seguir—; al final, el Job reconstruye la última en
otro destino, y los dos manifiestos tienen que decir lo mismo (filas y suma de ids). Limpia el banco al acabar.

  python pruebas-de-fuego/geo/i4/e2e.py            (necesita `gcloud` con sesión y Docker)
"""
import json
import os
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8")
S = os.path.dirname(os.path.abspath(__file__))
CUBO = "project-8853a180-450d-47be-b83-banco-geo"
RAIZ = "i4/e2e-%d" % int(time.time())
LOG = open(os.path.join(S, time.strftime("e2e-%Y%m%d-%H%M.log")), "w", encoding="utf-8")


def di(*a):
    t = " ".join(str(x) for x in a)
    print(t, flush=True)
    LOG.write(t + "\n")
    LOG.flush()


def sh(args, entrada=None, env=None):
    if args[0] == "gcloud" and os.name == "nt":
        args = ["gcloud.cmd"] + args[1:]
    p = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", input=entrada, env=env)
    return p.returncode, p.stdout, p.stderr


# uno recién renovado: el de la caché puede caducar a mitad de la prueba
tok = sh(["gcloud", "config", "config-helper", "--force-auth-refresh",
          "--format=value(credential.access_token)"])[1].strip()
assert tok, "sin token de gcloud"
env = dict(os.environ, TOKEN=tok)


def job(ml, destino, trabajo):
    pet = {"tarea": "indice", "capa": "e2e.viajes", "suelo": 13, "trabajo": "/trabajo",
           "fuente": {"metadata_location": ml, "geometria": "t_pickuploc", "crs": "EPSG:4326",
                      "columnas": ["t_tripkey", "t_fare"], "credencial": {"gcs.oauth2.token": tok}},
           "destino": {"ubicacion": destino, "credencial": {"gcs.oauth2.token": tok}}}
    c, o, e = sh(["docker", "run", "--rm", "-i", "-v", trabajo + ":/trabajo", "ore-piramide:local"],
                 entrada=json.dumps(pet))
    lineas = [l for l in o.splitlines() if l.startswith("INDICE|")]
    return c, lineas, e


try:
    c, o, e = sh(["docker", "run", "--rm", "-e", "TOKEN", "-e", "RAIZ=" + RAIZ, "-e", "FETCH_RAW_TOKEN_EXPIRY=false", "-v", S + ":/i4", "python:3.12-slim",
                  "sh", "-c", "pip -q install --root-user-action=ignore 'pyiceberg[gcsfs,sql-sqlite]' duckdb==1.5.6 pyarrow "
                  "2>/dev/null && python -I /i4/historia.py"], env=dict(env, RAIZ=RAIZ))
    linea = [l for l in o.splitlines() if l.startswith("INSTANTANEAS|")]
    assert c == 0 and linea, e[-1500:]
    pasos = json.loads(linea[0].split("|", 1)[1])
    di("# historia:", len(pasos), "instantáneas;", [l for l in o.splitlines() if l.startswith("FILAS|")])
    tabla = pasos[0].rsplit("/metadata/", 1)[0]
    seguido, rehecho = f"gs://{CUBO}/{RAIZ}/seguido", f"gs://{CUBO}/{RAIZ}/rehecho"
    trabajo = tempfile.mkdtemp()
    for i, ml in enumerate(pasos):
        t = time.time()
        c, ls, e = job(ml, seguido, trabajo)
        di(f"\n## S{i} · salida {c} · {time.time() - t:.0f} s")
        for l in ls:
            di("  " + l[:400])
        if c != 0:
            di("  ✗ " + e[-800:])
    t = time.time()
    c, ls, e = job(pasos[-1], rehecho, trabajo)
    di(f"\n## reconstruir S{len(pasos) - 1} · salida {c} · {time.time() - t:.0f} s")
    for l in ls:
        di("  " + l[:400])
    # lo mismo por contenido: los ids no se comparan (son `ordinal << 40 | fila`, y reconstruir numera
    # los ficheros de nuevo); DuckDB lee las dos tablas del índice con su extensión Iceberg, aplicando
    # los borrados por posición (un lector que no es nuestro), y cuenta filas y suma de `t_tripkey`
    stem = pasos[-1].rsplit("/", 1)[1].removesuffix(".metadata.json")
    mls = {}
    for d in (seguido, rehecho):
        _, o, _ = sh(["gcloud", "storage", "cat", f"{d}/_ore_capa/i1/{stem}/manifiesto.json"])
        m = json.loads(o or "{}")
        mls[d] = m.get("indice", "")
        di(f"# {d.rsplit('/', 1)[1]}: filas {m.get('filas')} · generación {m.get('generacion')} · "
           f"profundidad {m.get('profundidad')}")
    c, o, e = sh(["docker", "run", "--rm", "-e", "TOKEN", "-e", "MLS", "-v", S + ":/i4", "python:3.12-slim",
                  "sh", "-c", "pip -q install --root-user-action=ignore duckdb==1.5.6 2>/dev/null && "
                  "python -I /i4/leer.py"],
                 env=dict(env, MLS=json.dumps([mls[seguido], mls[rehecho]])))
    leido = [json.loads(l.split("|", 1)[1]) for l in o.splitlines() if l.startswith("LEIDO|")]
    if len(leido) != 2:
        di("# ✗ DuckDB no leyó las tablas del índice:", (e or o)[-800:])
    else:
        di("# por iceberg_scan · seguido:", leido[0][1:], "· rehecho:", leido[1][1:])
        di("# igual:", "✓" if leido[0][1:] == leido[1][1:] else "✗")
finally:
    sh(["gcloud", "storage", "rm", "-r", f"gs://{CUBO}/{RAIZ}/"])
    di("limpio:", RAIZ)
