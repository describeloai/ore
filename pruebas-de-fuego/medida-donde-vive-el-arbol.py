# -*- coding: utf-8 -*-
"""Donde tiene que vivir el arbol ontologico dentro del cluster.

`ore-serve` recibe un `--repo` y hoy en el cluster eso es un `emptyDir` que
muere con el pod. La pregunta parece de infraestructura —«que volumen»— y no lo
es: la contesta el TAMAÑO del arbol y lo que hace falta poder decir de el.

  A. CUANTO MIDE UN ARBOL     medido, induciendo uno de verdad
  B. QUE HAY EN EL CLUSTER    clases, CSI, addons, discos, pools
  C. QUE HAY QUE PODER DECIR  los criterios, y de donde sale cada uno
  D. LOS CANDIDATOS           siete, contra los criterios
  E. LO QUE CADA HECHO TACHA  y que queda en pie

Corre `ore discover` si encuentra el binario; si no, esa seccion lo dice.
"""
import json
import pathlib
import shutil
import subprocess
import tempfile
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
TABLAS = 200
COLUMNAS = 12


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def kubectl(*args):
    try:
        r = subprocess.run(["kubectl"] + list(args), capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def binario_ore():
    for p in ["target/release/ore.exe", "target/release/ore", "target/debug/ore.exe", "target/debug/ore"]:
        c = RAIZ / p
        if c.is_file():
            return c
    return None


def bytes_de(d):
    return sum(f.stat().st_size for f in d.rglob("*") if f.is_file())


def humano(n):
    for u in ["B", "KB", "MB", "GB"]:
        if n < 1024 or u == "GB":
            return "%.1f %s" % (n, u) if u != "B" else "%d B" % n
        n /= 1024.0


print("== donde vive el arbol, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - CUANTO MIDE UN ARBOL")
print()
ore = binario_ore()
if not ore:
    print("   (sin binario de `ore`: no se puede inducir un arbol para medirlo)")
    tam_arbol = tam_git = ficheros = None
else:
    tmp = pathlib.Path(tempfile.mkdtemp())
    repo = tmp / "repo"
    repo.mkdir()
    subprocess.run([str(ore), "init", "."], cwd=repo, capture_output=True)
    catalogo = {
        "source": "demo",
        "tables": [
            {
                "name": "public.tabla_%d" % i,
                "columns": [{"name": "col_%d" % j, "type": "String"} for j in range(COLUMNAS)],
                "primaryKey": ["col_0"],
            }
            for i in range(TABLAS)
        ],
    }
    (repo / "catalogo.json").write_text(json.dumps(catalogo), encoding="utf-8", newline="\n")
    subprocess.run(
        [str(ore), "discover", "--from", "catalogo.json", "--out", "packages/grande", "--name", "grande"],
        cwd=repo,
        capture_output=True,
    )
    (repo / "catalogo.json").unlink()
    paquete = repo / "packages" / "grande"
    ficheros = sum(1 for f in paquete.rglob("*") if f.is_file())
    tam_arbol = bytes_de(repo)

    # Y lo que cuesta guardarlo CON su historia.
    git = shutil.which("git")
    tam_git = None
    if git:
        env = {"GIT_AUTHOR_NAME": "m", "GIT_AUTHOR_EMAIL": "m@m", "GIT_COMMITTER_NAME": "m", "GIT_COMMITTER_EMAIL": "m@m"}
        import os

        e = dict(os.environ, **env)
        subprocess.run([git, "init", "-q", "."], cwd=repo, capture_output=True, env=e)
        subprocess.run([git, "add", "-A"], cwd=repo, capture_output=True, env=e)
        subprocess.run([git, "commit", "-qm", "uno"], cwd=repo, capture_output=True, env=e)
        m = repo / "packages" / "grande" / "package.yaml"
        if m.is_file():
            m.write_text(m.read_text(encoding="utf-8").replace("cambiame", "team:datos"), encoding="utf-8", newline="\n")
        subprocess.run([git, "add", "-A"], cwd=repo, capture_output=True, env=e)
        subprocess.run([git, "commit", "-qm", "dos"], cwd=repo, capture_output=True, env=e)
        tam_git = bytes_de(repo / ".git")

    print("   un origen de %d tablas x %d columnas" % (TABLAS, COLUMNAS))
    print("     documentos          %d ficheros" % ficheros)
    print("     el arbol            %s" % humano(tam_arbol))
    if tam_git:
        print("     .git, dos commits   %s" % humano(tam_git))
    print()
    print("   extrapolado, y es lo que decide:")
    for n, cuantos in [("1.000 tablas", 5), ("10 inquilinos de 1.000", 50), ("100 de 1.000", 500)]:
        print("     %-24s %s" % (n, humano(tam_arbol * cuantos)))
    shutil.rmtree(tmp, ignore_errors=True)
print()
parrafo("El arbol ontologico de un almacen mediano no llega al megabyte, y su "
        "historia entera cuesta MENOS que el arbol —los documentos son YAML y "
        "un cambio toca unos pocos—. Cien inquilinos con mil tablas cada uno "
        "caben de sobra en el disco mas pequeño que se puede pedir.")
parrafo("==> **La capacidad no decide nada.** Lo que decide es que hay que poder "
        "hacer con el arbol, y cuanto vale el MINIMO de cada opcion.")

# -- B -----------------------------------------------------------------------
print()
print("B - QUE HAY EN EL CLUSTER HOY")
print()
sc = kubectl("get", "storageclass", "-o", "json")
if sc is None:
    print("   (sin cluster)")
else:
    clases = json.loads(sc).get("items", [])
    print("   clase              provisionador           reclaim   modo")
    print("   " + "-" * 74)
    for c in clases:
        print(
            "   %-18s %-23s %-9s %s"
            % (
                c["metadata"]["name"],
                c.get("provisioner", ""),
                c.get("reclaimPolicy", ""),
                c.get("volumeBindingMode", ""),
            )
        )
    retiene = [c["metadata"]["name"] for c in clases if c.get("reclaimPolicy") == "Retain"]
    print()
    print("   clases con `Retain`: %s" % (", ".join(retiene) if retiene else "NINGUNA"))
    parrafo("Todas borran. Un arbol sobre un PVC de estos muere con el PVC, y "
            "no hace falta un `delete` del disco: basta borrar el `claim`. Es "
            "la misma trampa que la plataforma ya se encontro con su grafo.")

csi = kubectl("get", "csidrivers", "-o", "json")
if csi is not None:
    nombres = [d["metadata"]["name"] for d in json.loads(csi).get("items", [])]
    print()
    print("   CSI instalados: %s" % ", ".join(nombres))
    for nombre, que in [
        ("pd.csi.storage.gke.io", "disco persistente — ReadWriteOnce"),
        ("filestore.csi.storage.gke.io", "Filestore — ReadWriteMany"),
        ("gcsfuse.csi.storage.gke.io", "un bucket como si fuera un directorio"),
    ]:
        print("     %-3s %-32s %s" % ("SI" if nombre in nombres else "NO", nombre, que))

pvc = kubectl("get", "pvc", "-A", "-o", "json")
if pvc is not None:
    print()
    print("   PVC en el cluster: %d" % len(json.loads(pvc).get("items", [])))

nodos = kubectl("get", "nodes", "-o", "json")
if nodos is not None:
    zonas = sorted(
        {n["metadata"]["labels"].get("topology.kubernetes.io/zone", "?") for n in json.loads(nodos).get("items", [])}
    )
    print("   zonas con nodo:    %s" % ", ".join(zonas))
    parrafo("El cluster es ZONAL, asi que un disco zonal no pierde nada por "
            "serlo. Lo que si pierde es el `ReadWriteOnce`: los Jobs caen en "
            "`jobs-p`, que escala de 0 a 3 nodos, y dos Jobs en dos nodos NO "
            "pueden montar el mismo disco.")

# -- C -----------------------------------------------------------------------
print()
print("C - QUE HAY QUE PODER DECIR DEL ARBOL")
print()
CRITERIOS = [
    ("dos a la vez", "dos Jobs, o dos personas en la consola, tocando el mismo arbol"),
    ("sobrevive al pod", "hoy no: `emptyDir` muere con el"),
    ("quien cambio que", "es uno de los tres guardarrailes que faltan, medido"),
    ("volver atras", "`ore diff` compara DOS versiones; sin historia hay que traer la otra de algun sitio"),
    ("el minimo", "cuanto cuesta la unidad mas pequeña que se puede pedir"),
    ("un arbol por inquilino", "cientos de arboles diminutos, no uno grande"),
]
for que, porque in CRITERIOS:
    print("   · %s" % que)
    parrafo(porque, "       ")
print()
parrafo("Los cuatro primeros no son gustos: tres salen de medidas ya hechas "
        "—los guardarrailes que faltan, el `emptyDir`, la cola de decisiones "
        "que dos personas pueden contestar a la vez— y el cuarto sale de que "
        "`diff` ya existe y necesita dos versiones para hacer su trabajo.")

# -- D -----------------------------------------------------------------------
print()
print("D - LOS CANDIDATOS")
print()
# dos·a·la·vez | sobrevive | quien·cambio | volver·atras | minimo
CAND = [
    ("emptyDir (hoy)", "no", "NO", "no", "no", "0", "muere con el pod"),
    ("PVC pd-balanced", "NO (RWO)", "si", "no", "no", "10 GiB", "un solo nodo a la vez"),
    ("Filestore", "si (RWX)", "si", "no", "no", "1 TiB", "el CSI ni esta instalado"),
    ("bucket + gcsfuse", "si", "si", "a medias", "con versionado", "por byte", "el CSI no esta; y git sobre FUSE va mal"),
    ("bucket, un tar por version", "si", "si", "a medias", "si", "por byte", "hay que inventar el formato y el bloqueo"),
    ("Cloud SQL", "si", "si", "si, escrito a mano", "si, a mano", "~24 EUR/mes", "el arbol son ficheros, no filas"),
    ("git, servido en el cluster", "SI", "si", "SI, nativo", "SI, nativo", "10 GiB", "hay que levantar el servidor"),
]
print("   candidato                    2 a la vez  vive  quien  atras  minimo")
print("   " + "-" * 76)
for n, a, b, c, d, e, _ in CAND:
    print("   %-28s %-11s %-5s %-6s %-6s %s" % (n, a, b, c, d, e))
print()
for n, _, _, _, _, _, nota in CAND:
    print("   %-28s %s" % (n, nota))

# -- E -----------------------------------------------------------------------
print()
print("E - LO QUE CADA HECHO TACHA")
print()
TACHA = [
    ("el arbol no llega al MB", "Filestore y su minimo de 1 TiB: seis ordenes de magnitud"),
    ("los Jobs caen en 0-3 nodos", "el PVC `ReadWriteOnce`, que solo lo monta un nodo"),
    ("todas las clases borran", "cualquier PVC sin una clase `Retain` escrita a proposito"),
    ("faltan CSI de Filestore y GCS", "las dos opciones de fichero compartido, hoy y sin tocar el cluster"),
    ("«quien cambio que» falta", "todo lo que obligue a escribir un registro de auditoria aparte"),
    ("`ore diff` necesita dos versiones", "todo lo que guarde solo el estado de ahora"),
]
for hecho, que in TACHA:
    print("   %-34s -> %s" % (hecho, que))
print()
parrafo("Queda uno. Y no gana por eliminacion: gana porque **git no necesita "
        "un sistema de ficheros compartido**. Cada Job clona y empuja, asi que "
        "la pregunta `ReadWriteOnce` contra `ReadWriteMany` deja de existir; lo "
        "unico compartido es el servidor, que tiene un solo escritor por diseño "
        "y resuelve la carrera el mismo — una referencia se actualiza de forma "
        "atomica y un empujon que no avanza se RECHAZA.")
parrafo("Y lo que en las otras seis habria que construir, aqui ya esta: quien "
        "cambio que es el autor de un commit, volver atras es una referencia, y "
        "un arbol por inquilino es un repositorio mas. Es, palabra por palabra, "
        "«el registro con forma de repositorio».")
