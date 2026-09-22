#!/usr/bin/env python3
"""
MEDIDA · W3.7 gobierno, el gobierno de lo escrito (22 de septiembre), antes de
abordarlo.

Los cuatro verbos están (0031 §9, W3.5b–W3.7 ④): desde un puesto se lee, se
escribe, se declara y se corre, en nombre de la persona y en su rama, con
procedencia. Lo que 0031 («Lo medido fuera del verbo» §2, W3.7 ③ y ④), 0033
(«Lo que esto no decide») y 0034 (la capa Access) dejan apuntando al mismo
sitio es QUIÉN PUEDE hacer cada verbo sobre QUÉ, y qué lleva consigo lo que
sale de un verbo. Aquí se mide, verbo a verbo y con lo que ya existe, dónde
hay una puerta que decide y dónde no hay ninguna:

  §1  LEER       lo que un puesto lee y qué lo gobierna: un dataset que una
                 Entity clasifica `high` con un conducto que sólo admite
                 `low`; el conducto niega la copia (OOS4002) ¿y la lectura
                 desde código? ¿qué consulta `datos_del_puesto`?
  §2  ESCRIBIR   quién escribe encima de quién: bob sobre el dataset de ana,
                 en el paquete de ana, en un paquete de otro equipo; quién
                 retira; y lo que un puesto puede hacer por `/arbol` —
                 reescribir `conduits.yaml`, el `owner` de un paquete—: el
                 gobierno mismo, desde una celda
  §3  DECLARAR   bob redeclara la View y la Entity de ana (y la desclasifica:
                 `high` → `low`); declara en un paquete ajeno; lo que el
                 `owner` del paquete y del dataset deciden (nada) y por qué
                 (nadie dice a qué `team:` pertenece una persona)
  §4  CORRER     lo declarado en un transform y el servidor: dentro de
                 `@transform(inputs, output)`, `over()` de lo no declarado es
                 PermissionError en el SDK; `puesto.pedir()` a pelo, no; lo
                 que el servidor sabe del transform (nada); y la procedencia
                 que queda cuando se rodea el SDK
  §5  EL GRAFO   la clasificación por el linaje: `write(over(alto))` deja un
                 dataset con `sale_de` (procedencia) y SIN clasificación; una
                 copia mantenida encima compila con el conducto `low`; el
                 índice de assets lo enseña sin etiqueta
  §6  EN EL CÓDIGO  las tres vías que podrían negar —la concesión de ore-iam,
                 el conducto (`flow::check`), el `owner`— y dónde las
                 consulta `ore-serve` hoy (contadas en la fuente)

Uso:  python pruebas-de-fuego/medida-w3-gobierno.py [--solo 1,2,3,4,5,6]
Necesita target/debug (ore, ore-serve, ore-store-r2), git, pyarrow, duckdb.
Todo en local: una forja pelada, el S3 de mentira y `agente.py` de verdad.
No imprime ningún token.
"""
import json
import os
import re
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
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "4", "5", "6"}


def fila(a, b="", c=""):
    print("  %-56s %-22s %s" % (a, b, c))


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


CONDUCTO = "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: ventas }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      gdpr.sensitivity: %s\n"


def arbol_semilla(d):
    """Dos paquetes de dos equipos (`ventas` de team:data, `rrhh` de team:rrhh), un
    retículo y un conducto que sólo admite `low`; una Table de ficheros por paquete."""
    os.makedirs(d + "/datos")
    open(d + "/datos/pedidos.jsonl", "w").write("".join('{"order_id":"%d","pais":"ES","total":"%d.00"}\n' % (i, i) for i in range(1, 11)))
    open(d + "/ontology.config.yaml", "w").write("""apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: medida, version: 0.1.0 }
datasources:
  - { name: ficheros, type: jsonl, connectionEnv: FICHEROS_DIR }
  - { name: lago, type: lago, connectionEnv: LAGO_URL }
""")
    open(d + "/lattice.yaml", "w").write("apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
    open(d + "/conduits.yaml", "w").write(CONDUCTO % "low")
    for ns, owner in (("ventas", "team:data"), ("rrhh", "team:rrhh")):
        os.makedirs(d + "/packages/%s/tables" % ns)
        open(d + "/packages/%s/package.yaml" % ns, "w").write("apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: %s, version: 1.0.0, status: active, domain: %s }\nspec: { owner: %s }\n" % (ns, ns, owner))
        open(d + "/packages/%s/tables/pedidos.yaml" % ns, "w").write("""apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: pedidos, namespace: %s }
spec:
  datasource: ficheros
  object: "pedidos.jsonl"
  columns: { order_id: { type: Integer }, pais: {}, total: { type: Decimal } }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: snapshot }
""" % ns)


# ── lo que las celdas ejecutan ───────────────────────────────────────────────
# Una petición a pelo desde la celda, con el cuerpo como texto: el testigo del
# agente y `x-ore-puesto`, exactamente lo que el SDK pone.
CRUDO = """import json, ore, urllib.request, urllib.error
req = urllib.request.Request(ore.puesto.servidor + %r, data=%r.encode("utf-8"), method=%r)
for k, v in ore.puesto._cabeceras.items():
    req.add_header(k, v)
req.add_header("x-ore-puesto", ore.puesto.id)
req.add_header("content-type", "text/plain")
try:
    r = urllib.request.urlopen(req, timeout=60); print(json.dumps([r.status, r.read().decode("utf-8")])[:600])
except urllib.error.HTTPError as e:
    print(json.dumps([e.code, e.read().decode("utf-8", "replace")])[:600])
"""
TRES = ('import pyarrow as pa, decimal, datetime as dt; utc = dt.timezone.utc; '
        't3 = pa.table({"n": pa.array([1, 2, 3], pa.int64()), '
        '"cuando": pa.array([dt.datetime(2026, 1, i, tzinfo=utc) for i in (1, 2, 3)], pa.timestamp("us", "UTC")), '
        '"pais": pa.array(["ES", "PT", None]), '
        '"total": pa.array([decimal.Decimal("1.50"), decimal.Decimal("2.25"), None], pa.decimal128(10, 2))})')
VISTA = "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: ventas, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { dataset: ventas.salida }\n  fields: { n: n, cuando: cuando, pais: pais, total: total }\n"
ENTIDAD = "apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: venta, namespace: ventas }\nspec:\n  nature: event\n  backedBy: ventas.ventas\n  primaryKey: [n]\n  timeKey: cuando\n  properties:\n    n: { type: Integer }\n    cuando: { type: DateTimeTz }\n    pais: { type: String }\n    total: { type: Decimal, labels: { gdpr.sensitivity: %s } }\n"
COPIA = "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: %s, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { view: ventas.%s }\n"


def main():
    for b in (ORE, SERVE, STORE):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve -p ore-store"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-gobierno-").replace("\\", "/")
    procs = []
    try:
        local(tmp, procs)
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

    def abre_puesto(persona, rama=None):
        c, r = pide(base, "POST", "/puestos", {"rama": rama} if rama else {}, sujeto=persona)
        assert c in (200, 201), (c, r)
        pid = r["id"]
        ag = subprocess.Popen([PY, RAIZ + "/puesto/python/agente.py"], env=dict(env, ORE_SERVE=base, PUESTO=pid, ORE_SUJETO="agente:local", ORE_ALMACEN="dir:" + tmp + "/almacen", TTL="900"),
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

    def pedir_desde(pid, persona, metodo, ruta, cuerpo=None):
        """Una celda que llama a ore-serve a pelo con el testigo del agente y `x-ore-puesto`: lo que
        un `.py` puede hacer rodeando el SDK."""
        s = celda(pid, 'import json, ore; c, r = ore.puesto.pedir(%r, %r, %s); print(json.dumps([c, r])[:600])' % (metodo, ruta, "json.loads(%r)" % json.dumps(cuerpo) if cuerpo is not None else "None"), persona=persona)
        try:
            c, r = json.loads(texto(s)); return c, r, s["_ms"]
        except Exception:
            return "?", texto(s)[:200], s["_ms"]

    def texto_desde(pid, persona, metodo, ruta, doc):
        """Como `pedir_desde`, con el cuerpo como texto (lo que `PUT /arbol` espera): el mismo testigo
        del agente y la misma cabecera `x-ore-puesto` que el SDK pone."""
        s = celda(pid, CRUDO % (ruta, doc, metodo), persona=persona)
        try:
            c, r = json.loads(texto(s))
            try:
                r = json.loads(r)
            except Exception:
                pass
            return c, r, s["_ms"]
        except Exception:
            return "?", texto(s)[:200], s["_ms"]

    def arbol(ruta):
        c, r = pide(base, "GET", "/arbol/" + ruta)
        return (r or {}).get("texto") if c == 200 else None

    def cabeza():
        c, r = pide(base, "GET", "/arbol")
        return (r or {}).get("cabeza") or (r or {}).get("commit") or ""

    def clon(nombre):
        m = tmp + "/" + nombre
        shutil.rmtree(m, ignore_errors=True)
        git("clone", "-q", forja, m)
        return m

    def valida(m, tras=""):
        c, s = corre([ORE, "validate", m], env)
        oos = re.findall(r"OOS\d{4}", s)
        return c, (", ".join(sorted(set(oos))) if oos else ("compila" if c == 0 else s[:120]))

    def indice(m):
        c, s = corre([ORE, "assets", m, "--json"], env)
        try:
            return json.loads(s[s.index("{"):])
        except Exception:
            return {"items": {}, "_error": s[:200]}

    def clasificacion(m, ref):
        it = indice(m).get("items", {}).get(ref)
        if not it:
            return "no está en el índice"
        cl = it.get("acceso", {}).get("clasificacion", {})
        return json.dumps(cl) if cl else "{} (sin clasificar)"

    def diag(r):
        if isinstance(r, dict):
            return r.get("error") or ((r.get("diagnosticos") or [{}])[0].get("codigo", "") + " " + (r.get("diagnosticos") or [{}])[0].get("mensaje", "")).strip() or ", ".join("%s=%s" % (k, str(v)[:24]) for k, v in sorted(r.items()))
        return str(r)

    ana = abre_puesto("persona:ana")
    bob = abre_puesto("persona:bob")
    # el material: ana escribe `ventas.salida`, la vista identidad encima y la Entity que clasifica `total` como high
    s = celda(ana, TRES + '; e = write("ventas.salida", t3); e["filas"]')
    assert texto(s) == "3", s
    s = celda(ana, 'd = declare(%r); d["commit"]' % VISTA); assert s.get("tipo") != "error", s
    s = celda(ana, 'd = declare(%r); d["commit"]' % (ENTIDAD % "high")); assert s.get("tipo") != "error", s

    # ── §1 · leer ────────────────────────────────────────────────────────────
    if "1" in SOLO:
        print("§1 · leer: un dataset que una Entity clasifica `high`, un conducto que admite `low`, y un puesto que lee")
        m = clon("mira1")
        c, d = valida(m)
        fila("el árbol: dataset escrito + View + Entity (total: high)", "código %d" % c, d)
        fila("  clasificación en el índice · entity:ventas.venta", "", clasificacion(m, "entity:ventas.venta"))
        fila("  clasificación en el índice · view:ventas.ventas", "", clasificacion(m, "view:ventas.ventas"))
        fila("  clasificación en el índice · dataset:ventas.salida", "", clasificacion(m, "dataset:ventas.salida"))
        open(m + "/packages/ventas/datasets/copia.yaml" if os.path.isdir(m + "/packages/ventas/datasets") else (os.makedirs(m + "/packages/ventas/datasets", exist_ok=True) or m + "/packages/ventas/datasets/copia.yaml"), "w").write(COPIA % ("copia", "ventas"))
        c, d = valida(m)
        fila("un Dataset mantenido from view ventas.ventas, conducto low", "código %d" % c, d + "  ← el conducto decide sobre la copia declarada")
        os.remove(m + "/packages/ventas/datasets/copia.yaml")
        for quien, pid in (("ana", ana), ("bob", bob)):
            s = celda(pid, 'over("ventas.salida", como="arrow").column("total").to_pylist()', persona="persona:" + quien)
            fila("over(\"ventas.salida\") desde el puesto de %s" % quien, "%s · %d ms" % (s.get("tipo"), s["_ms"]), texto(s)[:80] + "  ← la columna high, entera")
        s = celda(bob, 'over("ventas.ventas", como="arrow").num_rows', persona="persona:bob")
        fila("over(\"ventas.ventas\") (la View sobre el dataset) desde bob", "%s" % s.get("tipo"), texto(s)[:80])
        s = celda(bob, 'sql("select pais, sum(total) t from ventas.salida group by pais", como="arrow").num_rows', persona="persona:bob")
        fila("sql() sobre ventas.salida desde bob", "%s" % s.get("tipo"), texto(s)[:80])
        c, r = pide(base, "GET", "/puestos/%s/datos/ventas.salida" % bob, sujeto="agente:local", cabeceras={"x-ore-puesto": bob})
        fila("lo que GET /puestos/{id}/datos/ventas.salida contesta", "HTTP %s" % c, "claves: " + ", ".join(sorted(r))[:90] if isinstance(r, dict) else str(r)[:90])
        fila("  ¿trae clasificación, conducto o concesión?", "", ", ".join(k for k in ("clasificacion", "conducto", "concesion", "labels", "acceso") if isinstance(r, dict) and k in r) or "ninguna de las cinco")
        print()

    # ── §2 · escribir ────────────────────────────────────────────────────────
    if "2" in SOLO:
        print("§2 · escribir: quién escribe encima de quién, quién retira, y lo que un puesto hace por /arbol")
        s = celda(bob, TRES + '; e = write("ventas.salida", t3, modo="anexar"); e["filas"]', persona="persona:bob")
        c, r = pide(base, "GET", "/datasets/ventas/salida")
        fila("bob anexa a ventas.salida (dataset de ana, paquete team:data)", "%s · %d ms" % (s.get("tipo"), s["_ms"]), "%s · escrito_por %s · owner %s" % (texto(s)[:30], r.get("escrito_por"), (arbol("packages/ventas/datasets/salida.yaml") or "").split("owner:")[-1].split("\n")[0].strip() if arbol("packages/ventas/datasets/salida.yaml") else "?"))
        s = celda(bob, TRES + '; e = write("ventas.salida", t3); e["filas"]', persona="persona:bob")
        fila("bob sobrescribe ventas.salida entero", "%s" % s.get("tipo"), texto(s)[:60])
        s = celda(bob, TRES + '; e = write("ventas.deBob", t3); e["filas"]', persona="persona:bob")
        fila("bob escribe ventas.deBob (paquete de team:data)", "%s" % s.get("tipo"), "%s · owner %s" % (texto(s)[:20], (arbol("packages/ventas/datasets/deBob.yaml") or "?").split("owner:")[-1].split("\n")[0].strip()))
        s = celda(bob, TRES + '; e = write("rrhh.deBob", t3); e["filas"]', persona="persona:bob")
        fila("bob escribe rrhh.deBob (paquete de team:rrhh)", "%s" % s.get("tipo"), "%s · owner %s" % (texto(s)[:20], (arbol("packages/rrhh/datasets/deBob.yaml") or "?").split("owner:")[-1].split("\n")[0].strip()))
        s = celda(bob, TRES + '; e = write("nadie.x", t3); e["filas"]', persona="persona:bob")
        fila("bob escribe nadie.x (paquete que no existe)", "%s" % s.get("tipo"), texto(s)[:100])
        s = celda(ana, TRES + '; write("ventas.deAna", t3)["filas"]')
        c, r, t = pedir_desde(bob, "persona:bob", "DELETE", "/documentos/Dataset/ventas/deAna")
        fila("bob retira ventas.deAna por DELETE /documentos", "HTTP %s · %d ms" % (c, t), ("queda: %s" % ("sí" if arbol("packages/ventas/datasets/deAna.yaml") else "no; puntero: %s" % ("sí" if arbol("datasets/ventas_deAna.json") else "no"))) if c == 200 else diag(r)[:100])
        c, r, t = pedir_desde(bob, "persona:bob", "DELETE", "/arbol/datasets/ventas_salida.json")
        fila("bob retira el puntero de ventas.salida por DELETE /arbol", "HTTP %s" % c, ("puntero queda: %s" % ("sí" if arbol("datasets/ventas_salida.json") else "no")) if c == 200 else diag(r)[:100])
        if c == 200:
            # lo repone ana escribiendo otra vez (la Table/Dataset sigue; el puntero nace de nuevo)
            celda(ana, TRES + '; write("ventas.salida", t3)["filas"]')
        # el gobierno mismo, desde una celda
        c, r, t = texto_desde(bob, "persona:bob", "PUT", "/arbol/conduits.yaml", CONDUCTO % "high")
        fila("bob reescribe conduits.yaml (low → high) por PUT /arbol", "HTTP %s · %d ms" % (c, t), diag(r)[:60] if c not in (200, 201) else "firma " + autor("conduits.yaml")[0])
        if c in (200, 201):
            m = clon("mira2")
            os.makedirs(m + "/packages/ventas/datasets", exist_ok=True)
            open(m + "/packages/ventas/datasets/copia.yaml", "w").write(COPIA % ("copia", "ventas"))
            c2, d = valida(m)
            fila("  y la copia mantenida que §1 negaba", "código %d" % c2, d)
            c, r, t = texto_desde(bob, "persona:bob", "PUT", "/arbol/conduits.yaml", CONDUCTO % "low")
        paquete = arbol("packages/ventas/package.yaml") or ""
        c, r, t = texto_desde(bob, "persona:bob", "PUT", "/arbol/packages/ventas/package.yaml", paquete.replace("team:data", "user:bob"))
        fila("bob se hace owner del paquete ventas por PUT /arbol", "HTTP %s" % c, diag(r)[:60] if c not in (200, 201) else "owner ahora: " + (arbol("packages/ventas/package.yaml") or "").split("owner:")[-1].strip(" }\n"))
        if c in (200, 201):
            texto_desde(bob, "persona:bob", "PUT", "/arbol/packages/ventas/package.yaml", paquete)
        c, r, t = texto_desde(bob, "persona:bob", "PUT", "/arbol/lattice.yaml", "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none]\n")
        fila("bob deja el retículo en un nivel por PUT /arbol", "HTTP %s" % c, diag(r)[:100] if c not in (200, 201) else "aceptado")
        if c in (200, 201):
            texto_desde(bob, "persona:bob", "PUT", "/arbol/lattice.yaml", "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
        a, n = autor("datasets/ventas_salida.json")
        fila("dónde quedó todo esto", "en main", "el puesto de bob no tiene rama; último commit del puntero: %s" % a)
        print()

    # ── §3 · declarar ────────────────────────────────────────────────────────
    if "3" in SOLO:
        print("§3 · declarar: bob redeclara lo de ana, la desclasifica, y declara en un paquete ajeno")
        s = celda(bob, 'd = declare(%r); [d["nueva"], d["commit"]]' % VISTA.replace("total: total", "total: total, doble: total"), persona="persona:bob")
        fila("bob redeclara la View ventas.ventas de ana (un campo más)", "%s · %d ms" % (s.get("tipo"), s["_ms"]), texto(s)[:40] + " · firma " + autor("packages/ventas/views/ventas.yaml")[0])
        s = celda(bob, 'd = declare(%r); [d["nueva"], d["commit"]]' % (ENTIDAD % "low"), persona="persona:bob")
        m = clon("mira3")
        fila("bob redeclara la Entity: total high → low", "%s · %d ms" % (s.get("tipo"), s["_ms"]), "firma %s · índice: %s" % (autor("packages/ventas/entities/venta.yaml")[0], clasificacion(m, "entity:ventas.venta")))
        celda(ana, 'declare(%r)["commit"]' % (ENTIDAD % "high"))
        s = celda(bob, 'd = declare(%r); [d["nueva"], d["commit"]]' % ("apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: nominas, namespace: rrhh }\nspec:\n  owner: team:ventas\n  from: { table: rrhh.pedidos }\n  fields: { id: order_id }\n"), persona="persona:bob")
        fila("bob declara una View en rrhh (team:rrhh) con owner team:ventas", "%s" % s.get("tipo"), texto(s)[:60])
        s = celda(bob, 'd = declare(%r); d' % (CONDUCTO % "high"), persona="persona:bob")
        fila("bob declara un ConduitPolicy por declare()", "%s" % s.get("tipo"), texto(s)[:100])
        s = celda(bob, 'declare(%r)["commit"]' % VISTA.replace("ventas.salida", "ventas.noExiste"), persona="persona:bob")
        fila("bob declara una View rota", "%s" % s.get("tipo"), texto(s)[:90] + "  ← lo único que niega: el compilador")
        # a qué equipo pertenece una persona: dónde se dice
        pertenece = (subprocess.run(["git", "grep", "-n", "-i", "-E", "pertenece|miembro|members|equipo", "--", "crates/ore-serve/src"], cwd=RAIZ, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout or "").strip().splitlines()
        pertenece = [l for l in pertenece if not re.match(r"^[^:]+:\d+:\s*//", l)]
        fila("persona → team: dónde lo resuelve ore-serve", "%d líneas de código" % len(pertenece), ("; ".join(sorted({l.split(":")[0].split("/")[-1] for l in pertenece}))) if pertenece else "en ningún sitio: `owner: team:x` no nombra a nadie que ore-serve conozca")
        print()

    # ── §4 · correr: lo declarado y el servidor ──────────────────────────────
    if "4" in SOLO:
        print("§4 · correr: lo que un transform declara, y lo que el servidor sabe de ello")
        s = celda(ana, TRES + '; write("ventas.otro", t3)["filas"]')
        s = celda(ana, '''
@transform(inputs=["ventas.salida"], output="ventas.resumen")
def resumir():
    try:
        over("ventas.otro"); a = "leyó"
    except PermissionError as e:
        a = "PermissionError"
    import ore
    c, r = ore.puesto.pedir("GET", "/puestos/%s/datos/ventas.otro" % ore.puesto.id)
    c2, r2 = ore.puesto.pedir("GET", "/arbol/packages/ventas/datasets/otro.yaml")
    t = sql("select pais, count(*) n from ventas.salida group by pais", como="arrow")
    e = write("ventas.resumen", t)
    return [a, c, c2, e["filas"]]
resumir()''')
        fila("dentro del transform: over() de lo no declarado", "%s · %d ms" % (s.get("tipo"), s["_ms"]), texto(s)[:100])
        fila("  … y puesto.pedir(GET /puestos/{id}/datos/ventas.otro) a pelo", "", "(el segundo valor: HTTP)")
        c, r = pide(base, "GET", "/puestos/" + ana)
        fila("lo que GET /puestos/{id} sabe del transform", "", ", ".join(sorted(r)) [:90] + " → transform/inputs/output: %s" % ("sí" if any(k in r for k in ("transform", "inputs", "output", "declarado")) else "no"))
        c, r = pide(base, "GET", "/datasets/ventas/resumen")
        fila("la procedencia de ventas.resumen", "", json.dumps(r.get("procedencia"))[:120])
        fila("  ¿dice que también leyó ventas.otro a pelo?", "", "no: la procedencia es lo que el SDK vio" if "otro" not in json.dumps(r.get("procedencia")) else "sí")
        s = celda(ana, '''
@transform(inputs=["ventas.salida"], output="ventas.resumen")
def escapa():
    import ore
    c, r = ore.puesto.pedir("PUT", "/arbol/notas/desde-un-transform.md", "escrito desde dentro de un transform")
    return c
escapa()''')
        fila("dentro del transform: PUT /arbol de un fichero cualquiera", "%s" % s.get("tipo"), "HTTP %s · en el árbol: %s" % (texto(s)[:5], "sí" if arbol("notas/desde-un-transform.md") else "no"))
        if arbol("notas/desde-un-transform.md"):
            pide(base, "DELETE", "/arbol/notas/desde-un-transform.md")
        c, r = pide(base, "POST", "/trabajos", {"codigo": "packages/ventas/transforms/x.py"}, sujeto="agente:local")
        fila("POST /trabajos con el testigo del agente (un Job lanzando otro)", "HTTP %s" % c, diag(r)[:90])
        print()

    # ── §5 · el grafo: la clasificación por el linaje ────────────────────────
    if "5" in SOLO:
        print("§5 · el grafo: lo que write(over(alto)) deja, y lo que compila encima")
        s = celda(ana, 'e = write("ventas.derivado", over("ventas.salida", como="arrow")); e["filas"]')
        c, r = pide(base, "GET", "/datasets/ventas/derivado")
        fila("ana: write(\"ventas.derivado\", over(\"ventas.salida\"))", "%s · %d ms" % (s.get("tipo"), s["_ms"]), "procedencia " + json.dumps(r.get("procedencia"))[:80])
        m = clon("mira5")
        it = indice(m).get("items", {}).get("dataset:ventas.derivado", {})
        fila("  en el índice: relaciones", "", ", ".join("%s %s" % (x["tipo"], x["ref"]) for x in it.get("relaciones", []))[:100] or "ninguna")
        fila("  en el índice: clasificación", "", clasificacion(m, "dataset:ventas.derivado") + "  ← la de ventas.salida era " + clasificacion(m, "dataset:ventas.salida"))
        fila("  el documento Dataset escrito", "", "labels: %s · columns: %s" % ("no (OOS1005)" if "labels" not in (arbol("packages/ventas/datasets/derivado.yaml") or "") else "sí", ", ".join(re.findall(r"^\s{4}(\w+):", arbol("packages/ventas/datasets/derivado.yaml") or "", re.M))))
        os.makedirs(m + "/packages/ventas/views", exist_ok=True)
        open(m + "/packages/ventas/views/derivadoV.yaml", "w").write("apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: derivadoV, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { dataset: ventas.derivado }\n  fields: { n: n, total: total }\n")
        os.makedirs(m + "/packages/ventas/datasets", exist_ok=True)
        open(m + "/packages/ventas/datasets/derivadoCopia.yaml", "w").write(COPIA % ("derivadoCopia", "derivadoV"))
        c, d = valida(m)
        fila("View sobre derivado + Dataset mantenido, conducto low", "código %d" % c, d + "  ← lo que §1 negaba sobre ventas.ventas, aquí pasa")
        os.remove(m + "/packages/ventas/datasets/derivadoCopia.yaml")
        # y una Function que lee la vista derivada (0029: `reads` sólo vistas), con el mismo conducto
        os.makedirs(m + "/packages/ventas/functions", exist_ok=True)
        open(m + "/packages/ventas/functions/lee.yaml", "w").write("apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: lee, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: dist/lee.wasm\n  reads: [ventas.derivadoV]\n")
        c, d = valida(m)
        fila("una Function con reads: [ventas.derivadoV]", "código %d" % c, d)
        os.remove(m + "/packages/ventas/functions/lee.yaml")
        # el índice sabe de dónde salió: lo que haría falta para bajar la etiqueta
        cadena = []
        items = indice(m).get("items", {})
        ref = "dataset:ventas.derivado"
        for _ in range(4):
            it = items.get(ref, {})
            padres = [x["ref"] for x in it.get("relaciones", []) if x["tipo"] == "sale_de"]
            cadena.append(ref)
            if not padres or padres[0] in cadena:
                if padres and padres[0] in cadena:
                    cadena.append(padres[0] + " (ciclo)")
                break
            ref = padres[0]
        fila("la cadena sale_de que el índice ya tiene", "%d eslabones" % len(cadena), " ← ".join(cadena))
        quien = [x["ref"] for x in items.get("dataset:ventas.salida", {}).get("relaciones", []) if x["tipo"] in ("respaldada_por", "leido_por", "produce")]
        fila("  y las relaciones de ventas.salida", "%d" % len(quien), ", ".join(quien)[:100])
        c, r = pide(base, "GET", "/datasets/ventas/salida")
        fila("  la procedencia vigente de ventas.salida (reescrito por ana en §2)", "", json.dumps(r.get("procedencia"))[:110] + "  ← `leidas` es la sesión entera")
        print()

    # ── §6 · en el código: las tres vías, contadas ───────────────────────────
    if "6" in SOLO:
        print("§6 · en la fuente de ore-serve: dónde se consulta cada vía que podría negar")

        def cuenta(patron, ficheros="crates/ore-serve/src"):
            out = (subprocess.run(["git", "grep", "-n", "-i", "-E", patron, "--", *ficheros.split()], cwd=RAIZ, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout or "").strip().splitlines()
            codigo = [l for l in out if not re.match(r"^[^:]+:\d+:\s*//", l)]
            return len(out), len(codigo), sorted({l.split(":")[0].split("/")[-1] for l in codigo})

        for nombre, patron in (
            ("la concesión (ore-iam, AuthZEN)", r"concesion|authzen|permitid"),
            ("el conducto (flow::check, conducto)", r"flow::check|conducto"),
            ("el owner (de paquete o dataset)", r"\bowner\b|dueno|dueño"),
            ("escrito_por / persona_del_puesto (quién hizo)", r"escrito_por|persona_del_puesto|sujeto_del_puesto"),
            ("es_agente / es_persona (qué tipo de sujeto)", r"es_agente|es_persona"),
        ):
            total, codigo, fich = cuenta(patron)
            fila(nombre, "%d líneas, %d en código" % (total, codigo), ", ".join(fich)[:90] or "ninguna")
        total, codigo, fich = cuenta(r"flow::check", "crates/ore-core/src crates/ore-cli/src")
        fila("flow::check en ore-core/ore-cli (el compilador y el índice)", "%d en código" % codigo, ", ".join(fich)[:90])
        print()


if __name__ == "__main__":
    main()
