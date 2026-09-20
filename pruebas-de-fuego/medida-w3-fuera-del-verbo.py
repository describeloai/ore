#!/usr/bin/env python3
"""
MEDIDA · lo que quedó fuera del verbo escribir (20 de septiembre), antes de
abordarlo. W3.6c cerró `write()` (0031 §11) y dejó cinco materias con dueño
pero sin medida. Aquí se miden, con lo que ya existe, para decidir con
números y no con opinión:

  §1  LA FICHA        lo que `GET /datasets/{ns}/{n}` da hoy (campos, tamaño,
                      ms) frente a lo que la consola pinta (`ItemDetailsTab`
                      dice: «no hay endpoint: inventar un snapshot id sería
                      peor») y a lo que ya consume (`/paquetes/{n}/copias`):
                      ¿aparece ahí lo que `write()` escribió?
  §2  EL GOBIERNO     la Table que nace de `write()`: ¿lleva dueño, etiquetas,
                      `reads`? Si una persona la etiqueta a mano (`labels` en
                      una columna, `owner`), ¿lo respeta el lattice/conducto
                      al materializar una View encima? ¿sobrevive a la
                      escritura siguiente que evoluciona el esquema y
                      REGENERA el documento? Y quién puede escribir encima:
                      ¿otra persona del inquilino obtiene una credencial
                      prestada para la tabla de ana y anexa?
  §3  UPSERT          PyIceberg `upsert()` contra ore-serve (lo que un cliente
                      de fuera hace hoy sin que escribamos nada): qué
                      snapshots deja, si DuckDB y `ore-store` lo leen, si
                      `recoger` retira lo reescrito; DuckDB `UPDATE`/`DELETE`/
                      `MERGE`; y el coste de un upsert copy-on-write sobre 1 M
                      de filas (PyIceberg y DuckDB en proceso), que es lo que
                      `modo: upsert` en `ore-store` costaría
  §4  LA CUENTA       (--cluster, `jobs-p` 0 → 1 → 0) un puesto con una cuenta
                      que SÓLO LEE el bucket (`objectViewer`, temporal): ¿lee
                      `over()`? ¿escribe `write()` con el token prestado?
                      ¿falla el pod escribiendo directo (403)? ¿y el token
                      prestado fuera de su prefijo? Y de paso, la ficha en
                      demo: ms de `GET /datasets` y de una ficha de verdad
  §5  ORE-READ-LAGO   una View sobre la Table escrita: `materialize` y `ask`
                      hoy (qué dicen), qué recibe un driver (URL + objeto: sin
                      el puntero), y `ore-store leer` como lector de 1 M de
                      filas frente a `ore-read-jsonl` (el protocolo del driver)

Uso:  python pruebas-de-fuego/medida-w3-fuera-del-verbo.py [--cluster] [--filas N] [--solo 1,2,3,5]
Necesita target/debug (ore, ore-serve, ore-store-r2, ore-read-jsonl), git,
pyiceberg, duckdb, pyarrow. §4 necesita kubectl (ore-mesh) y gcloud, crea una
cuenta y un KSA temporales y los retira. No imprime ningún token.
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
JSONL = "%s/ore-read-jsonl%s" % (BIN, EXE)
# Para el caudal de §5 valen los de release si están (un debug lee 1 M de filas en 37 s: no mide nada)
REL = RAIZ + "/target/release"
STORE_REL = "%s/ore-store-r2%s" % (REL, EXE) if os.path.exists("%s/ore-store-r2%s" % (REL, EXE)) else STORE
JSONL_REL = "%s/ore-read-jsonl%s" % (REL, EXE) if os.path.exists("%s/ore-read-jsonl%s" % (REL, EXE)) else JSONL
PY = sys.executable
CLUSTER = "--cluster" in sys.argv
FILAS = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 1_000_000
SOLO = set(sys.argv[sys.argv.index("--solo") + 1].split(",")) if "--solo" in sys.argv else {"1", "2", "3", "5"} | ({"4"} if CLUSTER else set())


def fila(a, b="", c=""):
    print("  %-46s %-26s %s" % (a, b, c))


def ms(t):
    return int((time.time() - t) * 1000)


def git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8",
                          env=dict(os.environ, GIT_AUTHOR_NAME="semilla", GIT_AUTHOR_EMAIL="s@x",
                                   GIT_COMMITTER_NAME="semilla", GIT_COMMITTER_EMAIL="s@x")).stdout.strip()


def pide(base, metodo, ruta, cuerpo=None, cabeceras=None, sujeto="persona:ana"):
    datos = cuerpo.encode("utf-8") if isinstance(cuerpo, str) else cuerpo
    r = urllib.request.Request(base + ruta, data=datos, method=metodo)
    r.add_header("x-ore-sujeto", sujeto)
    r.add_header("content-type", "application/json")
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=300) as resp:
            return resp.status, resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def puerto_libre():
    import socket
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


def corre(args, env, cwd=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8", env=env, cwd=cwd)
    return r.returncode, " ".join((r.stdout + "\n" + r.stderr).split())


def arbol_semilla(d):
    """El árbol de medida-w3-swap.py: ficheros + lago, lattice y conducto; sin la Table del lago (la escribe PyIceberg)."""
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


PYICEBERG = r'''
import sys, json, time
import pyarrow as pa
from pyiceberg.catalog import load_catalog
base, sujeto, que = sys.argv[1], sys.argv[2], sys.argv[3]
cat = load_catalog("ore", **{"type": "rest", "uri": base, "header.x-ore-sujeto": sujeto})
def datos(desde, n, extra=False):
    cols = {"id": pa.array(range(desde, desde + n), pa.int64()), "pais": pa.array(["ES"] * n),
            "total": pa.array([i * 1.5 for i in range(desde, desde + n)], pa.float64()).cast(pa.decimal128(18, 2)),
            "cuando": pa.array([1_700_000_000_000_000 + i for i in range(desde, desde + n)], pa.timestamp("us", tz="UTC"))}
    if extra:
        cols["nota"] = pa.array(["x"] * n)
    return pa.table(cols)
out = {}
if que == "crear":
    ns, nombre, n = sys.argv[4], sys.argv[5], int(sys.argv[6])
    t0 = time.time()
    t = cat.create_table((ns, nombre), schema=datos(0, 1).schema)
    t.append(datos(0, n))
    out = {"ms": int((time.time() - t0) * 1000), "filas": n, "snapshots": len(cat.load_table((ns, nombre)).metadata.snapshots)}
elif que == "anexar":
    ns, nombre, n = sys.argv[4], sys.argv[5], int(sys.argv[6])
    t = cat.load_table((ns, nombre))
    try:
        t.append(datos(10_000, n)); out = {"codigo": 200, "filas": t.scan().to_arrow().num_rows}
    except Exception as e:
        out = {"codigo": "error", "error": str(e)[:160]}
elif que == "evolucionar":
    ns, nombre = sys.argv[4], sys.argv[5]
    t = cat.load_table((ns, nombre))
    with t.update_schema() as u:
        u.add_column("nota", __import__("pyiceberg.types", fromlist=["StringType"]).StringType())
    t = cat.load_table((ns, nombre))
    t.append(datos(20_000, 5, extra=True))
    out = {"columnas": [f.name for f in t.schema().fields], "snapshots": len(cat.load_table((ns, nombre)).metadata.snapshots)}
elif que == "upsert":
    ns, nombre, desde, n = sys.argv[4], sys.argv[5], int(sys.argv[6]), int(sys.argv[7])
    t = cat.load_table((ns, nombre))
    # la mitad cambia (misma clave, total distinto), la otra mitad es nueva
    cambio = pa.table({"id": pa.array(range(desde, desde + n), pa.int64()), "pais": pa.array(["PT"] * n),
                       "total": pa.array([999.0] * n, pa.float64()).cast(pa.decimal128(18, 2)),
                       "cuando": pa.array([1_800_000_000_000_000 + i for i in range(n)], pa.timestamp("us", tz="UTC"))})
    t0 = time.time()
    r = t.upsert(cambio, join_cols=["id"])
    t = cat.load_table((ns, nombre))
    snaps = [(s.summary.operation, dict(s.summary.additional_properties)) for s in t.metadata.snapshots]
    tabla = t.scan().to_arrow()
    import pyarrow.compute as pc
    out = {"ms": int((time.time() - t0) * 1000), "actualizadas": r.rows_updated, "insertadas": r.rows_inserted,
           "filas": tabla.num_rows, "pt": pc.sum(pc.equal(tabla["pais"], "PT")).as_py(),
           "snapshots": [[op, {k: v for k, v in p.items() if k in ("added-data-files", "deleted-data-files", "added-records", "deleted-records", "total-data-files", "total-records")}] for op, p in snaps]}
elif que == "retencion":
    ns, nombre, edad = sys.argv[4], sys.argv[5], sys.argv[6]
    t = cat.load_table((ns, nombre))
    with t.transaction() as tx:
        tx.set_properties({"history.expire.max-snapshot-age-ms": edad, "history.expire.min-snapshots-to-keep": "1"})
    out = {"propiedades": {k: v for k, v in cat.load_table((ns, nombre)).properties.items() if k.startswith("history")}}
elif que == "leer":
    ns, nombre = sys.argv[4], sys.argv[5]
    t = cat.load_table((ns, nombre)).scan().to_arrow().to_pylist()
    por = {f["id"]: f["pais"] for f in t}
    out = {"filas": len(t), "id1": por.get(1), "id2": por.get(2, "no está"), "id3": por.get(3)}
elif que == "deletes":
    ns, nombre = sys.argv[4], sys.argv[5]
    t = cat.load_table((ns, nombre))
    d = t.inspect.delete_files().to_pylist()
    import pyarrow.parquet as pq, io
    def dentro(path):
        with t.io.new_input(path).open() as f:
            return [(r["file_path"].rsplit("/", 1)[1][-24:], r["pos"]) for r in pq.read_table(io.BytesIO(f.read())).to_pylist()]
    out = {"version": t.metadata.format_version, "delete_files": [{"content": f["content"], "formato": f["file_format"], "filas": f["record_count"], "fichero": f["file_path"].rsplit("/", 1)[1][-24:], "dentro": dentro(f["file_path"])} for f in d],
           "datos": [(f["file_path"].rsplit("/", 1)[1][-24:], f["record_count"]) for f in t.inspect.data_files().to_pylist()]}
elif que == "credencial":
    ns, nombre = sys.argv[4], sys.argv[5]
    import urllib.request
    r = urllib.request.Request(base + "/v1/namespaces/%s/tables/%s" % (ns, nombre), headers={"x-ore-sujeto": sujeto, "X-Iceberg-Access-Delegation": "vended-credentials"})
    with urllib.request.urlopen(r) as resp:
        j = json.load(resp)
    out = {"codigo": resp.status, "config": sorted(j.get("config", {}).keys()), "credenciales": len(j.get("storage-credentials", []))}
print(json.dumps(out, default=str))
'''

DUCK = r'''
import sys, json, time, duckdb
base, s3, que = sys.argv[1], sys.argv[2], sys.argv[3]
con = duckdb.connect()
con.execute("install iceberg; install httpfs; load iceberg; load httpfs;")
con.execute("create secret s3 (type s3, key_id 'de', secret 'mentira', endpoint '%s', url_style 'path', use_ssl false, region 'auto')" % s3.replace("http://", ""))
con.execute("create secret ice (type iceberg, token 'persona:ana')")
con.execute("attach '' as lago (type iceberg, endpoint '%s', secret ice)" % base)
out = {}
if que == "leer":
    t0 = time.time()
    r = con.execute("select count(*), count(*) filter (where pais = 'PT'), sum(total) from lago.ventas." + sys.argv[4]).fetchone()
    out = {"ms": int((time.time() - t0) * 1000), "filas": r[0], "pt": r[1], "suma": str(r[2])}
elif que == "mutar":
    for nombre, sql in (("update", "update lago.ventas.%s set pais = 'FR' where id = 1"), ("delete", "delete from lago.ventas.%s where id = 2"),
                        ("merge", "merge into lago.ventas.%s t using (select 3 as id, 'IT' as pais) s on t.id = s.id when matched then update set pais = s.pais")):
        try:
            con.execute(sql % sys.argv[4]); out[nombre] = "ok"
        except Exception as e:
            out[nombre] = str(e).split("\n")[0][:110]
    try:
        por = dict(con.execute("select id, pais from lago.ventas.%s where id in (1, 2, 3)" % sys.argv[4]).fetchall())
        out.update(filas=con.execute("select count(*) from lago.ventas.%s" % sys.argv[4]).fetchone()[0], id1=por.get(1), id2=por.get(2, "no está"), id3=por.get(3))
    except Exception as e:
        out["error"] = str(e)[:120]
elif que == "fundir":
    # lo que un upsert copy-on-write hace por dentro: leer la tabla, quitar las claves que cambian, unir, escribir
    ml = sys.argv[4]; n = int(sys.argv[5])
    raiz = ml[: ml.index("/metadata/")]
    version = ml[ml.index("/metadata/") + len("/metadata/"):].replace(".metadata.json", "")
    t0 = time.time()
    con.execute("create temp table cambio as select 500 + i as id, 'PT' as pais, 999.00::decimal(18,2) as total, now()::timestamptz as cuando from range(%d) r(i)" % n)
    con.execute("copy (select t.* from iceberg_scan('%s', version => '%s', allow_moved_paths => true) t anti join cambio c on t.id = c.id union all select * from cambio) to '%s' (format parquet)" % (raiz, version, sys.argv[6].replace("\\", "/")))
    out = {"ms": int((time.time() - t0) * 1000), "filas": con.execute("select count(*) from read_parquet('%s')" % sys.argv[6].replace("\\", "/")).fetchone()[0]}
print(json.dumps(out, default=str))
'''


def main():
    for b in (ORE, SERVE, STORE, JSONL):
        if not os.path.exists(b):
            print("falta", b, "— cargo build -p ore-cli -p ore-serve -p ore-store -p ore-read-jsonl"); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-fuera-").replace("\\", "/")
    procs = []
    try:
        s3 = subprocess.Popen([PY, RAIZ + "/pruebas-de-fuego/de-mentira.py", "s3", "0"], stdout=open(tmp + "/s3.log", "w"), stderr=subprocess.STDOUT)
        procs.append(s3)
        for _ in range(50):
            if os.path.exists(tmp + "/s3.log") and "listo" in open(tmp + "/s3.log").read():
                break
            time.sleep(0.2)
        s3p = open(tmp + "/s3.log").read().split()[1]
        s3url = "http://127.0.0.1:" + s3p
        env = dict(os.environ, ORE_STORE="r2", ORE_R2_S3_ENDPOINT=s3url, ORE_R2_BUCKET="copia",
                   ORE_R2_ACCESS_KEY_ID="de", ORE_R2_SECRET_ACCESS_KEY="mentira", PATH=BIN + os.pathsep + os.environ["PATH"],
                   FICHEROS_DIR=tmp + "/semilla/datos", LAGO_URL="s3://copia", FORJA_TOKEN="no-hace-falta", ORE_RETENCION="7d")
        forja = tmp + "/arbol.git"
        git("init", "-q", "--bare", "-b", "main", forja)
        git("clone", "-q", forja, tmp + "/semilla")
        arbol_semilla(tmp + "/semilla")
        git("add", "-A", cwd=tmp + "/semilla"); git("commit", "-qm", "semilla", cwd=tmp + "/semilla"); git("push", "-q", "origin", "HEAD:main", cwd=tmp + "/semilla")
        puerto = puerto_libre()
        base = "http://127.0.0.1:%d" % puerto
        srv = subprocess.Popen([SERVE, "--forja", "file://" + forja, "--ore", ORE, "--bind", "127.0.0.1:%d" % puerto,
                                "--identidad", "cabecera", "--no-es-produccion", "--organizacion", "lago"], env=env, stdout=open(tmp + "/serve.log", "w"), stderr=subprocess.STDOUT)
        procs.append(srv)
        for _ in range(80):
            try:
                if pide(base, "GET", "/salud")[0] == 200:
                    break
            except Exception:
                pass
            time.sleep(0.25)

        def pyice(que, *args, sujeto="persona:ana"):
            r = subprocess.run([PY, tmp + "/pyice.py", base, sujeto, que, *[str(a) for a in args]], capture_output=True, text=True, encoding="utf-8", env=env)
            try:
                return json.loads(r.stdout.strip().splitlines()[-1])
            except Exception:
                return {"error": (r.stderr.strip().splitlines() or ["?"])[-1][:200]}

        def duck(que, *args):
            r = subprocess.run([PY, tmp + "/duck.py", base, s3url, que, *[str(a) for a in args]], capture_output=True, text=True, encoding="utf-8", env=env)
            try:
                return json.loads(r.stdout.strip().splitlines()[-1])
            except Exception:
                return {"error": (r.stderr.strip().splitlines() or ["?"])[-1][:200]}

        open(tmp + "/pyice.py", "w", encoding="utf-8").write(PYICEBERG)
        open(tmp + "/duck.py", "w", encoding="utf-8").write(DUCK)

        miradas = [0]

        def mira():
            # un clon nuevo cada vez: en Windows, rmtree de un .git deja restos y el clon siguiente no se hace
            miradas[0] += 1
            d = "%s/mira%d" % (tmp, miradas[0])
            git("clone", "-q", forja, d)
            return d

        # ── lo que escribe alguien: ventas.salida, 1000 filas, por el catálogo ──
        r = pyice("crear", "ventas", "salida", 1000)
        fila("ventas.salida nace por el catálogo (PyIceberg)", "%s ms" % r.get("ms"), "%s filas · %s snapshot" % (r.get("filas"), r.get("snapshots")))

        # ── §1 · la ficha ───────────────────────────────────────────────────
        if "1" in SOLO:
            print()
            print("§1 · la ficha del dataset: lo que ore-serve da y lo que la consola pinta")
            t0 = time.time(); c, t = pide(base, "GET", "/datasets/ventas/salida"); m = ms(t0)
            f = json.loads(t) if c == 200 else {}
            fila("GET /datasets/ventas/salida", "HTTP %d · %d ms · %d B" % (c, m, len(t)), "campos: " + ", ".join(sorted(f.keys()))[:120])
            if f:
                s0 = (f.get("snapshots") or [{}])[0]
                fila("  un snapshot", "", "campos: " + ", ".join(sorted(s0.keys()))[:120])
                fila("  esquema / retención / escrito_por", "%d columnas" % len(f.get("esquema") or f.get("columnas") or []), "%s · %s" % (f.get("retencion"), f.get("escrito_por")))
            t0 = time.time(); c, t = pide(base, "GET", "/datasets"); m = ms(t0)
            l = json.loads(t) if c == 200 else {}
            items = l if isinstance(l, list) else l.get("datasets", l)
            fila("GET /datasets (la lista)", "HTTP %d · %d ms" % (c, m), ("%d entradas · campos: " % len(items) + ", ".join(sorted(items[0].keys()))[:100]) if isinstance(items, list) and items else t[:100])
            t0 = time.time(); c, t = pide(base, "GET", "/paquetes/ventas/copias"); m = ms(t0)
            fila("GET /paquetes/ventas/copias (lo que la consola lee)", "HTTP %d · %d ms" % (c, m), ("salida aparece" if "salida" in t else "salida NO aparece") + " · " + t[:90])
            # lo que la consola pinta hoy en la ficha de un objeto
            consola = "C:/rubix-platform/components/catalog/ItemDetailsTab.tsx"
            if os.path.exists(consola):
                src = open(consola, encoding="utf-8").read()
                etiquetas = [l.split(">")[1].split("<")[0] for l in src.splitlines() if "detailsLabel" in l and ">" in l]
                fila("la consola: ItemDetailsTab pinta", "%d campos" % len(etiquetas), ", ".join(etiquetas)[:120])
                fila("  y dice de sí misma", "", "«namespace físico, snapshot, freshness y governance salen de un endpoint /item que acá no hay»")

        # ── §2 · el gobierno de lo escrito ───────────────────────────────────
        if "2" in SOLO:
            print()
            print("§2 · el gobierno: la Table que nace de write(), etiquetada a mano, y quién escribe encima")
            c, t = pide(base, "GET", "/arbol/packages/ventas/tables/salida.yaml")
            doc = json.loads(t) if c == 200 else {}
            texto = doc.get("texto") or ""
            commit = doc.get("cabeza") or doc.get("commit")
            claves = [l.strip().split(":")[0] for l in texto.splitlines() if l.startswith("  ") and not l.startswith("    ") and ":" in l]
            fila("la Table que nació", "spec: " + ", ".join(claves)[:60], "owner: %s · labels: %s" % ("sí" if "owner:" in texto else "no", "sí" if "labels" in texto else "no"))
            # (a) ana etiqueta la Table a mano: OOS dice que una tabla es un hecho y no lleva `labels` (OOS1005)
            etiquetado = texto.replace("total: { type: Decimal }", "total: { type: Decimal, labels: { gdpr.sensitivity: high } }")
            c, t = pide(base, "PUT", "/arbol/packages/ventas/tables/salida.yaml", etiquetado, {"if-match": str(commit or "")})
            d0 = ((json.loads(t).get("diagnosticos") or [{}])[0] if c == 422 else {}) or {}
            fila("ana etiqueta la Table a mano (labels en `total`)", "HTTP %d" % c, (str(d0.get("codigo", "")) + " " + str(d0.get("mensaje", "")))[:110] if d0 else ("aceptado (commit %s)" % json.loads(t).get("commit") if c in (200, 201) else t[:100]))
            # (b) y una descripción en metadata; las dos cosas, ¿sobreviven a la regeneración?
            c, t = pide(base, "GET", "/arbol/packages/ventas/tables/salida.yaml")
            doc = json.loads(t) if c == 200 else {}
            texto, commit = doc.get("texto") or texto, doc.get("cabeza") or doc.get("commit")
            descrito = texto.replace("metadata: { name: salida, namespace: ventas }", "metadata: { name: salida, namespace: ventas, description: \"lo que ana escribió\" }")
            c, t = pide(base, "PUT", "/arbol/packages/ventas/tables/salida.yaml", descrito, {"if-match": str(commit or "")})
            fila("ana le pone `description` a la Table", "HTTP %d" % c, t[:90] if c not in (200, 201) else "aceptado")
            # (c) el gobierno que OOS SÍ tiene: la View encima y una Entity `backedBy` esa View con la etiqueta; el conducto decide
            m = mira()
            os.makedirs(m + "/packages/ventas/views", exist_ok=True); os.makedirs(m + "/packages/ventas/entities", exist_ok=True)
            open(m + "/packages/ventas/views/totales.yaml", "w").write('apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: totales, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { id: id, total: total, cuando: cuando }\n  materialized: { datasource: lago, table: "cache.totales" }\n')
            c, s = corre([ORE, "validate", m], env)
            fila("una View materializada sobre la Table escrita, sin Entity", "código %d" % c, s[:110] or "sin diagnósticos")
            open(m + "/packages/ventas/entities/Venta.yaml", "w").write('apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: Venta, namespace: ventas }\nspec:\n  nature: entity\n  primaryKey: [id]\n  backedBy: totales\n  properties:\n    id: { type: Integer }\n    total: { type: Decimal, labels: { gdpr.sensitivity: high } }\n')
            c, s = corre([ORE, "validate", m], env)
            fila("  y con una Entity (`nature: entity`) `backedBy: totales`, `total: high`", "código %d" % c, s[:150] or "sin diagnósticos")
            # la Table escrita «solo anexa» (changes.mode: append): la entidad tiene que ser un hecho ocurrido
            open(m + "/packages/ventas/entities/Venta.yaml", "w").write('apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: Venta, namespace: ventas }\nspec:\n  nature: event\n  primaryKey: [id]\n  timeKey: cuando\n  backedBy: totales\n  properties:\n    id: { type: Integer }\n    cuando: { type: DateTimeTz }\n    total: { type: Decimal, labels: { gdpr.sensitivity: high } }\n')
            c, s = corre([ORE, "validate", m], env)
            fila("  y como `nature: event` con `timeKey: cuando`", "código %d" % c, s[:150] or "sin diagnósticos")
            c, s = corre([ORE, "materialize", m, "--vista", "ventas.totales", "--seco"], env)
            fila("  ore materialize --seco de esa View", "código %d" % c, s[:120])
            # la escritura siguiente evoluciona el esquema y regenera el documento
            r = pyice("evolucionar", "ventas", "salida")
            fila("ana escribe con una columna nueva (`nota`)", "", "columnas: %s · snapshots: %s" % (r.get("columnas"), r.get("snapshots")))
            c, t = pide(base, "GET", "/arbol/packages/ventas/tables/salida.yaml")
            texto2 = (json.loads(t) if c == 200 else {}).get("texto", "")
            fila("  el documento regenerado", "nota: %s" % ("sí" if "nota:" in texto2 else "no"), "labels: %s · description: %s" % ("sobreviven" if "labels" in texto2 else "SE PIERDEN", "sobrevive" if "lo que ana escribió" in texto2 else "SE PIERDE"))
            # quién escribe encima: bob pide la credencial prestada para la tabla de ana, y anexa
            r = pyice("credencial", "ventas", "salida", sujeto="persona:bob")
            fila("bob pide la credencial prestada de ventas.salida", "HTTP %s" % r.get("codigo", r.get("error", "")), "config: %s · %s credenciales" % (r.get("config"), r.get("credenciales")))
            r = pyice("anexar", "ventas", "salida", 10, sujeto="persona:bob")
            fila("bob anexa 10 filas a la tabla de ana", "HTTP %s" % r.get("codigo"), (r.get("error") or "%s filas" % r.get("filas")))
            c, t = pide(base, "GET", "/datasets/ventas/salida")
            f = json.loads(t) if c == 200 else {}
            c, h = pide(base, "GET", "/arbol/historia/datasets/ventas_salida.json")
            autores = [v.get("autor") for v in (json.loads(h) if c == 200 else {}).get("versiones", [])][:3]
            fila("  escrito_por / los últimos commits del puntero", "%s" % f.get("escrito_por"), "%s" % autores)
            src = open(RAIZ + "/crates/ore-serve/src/puestos.rs", encoding="utf-8").read()
            fila("datos_de (lo que over() resuelve) consulta una concesión", "no" if "concesion" not in src[src.index("fn datos_de"):] else "sí", "quien tiene puesto en el inquilino lee cualquier dataset")

        # ── §3 · upsert ─────────────────────────────────────────────────────
        if "3" in SOLO:
            print()
            print("§3 · upsert: PyIceberg upsert() contra ore-serve, DuckDB mutando, y el coste copy-on-write")
            r = pyice("crear", "ventas", "ups", 1000)
            fila("ventas.ups nace", "%s ms" % r.get("ms"), "1000 filas")
            r = pyice("upsert", "ventas", "ups", 995, 10)
            fila("upsert de 10 (5 cambian, 5 nuevas)", "%s ms" % r.get("ms"), "actualizadas %s · insertadas %s · filas %s · PT %s" % (r.get("actualizadas"), r.get("insertadas"), r.get("filas"), r.get("pt")) if "error" not in r else r["error"])
            for op, p in r.get("snapshots", []):
                fila("  snapshot", op, json.dumps(p))
            d = duck("leer", "ups")
            fila("DuckDB lee lo que dejó el upsert", "%s ms" % d.get("ms"), "filas %s · PT %s · suma %s" % (d.get("filas"), d.get("pt"), d.get("suma")) if "error" not in d else d["error"])
            c, t = pide(base, "GET", "/datasets/ventas/ups")
            f = json.loads(t) if c == 200 else {}
            fila("la ficha tras el upsert", "%d snapshots" % len(f.get("snapshots", [])), ", ".join("%s(%s)" % (s.get("operacion"), s.get("filas")) for s in f.get("snapshots", []))[:100])
            # el mantenimiento: la tabla nació con 7d (ORE_RETENCION); `--recoger --edad 0` la obedece a ella
            m = mira()
            c, s = corre([ORE, "datasets", m, "--recoger", "--edad", "0"], env)
            fila("ore datasets --recoger --edad 0 (la tabla dice 7d)", "código %d" % c, s[-150:])
            # PyIceberg declara retención 0 por `set-properties` (lo que un cliente de fuera puede hacer), y el mantenimiento la obedece
            r = pyice("retencion", "ventas", "ups", 0)
            fila("PyIceberg pone history.expire.max-snapshot-age-ms=0", "", str(r.get("propiedades", r.get("error")))[:100])
            m = mira()
            c, s = corre([ORE, "datasets", m, "--recoger"], env)
            git("add", "-A", ".", cwd=m); git("commit", "-qm", "mantenimiento", cwd=m); git("push", "-q", "origin", "HEAD:main", cwd=m)
            fila("  ore datasets --recoger (sin --edad) y push", "código %d" % c, s[-150:])
            d = duck("leer", "ups")
            fila("  y DuckDB sigue leyendo", "%s ms" % d.get("ms"), "filas %s · PT %s" % (d.get("filas"), d.get("pt")) if "error" not in d else d["error"])
            c, t = pide(base, "GET", "/datasets/ventas/ups")
            f = json.loads(t) if c == 200 else {}
            c, h = pide(base, "GET", "/arbol/historia/datasets/ventas_ups.json")
            vs = (json.loads(h) if c == 200 else {}).get("versiones", [])
            fila("  la ficha", "%d snapshots" % len(f.get("snapshots", [])), ", ".join("%s(%s)" % (s.get("operacion"), s.get("filas")) for s in f.get("snapshots", []))[:60] + " · puntero: %d versiones, la última «%s» de %s" % (len(vs), (vs[0].get("mensaje") if vs else ""), (vs[0].get("autor") if vs else "")))
            # DuckDB muta por el catálogo: UPDATE, DELETE, MERGE — ¿qué deja, y quién lo lee?
            d = duck("mutar", "ups")
            for k in ("update", "delete", "merge"):
                fila("DuckDB %s sobre la tabla del catálogo" % k.upper(), "", str(d.get(k, d.get("error")))[:110])
            fila("  lo que DuckDB lee después", "", "filas %s · id1 %s · id2 %s · id3 %s" % (d.get("filas"), d.get("id1"), d.get("id2"), d.get("id3")))
            r = pyice("leer", "ventas", "ups")
            fila("  lo que PyIceberg lee después", "", ("filas %s · id1 %s · id2 %s · id3 %s" % (r.get("filas"), r.get("id1"), r.get("id2"), r.get("id3"))) if "error" not in r else r["error"])
            c, t = pide(base, "GET", "/datasets/ventas/ups")
            f = json.loads(t) if c == 200 else {}
            fila("  la ficha (ore-store historia)", "HTTP %d · %d snapshots" % (c, len(f.get("snapshots", []))), ", ".join("%s(%s)" % (s.get("operacion"), s.get("filas")) for s in f.get("snapshots", []))[:100] if c == 200 else t[:100])
            r = pyice("deletes", "ventas", "ups")
            fila("  lo que DuckDB dejó (PyIceberg inspect)", "formato v%s" % r.get("version"), json.dumps(r.get("delete_files", r.get("error")))[:160])
            r = subprocess.run([STORE, "leer"], input=json.dumps({"metadata_location": f.get("metadata_location", ""), "dataset": "ventas_ups"}) + "\n", capture_output=True, text=True, encoding="utf-8", env=env)
            fila("  ore-store leer (iceberg-rust)", "código %d" % r.returncode, ("%d filas" % (r.stdout.count("\n"))) if r.returncode == 0 else r.stderr.strip()[-120:])
            # el coste copy-on-write sobre 1 M
            r = pyice("crear", "ventas", "grande", FILAS)
            fila("ventas.grande nace (PyIceberg)", "%s ms" % r.get("ms"), "%s filas" % r.get("filas"))
            r = pyice("upsert", "ventas", "grande", 500, 1000)
            fila("PyIceberg upsert de 1000 (500 cambian) sobre %d" % FILAS, "%s ms" % r.get("ms"), ("filas %s · PT %s · " % (r.get("filas"), r.get("pt")) + " ".join("%s:%s/%s" % (op, p.get("added-data-files"), p.get("deleted-data-files")) for op, p in r.get("snapshots", []))[:80]) if "error" not in r else r["error"])
            c, t = pide(base, "GET", "/datasets/ventas/grande")
            f = json.loads(t) if c == 200 else {}
            d = duck("fundir", f.get("metadata_location", ""), 1000, tmp + "/fundido.parquet")
            fila("DuckDB en proceso: leer + anti join + unir + Parquet", "%s ms" % d.get("ms"), ("%s filas" % d.get("filas")) if "error" not in d else d["error"])

        # ── §5 · ore-read-lago ──────────────────────────────────────────────
        if "5" in SOLO:
            print()
            print("§5 · ore-read-lago: una View sobre la Table escrita, hoy; y ore-store leer como lector")
            m = mira()
            os.makedirs(m + "/packages/ventas/views", exist_ok=True)
            open(m + "/packages/ventas/views/porPais.yaml", "w").write('apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: porPais, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.salida }\n  fields: { pais: pais, id: id }\n  materialized: { datasource: lago, table: "cache.porPais" }\n')
            c, s = corre([ORE, "validate", m], env)
            fila("una View materializada sobre ventas.salida compila", "código %d" % c, s[:90] or "sin diagnósticos")
            c, s = corre([ORE, "materialize", m, "--vista", "ventas.porPais", "--seco"], env)
            fila("ore materialize --seco", "código %d" % c, s[:130])
            c, s = corre([ORE, "ask", m, "--vista", "ventas.porPais", "--seco"], env)
            fila("ore ask --seco", "código %d" % c, s[:130])
            src = open(RAIZ + "/crates/ore-cli/src/lector.rs", encoding="utf-8").read()
            fila("lo que un driver recibe", "URL por stdin + objeto", "`s3://copia` + `ventas_salida`: el puntero (metadata_location) está en el árbol, no en la URL")
            # ore-store leer como lector: 1 M de filas por el puntero
            if "3" in SOLO:
                c, t = pide(base, "GET", "/datasets/ventas/grande")
                f = json.loads(t) if c == 200 else {}
                ml = f.get("metadata_location", "")
                if ml:
                    t0 = time.time()
                    r = subprocess.run([STORE_REL, "leer"], input=json.dumps({"metadata_location": ml, "dataset": "ventas_grande"}) + "\n", capture_output=True, text=True, encoding="utf-8", env=env)
                    m1 = ms(t0)
                    lineas = r.stdout.count("\n")
                    fila("ore-store leer de ventas.grande (%d filas, %s)" % (FILAS, "release" if STORE_REL != STORE else "debug"), "%d ms · %d MB" % (m1, len(r.stdout) // 1_000_000), "%d líneas · cabecera + filas en JSON: %s" % (lineas, r.stdout[:70].replace("\n", "⏎")) if r.returncode == 0 else r.stderr[-120:])
                    fila("  una fila", "", r.stdout.splitlines()[1][:110] if r.returncode == 0 and lineas > 1 else "")
            # el driver que ya existe, sobre lo mismo en NDJSON: el protocolo (0008) por stdin/stdout
            os.makedirs(tmp + "/nd", exist_ok=True)
            with open(tmp + "/nd/grande.jsonl", "w") as f:
                for i in range(FILAS):
                    f.write('{"id":"%d","pais":"ES","total":"%d.50","cuando":"2023-11-14 22:13:20+00"}\n' % (i, i))
            t0 = time.time()
            r = subprocess.run([JSONL_REL, "leer"], input=json.dumps({"url": tmp + "/nd", "objeto": "grande", "proyeccion": {"id": "id", "pais": "pais", "total": "total", "cuando": "cuando"}}) + "\n", capture_output=True, text=True, encoding="utf-8", env=env)
            m2 = ms(t0)
            fila("ore-read-jsonl leer de %d filas NDJSON (%s)" % (FILAS, "release" if JSONL_REL != JSONL else "debug"), "%d ms · %d MB" % (m2, len(r.stdout) // 1_000_000), ("%d líneas · %s" % (r.stdout.count("\n"), r.stdout[:70].replace("\n", "⏎"))) if r.returncode == 0 else r.stderr[-120:])

        # ── §4 · la cuenta del puesto (clúster) ──────────────────────────────
        if "4" in SOLO:
            print()
            print("§4 · la cuenta del puesto: sólo leer el bucket, y escribir con el token prestado (t-demo)")
            cuenta_del_puesto()

    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        shutil.rmtree(tmp, ignore_errors=True)


# ══ §4 · en el clúster ═══════════════════════════════════════════════════════
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
NS = "t-demo"
BUCKET = "%s-t-demo-copia" % PROYECTO
GSA = "ore-puesto-medida"
KSA = "puesto-medida"


def k(*args, entrada=None):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    r = subprocess.run(["kubectl", "-n", NS, *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
    return r.returncode, r.stdout, r.stderr


def g(*args):
    r = subprocess.run(["gcloud", *args, "-q"], capture_output=True, text=True, encoding="utf-8", shell=(os.name == "nt"))
    return r.returncode, (r.stdout + r.stderr).strip()


GUION = r'''
import json, os, sys, time, urllib.parse, urllib.request
sys.path.insert(0, "/opt/ore")
import ore, pyarrow as pa
def di(que, **kw): print("### " + json.dumps(dict(que=que, **kw)), flush=True)
d = os.environ["DIRECCION"].rstrip("/")
cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
with urllib.request.urlopen(d + "/realms/rubix/protocol/openid-connect/token", data=datos, timeout=20) as r:
    ore.puesto._cabeceras = {"authorization": "Bearer " + json.load(r)["access_token"]}
ore.puesto.id = ""
_pedir = ore.puesto.pedir
def pedir(metodo, ruta, *a, **kw):
    if ruta.startswith("/puestos//datos/"):
        return _pedir("GET", "/datasets/" + ruta.rsplit("/", 1)[1].replace(".", "/"))
    return _pedir(metodo, ruta, *a, **kw)
ore.puesto.pedir = pedir
# quién soy en el bucket: el token del pod
tok = ore._token_de_google()
def gcs(metodo, objeto, cuerpo=None, token=None):
    r = urllib.request.Request("https://storage.googleapis.com/%s/%s" % (os.environ["BUCKET"], urllib.parse.quote(objeto)), data=cuerpo, method=metodo, headers={"authorization": "Bearer " + (token or tok)})
    try:
        with urllib.request.urlopen(r, timeout=20) as resp: return resp.status
    except urllib.error.HTTPError as e: return e.code
# §1 en demo: la lista y una ficha de verdad
t0 = time.time(); c, l = ore.puesto.pedir("GET", "/datasets"); di("lista", ms=int((time.time() - t0) * 1000), codigo=c, n=len(l) if isinstance(l, list) else len(l.get("datasets", [])))
items = l if isinstance(l, list) else l.get("datasets", [])
uno = (next((i for i in items if i.get("metadata_location")), None) or items[0]) if items else None
ns, nombre = (uno.get("nombre") or "olist.x").split(".") if uno else ("olist", "x")
t0 = time.time(); c, f = ore.puesto.pedir("GET", "/datasets/%s/%s" % (ns, nombre)); di("ficha", ms=int((time.time() - t0) * 1000), codigo=c, dataset="%s.%s" % (ns, nombre), clase=(f or {}).get("clase"), iceberg=bool((f or {}).get("metadata_location")), snapshots=len((f or {}).get("snapshots", [])), filas=(f or {}).get("filas"))
t0 = time.time(); c, cp = ore.puesto.pedir("GET", "/paquetes/%s/copias" % ns); di("copias", ms=int((time.time() - t0) * 1000), codigo=c, n=len(cp) if isinstance(cp, list) else len((cp or {}).get("copias", [])))
# leer con el token del pod (objectViewer)
t0 = time.time()
try:
    j = ore.tabla(ore.over("%s.%s" % (ns, nombre))); di("over_con_viewer", ms=int((time.time() - t0) * 1000), filas=len(j["filas"]), total=j["total"])
except Exception as e:
    di("over_con_viewer", error=str(e)[:160])
# escribir directo con el token del pod: tiene que ser 403
di("pod_escribe_directo", http=gcs("PUT", "ore/v2/datasets/%s_medida_cuenta_py/intruso.txt" % ns, b"x"))
# write() con el token prestado
t = pa.table({"n": pa.array([1, 2, 3], pa.int64()), "letra": pa.array(["a", "b", None], pa.string())})
t0 = time.time()
try:
    e = ore.write(ns + ".medida_cuenta_py", t); di("write_con_prestado", ms=int((time.time() - t0) * 1000), filas=e["filas"], repetida=e["repetida"])
except Exception as ex:
    di("write_con_prestado", error=str(ex)[:200])
# el token prestado: dentro de su prefijo escribe; fuera y encima, no
c, l = ore.puesto.pedir("GET", "/v1/namespaces/%s/tables/medida_cuenta_py" % ns, cabeceras={"X-Iceberg-Access-Delegation": "vended-credentials"})
prestado = (l or {}).get("config", {}).get("gcs.oauth2.token", "")
pref = "ore/v2/datasets/%s_medida_cuenta_py/" % ns
di("prestado", codigo=c, acotado=bool(prestado), dentro=gcs("PUT", pref + "data/prueba.txt", b"x", prestado), fuera=gcs("PUT", "ore/v2/datasets/%s_%s/intruso.txt" % (ns, nombre), b"x", prestado), borrar=gcs("DELETE", pref + "data/prueba.txt", None, prestado), leer_otra=gcs("GET", "ore/v2/datasets/%s_%s/" % (ns, nombre), None, prestado))
t0 = time.time()
try:
    j = ore.tabla(ore.over(ns + ".medida_cuenta_py")); di("over_lo_escrito", ms=int((time.time() - t0) * 1000), filas=j["filas"])
except Exception as e:
    di("over_lo_escrito", error=str(e)[:160])
# retirar lo escrito del árbol
for ruta in ("packages/%s/tables/medida_cuenta_py.yaml" % ns, "datasets/%s_medida_cuenta_py.json" % ns):
    c, r = ore.puesto.pedir("DELETE", "/arbol/" + ruta); di("retirado", ruta=ruta, codigo=c)
di("fin", ns=ns)
'''


def cuenta_del_puesto():
    sha = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=RAIZ).stdout.strip()[:12]
    correo = "%s@%s.iam.gserviceaccount.com" % (GSA, PROYECTO)
    nombre = "cuenta-" + sha[:8]
    creado = {"gsa": False, "ksa": False}
    try:
        # ── la cuenta temporal: sólo leer el bucket, y los dos secretos del agente ──
        c, s = g("iam", "service-accounts", "create", GSA, "--display-name", "medida: la cuenta del puesto (temporal)")
        creado["gsa"] = c == 0 or "already exists" in s
        fila("cuenta %s" % GSA, "creada" if c == 0 else s[:60])
        for _ in range(6):
            c, s = g("iam", "service-accounts", "add-iam-policy-binding", correo, "--role=roles/iam.workloadIdentityUser", "--member=serviceAccount:%s.svc.id.goog[%s/%s]" % (PROYECTO, NS, KSA))
            if c == 0:
                break
            time.sleep(5)
        fila("  enlace WI ← %s/%s" % (NS, KSA), "ok" if c == 0 else s[:80])
        c, s = g("storage", "buckets", "add-iam-policy-binding", "gs://" + BUCKET, "--member=serviceAccount:" + correo, "--role=roles/storage.objectViewer")
        fila("  objectViewer sobre el bucket de demo", "ok" if c == 0 else s[:80], "y nada más")
        for sec in ("cliente", "secreto"):
            c, s = g("secrets", "add-iam-policy-binding", "t-demo-agente-" + sec, "--member=serviceAccount:" + correo, "--role=roles/secretmanager.secretAccessor")
        fila("  secretAccessor de los dos secretos del agente", "ok" if c == 0 else s[:80])
        ksa = {"apiVersion": "v1", "kind": "ServiceAccount", "metadata": {"name": KSA, "namespace": NS, "annotations": {"iam.gke.io/gcp-service-account": correo}}}
        c, o, e = k("apply", "-f", "-", entrada=json.dumps(ksa)); creado["ksa"] = c == 0
        fila("  KSA %s" % KSA, "ok" if c == 0 else e[:80])
        # ── el Job: un puesto python con ESA cuenta ──
        cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": NS}, "data": {"medida.py": GUION}}
        job = {"apiVersion": "batch/v1", "kind": "Job", "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": "demo", "ore.dev/rol": "puesto"}},
               "spec": {"backoffLimit": 4, "ttlSecondsAfterFinished": 1800, "activeDeadlineSeconds": 900, "template": {"metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": "demo"}}, "spec": {
                   "restartPolicy": "Never", "serviceAccountName": KSA,
                   "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                   "initContainers": [{"name": "traer-el-testigo", "image": REGISTRO + "/ore-drivers:main", "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                                       "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}], "command": ["/bin/sh", "-c"],
                                       "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=t-demo-agente-$p --out-file=/puesto/agente-$p; chmod 0444 /puesto/agente-$p; done"],
                                       "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}}, "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}}}],
                   "containers": [{"name": "python", "image": "%s/puesto-python:%s" % (REGISTRO, sha), "imagePullPolicy": "Always",
                                   "env": [{"name": "HOME", "value": "/tmp"}, {"name": "ORE_SERVE", "value": "http://ore-serve.%s.svc.cluster.local:8080" % NS}, {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"}, {"name": "BUCKET", "value": BUCKET}, {"name": "ORE_CELDAS", "value": "/trabajo/celdas"}],
                                   "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
                                   "workingDir": "/trabajo", "command": ["python3", "/guiones/medida.py"],
                                   "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "3Gi"}},
                                   "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}}}]}}}}
        k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
        fila("  (90 s: que IAM propague la cuenta y el enlace; el Job reintenta el init 4 veces)")
        time.sleep(90)
        c, o, e = k("apply", "-f", "-", entrada=json.dumps(cm) + "\n---\n" + json.dumps(job))
        if c != 0:
            fila("el Job", "no se creó", e[:120]); return
        t0 = time.time(); estado = "plazo"
        while time.time() - t0 < 900:
            c, o, _ = k("get", "job", nombre, "-o", "jsonpath={.status.succeeded}/{range .status.conditions[*]}{.type}={.status} {end}")
            ok, cond = (o.split("/") + ["", ""])[:2]
            if ok == "1":
                estado = "ok"; break
            if "Failed=True" in cond:
                estado = "falló"; break
            time.sleep(10)
        fila("el Job %s (imagen puesto-python:%s)" % (nombre, sha), estado, "%d s" % int(time.time() - t0))
        c, log, err = k("logs", "job/" + nombre, "-c", "python")
        if c != 0 or not log.strip():
            fila("  (sin log)", "kubectl logs %d" % c, err[:150])
            c2, o2, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "jsonpath={range .items[*]}{.metadata.name} {.status.phase} {.status.containerStatuses[*].state}{end}")
            fila("  (pods)", "", o2[:200])
        ns = ""
        for l in log.splitlines():
            if l.startswith("### "):
                e = json.loads(l[4:]); q = e.pop("que")
                if q == "fin":
                    ns = e["ns"]; continue
                fila("  " + q, "%s ms" % e.pop("ms") if "ms" in e else "", json.dumps(e, ensure_ascii=False)[:120])
        if estado != "ok":
            print(log[-1500:])
            c2, o2, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "name")
            for pod in o2.split():
                c3, l3, _ = k("logs", pod, "-c", "traer-el-testigo", "--tail=6")
                print(pod, "· init:", " ".join(l3.split())[:300])
        if ns:
            c, s = g("storage", "rm", "-r", "gs://%s/ore/v2/datasets/%s_medida_cuenta_py/" % (BUCKET, ns))
            fila("  retirado del bucket", "ore/v2/datasets/%s_medida_cuenta_py/" % ns, "ok" if c == 0 else s[:60])
        if not os.environ.get("SIN_LIMPIAR"):
            k("delete", "job", nombre, "--ignore-not-found"); k("delete", "configmap", nombre, "--ignore-not-found")
    finally:
        # ── se retira todo lo temporal ──
        if creado["ksa"]:
            k("delete", "sa", KSA, "--ignore-not-found")
        if creado["gsa"]:
            g("storage", "buckets", "remove-iam-policy-binding", "gs://" + BUCKET, "--member=serviceAccount:" + correo, "--role=roles/storage.objectViewer")
            for sec in ("cliente", "secreto"):
                g("secrets", "remove-iam-policy-binding", "t-demo-agente-" + sec, "--member=serviceAccount:" + correo, "--role=roles/secretmanager.secretAccessor")
            c, s = g("iam", "service-accounts", "delete", correo)
            fila("la cuenta temporal y el KSA", "retirados" if c == 0 else s[:80])
        c, o, _ = k("get", "jobs", "-o", "name")
        fila("jobs de esta medida que quedan", "ninguno" if "cuenta-" not in o else o)


if __name__ == "__main__":
    main()
