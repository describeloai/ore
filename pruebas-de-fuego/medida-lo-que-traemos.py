# -*- coding: utf-8 -*-
"""Que infraestructura de la plataforma se trae, exactamente, y si cabe.

El IdP ya esta. Al entrar en `/governance/users` la consola pide
`http://127.0.0.1:8082` y no hay nadie: ese es `paladio-admin`, que necesita una
Postgres que NO es la del IdP — es la de la plataforma, `rubix-federation`, la
segunda Cloud SQL del cluster viejo.

  A. LAS PIEZAS           todas las del despliegue, con lo que piden
  B. EL MINIMO            lo que hace falta para que esa pagina conteste
  C. LO QUE NO SE TRAE    y por que cada una
  D. SI CABE              contra el hueco real del nodo
  E. LAS IMAGENES         cuales hay que cocer aqui

Lee `C:\\Rubix` sin escribir nada.
"""
import pathlib
import re
import subprocess
import textwrap

RUBIX = pathlib.Path(r"C:\Rubix")
BASE = RUBIX / "deploy" / "base"


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def texto(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def mili(v):
    return int(v[:-1]) if v.endswith("m") else int(float(v) * 1000)


def pide(fichero):
    """Replicas y CPU pedida por TODOS los contenedores de un despliegue."""
    t = texto(BASE / fichero)
    if not t:
        return None
    rep = re.search(r"^\s*replicas:\s*(\d+)", t, re.M)
    n = int(rep.group(1)) if rep else 1
    cpus = re.findall(r"requests:\s*\{?\s*cpu:\s*\"?([0-9a-z.]+)\"?", t)
    cpus += re.findall(r"requests:\n\s+cpu:\s*\"?([0-9a-z.]+)\"?", t)
    return n, [mili(c) for c in dict.fromkeys(cpus)]


print("== lo que traemos, medido ==")

if not BASE.is_dir():
    print("   (no esta `C:\\Rubix\\deploy`: esta medida no puede correr)")
    raise SystemExit(0)

# -- A -----------------------------------------------------------------------
print()
print("A - LAS PIEZAS DE LA PLATAFORMA")
print()
PIEZAS = [
    ("api", "api/deployment.yaml", "la puerta: verifica el token y decide"),
    ("admin", "admin/deployment.yaml", "personas, organizaciones e invitaciones"),
    ("consola", "consola/deployment.yaml", "la web — hoy corre en tu portatil"),
    ("consumidor", "consumidor/deployment.yaml", "el outbox, gobernado por KEDA"),
    ("grafo", "grafo/oxigraph.yaml", "Oxigraph, el RDF · PVC de 10 GB CON DATOS"),
    ("almacen", "almacen/proxy.yaml", "un proxy de Cloud SQL, y nada mas"),
]
print("   pieza        repl  CPU total  que es")
print("   " + "-" * 74)
total = 0
for nombre, f, que in PIEZAS:
    r = pide(f)
    if not r:
        print("   %-12s  ?         ?      %s" % (nombre, que))
        continue
    n, cpus = r
    c = sum(cpus) * n
    total += c
    print("   %-12s  %d    %5dm     %s" % (nombre, n, c, que))
print("   " + "-" * 74)
print("   %-12s       %5dm     todo lo que esta siempre en pie" % ("TOTAL", total))
print()
parrafo("Y a eso hay que sumarle lo que en el cluster viejo NO era un pod: dos "
        "Cloud SQL. La del IdP ya se sustituyo por un Postgres aqui; falta la "
        "otra, `rubix-federation`, que es de donde `admin` y `api` leen.")

# -- B -----------------------------------------------------------------------
print()
print("B - EL MINIMO PARA QUE `/governance/users` CONTESTE")
print()
MINIMO = [
    ("postgres de la plataforma", 100, "las 22 migraciones de `modelo/migraciones`"),
    ("admin", 50, "es a quien llama la pagina — el 8082"),
    ("api", 50, "el resto de la consola lo necesita — el 8081"),
]
m = 0
for n, c, por in MINIMO:
    m += c
    print("   %-28s %4dm   %s" % (n, c, por))
print("   " + "-" * 74)
print("   %-28s %4dm" % ("MINIMO", m))
print()
parrafo("`api` con UNA replica y no dos. Alla eran dos por nivel de servicio; "
        "aqui se esta levantando, y decir «dos» sin que quepan seria peor que "
        "decir «una» y que arranque.")

# -- C -----------------------------------------------------------------------
print()
print("C - LO QUE NO SE TRAE, Y POR QUE")
print()
FUERA = [
    ("los cuatro proxies de Cloud SQL", 80,
     "`api`, `admin`, `consumidor` y `almacen` llevan cada uno un sidecar "
     "`cloud-sql-proxy`. Con la base DENTRO del cluster no hay nada que "
     "proxiar: se habla con un Service"),
    ("el grafo (Oxigraph)", 100,
     "es el RDF, y su PVC de 10 GB SI tiene datos. No hace falta para ver "
     "personas, y traerlo es una mudanza con datos dentro — otra decision"),
    ("KEDA y el consumidor", 70,
     "el outbox escala de 0 a 5 segun una cola. Sin ingesta no hay cola, y "
     "KEDA es un controlador mas que reservar en un nodo que va justo"),
    ("cosecha, ingesta, ingesta-bq", 0,
     "Jobs contra origenes. No estan en pie: se disparan"),
    ("la consola desplegada", 50,
     "ya corre en tu portatil contra el IdP de aqui. Desplegarla es para "
     "cuando haya que entrar sin el portatil"),
]
ahorro = 0
for n, c, por in FUERA:
    ahorro += c
    print("   %-32s %4dm" % (n, c))
    parrafo(por, "       ")
print()
print("   se dejan de reservar: %dm" % ahorro)

# -- D -----------------------------------------------------------------------
print()
print("D - SI CABE")
print()
try:
    salida = subprocess.run(
        ["kubectl", "describe", "node", "-l", "cloud.google.com/gke-nodepool=default-pool"],
        capture_output=True, text=True, timeout=120).stdout
except Exception:
    salida = ""
usado = re.search(r"cpu\s+(\d+)m\s+\(", salida)
alloc = re.search(r"cpu:\s+(\d+)m", salida)
if usado and alloc:
    libre = int(alloc.group(1)) - int(usado.group(1))
    print("   asignable   %5dm" % int(alloc.group(1)))
    print("   reservado   %5dm" % int(usado.group(1)))
    print("   LIBRE       %5dm" % libre)
    print()
    print("   el minimo pide %dm  ->  %s" % (m, "CABE" if m <= libre else "NO CABE"))
    if m <= libre:
        print("   quedarian %dm de margen" % (libre - m))
        parrafo("Cabe, y por poco. Traer la plataforma ENTERA —%dm— no cabe: "
                "para eso hay que anadir un nodo, y son ~49 EUR/mes."
                % total, "     ")
else:
    print("   (sin cluster)")

# -- E -----------------------------------------------------------------------
print()
print("E - LAS IMAGENES QUE HAY QUE COCER")
print()
parrafo("Las suyas viven en el registro de `trino-k8s`, que esta cuenta no "
        "puede leer. Pero el codigo esta aqui y las recetas tambien, asi que "
        "se construyen igual que se construyo la del IdP.")
print()
for d in ["api", "admin", "consumidor", "cosechador", "extractor"]:
    hay = (RUBIX / d / "Dockerfile").is_file()
    receta = (RUBIX / ("cloudbuild-%s.yaml" % d)).is_file()
    print("   %-12s Dockerfile %-4s receta %s" % (d, "si" if hay else "NO", "si" if receta else "NO"))
print()
parrafo("Del minimo, dos: `api` y `admin`. Postgres ya esta espejado.")
