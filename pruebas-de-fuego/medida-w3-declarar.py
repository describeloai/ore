#!/usr/bin/env python3
"""
MEDIDA · W3.7, declarar y correr (21 de septiembre), antes de abordarlo.

0031 §9 dejó cuatro verbos —leer, escribir, declarar, correr— y los dos
primeros están cerrados. El tercero es el puente entre lo que una sesión de
código produce y el registro de assets del inquilino (el árbol; el Assets
Catalog lo lee): un dataset, una vista sobre él, una entidad, una interfaz,
una función, un modelo. Aquí se mide, con lo que ya existe, qué parte del
puente está y qué no, para decidir con números:

  §1  EL PUENTE HOY   desde un puesto de verdad (`agente.py` local), qué puede
                      dejar una celda en el árbol y por dónde: `write()` (el
                      dataset: quién firma, ms), `PUT /documentos/{kind}` (la
                      puerta de Forge, con el testigo del agente y
                      `x-ore-puesto`: ¿escribe en nombre de la persona? ¿en su
                      rama? ¿cuánto tarda clonar + compilar + empujar?) para
                      View, Entity, Interface y Concept; y lo que NO tiene
                      puerta: Function y Model (qué contesta, qué dice OOS de
                      un `runtime: python` y de un `kind: Model`)
  §2  LA RAMA         un puesto abierto en una rama: `write()` deja el commit
                      en la rama y no en `main`; `over()` de lo que sólo está
                      en `main` (el fallback de §4); y qué hace falta para
                      publicar (fusionar) desde aquí
  §3  EL LINAJE       lo que un dataset escrito sabe de sí (el puntero, la
                      ficha, `ore view` de una View encima): ¿de qué salió?
                      ¿con qué conducto? Y lo que una `Function` de OOS ya
                      valida (reads/over/effects) como contrato de
                      `transform(inputs, output)`
  §4  CORRER (local)  el coste de un transform en el puesto: N filas escritas,
                      `sql()` que agrega y `write()` del resumen; y la copia
                      entera `write(over())` con la memoria del agente
  §5  CORRER (--cluster) el frío de un trabajo de código en `t-demo` con
                      `jobs-p` 0 → 1 → 0: nodo, imagen, testigo, y el guion
                      (over + sql + write) — lo que un `ore run` costaría hoy

Uso:  python pruebas-de-fuego/medida-w3-declarar.py [--cluster] [--sha <12 hex>] [--filas N] [--solo 1,2,3,4,5]
Necesita target/debug (ore, ore-serve, ore-store-r2), git, pyarrow, duckdb.
§5 necesita kubectl (ore-mesh) y gcloud; deja el nodo a 0 y retira lo suyo.
No imprime ningún token.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
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
REL = RAIZ + "/target/release"
STORE_DIR = REL if os.path.exists("%s/ore-store-r2%s" % (REL, EXE)) else BIN
PY = sys.executable
CLUSTER = "--cluster" in sys.argv
FILAS = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 1_000_000
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "4"} | ({"5"} if CLUSTER else set())


def fila(a, b="", c=""):
    print("  %-50s %-24s %s" % (a, b, c))


def ms(t):
    return int((time.time() - t) * 1000)


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                       env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x"))
    return (r.stdout + r.stderr).strip()


def pide(base, metodo, ruta, cuerpo=None, cabeceras=None, sujeto="persona:ana"):
    datos = cuerpo.encode("utf-8") if isinstance(cuerpo, str) else (json.dumps(cuerpo).encode("utf-8") if cuerpo is not None else None)
    r = urllib.request.Request(base + ruta, data=datos, method=metodo)
    r.add_header("x-ore-sujeto", sujeto)
    r.add_header("content-type", "application/json")
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=600) as resp:
            t = resp.read().decode("utf-8")
            return resp.status, (json.loads(t) if t.strip().startswith(("{", "[")) else t)
    except urllib.error.HTTPError as e:
        t = e.read().decode("utf-8", "replace")
        try:
            return e.code, json.loads(t)
        except ValueError:
            return e.code, {"error": t.strip()[:300]}


def puerto_libre():
    import socket
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def corre(args, env, cwd=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", env=env, cwd=cwd)
    return r.returncode, " ".join((r.stdout + "\n" + r.stderr).split())


def arbol_semilla(d):
    os.makedirs(d + "/datos"); os.makedirs(d + "/packages/ventas/tables"); os.makedirs(d + "/packages/ventas/views")
    open(d + "/datos/pedidos.jsonl", "w").write("".join('{"order_id":"%d","pais":"ES","total":"%d.00"}\n' % (i, i) for i in range(1, 11)))
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
  columns: { order_id: { type: Integer }, pais: {}, total: { type: Decimal } }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
""")
    open(d + "/packages/ventas/views/pedidos.yaml", "w").write("""apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: pedidos, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.pedidos }
  fields: { id: order_id, pais: pais, total: total }
""")


# ── lo que las celdas ejecutan ───────────────────────────────────────────────
TRES = 'import pyarrow as pa, decimal, datetime as dt; utc = dt.timezone.utc; t3 = pa.table({"n": pa.array([1, 2, 3], pa.int64()), "pais": pa.array(["ES", "PT", None]), "total": pa.array([decimal.Decimal("1.50"), decimal.Decimal("2.25"), None], pa.decimal128(10, 2))})'
GRANDE = 'import pyarrow as pa, pyarrow.compute as pc; N = %d; ids = pa.array(range(N), pa.int64()); tg = pa.table({"id": ids, "pais": pc.choose(pc.cast(pc.bit_wise_and(ids, 3), pa.int32()), "ES", "PT", "FR", "IT"), "total": pc.cast(pc.divide(pc.cast(ids, pa.float64()), 100.0), pa.decimal128(18, 2))})'


def main():
    for b in (ORE, SERVE, STORE):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve -p ore-store"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-declarar-").replace("\\", "/")
    procs = []
    try:
        if SOLO & {"1", "2", "3", "4"}:
            local(tmp, procs)
        if "5" in SOLO:
            cluster()
    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        time.sleep(0.5)
        shutil.rmtree(tmp, ignore_errors=True)


def local(tmp, procs):
    s3 = subprocess.Popen([PY, RAIZ + "/pruebas-de-fuego/de-mentira.py", "s3", "0"], stdout=open(tmp + "/s3.log", "w"), stderr=subprocess.STDOUT)
    procs.append(s3)
    for _ in range(50):
        if os.path.exists(tmp + "/s3.log") and "listo" in open(tmp + "/s3.log").read():
            break
        time.sleep(0.2)
    s3url = "http://127.0.0.1:" + open(tmp + "/s3.log").read().split()[1]
    env = dict(os.environ, ORE_STORE="r2", ORE_R2_S3_ENDPOINT=s3url, ORE_R2_BUCKET="copia",
               ORE_R2_ACCESS_KEY_ID="de", ORE_R2_SECRET_ACCESS_KEY="mentira", PATH=BIN + os.pathsep + os.environ["PATH"],
               ORE_STORE_DIR=STORE_DIR, FICHEROS_DIR=tmp + "/semilla/datos", LAGO_URL="s3://copia", FORJA_TOKEN="no-hace-falta", ORE_RETENCION="7d")
    forja = tmp + "/arbol.git"
    git("init", "-q", "--bare", "-b", "main", forja)
    git("clone", "-q", forja, tmp + "/semilla")
    arbol_semilla(tmp + "/semilla")
    git("add", "-A", cwd=tmp + "/semilla"); git("commit", "-qm", "semilla", cwd=tmp + "/semilla"); git("push", "-q", "origin", "HEAD:main", cwd=tmp + "/semilla")
    # la cola, con la plantilla del puesto (lo que POST /puestos rinde)
    cola = tmp + "/cola.git"
    git("init", "-q", "--bare", "-b", "main", cola)
    git("clone", "-q", cola, tmp + "/cola-semilla")
    subprocess.run([PY, RAIZ + "/malla/gen-inquilino.py", "demo", "--a", tmp + "/rendido"], capture_output=True)
    for f in ("plantilla-puesto.txt", "plantilla-capa.txt"):
        shutil.copy(tmp + "/rendido/" + f, tmp + "/cola-semilla/" + f)
    git("add", "-A", cwd=tmp + "/cola-semilla"); git("commit", "-qm", "plantilla", cwd=tmp + "/cola-semilla"); git("push", "-q", "origin", "HEAD:main", cwd=tmp + "/cola-semilla")
    puerto = puerto_libre()
    base = "http://127.0.0.1:%d" % puerto
    srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--cola", "file://" + cola, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                            "--identidad", "cabecera", "--no-es-produccion", "--organizacion", "demo"], env=env, stdout=open(tmp + "/serve.log", "w"), stderr=subprocess.STDOUT)
    procs.append(srv)
    for _ in range(80):
        try:
            if pide(base, "GET", "/salud")[0] == 200:
                break
        except Exception:
            pass
        time.sleep(0.25)

    def abre_puesto(persona, sujeto_agente, rama=None):
        c, r = pide(base, "POST", "/puestos", {"rama": rama} if rama else {}, sujeto=persona)
        assert c in (200, 201), (c, r)
        pid = r["id"]
        ag = subprocess.Popen([PY, RAIZ + "/puesto/python/agente.py"], env=dict(env, ORE_SERVE=base, PUESTO=pid, ORE_SUJETO=sujeto_agente, ORE_ALMACEN="dir:" + tmp + "/almacen", TTL="900"),
                              stdout=open(tmp + "/agente-%s.log" % pid, "w"), stderr=subprocess.STDOUT)
        procs.append(ag)
        for _ in range(60):
            c, r = pide(base, "GET", "/puestos/" + pid, sujeto=persona)
            if r.get("estado") == "vivo":
                break
            time.sleep(0.25)
        assert r.get("estado") == "vivo", r
        return pid

    def celda(pid, texto, persona="persona:ana", plazo=600):
        t0 = time.time()
        c, r = pide(base, "POST", "/puestos/%s/ejecutar" % pid, {"texto": texto, "lenguaje": "python"}, sujeto=persona)
        assert c == 202, (c, r)
        n = r["celda"]
        while time.time() - t0 < plazo:
            c, r = pide(base, "GET", "/puestos/%s/celdas/%s" % (pid, n), sujeto=persona)
            if r.get("estado") == "hecha":
                s = r["salida"]; s["_ms"] = ms(t0)
                return s
            time.sleep(0.2)
        return {"tipo": "plazo", "_ms": ms(t0)}

    def texto(s):
        return (s.get("texto") or s.get("mensaje") or "").strip()

    def autor(ruta):
        c, r = pide(base, "GET", "/arbol/historia/" + ruta)
        v = (r or {}).get("versiones") or []
        return (v[0].get("autor", "?") if v else "sin historia (%s)" % c), len(v)

    ana = abre_puesto("persona:ana", "agente:local")
    # el dataset que todo lo demás usa: lo escribe ana desde su puesto
    s_salida = celda(ana, TRES + '; e = write("ventas.salida", t3); {k: e[k] for k in ("filas", "operacion", "repetida")}')

    # ── §1 · el puente hoy, desde un puesto ─────────────────────────────────
    if "1" in SOLO:
        print("§1 · el puente hoy: lo que una celda deja en el árbol, y por dónde")
        s = s_salida
        fila("write() → dataset ventas.salida", "%d ms" % s["_ms"], texto(s)[:80])
        a, n = autor("datasets/ventas_salida.json")
        fila("  el commit del puntero", "firma %s" % a, "%d versión(es) · Table en packages/ventas/tables" % n)
        c, r = pide(base, "GET", "/datasets/ventas/salida")
        fila("  la ficha", "escrito_por %s" % r.get("escrito_por"), "claves: " + ", ".join(sorted(k for k in r if k not in ("esquema", "snapshots")))[:90])

        def declara(kind, ns, n, doc, quien="celda"):
            cuerpo = {"yaml": doc}
            if quien == "celda":
                s = celda(ana, 'import json, ore; c, r = ore.puesto.pedir("PUT", "/documentos/%s/%s/%s", json.loads(%r)); print(json.dumps([c, r])[:700])' % (kind, ns, n, json.dumps(cuerpo)))
                try:
                    c, r = json.loads(texto(s)); return c, r, s["_ms"]
                except Exception:
                    return "?", texto(s)[:200], s["_ms"]
            t0 = time.time()
            c, r = pide(base, "PUT", "/documentos/%s/%s/%s" % (kind, ns, n), cuerpo, sujeto=quien)
            return c, r, ms(t0)

        VISTA = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porPais, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { pais: pais, n: \"count()\", total: \"sum(total)\" }\n  groupBy: [pais]\n"
        for kind, ns, n, doc in (
            ("View", "ventas", "porPais", VISTA),
            ("View", "ventas", "ventas", "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: ventas, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { n: n, pais: pais, total: total }\n"),
            ("Concept", "ventas", "pais", "apiVersion: oos.dev/v1alpha8\nkind: Concept\nmetadata: { name: pais, namespace: ventas }\nspec:\n  description: el país de la venta\n  type: String\n"),
            ("Entity", "ventas", "venta", "apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: venta, namespace: ventas }\nspec:\n  nature: entity\n  backedBy: ventas.ventas\n  primaryKey: [n]\n  properties:\n    n: { type: Integer }\n    pais: { is: ventas.pais }\n"),
            ("Interface", "ventas", "conPais", "apiVersion: oos.dev/v1alpha8\nkind: Interface\nmetadata: { name: conPais, namespace: ventas }\nspec:\n  requires: [ventas.pais]\n"),
            ("Function", "ventas", "resumir", "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: resumir, namespace: ventas }\nspec:\n  runtime: wasm\n  reads: [ventas.porPais]\n"),
            ("Model", "ventas", "prevision", "apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata: { name: prevision }\nspec:\n  profile: gpu/prevision\n  tier: shared\n  task: chat\n"),
        ):
            c, r, t = declara(kind, ns, n, doc)
            det = r if isinstance(r, str) else (r.get("error") or (r.get("diagnosticos") or [{}])[0].get("mensaje") or ", ".join("%s=%s" % (k, str(v)[:20]) for k, v in sorted(r.items())))
            firma = ""
            if c in (200, 201):
                firma = " · firma " + autor("packages/%s/%s/%s.yaml" % (ns, {"View": "views", "Entity": "entities", "Interface": "interfaces", "Concept": "concepts"}[kind], n))[0]
            fila("PUT /documentos/%s desde la celda" % kind, "%s · %d ms" % (c, t), (str(det)[:110]) + firma)
        # la misma vista, con el testigo de la persona (lo que la consola hace): para comparar la firma y el coste
        c, r, t = declara("View", "ventas", "porPais2", VISTA.replace("porPais", "porPais2"), quien="persona:ana")
        fila("PUT /documentos/View como persona:ana", "%s · %d ms" % (c, t), ("firma " + autor("packages/ventas/views/porPais2.yaml")[0]) if c in (200, 201) else str(r)[:120])
        # y lo que OOS dice de una Function que no es wasm, y de un Model
        m = tmp + "/mira1"; git("clone", "-q", forja, m)
        os.makedirs(m + "/packages/ventas/functions", exist_ok=True)
        open(m + "/packages/ventas/functions/resumirPy.yaml", "w").write("apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: resumirPy, namespace: ventas }\nspec:\n  runtime: python\n  entrypoint: transforms/resumir.py\n  reads: [ventas.pedidos]\n")
        c, s = corre([ORE, "validate", m], env)
        fila("ore validate · Function runtime: python", "código %d" % c, s[:150])
        os.remove(m + "/packages/ventas/functions/resumirPy.yaml")
        os.makedirs(m + "/packages/ventas/models", exist_ok=True)
        open(m + "/packages/ventas/models/prevision.yaml", "w").write("apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata: { name: prevision }\nspec:\n  profile: gpu/prevision\n  tier: shared\n  task: chat\n")
        c, s = corre([ORE, "validate", m], env)
        fila("ore validate · kind: Model (v1alpha9: el LLM servido)", "código %d" % c, s[:150])
        open(m + "/packages/ventas/models/prevision.yaml", "w").write("apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata: { name: prevision }\nspec:\n  base: sklearn\n  version: 3\n  artefactos: modelos/prevision/v3/\n")
        c, s = corre([ORE, "validate", m], env)
        fila("ore validate · kind: Model como 0031 §6 (pesos)", "código %d" % c, s[:150])
        os.remove(m + "/packages/ventas/models/prevision.yaml")
        print()

    # ── §2 · la rama ─────────────────────────────────────────────────────────
    if "2" in SOLO:
        print("§2 · la rama: un puesto abierto en una rama, y lo que escribe")
        # la rama, por git (POST /ramas necesita la API de la forja: se mide abajo)
        r2 = tmp + "/rama"; git("clone", "-q", forja, r2); git("checkout", "-q", "-b", "bea/w37", cwd=r2); git("push", "-q", "origin", "bea/w37", cwd=r2)
        c, r = pide(base, "POST", "/ramas", {"nombre": "w37"}, sujeto="persona:bea")
        fila("POST /ramas {nombre} sin API de forja", "código %s" % c, str(r.get("error") if isinstance(r, dict) else r)[:110])
        bea = abre_puesto("persona:bea", "agente:local", rama="bea/w37")
        c, r = pide(base, "GET", "/puestos/" + bea, sujeto="persona:bea")
        fila("el puesto de bea", r.get("estado"), "rama %s" % r.get("rama"))
        s = celda(bea, TRES + '; e = write("ventas.enRama", t3); e["filas"]', persona="persona:bea")
        fila("write() desde la rama", "%d ms" % s["_ms"], texto(s)[:80])
        git("fetch", "-q", "origin", cwd=r2)
        rama_tiene = git("cat-file", "-e", "origin/bea/w37:datasets/ventas_enRama.json", cwd=r2) == ""
        main_tiene = git("cat-file", "-e", "origin/main:datasets/ventas_enRama.json", cwd=r2) == ""
        fila("  el puntero datasets/ventas_enRama.json", "en la rama: %s" % ("sí" if rama_tiene else "no"), "en main: %s" % ("sí" if main_tiene else "no"))
        a, n = autor("datasets/ventas_enRama.json")
        fila("  GET /arbol/historia (main)", a, "%d versiones" % n)
        # declarar desde la rama (W3.7 ①): la View va a la rama, firmada por bea
        s = celda(bea, 'd = declare("apiVersion: oos.dev/v1alpha8\\nkind: View\\nmetadata: { name: enRama, namespace: ventas }\\nspec:\\n  owner: team:ventas\\n  from: { table: ventas.enRama }\\n  fields: { n: n, pais: pais }\\n"); d["commit"]', persona="persona:bea")
        fila("declare(View) desde la rama", "%d ms" % s["_ms"], texto(s)[:80])
        git("fetch", "-q", "origin", cwd=r2)
        rama_tiene = git("cat-file", "-e", "origin/bea/w37:packages/ventas/views/enRama.yaml", cwd=r2) == ""
        main_tiene = git("cat-file", "-e", "origin/main:packages/ventas/views/enRama.yaml", cwd=r2) == ""
        firma = git("log", "-1", "--format=%an", "origin/bea/w37", cwd=r2)
        fila("  packages/ventas/views/enRama.yaml", "en la rama: %s" % ("sí" if rama_tiene else "no"), "en main: %s · firma %s" % ("sí" if main_tiene else "no", firma))
        # lo que sólo está en main, desde la rama: el fallback de §4
        s = celda(ana, TRES + '; write("ventas.despues", t3)["filas"]')
        fila("write() de ana en main, tras abrir la rama", "%d ms" % s["_ms"], texto(s)[:80])
        s = celda(bea, 'over("ventas.despues", como="arrow").num_rows', persona="persona:bea")
        fila("over() desde la rama de lo escrito en main después", s.get("tipo"), texto(s)[:110])
        s = celda(bea, 'over("ventas.salida", como="arrow").num_rows', persona="persona:bea")
        fila("over() desde la rama de lo escrito en main antes", s.get("tipo"), texto(s)[:110])
        s = celda(ana, 'over("ventas.enRama", como="arrow").num_rows')
        fila("over() desde main de lo escrito en la rama", s.get("tipo"), texto(s)[:110])
        c, r = pide(base, "POST", "/ramas/bea/w37/fusionar", {}, sujeto="persona:bea")
        fila("POST /ramas/bea/w37/fusionar sin API de forja", "código %s" % c, str(r.get("error") if isinstance(r, dict) else r)[:110])
        print()

    # ── §3 · el linaje y el gobierno de lo escrito ───────────────────────────
    if "3" in SOLO:
        print("§3 · el linaje: lo que un dataset escrito sabe de sí, y lo que una Function de OOS ya exige")
        m = tmp + "/mira3"; git("clone", "-q", forja, m)
        p = json.load(open(m + "/datasets/ventas_salida.json"))
        fila("el puntero de ventas.salida", "%d claves" % len(p), ", ".join(sorted(p)))
        c, r = pide(base, "GET", "/datasets/ventas/salida")
        fila("la ficha", "", "de qué salió: %s · conducto: %s" % (json.dumps(r.get("procedencia")) if r.get("procedencia") else "no lo dice", r.get("conducto", "no lo dice")))
        r = subprocess.run([STORE, "metadatos"], input=json.dumps({"metadata_location": p["metadata_location"]}) + "\n", capture_output=True, text=True, encoding="utf-8", env=env)
        props = json.loads(r.stdout).get("properties", {}) if r.returncode == 0 else {}
        fila("las propiedades de la tabla Iceberg", "%d" % len(props), ", ".join(sorted(props))[:100])
        if not os.path.exists(m + "/packages/ventas/views/porPais.yaml"):
            open(m + "/packages/ventas/views/porPais.yaml", "w").write("apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porPais, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { pais: pais, n: \"count()\", total: \"sum(total)\" }\n  groupBy: [pais]\n")
        if not os.path.exists(m + "/packages/ventas/views/ventas.yaml"):
            open(m + "/packages/ventas/views/ventas.yaml", "w").write("apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: ventas, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { n: n, pais: pais, total: total }\n")
            os.makedirs(m + "/packages/ventas/entities", exist_ok=True)
            open(m + "/packages/ventas/entities/venta.yaml", "w").write("apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: venta, namespace: ventas }\nspec:\n  nature: entity\n  backedBy: ventas.ventas\n  primaryKey: [n]\n  properties:\n    n: { type: Integer }\n    pais: { type: String }\n")
        c, s = corre([ORE, "view", m], env)
        i = s.find("ventas.porPais")
        fila("ore view · ventas.porPais (el linaje)", "código %d" % c, s[i:i + 220] if i >= 0 else s[:200])
        # una Function de OOS con reads/over/effects: lo que el compilador ya exige, como contrato de transform()
        os.makedirs(m + "/packages/ventas/functions", exist_ok=True)
        for nombre, spec in (
            ("lee", "  runtime: wasm\n  entrypoint: dist/lee.wasm\n  reads: [ventas.porPais]\n"),
            ("leeTabla", "  runtime: wasm\n  entrypoint: dist/lee.wasm\n  reads: [ventas.salida]\n"),
            ("escribe", "  runtime: wasm\n  entrypoint: dist/e.wasm\n  reads: [ventas.pedidos]\n  effects: [{ writes: ventas.venta.pais }]\n"),
            ("escribeSobre", "  runtime: wasm\n  entrypoint: dist/e.wasm\n  over: ventas.ventas\n  effects: [{ writes: ventas.venta.pais }]\n"),
        ):
            open(m + "/packages/ventas/functions/%s.yaml" % nombre, "w").write("apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: %s, namespace: ventas }\nspec:\n%s" % (nombre, spec))
            c, s = corre([ORE, "validate", m], env)
            fila("Function %s" % nombre, "código %d" % c, (s[:150] if c else "compila"))
            os.remove(m + "/packages/ventas/functions/%s.yaml" % nombre)
        print()

    # ── §4 · correr, en local: el coste de un transform en el puesto ─────────
    if "4" in SOLO:
        print("§4 · correr en local: N = {:,} filas, en el puesto de ana (ore-store de {})".format(FILAS, "release" if STORE_DIR == REL else "debug").replace(",", " "))
        s = celda(ana, GRANDE % FILAS + '; tg.num_rows')
        fila("la tabla en Arrow, en la celda", "%d ms" % s["_ms"], texto(s))
        s = celda(ana, 'e = write("ventas.grande", tg); e["filas"]')
        fila("write() de N filas (IPC → ore-store escribir → commit)", "%d ms" % s["_ms"], texto(s)[:80])
        s = celda(ana, 'r = sql("select pais, count(*) as n, sum(total) as total from ventas.grande group by pais order by pais", como="arrow"); r.num_rows')
        fila("sql() que agrega sobre ventas.grande", "%d ms" % s["_ms"], texto(s)[:80])
        s = celda(ana, 'e = write("ventas.resumen", r); e["filas"]')
        fila("write() del resumen (4 filas)", "%d ms" % s["_ms"], texto(s)[:80])
        s = celda(ana, 'import os; (__import__("resource").getrusage(__import__("resource").RUSAGE_SELF).ru_maxrss // 1024) if os.name != "nt" else "sin resource en Windows"')
        fila("memoria pico del agente hasta aquí", "", texto(s))
        s = celda(ana, 'e = write("ventas.copia", over("ventas.grande", como="arrow")); e["filas"]')
        fila("write(over()) de N filas (la copia entera)", "%d ms" % s["_ms"], texto(s)[:80])
        try:
            import psutil
            for p in psutil.process_iter(["pid", "cmdline"]):
                if p.info["cmdline"] and "agente.py" in " ".join(p.info["cmdline"]) and ("PUESTO" not in os.environ):
                    pass
            ag = [pr for pr in procs if "agente.py" in " ".join(pr.args)]
            if ag:
                rss = psutil.Process(ag[0].pid).memory_info().rss // (1 << 20)
                fila("RSS del agente de ana ahora", "%d MB" % rss, "(la tabla de N filas sigue viva en la sesión: `tg`)")
        except Exception as e:
            fila("RSS del agente", "", "psutil no está: %s" % str(e)[:60])
        s = celda(ana, 'del tg; import gc; gc.collect(); over("ventas.copia", como="arrow").num_rows')
        fila("over() de la copia", "%d ms" % s["_ms"], texto(s))
        print()


# ══ §5 · en el clúster ═══════════════════════════════════════════════════════
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
NS = "t-demo"
BUCKET = "%s-t-demo-copia" % PROYECTO
SHA = sys.argv[sys.argv.index("--sha") + 1] if "--sha" in sys.argv else subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=RAIZ).stdout.strip()[:12]


def k(*args, entrada=None):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    r = subprocess.run(["kubectl", "-n", NS, *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
    return r.returncode, r.stdout, r.stderr


TESTIGO_PY = r'''
import json, os, time, urllib.parse, urllib.request
def testigo():
    d = os.environ["DIRECCION"].rstrip("/"); realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}
'''

TRABAJO = TESTIGO_PY + r'''
import sys, time
T0 = time.time()
sys.path.insert(0, "/opt/ore")
import ore, pyarrow as pa, pyarrow.compute as pc
def di(que, **kw): print("### " + json.dumps(dict(que=que, ms=int((time.time() - T0) * 1000), **kw)), flush=True)
di("python arrancado")
ore.puesto._cabeceras = testigo(); ore.puesto.id = ""
di("testigo")
_pedir = ore.puesto.pedir
def pedir(metodo, ruta, *a, **kw):
    if ruta.startswith("/puestos//datos/"):
        return _pedir("GET", "/datasets/" + ruta.rsplit("/", 1)[1].replace(".", "/"))
    return _pedir(metodo, ruta, *a, **kw)
ore.puesto.pedir = pedir
c, r = ore.puesto.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
N = int(os.environ.get("FILAS", "1000000"))
ids = pa.array(range(N), pa.int64())
tg = pa.table({"id": ids, "pais": pc.choose(pc.cast(pc.bit_wise_and(ids, 3), pa.int32()), "ES", "PT", "FR", "IT"), "total": pc.cast(pc.divide(pc.cast(ids, pa.float64()), 100.0), pa.decimal128(18, 2))})
di("tabla", filas=N)
e = ore.write(ns + ".medida_trabajo_grande", tg)
di("write grande", filas=e["filas"])
r = ore.sql("select pais, count(*) as n, sum(total) as total from %s.medida_trabajo_grande group by pais order by pais" % ns, como="arrow")
di("sql agrega", filas=r.num_rows)
e = ore.write(ns + ".medida_trabajo_resumen", r)
di("write resumen", filas=e["filas"])
open("/trabajo/ns", "w").write(ns)
di("fin")
'''

LIMPIAR = TESTIGO_PY + r'''
import sys
sys.path.insert(0, "/opt/ore")
import ore
ore.puesto._cabeceras = testigo(); ore.puesto.id = ""
c, r = ore.puesto.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
for t in ("grande", "resumen"):
    for ruta in ("packages/%s/tables/medida_trabajo_%s.yaml" % (ns, t), "datasets/%s_medida_trabajo_%s.json" % (ns, t)):
        c, r = ore.puesto.pedir("DELETE", "/arbol/" + ruta)
        print("### " + json.dumps({"que": "retirado", "ruta": ruta, "codigo": c}), flush=True)
print("### " + json.dumps({"que": "paquete", "ns": ns}), flush=True)
'''


def job(nombre, mando, guion, filas):
    cont = {
        "name": "python", "image": "%s/puesto-python:%s" % (REGISTRO, SHA), "imagePullPolicy": "Always",
        "env": [{"name": "HOME", "value": "/tmp"}, {"name": "ORE_SERVE", "value": "http://ore-serve.%s.svc.cluster.local:8080" % NS},
                {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"}, {"name": "REALM", "value": "rubix"},
                {"name": "FILAS", "value": str(filas)}],
        "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
        "workingDir": "/trabajo", "command": ["/bin/sh", "-c"], "args": [mando],
        "resources": {"requests": {"cpu": "1", "memory": "2Gi"}, "limits": {"cpu": "2", "memory": "3Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
    }
    j = {"apiVersion": "batch/v1", "kind": "Job",
         "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": "demo", "ore.dev/rol": "puesto"}},
         "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 1800, "activeDeadlineSeconds": 1500,
                  "template": {"metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": "demo"}},
                               "spec": {"restartPolicy": "Never", "serviceAccountName": "puesto",
                                        "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                                        "initContainers": [{"name": "traer-el-testigo", "image": REGISTRO + "/ore-drivers:main",
                                                            "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                                                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}], "command": ["/bin/sh", "-c"],
                                                            "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=t-demo-agente-$p --out-file=/puesto/agente-$p; chmod 0444 /puesto/agente-$p; done\necho testigo puesto"],
                                                            "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                                            "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}}}],
                                        "containers": [cont]}}}}
    cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": NS}, "data": {"guion.py": guion}}
    return json.dumps(cm) + "\n---\n" + json.dumps(j)


def esperar(nombre, plazo=1500):
    t0 = time.time(); fase = ""
    while time.time() - t0 < plazo:
        c, out, _ = k("get", "job", nombre, "-o", "jsonpath={.status.succeeded}/{.status.failed}")
        ok, mal = (out.split("/") + [""])[:2]
        if ok == "1":
            return "ok", int(time.time() - t0)
        if mal and mal != "0":
            return "falló", int(time.time() - t0)
        c, out, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "jsonpath={.items[0].status.phase}")
        if out != fase:
            fase = out; fila("  el pod", fase or "(esperando nodo)", "%ds" % int(time.time() - t0))
        time.sleep(5)
    return "plazo", int(time.time() - t0)


def linea_de_tiempo(nombre):
    c, out, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "json")
    p = json.loads(out)["items"][0]
    from datetime import datetime
    def t(s):
        return datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").timestamp() if s else None
    creado = t(p["metadata"]["creationTimestamp"])
    cond = {c["type"]: t(c.get("lastTransitionTime")) for c in p["status"].get("conditions", [])}
    init = (p["status"].get("initContainerStatuses") or [{}])[0].get("state", {}).get("terminated", {})
    cont = (p["status"].get("containerStatuses") or [{}])[0].get("state", {}).get("terminated", {})
    return {"nodo (creado → programado)": (cond.get("PodScheduled") or creado) - creado,
            "imagen del testigo (programado → testigo empieza)": (t(init.get("startedAt")) or 0) - (cond.get("PodScheduled") or creado),
            "el testigo (gcloud secrets)": (t(init.get("finishedAt")) or 0) - (t(init.get("startedAt")) or 0),
            "imagen del puesto (testigo acaba → python empieza)": (t(cont.get("startedAt")) or 0) - (t(init.get("finishedAt")) or 0),
            "el guion (python)": (t(cont.get("finishedAt")) or 0) - (t(cont.get("startedAt")) or 0),
            "total": (t(cont.get("finishedAt")) or 0) - creado, "nodo": p["spec"].get("nodeName", "?")}


def cluster():
    print("§5 · correr en el clúster (t-demo, jobs-p 0 → 1 → 0): el frío de un trabajo de código con puesto-python:%s" % SHA)
    nombre = "medida-w37-" + SHA[:8]
    k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
    t0 = time.time()
    c, out, err = k("apply", "-f", "-", entrada=job(nombre, "python3 /guiones/guion.py", TRABAJO, FILAS))
    if c:
        print("  no se pudo crear el Job:", err[:300]); return
    fila("Job creado", "%d ms" % ms(t0), nombre)
    estado, seg = esperar(nombre)
    fila("el Job", estado, "%d s desde aquí" % seg)
    try:
        for kk, v in linea_de_tiempo(nombre).items():
            fila("  " + kk, ("%.0f s" % v) if isinstance(v, float) else str(v))
    except Exception as e:
        fila("  la línea de tiempo", "", str(e)[:120])
    c, log, _ = k("logs", "job/" + nombre, "-c", "python")
    for l in log.splitlines():
        if l.startswith("### "):
            j = json.loads(l[4:]); fila("  guion · " + j.pop("que"), "%d ms" % j.pop("ms"), json.dumps(j) if j else "")
    raro = [l for l in log.splitlines() if not l.startswith("### ") and l.strip()]
    if raro:
        fila("  el guion dijo además", "", " | ".join(raro[-4:])[:200])
    # limpiar: los documentos y punteros por un segundo Job; el bucket desde aquí
    lim = nombre + "-limpiar"
    k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
    k("apply", "-f", "-", entrada=job(lim, "python3 /guiones/guion.py", LIMPIAR, 0))
    estado, seg = esperar(lim, 600)
    c, log, _ = k("logs", "job/" + lim, "-c", "python")
    ret = [json.loads(l[4:]) for l in log.splitlines() if l.startswith("### ")]
    fila("retirados del árbol", estado, ", ".join("%s %s" % (r.get("ruta", "").rsplit("/", 1)[-1], r.get("codigo")) for r in ret if r.get("que") == "retirado")[:160])
    ns = next((r["ns"] for r in ret if r.get("que") == "paquete"), None)
    k("delete", "job", lim, "--ignore-not-found"); k("delete", "configmap", lim, "--ignore-not-found")
    if ns:
        for t in ("grande", "resumen"):
            pref = "gs://%s/ore/v2/datasets/%s_medida_trabajo_%s" % (BUCKET, ns, t)
            r = subprocess.run(["gcloud", "storage", "rm", "-r", "-q", pref], capture_output=True, text=True, shell=(os.name == "nt"))
            fila("retirado del bucket", pref.split("/ore/v2/")[1], "ok" if r.returncode == 0 else r.stderr.strip()[-80:])
    c, out, _ = k("get", "jobs", "-o", "name")
    fila("jobs que quedan en t-demo", str(len(out.split())) if out.strip() else "0", out.replace("\n", " ")[:120])


if __name__ == "__main__":
    main()
