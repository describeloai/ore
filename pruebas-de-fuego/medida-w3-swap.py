#!/usr/bin/env python3
"""
MEDIDA · W3.6b · el swap y el lago (20 de septiembre), antes de construir.

0031 §10 dice que el árbol es el catálogo: el puntero de un dataset vive en
`copias/` o `datasets/`, moverlo es un commit, y la forja —al rechazar lo que
no avanza en línea recta— es el compare-and-set. El Job de copia ya lo hace
solo (empuja). Lo que W3.6b añade es que **ore-serve lo haga por quien no
puede empujar** (un puesto que escribe con `write()`, W3.6c), que el lago sea
una fuente (`datasource: lago`) y que alguien mantenga las tablas. Antes de
escribir nada, cinco medidas con lo que ya existe:

  §1  LA CARRERA     ocho hilos escriben el MISMO puntero a la vez por
                     `PUT /arbol/datasets/<p>_<t>.json` con `If-Match` (el
                     CAS que ore-serve ya tiene, a nivel de commit) contra
                     una forja de verdad (repositorio pelado, `file://`):
                     ¿gana exactamente uno? ¿los demás ven 409 y nada a
                     medias? ¿cuánto cuesta cada commit (clonar+commit+push)
                     y cuánto una ronda entera de reintentos?
  §2  EL LAGO        `datasource: lago` (tipo abierto en OOS) con una `Table`
                     tipada encima y una `View` sobre ella: ¿compila? ¿qué
                     dice `ore materialize` de una vista que lee del lago?
                     ¿qué contesta `GET /puestos/{id}/datos/<p>.<t>` para una
                     Table (hoy sólo resuelve Views)?
  §3  LOS METADATOS  300 snapshots seguidos sobre una tabla (ore-store-r2 con
                     el S3 de mentira): cuánto crece `metadata.json`, cuánto
                     tarda en abrirse, y qué queda tras `recoger` — lo que
                     decide la política de retención del mantenimiento
  §4  EL BARRIDO     lo que un CronJob de mantenimiento tendría que recorrer:
                     los objetos del bucket de demo (sólo listar, con gcloud),
                     y lo que `ore materialize --recoger` cuesta HOY para no
                     hacer nada (pregunta el testigo al origen: no vale como
                     mantenimiento)
  §5  LA HISTORIA    `git log` del puntero por `GET /arbol/historia/…` y cada
                     versión por `GET /arbol/version/{hash}/…`: ¿es la
                     historia de la tabla?

Uso:  python pruebas-de-fuego/medida-w3-swap.py [--sin-bucket]
Necesita target/debug (ore, ore-serve, ore-store-r2, ore-read-jsonl), git y
python. No toca el clúster; §4 lista el bucket de demo con la sesión de gcloud.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace("\\", "/")
BIN = RAIZ + "/target/debug"
EXE = ".exe" if os.name == "nt" else ""
ORE = "%s/ore%s" % (BIN, EXE)
SERVE = "%s/ore-serve%s" % (BIN, EXE)
STORE = "%s/ore-store-r2%s" % (BIN, EXE)
PY = sys.executable
SIN_BUCKET = "--sin-bucket" in sys.argv


def fila(a, b="", c=""):
    print("  %-42s %-28s %s" % (a, b, c))


def ms(t):
    return int((time.time() - t) * 1000)


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                          env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                   GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x")).stdout.strip()


def pide(base, metodo, ruta, cuerpo=None, cabeceras=None):
    datos = cuerpo.encode("utf-8") if isinstance(cuerpo, str) else cuerpo
    r = urllib.request.Request(base + ruta, data=datos, method=metodo)
    r.add_header("x-ore-sujeto", "persona:ana")
    r.add_header("content-type", "application/json")
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            return resp.status, resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def puerto_libre():
    import socket
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def arbol_semilla(d):
    """El árbol de refresco.sh, más un `datasource: lago` y una Table encima."""
    os.makedirs(d + "/datos"); os.makedirs(d + "/packages/ventas/tables"); os.makedirs(d + "/packages/ventas/views")
    open(d + "/datos/pedidos.jsonl", "w").write("".join('{"order_id":"%d","pais":"ES","total":"%d.00","actualizado_en":"%010d"}\n' % (i, i, i) for i in range(1, 101)))
    open(d + "/ontology.config.yaml", "w").write("""apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: medida, version: 0.1.0 }
datasources:
  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR }
  - { name: lago, type: lago, connectionEnv: LAGO_URL }
""")
    open(d + "/lattice.yaml", "w").write("apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
    open(d + "/conduits.yaml", "w").write("apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: ventas }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      gdpr.sensitivity: low\n")
    open(d + "/packages/ventas/package.yaml", "w").write("apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 1.0.0, status: active, domain: sales }\nspec: { owner: team:data }\n")
    open(d + "/packages/ventas/tables/pedidos.yaml", "w").write("""apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: pedidos, namespace: ventas }
spec:
  datasource: ficheros
  object: "pedidos.jsonl"
  columns: { order_id: { type: Integer }, pais: {}, total: { type: Decimal }, actualizado_en: {} }
  reads: { fullScan: cheap }
  changes: { mode: upsert, key: [order_id], witness: field, field: actualizado_en, retention: 7d }
""")
    open(d + "/packages/ventas/views/copia.yaml", "w").write("""apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: copia, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.pedidos }
  fields: { id: order_id, pais: pais, total: total, cuando: actualizado_en }
  materialized: { datasource: lago, table: "cache.pedidos" }
""")
    # la Table DEL LAGO: lo que write() dejaría (W3.6c), escrita a mano hoy
    open(d + "/packages/ventas/tables/salida.yaml", "w").write("""apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: salida, namespace: ventas }
spec:
  datasource: lago
  object: "ventas_salida"
  columns: { id: { type: Integer }, pais: {}, total: { type: Decimal }, cuando: { type: DateTimeTz } }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
""")
    open(d + "/packages/ventas/views/porPais.yaml", "w").write("""apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: porPais, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.salida }
  fields: { pais: pais, total: total }
""")


def main():
    for b in (ORE, SERVE, STORE, "%s/ore-read-jsonl%s" % (BIN, EXE)):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve -p ore-store -p ore-read-jsonl"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-swap-").replace("\\", "/")
    procs = []
    try:
        # ── el S3 de mentira y la forja ─────────────────────────────────────
        s3 = subprocess.Popen([PY, RAIZ + "/pruebas-de-fuego/de-mentira.py", "s3", "0"], stdout=open(tmp + "/s3.log", "w"), stderr=subprocess.STDOUT)
        procs.append(s3)
        for _ in range(50):
            if os.path.exists(tmp + "/s3.log") and "listo" in open(tmp + "/s3.log").read():
                break
            time.sleep(0.2)
        s3p = open(tmp + "/s3.log").read().split()[1]
        env = dict(os.environ, ORE_STORE="r2", ORE_R2_S3_ENDPOINT="http://127.0.0.1:" + s3p, ORE_R2_BUCKET="copia",
                   ORE_R2_ACCESS_KEY_ID="de", ORE_R2_SECRET_ACCESS_KEY="mentira", PATH=BIN + os.pathsep + os.environ["PATH"],
                   FICHEROS_DIR=tmp + "/semilla/datos", LAGO_URL="s3://copia", FORJA_TOKEN="no-hace-falta")
        forja = tmp + "/arbol.git"
        git("init", "-q", "--bare", "-b", "main", forja)
        git("clone", "-q", forja, tmp + "/semilla")
        arbol_semilla(tmp + "/semilla")
        git("add", "-A", cwd=tmp + "/semilla"); git("commit", "-qm", "semilla", cwd=tmp + "/semilla"); git("push", "-q", "origin", "HEAD:main", cwd=tmp + "/semilla")
        puerto = puerto_libre()
        base = "http://127.0.0.1:%d" % puerto
        srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                                "--identidad", "cabecera", "--no-es-produccion"], env=env, stdout=open(tmp + "/serve.log", "w"), stderr=subprocess.STDOUT)
        procs.append(srv)
        for _ in range(60):
            try:
                if pide(base, "GET", "/salud")[0] == 200:
                    break
            except Exception:
                pass
            time.sleep(0.25)

        # ── §2 primero: ¿compila el lago? (lo necesita §1 para el If-Match) ──
        print()
        print("§2 · el lago como fuente: `datasource: lago`, una Table tipada encima, una View sobre ella")
        r = subprocess.run([ORE, "validate", tmp + "/semilla"], capture_output=True, text=True, encoding="utf-8", env=env)
        fila("ore validate con lago + Table + View", "código %d" % r.returncode, (r.stdout + r.stderr).strip().splitlines()[0][:80] if (r.stdout + r.stderr).strip() else "sin diagnósticos")
        r = subprocess.run([ORE, "view", tmp + "/semilla"], capture_output=True, text=True, encoding="utf-8", env=env)
        for l in (r.stdout + r.stderr).splitlines():
            if "porPais" in l or "salida" in l:
                fila("  ore view", "", l.strip()[:90])
        # ¿y materializar una vista que lee del lago? (una View materializada sobre salida)
        os.makedirs(tmp + "/lago2", exist_ok=True)
        shutil.copytree(tmp + "/semilla", tmp + "/lago2/arbol", ignore=shutil.ignore_patterns(".git"))
        open(tmp + "/lago2/arbol/packages/ventas/views/porPais.yaml", "a").write('  materialized: { datasource: lago, table: "cache.porPais" }\n')
        r = subprocess.run([ORE, "materialize", tmp + "/lago2/arbol", "--vista", "ventas.porPais", "--seco"], capture_output=True, text=True, encoding="utf-8", env=env)
        fila("materialize --seco de una View sobre el lago", "código %d" % r.returncode, " ".join((r.stdout + r.stderr).split())[:110])
        # el resolutor del puesto (`datos_de`, puestos.rs): ¿resuelve una Table del lago?
        # Sin clúster no hay puesto que reclamar; se mira lo que `ore ask` decide
        # de una View sobre el lago, y el código del resolutor.
        r = subprocess.run([ORE, "ask", tmp + "/semilla", "--vista", "ventas.porPais", "--seco"], capture_output=True, text=True, encoding="utf-8", env=env)
        fila("ore ask --seco ventas.porPais (View sobre el lago)", "código %d" % r.returncode, " ".join((r.stdout + r.stderr).split())[:110])
        src = open(RAIZ + "/crates/ore-serve/src/puestos.rs", encoding="utf-8").read()
        fila("datos_de resuelve", "Views (packages/<ns>/views)", "Table del lago → 404 «no hay ninguna View»" if 'join("views")' in src else "?")

        # ── §1 · la carrera ─────────────────────────────────────────────────
        print()
        print("§1 · la carrera: 8 hilos escriben el mismo puntero con el mismo If-Match")
        c, t = pide(base, "GET", "/arbol/ontology.config.yaml")
        commit0 = json.loads(t).get("cabeza") or json.loads(t).get("commit")
        fila("el commit del que parten todos", commit0 or "?")
        t0 = time.time()
        c, t = pide(base, "PUT", "/arbol/datasets/ventas_salida.json", json.dumps({"estado": "copiada", "metadata_location": "s3://copia/ore/v2/datasets/ventas_salida/metadata/00000-a.metadata.json", "snapshot": "1"}), {"if-match": commit0})
        fila("un PUT solo (clonar+compilar+commit+push)", "HTTP %d · %d ms" % (c, ms(t0)), t[:80])
        commit1 = json.loads(t).get("commit") if c in (200, 201) else commit0
        resultados = []
        lock = threading.Lock()

        def escribe(i):
            t1 = time.time()
            c, t = pide(base, "PUT", "/arbol/datasets/ventas_salida.json",
                        json.dumps({"estado": "copiada", "metadata_location": "s3://copia/ore/v2/datasets/ventas_salida/metadata/00001-%d.metadata.json" % i, "snapshot": str(i)}),
                        {"if-match": commit1})
            with lock:
                resultados.append((i, c, ms(t1), t[:70]))
        hilos = [threading.Thread(target=escribe, args=(i,)) for i in range(8)]
        t0 = time.time()
        for h in hilos: h.start()
        for h in hilos: h.join()
        total = ms(t0)
        ganan = [r for r in resultados if r[1] in (200, 201)]
        pierden = [r for r in resultados if r[1] == 409]
        otros = [r for r in resultados if r[1] not in (200, 201, 409)]
        fila("ganan / 409 / otros", "%d / %d / %d" % (len(ganan), len(pierden), len(otros)), "en %d ms" % total)
        for r in sorted(resultados, key=lambda r: r[1]):
            fila("  hilo %d" % r[0], "HTTP %d · %d ms" % (r[1], r[2]), r[3])
        # lo que quedó en la forja: UN commit más, y el puntero del que ganó
        git("clone", "-q", forja, tmp + "/mira")
        p = json.load(open(tmp + "/mira/datasets/ventas_salida.json"))
        n = int(git("rev-list", "--count", "main", cwd=tmp + "/mira"))
        fila("en la forja", "%d commits" % n, "puntero → snapshot %s (ganó el hilo %s)" % (p["snapshot"], ganan[0][0] if len(ganan) == 1 else "?"))
        # y los que perdieron, reintentando con el commit nuevo, se serializan
        t0 = time.time()
        rondas = 0
        pendientes = [r[0] for r in pierden]
        while pendientes and rondas < 20:
            rondas += 1
            c, t = pide(base, "GET", "/arbol/ontology.config.yaml")
            actual = json.loads(t).get("cabeza") or json.loads(t)["commit"]
            res = []

            def reintenta(i):
                c, t = pide(base, "PUT", "/arbol/datasets/ventas_salida.json", json.dumps({"estado": "copiada", "metadata_location": "s3://copia/ore/v2/datasets/ventas_salida/metadata/00002-%d.metadata.json" % i, "snapshot": "r%d" % i}), {"if-match": actual})
                with lock:
                    res.append((i, c))
            hs = [threading.Thread(target=reintenta, args=(i,)) for i in pendientes]
            for h in hs: h.start()
            for h in hs: h.join()
            pendientes = [i for i, c in res if c == 409]
        fila("los 7 que perdieron, reintentando", "%d rondas · %d ms" % (rondas, ms(t0)), "cada ronda gana exactamente uno" if rondas == 7 else "⚠ %d rondas para 7" % rondas)

        # ── §5 · la historia del puntero ────────────────────────────────────
        print()
        print("§5 · la historia del puntero es la historia de la tabla")
        c, t = pide(base, "GET", "/arbol/historia/datasets/ventas_salida.json")
        hist = json.loads(t) if c == 200 else {}
        commits = hist.get("versiones") or []
        fila("GET /arbol/historia/datasets/ventas_salida.json", "HTTP %d · %d versiones" % (c, len(commits)), str(list(hist.keys()))[:60])
        for h in commits[:3]:
            hsh = h.get("commit") or h.get("hash") or ""
            c2, t2 = pide(base, "GET", "/arbol/version/%s/datasets/ventas_salida.json" % hsh)
            snap = ""
            try:
                snap = json.loads(json.loads(t2).get("texto", "{}")).get("snapshot", "")
            except Exception:
                snap = t2[:40]
            fila("  %s · %s" % (hsh[:8], (h.get("autor") or h.get("quien") or "")[:16]), "HTTP %d" % c2, "snapshot %s" % snap)

        # ── §3 · los metadatos crecen ───────────────────────────────────────
        print()
        print("§3 · 300 snapshots sobre una tabla: metadata.json, apertura y lo que queda tras recoger")
        cab = '{"clave":["id"],"conducto":"c","esquema":{"id":"Integer","v":"String"},"plan":"sha256:p","testigo":{"modo":"log","valor":"%d"}}'
        ml = None
        tam = []
        t_sellar = []
        for i in range(300):
            extra = '"dataset":"datasets/medida","fundir":false,' + ('"base":"%s",' % ml if ml else "")
            entrada = "{" + extra + cab[1:] % i + "\n" + '{"id":"%d","v":"x"}\n' % i
            t1 = time.time()
            r = subprocess.run([STORE, "sellar"], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
            t_sellar.append(ms(t1))
            if r.returncode != 0:
                fila("sellar %d falló" % i, "", r.stderr.strip()[:80]); break
            ml = json.loads(r.stdout)["metadata_location"]
            if i in (0, 9, 49, 99, 199, 299):
                k = ml[len("s3://copia/"):]
                with urllib.request.urlopen("http://127.0.0.1:%s/copia/%s" % (s3p, k)) as resp:
                    b = resp.read()
                m = json.loads(b)
                tam.append((i + 1, len(b), len(m.get("snapshots", [])), len(m.get("metadata-log", []))))
        for n, b, s, l in tam:
            fila("  tras %d snapshots" % n, "%d KB" % (b // 1024), "%d snapshots en el fichero · metadata-log %d" % (s, l))
        fila("sellar (1 fila): primero / mediana / último", "%d / %d / %d ms" % (t_sellar[0], sorted(t_sellar)[len(t_sellar) // 2], t_sellar[-1]))
        t1 = time.time()
        r = subprocess.run([STORE, "leer"], input='{"metadata_location":"%s"}\n' % ml, capture_output=True, text=True, encoding="utf-8", env=env)
        fila("leer con 300 snapshots (abre + 1 fichero)", "%d ms" % ms(t1), "%d filas" % (len(r.stdout.splitlines()) - 1))
        with urllib.request.urlopen("http://127.0.0.1:%s/copia?list-type=2&prefix=ore/v2/datasets/medida/" % s3p) as resp:
            objetos = resp.read().decode().count("<Key>")
        fila("objetos en el bucket", str(objetos))
        t1 = time.time()
        r = subprocess.run([STORE, "recoger"], input='{"dataset":"datasets/medida","metadata_location":"%s"}\n' % ml, capture_output=True, text=True, encoding="utf-8", env=env)
        g = json.loads(r.stdout)
        ml2 = g["metadata_location"]
        with urllib.request.urlopen("http://127.0.0.1:%s/copia/%s" % (s3p, ml2[len("s3://copia/"):])) as resp:
            b = resp.read()
        m = json.loads(b)
        with urllib.request.urlopen("http://127.0.0.1:%s/copia?list-type=2&prefix=ore/v2/datasets/medida/" % s3p) as resp:
            objetos2 = resp.read().decode().count("<Key>")
        fila("recoger (todo lo superado)", "%d ms" % ms(t1), "%d expirados · %d ficheros fuera · quedan %d objetos" % (g["expirados"], g["ficheros"], objetos2))
        fila("  metadata.json tras recoger", "%d KB" % (len(b) // 1024), "%d snapshots · metadata-log %d" % (len(m.get("snapshots", [])), len(m.get("metadata-log", []))))
        t1 = time.time()
        subprocess.run([STORE, "leer"], input='{"metadata_location":"%s"}\n' % ml2, capture_output=True, text=True, encoding="utf-8", env=env)
        fila("leer tras recoger", "%d ms" % ms(t1))

        # ── §4 · el barrido ─────────────────────────────────────────────────
        print()
        print("§4 · lo que un mantenimiento tendría que recorrer, y lo que `materialize --recoger` cuesta hoy sin hacer nada")
        d = tmp + "/semilla"
        t1 = time.time()
        r = subprocess.run([ORE, "materialize", d, "--vista", "ventas.copia", "--informe", d + "/copias"], capture_output=True, text=True, encoding="utf-8", env=env)
        fila("materialize (primera): 100 filas", "%d ms" % ms(t1), r.stdout.strip().splitlines()[-1].strip()[:70] if r.stdout.strip() else r.stderr[:70])
        t1 = time.time()
        r = subprocess.run([ORE, "materialize", d, "--vista", "ventas.copia", "--informe", d + "/copias", "--recoger"], capture_output=True, text=True, encoding="utf-8", env=env)
        fila("materialize --recoger al día", "%d ms" % ms(t1), "compila + testigo al ORIGEN + buscar + recoger + huérfanas")
        # cuántas llamadas al origen hizo: el driver de jsonl deja rastro en el fichero? no; se cuenta el tiempo del testigo
        t1 = time.time()
        r = subprocess.run(["%s/ore-read-jsonl%s" % (BIN, EXE), "testigo"], input='{"objeto":"pedidos.jsonl","url":"%s/semilla/datos","cursor":"actualizado_en"}\n' % tmp, capture_output=True, text=True, encoding="utf-8", env=env)
        fila("  de eso, el testigo al origen", "%d ms" % ms(t1), r.stdout.strip()[:60])
        if not SIN_BUCKET:
            gcloud = shutil.which("gcloud.cmd") or shutil.which("gcloud")
            if gcloud:
                t1 = time.time()
                r = subprocess.run([gcloud, "storage", "ls", "gs://project-8853a180-450d-47be-b83-t-demo-copia/**"], capture_output=True, text=True, encoding="utf-8")
                objs = [l for l in r.stdout.splitlines() if l.strip()]
                v1 = [o for o in objs if "/ore/v1/" in o]
                v2 = [o for o in objs if "/ore/v2/" in o]
                fila("el bucket de demo, listado", "%d objetos · %d ms" % (len(objs), ms(t1)), "%d de ore/v1 (sobres y recibos) · %d de ore/v2 · %d otros" % (len(v1), len(v2), len(objs) - len(v1) - len(v2)))
            else:
                fila("gcloud no está", "", "§4 sin el bucket")

        print()
        print("### resumen")
        print(json.dumps({"carrera": {"ganan": len(ganan), "pierden": len(pierden), "otros": len(otros), "ms": total, "rondas_reintento": rondas},
                          "metadatos_300": tam, "recoger": {"expirados": g["expirados"], "ficheros": g["ficheros"], "quedan": objetos2}}))
    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
