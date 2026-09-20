#!/usr/bin/env python3
"""
MEDIDA · W3.6c · escribir (20 de septiembre), antes de la spec del verbo.

Lo mirado (`docs/investigacion/w3-escribir-estado-del-arte.md`) deja dos ideas y
tres correcciones. Antes de escribir la spec, se miden con lo que ya existe
(ore-serve + `confirmar` de W3.6b, ore-store de W3.6a, PyIceberg, DuckDB, Node)
y un catálogo REST de Iceberg DE MENTIRA de cien líneas que hace de la cara que
ore-serve tendría: carga la tabla por el puntero y confirma por `confirmar`.

  §1  EL CATÁLOGO REST   PyIceberg y DuckDB contra el catálogo de mentira:
                         ¿escriben SIN parches? qué rutas piden, cuántas
                         peticiones por escritura, qué `requirements` y
                         `updates` mandan, qué cabeceras (delegación,
                         Idempotency-Key), qué claves dejan en el resumen del
                         snapshot, cuánto tarda el commit (el swap de W3.6b
                         detrás). La tabla nace en el árbol con la primera
                         escritura (columnas de Iceberg → OOS).
  §1b LA CARRERA Y EL 5xx dos manos con la misma base: ¿409 y quién reintenta?
                         y un 502 DESPUÉS de confirmar: ¿qué hace el cliente,
                         y qué dice el catálogo cuando se le pregunta?
  §2  EL TOKEN ACOTADO   Credential Access Boundary de GCS sobre el bucket de
                         prueba: un token acotado a `ore/v2/datasets/<p>_<t>/`
                         escribe dentro, no escribe fuera, no borra; PyIceberg
                         lo acepta como credencial prestada (`gcs.oauth2.token`)
                         por el catálogo; y qué puede DuckDB con él.
  §3  NODE SIN ESCRITOR  la tabla Arrow por IPC (stdin) a un escritor de Rust
                         (iceberg-rust, lo que ore-store es): 10 M filas, contra
                         el camino de hoy (filas JSON a `ore-store sellar`) y
                         contra PyIceberg desde Python.
  §4  LA RETENCIÓN       `history.expire.*` como propiedades de la tabla: ¿llegan
                         al metadata.json? ¿las ve `ore datasets`? ¿las obedece
                         `--recoger`?

Uso:  python pruebas-de-fuego/medida-w3-escribir.py [--sin-gcs] [--filas 10000000]
Necesita target/debug (ore, ore-serve, ore-store-r2, ore-read-jsonl), el crate
desechable `medida-w3-iceberg-rust` compilado en release (`ipc`), pyiceberg,
duckdb, pyarrow, node con apache-arrow (`--nodo <dir con node_modules>`), git y
gcloud (sesión propia; §2 usa el bucket de PRUEBA y lo limpia al final). No
toca el clúster.
"""
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__))).replace("\\", "/")
BIN = RAIZ + "/target/debug"
EXE = ".exe" if os.name == "nt" else ""
ORE = "%s/ore%s" % (BIN, EXE)
SERVE = "%s/ore-serve%s" % (BIN, EXE)
STORE = "%s/ore-store-r2%s" % (BIN, EXE)
IPC = RAIZ + "/pruebas-de-fuego/medida-w3-iceberg-rust/target/release/ipc" + EXE
PY = sys.executable
SIN_GCS = "--sin-gcs" in sys.argv
FILAS = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 10_000_000
NODO = sys.argv[sys.argv.index("--nodo") + 1] if "--nodo" in sys.argv else ""
PRUEBA = "project-8853a180-450d-47be-b83-t-prueba-copia"

espec = importlib.util.spec_from_file_location("swap", RAIZ + "/pruebas-de-fuego/medida-w3-swap.py")
swap = importlib.util.module_from_spec(espec)
espec.loader.exec_module(swap)
fila, ms, git, pide, puerto_libre = swap.fila, swap.ms, swap.git, swap.pide, swap.puerto_libre

import pyarrow as pa  # noqa: E402
from pyiceberg.catalog.rest import CommitTableRequest  # noqa: E402
from pyiceberg.exceptions import CommitFailedException  # noqa: E402
from pyiceberg.partitioning import PartitionSpec  # noqa: E402
from pyiceberg.schema import Schema  # noqa: E402
from pyiceberg.table.metadata import TableMetadataUtil, new_table_metadata  # noqa: E402
from pyiceberg.table.sorting import SortOrder  # noqa: E402
from pyiceberg.table.update import update_table_metadata  # noqa: E402


# ═════════════════════════════════════════════════════════════════════════════
# EL CATÁLOGO REST DE MENTIRA — la cara que ore-serve tendría
#
# El puntero vive en el árbol (ore-serve `GET /datasets/{ns}/{n}`); el
# `metadata.json` en el bucket; el commit es `requirements` + `updates`
# aplicados aquí (con PyIceberg; en ore-serve lo hace `ore-store confirmar`,
# que ya aplica `TableUpdate`) y el swap por `POST …/confirmar` (W3.6b).
# ═════════════════════════════════════════════════════════════════════════════
CAT = {"serve": "", "s3": "", "bucket": "copia", "modo": "s3", "gcs_bucket": PRUEBA, "token_pleno": "",
       "token_acotado": "", "registro": [], "romper_502": False, "punteros": {}, "raiz": "s3://copia/ore/v2"}
OOS = {"long": "Integer", "int": "Integer", "string": "String", "boolean": "Boolean", "double": "Float", "float": "Float",
       "date": "Date", "time": "Time", "timestamp": "DateTime", "timestamptz": "DateTimeTz"}


def oos_de(tipo):
    t = str(tipo)
    if t.startswith("decimal"):
        return "Decimal"
    return OOS.get(t, "String")


def clave_de(ml):
    # s3://copia/x/y → x/y ; gs://bucket/x/y → x/y
    return ml.split("://", 1)[1].split("/", 1)[1]


def http(metodo, url, cuerpo=None, cabeceras=None):
    r = urllib.request.Request(url, data=cuerpo, method=metodo)
    for k, v in (cabeceras or {}).items():
        r.add_header(k, v)
    try:
        with urllib.request.urlopen(r, timeout=120) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def objeto_leer(ml):
    k = clave_de(ml)
    if CAT["modo"] == "s3":
        c, b = http("GET", "%s/%s/%s" % (CAT["s3"], CAT["bucket"], k))
    else:
        c, b = http("GET", "https://storage.googleapis.com/storage/v1/b/%s/o/%s?alt=media" % (CAT["gcs_bucket"], urllib.parse.quote(k, safe="")),
                    cabeceras={"authorization": "Bearer " + CAT["token_pleno"]})
    if c != 200:
        raise RuntimeError("no se pudo leer %s: %d" % (ml, c))
    return b


def objeto_escribir(ml, datos):
    k = clave_de(ml)
    if CAT["modo"] == "s3":
        c, _ = http("PUT", "%s/%s/%s" % (CAT["s3"], CAT["bucket"], k), datos, {"If-None-Match": "*"})
    else:
        c, _ = http("POST", "https://storage.googleapis.com/upload/storage/v1/b/%s/o?uploadType=media&name=%s&ifGenerationMatch=0" % (CAT["gcs_bucket"], urllib.parse.quote(k, safe="")),
                    datos, {"authorization": "Bearer " + CAT["token_pleno"], "content-type": "application/json"})
    if c not in (200, 201):
        raise RuntimeError("no se pudo escribir %s: %d" % (ml, c))


def documento(base, ruta):
    c, b = pide(base, "GET", "/arbol/" + ruta)
    if c != 200:
        return c, b[:80]
    try:
        j = json.loads(b)
        txt = j.get("texto", b)
    except Exception:
        txt = b
    return c, " ".join(l.strip() for l in txt.splitlines() if "type" in l or "datasource" in l or "columns" in l)[:95]


def puntero_de(ns, n):
    """El puntero del árbol (por ore-serve) o, en modo gcs, el de memoria."""
    if CAT["modo"] == "gcs":
        return CAT["punteros"].get((ns, n))
    c, b = pide(CAT["serve"], "GET", "/datasets/%s/%s" % (ns, n))
    if c != 200:
        return None
    j = json.loads(b)
    return j if j.get("metadata_location") else None


def confirmar(ns, n, ml, esperado, md, columnas=None):
    """El swap de W3.6b. Devuelve (código, cuerpo)."""
    snap = md.current_snapshot()
    cuerpo = {"metadata_location": ml, "esperado": esperado or ""}
    if snap is not None:
        cuerpo["snapshot"] = str(snap.snapshot_id)
        cuerpo["filas"] = str(snap.summary.get("total-records", "") if snap.summary else "")
    if columnas:
        cuerpo["columnas"] = columnas
    if CAT["modo"] == "gcs":
        actual = CAT["punteros"].get((ns, n), {}).get("metadata_location", "")
        if (esperado or "") != actual:
            return 409, json.dumps({"error": "adelantado", "actual": actual})
        CAT["punteros"][(ns, n)] = {"metadata_location": ml}
        return (201 if not actual else 200), "{}"
    return pide(CAT["serve"], "POST", "/datasets/%s/%s/confirmar" % (ns, n), json.dumps(cuerpo))


def credenciales(ubicacion):
    if CAT["modo"] == "s3":
        cfg = {"s3.endpoint": CAT["s3"], "s3.access-key-id": "de", "s3.secret-access-key": "mentira",
               "s3.path-style-access": "true", "s3.region": "auto"}
    else:
        cfg = {"gcs.oauth2.token": CAT["token_acotado"] or CAT["token_pleno"],
               "gcs.oauth2.token-expires-at": str(int(time.time() * 1000) + 3000_000)}
    return cfg, [{"prefix": ubicacion, "config": cfg}]


class Catalogo(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _json(self, codigo, obj):
        b = json.dumps(obj).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def _error(self, codigo, tipo, msg):
        self._json(codigo, {"error": {"message": msg, "type": tipo, "code": codigo}})
        return True

    def _cuerpo(self):
        n = int(self.headers.get("content-length", "0"))
        return self.rfile.read(n) if n else b""

    def _anota(self, codigo, t0, n, cuerpo=b""):
        req, upd = [], []
        if cuerpo:
            try:
                j = json.loads(cuerpo)
                j = (j.get("table-changes") or [j])[0]
                req = [r.get("type") for r in j.get("requirements", [])]
                upd = [u.get("action") for u in j.get("updates", [])]
                if "stage-create" in j:
                    upd = ["stage-create=%s" % j["stage-create"]]
            except Exception:
                pass
        CAT["registro"].append({"req": req, "upd": upd,
            "metodo": self.command, "ruta": urllib.parse.urlparse(self.path).path, "consulta": urllib.parse.urlparse(self.path).query,
            "delegacion": self.headers.get("X-Iceberg-Access-Delegation", ""), "idem": self.headers.get("Idempotency-Key", ""),
            "agente": self.headers.get("User-Agent", "")[:40], "bytes": n, "codigo": codigo, "ms": ms(t0)})

    def _partes(self):
        p = urllib.parse.urlparse(self.path).path.strip("/").split("/")
        return [urllib.parse.unquote(x) for x in p]

    def do_GET(self):
        t0 = time.time(); p = self._partes()
        try:
            if p[:2] == ["v1", "config"]:
                self._json(200, {"defaults": {}, "overrides": {}})
            elif p == ["v1", "namespaces"]:
                self._json(200, {"namespaces": [["ventas"]]})
            elif len(p) == 3 and p[1] == "namespaces":
                self._json(200, {"namespace": [p[2]], "properties": {}})
            elif len(p) == 4 and p[3] == "tables":
                ns = p[2]
                c, b = pide(CAT["serve"], "GET", "/datasets") if CAT["modo"] == "s3" else (200, "{}")
                nombres = [d for d in (json.loads(b).get("datasets", []) if c == 200 else [])] if CAT["modo"] == "s3" else []
                ids = [{"namespace": [ns], "name": d.split(".", 1)[1]} for d in (x.get("dataset", x) if isinstance(x, dict) else x for x in nombres) if isinstance(d, str) and d.startswith(ns + ".")]
                if CAT["modo"] == "gcs":
                    ids = [{"namespace": [a], "name": b} for (a, b) in CAT["punteros"]]
                self._json(200, {"identifiers": ids})
            elif len(p) == 5 and p[3] == "tables":
                ns, n = p[2], p[4]
                pt = puntero_de(ns, n)
                if not pt:
                    return self._error(404, "NoSuchTableException", "no hay dataset %s.%s" % (ns, n))
                ml = pt["metadata_location"]
                md = json.loads(objeto_leer(ml))
                cfg, cred = credenciales(md.get("location", ""))
                self._json(200, {"metadata-location": ml, "metadata": md, "config": cfg, "storage-credentials": cred})
            else:
                self._error(404, "NotFound", self.path)
        except Exception as e:
            self._error(500, "Internal", str(e))
        finally:
            self._anota(getattr(self, "_codigo", 0), t0, 0)

    def do_HEAD(self):
        t0 = time.time(); p = self._partes()
        if len(p) == 5 and p[3] == "tables" and puntero_de(p[2], p[4]):
            self.send_response(204)
        else:
            self.send_response(404)
        self.send_header("content-length", "0"); self.end_headers()
        self._anota(204, t0, 0)

    def send_response(self, code, message=None):
        self._codigo = code
        super().send_response(code, message)

    def do_POST(self):
        t0 = time.time(); p = self._partes(); cuerpo = self._cuerpo()
        try:
            if len(p) == 4 and p[3] == "tables":
                self._crear(p[2], json.loads(cuerpo))
            elif len(p) == 5 and p[3] == "tables":
                self._commit(p[2], p[4], json.loads(cuerpo))
            elif p == ["v1", "transactions", "commit"]:
                # commitTransaction: varias tablas en un commit (DuckDB lo usa
                # SIEMPRE, también para una). Aquí: una a una; en ore-serve,
                # un solo commit del árbol con N punteros.
                j = json.loads(cuerpo)
                for cambio in j.get("table-changes", []):
                    ident = cambio["identifier"]
                    if self._commit(ident["namespace"][-1], ident["name"], cambio, responder=False):
                        return
                self.send_response(204); self.send_header("content-length", "0"); self.end_headers()
            else:
                self._error(404, "NotFound", self.path)
        except Exception as e:
            import traceback; traceback.print_exc()
            self._error(500, "Internal", str(e))
        finally:
            self._anota(getattr(self, "_codigo", 0), t0, len(cuerpo), cuerpo)

    def _crear(self, ns, cuerpo):
        n = cuerpo["name"]
        if puntero_de(ns, n):
            return self._error(409, "AlreadyExistsException", "ya existe %s.%s" % (ns, n))
        esquema = Schema.model_validate(cuerpo["schema"])
        spec = PartitionSpec.model_validate(cuerpo["partition-spec"]) if cuerpo.get("partition-spec") else PartitionSpec()
        orden = SortOrder.model_validate(cuerpo["write-order"]) if cuerpo.get("write-order") else SortOrder()
        ubicacion = "%s/datasets/%s_%s" % (CAT["raiz"], ns, n)
        md = new_table_metadata(esquema, spec, orden, ubicacion, cuerpo.get("properties") or {})
        ml = "%s/metadata/00000-%s.metadata.json" % (ubicacion, uuid.uuid4())
        objeto_escribir(ml, md.model_dump_json().encode())
        cfg, cred = credenciales(ubicacion)
        if cuerpo.get("stage-create"):
            CAT.setdefault("preparadas", {})[(ns, n)] = (ml, md)
            return self._json(200, {"metadata": json.loads(md.model_dump_json()), "config": cfg, "storage-credentials": cred})
        columnas = {f.name: oos_de(f.field_type) for f in esquema.fields}
        c, b = confirmar(ns, n, ml, "", md, columnas)
        if c not in (200, 201):
            return self._error(409 if c == 409 else 422, "CommitFailedException" if c == 409 else "BadRequestException", b[:300])
        self._json(200, {"metadata-location": ml, "metadata": json.loads(md.model_dump_json()), "config": cfg, "storage-credentials": cred})

    def _commit(self, ns, n, cuerpo, responder=True):
        cuerpo = dict(cuerpo); cuerpo.setdefault("identifier", {"namespace": [ns], "name": n})
        req = CommitTableRequest.model_validate(cuerpo)
        pt = puntero_de(ns, n)
        preparada = CAT.get("preparadas", {}).get((ns, n))
        if not pt and preparada is None:
            return self._error(404, "NoSuchTableException", "no hay dataset %s.%s" % (ns, n))
        if pt:
            ml0 = pt["metadata_location"]
            base = TableMetadataUtil.parse_raw(objeto_leer(ml0))
        else:
            # `stage-create` + commit con `assert-create`: la tabla nace en este commit
            ml0, base = preparada
        try:
            for r in req.requirements:
                # `assert-create` se valida contra «no hay tabla»
                r.validate(base if pt else None)
        except CommitFailedException as e:
            return self._error(409, "CommitFailedException", str(e))
        nuevo = update_table_metadata(base, tuple(req.updates), enforce_validation=False, metadata_location=ml0 if pt else None)
        v = int(os.path.basename(ml0).split("-")[0]) + 1
        ml = "%s/metadata/%05d-%s.metadata.json" % (nuevo.location, v, uuid.uuid4())
        objeto_escribir(ml, nuevo.model_dump_json().encode())
        columnas = None if pt else {f.name: oos_de(f.field_type) for f in nuevo.schema().fields}
        c, b = confirmar(ns, n, ml, ml0 if pt else "", nuevo, columnas)
        if not pt:
            CAT["preparadas"].pop((ns, n), None)
        if c == 409:
            return self._error(409, "CommitFailedException", b[:300])
        if c not in (200, 201):
            return self._error(500, "CommitStateUnknownException", b[:300])
        if CAT["romper_502"]:
            CAT["romper_502"] = False
            return self._error(502, "CommitStateUnknownException", "la puerta se cayó DESPUÉS de confirmar")
        if responder:
            self._json(200, {"metadata-location": ml, "metadata": json.loads(nuevo.model_dump_json())})


def registro_desde(i):
    return CAT["registro"][i:]


def resumen_registro(reg):
    por = {}
    for r in reg:
        k = "%s %s" % (r["metodo"], r["ruta"].replace("/v1/namespaces/ventas", "…/ns").replace("/v1/", "/"))
        por.setdefault(k, []).append(r)
    for k, rs in por.items():
        fila("  " + k, "%d× · %d ms" % (len(rs), sum(r["ms"] for r in rs)), "delegación=%s idem=%s" % (rs[0]["delegacion"] or "·", rs[0]["idem"] or "·"))
        for r in rs:
            if r["req"] or r["upd"]:
                fila("    requirements · updates", "", "%s · %s" % (", ".join(map(str, r["req"])) or "·", ", ".join(map(str, r["upd"])))[:110])


# ═════════════════════════════════════════════════════════════════════════════
def datos(n, desde=0):
    ids = pa.array(range(desde, desde + n), pa.int64())
    pais = pa.array(["ES", "PT", "FR", "DE"][i % 4] for i in range(n))
    total = pa.array([i * 10 + 0.5 for i in range(n)], pa.float64()).cast(pa.decimal128(18, 2))
    cuando = pa.array([1_700_000_000_000_000 + i for i in range(n)], pa.timestamp("us", tz="UTC"))
    return pa.table({"id": ids, "pais": pais, "total": total, "cuando": cuando})


def store(verbo, peticion):
    r = subprocess.run([STORE, verbo], input=json.dumps(peticion) + "\n", capture_output=True, text=True, encoding="utf-8", env=CAT["env"])
    return r.returncode, r.stdout, r.stderr


LEER_DICHO = []


def filas_por_puntero(ns, n):
    """Lo que el árbol + ore-store saben del dataset: filas del snapshot vigente
    por `historia` (por `leer` no: exige la cabecera de ORE, y lo escrito por
    otro escritor no la lleva — se dice una vez)."""
    pt = puntero_de(ns, n)
    pet = {"metadata_location": pt["metadata_location"], "dataset": "datasets/%s_%s" % (ns, n)}
    if not LEER_DICHO:
        c, out, err = store("leer", pet)
        LEER_DICHO.append(1)
        if c != 0:
            fila("  (ore-store leer de lo que escribió otro)", "código %d" % c, err.strip().replace(chr(10), " ")[:100])
    c, out, err = store("historia", pet)
    if c != 0:
        return -1
    h = json.loads(out)
    return int((h.get("snapshots") or [{}])[0].get("filas") or 0)


def s1(cat, base, tmp):
    from pyiceberg.catalog import load_catalog
    print()
    print("§1 · el catálogo REST de mentira delante de ore-serve: PyIceberg y DuckDB escriben")
    catalogo = load_catalog("ore", **{"type": "rest", "uri": cat})
    t = datos(100_000)
    i0 = len(CAT["registro"])
    t0 = time.time()
    tabla = catalogo.create_table(("ventas", "escrita"), schema=t.schema,
                                  properties={"history.expire.max-snapshot-age-ms": "1", "history.expire.min-snapshots-to-keep": "1", "ore.retencion": "1ms"})
    fila("PyIceberg create_table ventas.escrita", "%d ms" % ms(t0), "la Table nace en el árbol por `confirmar --columnas`")
    c, b = documento(base, "packages/ventas/tables/escrita.yaml")
    fila("  el documento en el árbol", "HTTP %d" % c, b)
    resumen_registro(registro_desde(i0))
    i0 = len(CAT["registro"])
    t0 = time.time(); tabla.append(t); a1 = ms(t0)
    fila("PyIceberg append 100 000", "%d ms" % a1, "filas por el puntero: %d" % filas_por_puntero("ventas", "escrita"))
    resumen_registro(registro_desde(i0))
    commit = [r for r in registro_desde(i0) if r["metodo"] == "POST"]
    if commit:
        fila("  el commit (POST tables/escrita)", "%d ms · %d bytes" % (commit[0]["ms"], commit[0]["bytes"]))
    # ¿qué manda? requirements y updates, y las claves del resumen
    md = tabla.metadata
    snap = md.current_snapshot()
    fila("  resumen del snapshot (claves)", "", ", ".join(sorted(snap.summary.additional_properties.keys()) + ["operation"])[:110])
    # la MISMA celda otra vez: ¿dos snapshots iguales?
    t0 = time.time(); tabla.append(t); a2 = ms(t0)
    fila("PyIceberg el MISMO append otra vez", "%d ms" % a2, "snapshots: %d · filas: %d  ← sin clave de operación, se duplica" % (len(tabla.metadata.snapshots), filas_por_puntero("ventas", "escrita")))
    t0 = time.time(); tabla.overwrite(t); a3 = ms(t0)
    fila("PyIceberg overwrite", "%d ms" % a3, "snapshots: %d · filas: %d" % (len(tabla.metadata.snapshots), filas_por_puntero("ventas", "escrita")))
    # el commit del overwrite: qué requirements/updates viajan
    ult = [r for r in CAT["registro"] if r["metodo"] == "POST" and r["ruta"].endswith("/tables/escrita")][-1]
    fila("  el commit del overwrite", "%d ms · %d bytes" % (ult["ms"], ult["bytes"]))

    # ── §1b · la carrera y el 5xx ────────────────────────────────────────────
    print()
    print("§1b · dos manos con la misma base, y la puerta que se cae después de confirmar")
    a = catalogo.load_table(("ventas", "escrita")); b_ = catalogo.load_table(("ventas", "escrita"))
    a.append(datos(10, 1_000_000))
    i0 = len(CAT["registro"])
    try:
        t0 = time.time(); b_.append(datos(10, 2_000_000)); r = "commit OK (%d ms) ← ¿reintentó solo?" % ms(t0)
    except Exception as e:
        r = "%s: %s" % (type(e).__name__, str(e)[:70])
    fila("la segunda mano (base vieja)", "", r)
    posts = [x for x in registro_desde(i0) if x["metodo"] == "POST"]
    fila("  POST del cliente en la segunda mano", "%d" % len(posts), " · ".join("%d" % x["codigo"] for x in posts) + " (¿refresca y reintenta el cliente?)")
    fila("  filas por el puntero", "%d" % filas_por_puntero("ventas", "escrita"), "(100 010 si sólo entró la primera)")
    CAT["romper_502"] = True
    a.refresh()
    try:
        t0 = time.time(); a.append(datos(10, 3_000_000)); r = "sin excepción (%d ms)" % ms(t0)
    except Exception as e:
        r = "%s: %s" % (type(e).__name__, str(e)[:60])
    n = filas_por_puntero("ventas", "escrita")
    fila("502 DESPUÉS de confirmar: el cliente ve", "", r)
    fila("  y el catálogo dice", "%d filas" % n, "← el commit entró; el cliente no lo sabe: tiene que MIRAR")
    try:
        a.append(datos(10, 4_000_000)); r = "commit OK: el cliente refrescó solo"
    except Exception as e:
        r = "%s (la base del cliente es vieja: hay que refrescar antes de reintentar)" % type(e).__name__
    fila("  reintentar sin mirar", "", r)
    a.refresh()

    # ── DuckDB ───────────────────────────────────────────────────────────────
    print()
    print("§1 · DuckDB (extensión iceberg) contra el mismo catálogo")
    import duckdb
    con = duckdb.connect()
    con.execute("load iceberg; load httpfs;")
    con.execute("create secret s3 (type s3, key_id 'de', secret 'mentira', endpoint '%s', url_style 'path', use_ssl false, region 'auto')" % CAT["s3"].replace("http://", ""))
    i0 = len(CAT["registro"])
    try:
        t0 = time.time()
        con.execute("attach '' as lago (type iceberg, endpoint '%s', authorization_type 'none')" % cat)
        fila("DuckDB attach (rest)", "%d ms" % ms(t0), "duckdb %s" % duckdb.__version__)
        resumen_registro(registro_desde(i0))
    except Exception as e:
        fila("DuckDB attach", "✗", str(e)[:100]); return
    i0 = len(CAT["registro"])
    try:
        t0 = time.time()
        n0 = con.execute("select count(*) from lago.ventas.escrita").fetchone()[0]
        fila("DuckDB lee lo que PyIceberg escribió", "%d ms" % ms(t0), "%d filas" % n0)
    except Exception as e:
        fila("DuckDB lee ventas.escrita", "✗", str(e)[:100])
    i0 = len(CAT["registro"])
    try:
        t0 = time.time()
        con.execute("insert into lago.ventas.escrita select i as id, 'IT' as pais, (i*10+0.5)::decimal(18,2) as total, (timestamp '2023-11-14 22:13:20' + interval (i) microsecond)::timestamptz as cuando from range(100000) r(i)")
        fila("DuckDB INSERT 100 000", "%d ms" % ms(t0), "filas por el puntero: %d" % filas_por_puntero("ventas", "escrita"))
        resumen_registro(registro_desde(i0))
        commit = [r for r in registro_desde(i0) if r["metodo"] == "POST"]
        if commit:
            fila("  el commit", "%d ms · %d bytes" % (commit[-1]["ms"], commit[-1]["bytes"]), "idem=%s" % (commit[-1]["idem"] or "·"))
    except Exception as e:
        fila("DuckDB INSERT", "✗", str(e)[:400])
        for r in registro_desde(i0):
            fila("    " + r["metodo"] + " " + r["ruta"], "%d" % r["codigo"], "%d bytes" % r["bytes"])
    i0 = len(CAT["registro"])
    try:
        t0 = time.time()
        con.execute("create table lago.ventas.pato as select i as id, 'PT' as pais from range(10) r(i)")
        fila("DuckDB CREATE TABLE … AS", "%d ms" % ms(t0), "filas por el puntero: %d" % filas_por_puntero("ventas", "pato"))
        c, b = documento(base, "packages/ventas/tables/pato.yaml")
        fila("  el documento en el árbol", "HTTP %d" % c, b)
        resumen_registro(registro_desde(i0))
    except Exception as e:
        fila("DuckDB CREATE TABLE", "✗", str(e).replace(chr(10), " ")[:300])
        for r in registro_desde(i0):
            fila("    " + r["metodo"] + " " + r["ruta"], "%d" % r["codigo"], "%d bytes" % r["bytes"])
    try:
        tabla = catalogo.load_table(("ventas", "escrita"))
        n = tabla.scan().to_arrow().num_rows
        fila("PyIceberg lee lo que DuckDB escribió", "", "%d filas · snapshots: %d" % (n, len(tabla.metadata.snapshots)))
        ult = tabla.metadata.current_snapshot()
        fila("  resumen del snapshot de DuckDB (claves)", "", ", ".join(sorted(ult.summary.additional_properties.keys()))[:110])
    except Exception as e:
        fila("PyIceberg lee", "✗", str(e)[:100])
    con.close()
    # la ficha por ore-serve
    c, b = pide(base, "GET", "/datasets/ventas/escrita")
    if c == 200:
        j = json.loads(b)
        fila("GET /datasets/ventas/escrita", "%d snapshots" % len(j.get("snapshots", [])), "filas=%s · escrito_por=%s" % (j.get("filas"), j.get("escrito_por")))
    return catalogo


def s4(base, tmp):
    print()
    print("§4 · la retención como propiedades de la tabla (history.expire.*)")
    pt = puntero_de("ventas", "escrita")
    md = json.loads(objeto_leer(pt["metadata_location"]))
    props = {k: v for k, v in md.get("properties", {}).items() if "expire" in k or "ore." in k}
    fila("propiedades en el metadata.json", "", json.dumps(props)[:100])
    c, b = pide(base, "GET", "/datasets/ventas/escrita")
    j = json.loads(b) if c == 200 else {}
    fila("¿las enseña la ficha (GET /datasets)?", "sí" if any("expire" in k for k in json.dumps(j)) else "no", "claves: %s" % ", ".join(sorted(j.keys()))[:80])
    clon = tmp + "/clon-ret"
    git("clone", "-q", "file://" + tmp + "/arbol.git", clon)
    r = subprocess.run([ORE, "datasets", clon, "--recoger", "--seco", "--json"], capture_output=True, text=True, encoding="utf-8", env=CAT["env"])
    fila("ore datasets --recoger --seco (sin --edad)", "código %d" % r.returncode, (r.stdout.strip().splitlines() or [r.stderr.strip()[:90]])[-1][:100])
    r = subprocess.run([ORE, "datasets", clon, "--recoger", "--seco", "--edad", "0s", "--json"], capture_output=True, text=True, encoding="utf-8", env=CAT["env"])
    fila("ore datasets --recoger --seco --edad 0s", "código %d" % r.returncode, (r.stdout.strip().splitlines() or [r.stderr.strip()[:90]])[-1][:100])
    fila("  ⇒", "", "la tabla dice 1 ms y nadie la lee: la política vive en el cron, no en la tabla")


def token(*args):
    r = subprocess.run(["gcloud", "auth", "print-access-token", *args], capture_output=True, text=True, shell=(os.name == "nt"))
    return r.stdout.strip()


def acotar(tok, bucket, prefijo, permisos):
    cab = {"accessBoundary": {"accessBoundaryRules": [{
        "availableResource": "//storage.googleapis.com/projects/_/buckets/" + bucket,
        "availablePermissions": permisos,
        "availabilityCondition": {"expression": "resource.name.startsWith('projects/_/buckets/%s/objects/%s')" % (bucket, prefijo)},
    }]}}
    cuerpo = urllib.parse.urlencode({
        "grant_type": "urn:ietf:params:oauth:grant-type:token-exchange",
        "subject_token_type": "urn:ietf:params:oauth:token-type:access_token",
        "requested_token_type": "urn:ietf:params:oauth:token-type:access_token",
        "subject_token": tok, "options": json.dumps(cab)}).encode()
    t0 = time.time()
    c, b = http("POST", "https://sts.googleapis.com/v1/token", cuerpo, {"content-type": "application/x-www-form-urlencoded"})
    j = json.loads(b) if c == 200 else {}
    return c, j.get("access_token", ""), j.get("expires_in"), ms(t0)


def gcs(metodo, tok, nombre, datos=None):
    if metodo == "PUT":
        return http("POST", "https://storage.googleapis.com/upload/storage/v1/b/%s/o?uploadType=media&name=%s" % (PRUEBA, urllib.parse.quote(nombre, safe="")), datos or b"x",
                    {"authorization": "Bearer " + tok, "content-type": "application/octet-stream"})[0]
    return http(metodo, "https://storage.googleapis.com/storage/v1/b/%s/o/%s%s" % (PRUEBA, urllib.parse.quote(nombre, safe=""), "?alt=media" if metodo == "GET" else ""),
                cabeceras={"authorization": "Bearer " + tok})[0]


def s2(tmp):
    from pyiceberg.catalog import load_catalog
    print()
    print("§2 · el token acotado a la tabla (Credential Access Boundary) en el bucket de prueba")
    pleno = token()
    if not pleno:
        fila("gcloud", "✗", "sin sesión"); return
    pref = "ore/v2/datasets/ventas_escrita/"
    c, acotado, exp, t = acotar(pleno, PRUEBA, pref, ["inRole:roles/storage.objectCreator", "inRole:roles/storage.objectViewer"])
    fila("STS token-exchange con accessBoundary", "HTTP %d · %d ms" % (c, t), "objectCreator+objectViewer sólo bajo %s" % pref)
    if c != 200:
        return
    fila("  PUT dentro del prefijo", "HTTP %d" % gcs("PUT", acotado, pref + "sonda.txt"))
    fila("  GET dentro del prefijo", "HTTP %d" % gcs("GET", acotado, pref + "sonda.txt"))
    fila("  PUT fuera (otra tabla)", "HTTP %d" % gcs("PUT", acotado, "ore/v2/datasets/otra_tabla/sonda.txt"), "← 403 esperado")
    fila("  PUT fuera (copias/)", "HTTP %d" % gcs("PUT", acotado, "ore/v2/copias/ventas_copia/sonda.txt"), "← 403 esperado")
    fila("  DELETE dentro", "HTTP %d" % gcs("DELETE", acotado, pref + "sonda.txt"), "← 403: el puesto escribe, no retira (eso es del mantenimiento)")
    fila("  PUT encima de lo escrito", "HTTP %d" % gcs("PUT", acotado, pref + "sonda.txt", b"y"), "← objectCreator no sobrescribe: los ficheros de Iceberg son inmutables")
    # PyIceberg con el token acotado como credencial prestada por el catálogo
    CAT["modo"] = "gcs"; CAT["raiz"] = "gs://%s/ore/v2" % PRUEBA; CAT["token_pleno"] = pleno; CAT["token_acotado"] = acotado
    CAT["punteros"] = {}
    cat = "http://127.0.0.1:%d" % CAT["puerto"]
    catalogo = load_catalog("ore", **{"type": "rest", "uri": cat})
    i0 = len(CAT["registro"])
    try:
        t0 = time.time()
        tabla = catalogo.create_table(("ventas", "escrita"), schema=datos(1).schema)
        tabla.append(datos(100_000))
        fila("PyIceberg create + append 100 000 en gs://", "%d ms" % ms(t0), "con el token acotado que el catálogo prestó (`gcs.oauth2.token`)")
        c, b = http("GET", "https://storage.googleapis.com/storage/v1/b/%s/o?prefix=%s" % (PRUEBA, urllib.parse.quote(pref)), cabeceras={"authorization": "Bearer " + pleno})
        objs = json.loads(b).get("items", []) if c == 200 else []
        fila("  objetos bajo el prefijo", "%d" % len(objs), " · ".join(sorted(set(o["name"].split("/")[4] for o in objs))))
        n = catalogo.load_table(("ventas", "escrita")).scan().to_arrow().num_rows
        fila("  PyIceberg lee de vuelta", "", "%d filas" % n)
    except Exception as e:
        fila("PyIceberg en gs:// con token acotado", "✗", "%s: %s" % (type(e).__name__, str(e)[:90]))
    resumen_registro(registro_desde(i0))
    # y con el token acotado a OTRA tabla: el catálogo presta el de esta, pero ¿y si el cliente trae el suyo?
    c2, ajeno, _, _ = acotar(pleno, PRUEBA, "ore/v2/datasets/otra_tabla/", ["inRole:roles/storage.objectCreator"])
    catalogo2 = load_catalog("ore2", **{"type": "rest", "uri": cat, "gcs.oauth2.token": ajeno})
    CAT["token_acotado"] = ajeno
    try:
        catalogo2.load_table(("ventas", "escrita")).append(datos(10))
        fila("append con un token de OTRA tabla", "✗ entró", "")
    except Exception as e:
        fila("append con un token de OTRA tabla", "negado", "%s: %s" % (type(e).__name__, str(e)[:70]))
    CAT["token_acotado"] = acotado
    # DuckDB: ¿escribe en gs:// con un token OAuth?
    import duckdb
    con = duckdb.connect(); con.execute("load iceberg; load httpfs;")
    try:
        con.execute("create secret g (type gcs, bearer_token '%s')" % acotado)
        fila("DuckDB secreto gcs con bearer_token", "acepta", "")
    except Exception as e:
        fila("DuckDB secreto gcs con bearer_token", "✗", str(e)[:100])
    try:
        con.execute("attach '' as lago (type iceberg, endpoint '%s', authorization_type 'none')" % cat)
        t0 = time.time()
        con.execute("insert into lago.ventas.escrita select i as id, 'IT' as pais, 1.5::decimal(18,2) as total, now()::timestamptz as cuando from range(10) r(i)")
        fila("DuckDB INSERT en gs:// (token prestado por el catálogo)", "%d ms" % ms(t0), "filas: %d" % catalogo.load_table(("ventas", "escrita")).scan().to_arrow().num_rows)
    except Exception as e:
        fila("DuckDB INSERT en gs://", "✗", str(e)[:110])
    try:
        n = con.execute("select count(*) from lago.ventas.escrita").fetchone()[0]
        fila("DuckDB lee gs:// por el catálogo", "", "%d filas" % n)
    except Exception as e:
        fila("DuckDB lee gs:// por el catálogo", "✗", str(e)[:110])
    con.close()
    # limpieza del prefijo de prueba
    r = subprocess.run(["gcloud", "storage", "rm", "-r", "-q", "gs://%s/ore/" % PRUEBA], capture_output=True, text=True, shell=(os.name == "nt"))
    c, b = http("GET", "https://storage.googleapis.com/storage/v1/b/%s/o" % PRUEBA, cabeceras={"authorization": "Bearer " + pleno})
    fila("limpieza del bucket de prueba", "quedan %d objetos" % len(json.loads(b).get("items", [])) if c == 200 else "HTTP %d" % c)
    CAT["modo"] = "s3"; CAT["raiz"] = "s3://copia/ore/v2"


GENERA_MJS = r"""
import { tableFromArrays, RecordBatchStreamWriter, Table, makeVector, Int64, Utf8, Float64, TimestampMicrosecond, vectorFromArray } from 'apache-arrow';
const n = Number(process.argv[2] || 1000000);
const modo = process.argv[3] || 'ipc';
const LOTE = 1000000;
const paises = ['ES', 'PT', 'FR', 'DE'];
const t0 = Date.now();
if (modo === 'jsonl') {
  const out = [];
  for (let i = 0; i < n; i++) {
    out.push(JSON.stringify({ id: String(i), pais: paises[i % 4], total: (i * 10 + 0.5).toFixed(2), cuando: String(1700000000000000 + i) }));
    if (out.length === 100000) { process.stdout.write(out.join('\n') + '\n'); out.length = 0; }
  }
  if (out.length) process.stdout.write(out.join('\n') + '\n');
} else {
  const writer = new RecordBatchStreamWriter();
  writer.pipe(process.stdout);
  for (let d = 0; d < n; d += LOTE) {
    const m = Math.min(LOTE, n - d);
    const id = new BigInt64Array(m); const total = new Float64Array(m); const cuando = new BigInt64Array(m); const pais = new Array(m);
    for (let i = 0; i < m; i++) { id[i] = BigInt(d + i); total[i] = (d + i) * 10 + 0.5; cuando[i] = BigInt(1700000000000000 + d + i); pais[i] = paises[(d + i) % 4]; }
    const t = new Table({
      id: makeVector(id),
      pais: vectorFromArray(pais, new Utf8()),
      total: makeVector(total),
      cuando: vectorFromArray(cuando, new TimestampMicrosecond()),
    });
    for (const b of t.batches) writer.write(b);
  }
  writer.close();
}
process.stderr.write(JSON.stringify({ generar_ms: Date.now() - t0, filas: n }) + '\n');
"""


def s3_nodo(tmp):
    print()
    print("§3 · Node sin escritor: la tabla Arrow por IPC a Rust (iceberg-rust), contra JSON a `ore-store sellar` y contra PyIceberg")
    if not NODO or not os.path.exists(IPC):
        fila("falta", "", "--nodo <dir con node_modules/apache-arrow> y/o %s" % IPC); return
    # el .mjs vive junto a node_modules: los `import` de ESM no miran NODE_PATH
    open(NODO + "/genera.mjs", "w").write(GENERA_MJS)
    env = dict(os.environ)
    # (a) IPC → ipc (Rust) en local
    for n in sorted({1_000_000, FILAS}):
        bodega = tmp + "/bodega-ipc-%d" % n
        os.makedirs(bodega)
        t0 = time.time()
        gen = subprocess.Popen(["node", NODO + "/genera.mjs", str(n), "ipc"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
        r = subprocess.run([IPC, "--bodega", "file:///" + bodega.lstrip("/")], stdin=gen.stdout, capture_output=True, text=True, encoding="utf-8")
        gen.stdout.close(); gen_err = gen.stderr.read().decode(); gen.wait()
        total = ms(t0)
        j = [json.loads(l[4:]) for l in r.stdout.splitlines() if l.startswith("### ")]
        try:
            g = json.loads(gen_err.strip().splitlines()[-1]) if gen_err.strip() else {}
        except Exception:
            g = {"error": gen_err.strip()[:200]}
        if j:
            j = j[0]
            fila("Node → IPC → Rust · %s filas" % format(n, ",").replace(",", " "), "%d ms de pared" % total,
                 "node genera %s ms · rust lee %s ms + escribe %s ms + commit %s ms · %d ficheros · %.0f MB · %.1f M filas/s" % (g.get("generar_ms"), j["leer_ms"], j["escribir_ms"], j["commit_ms"], j["ficheros"], j["bytes"] / 1e6, n / max(total, 1) / 1000))
            fila("  tipos que llegaron", "", ", ".join(j["columnas"])[:100])
        else:
            fila("Node → IPC → Rust · %d" % n, "✗", (r.stderr or gen_err)[:110])
    # (b) el camino de hoy: filas JSON a ore-store sellar (S3 de mentira)
    n = 1_000_000
    t0 = time.time()
    gen = subprocess.Popen(["node", NODO + "/genera.mjs", str(n), "jsonl"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    cab = json.dumps({"dataset": "datasets/ventas_json", "fundir": False, "clave": [], "conducto": "puesto:ana",
                      "esquema": {"id": "Integer", "pais": "String", "total": "Decimal", "cuando": "DateTimeTz"}, "plan": "sha256:nodo", "testigo": {"modo": "snapshot", "valor": "1"}}, sort_keys=True, separators=(",", ":"))
    p = subprocess.Popen([STORE, "sellar"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=CAT["env"])
    p.stdin.write((cab + "\n").encode())
    shutil.copyfileobj(gen.stdout, p.stdin)
    p.stdin.close(); out, err = p.communicate(); gen.wait()
    total = ms(t0)
    try:
        j = json.loads(out.decode())
        fila("Node → JSON → ore-store sellar · 1 000 000", "%d ms de pared" % total, "%s filas · %d ficheros · %.0f MB · sin_estrechar=%s" % (j.get("filas"), j.get("ficheros", 0), j.get("bytes", 0) / 1e6, j.get("sin_estrechar")))
    except Exception:
        fila("Node → JSON → ore-store sellar", "✗", err.decode("utf-8", "replace")[:110])
    # (c) PyIceberg desde Python, 1 M y FILAS por el catálogo de mentira (S3 de mentira)
    from pyiceberg.catalog import load_catalog
    catalogo = load_catalog("ore3", **{"type": "rest", "uri": "http://127.0.0.1:%d" % CAT["puerto"]})
    for n in sorted({1_000_000, FILAS}):
        try:
            t0 = time.time(); t = datos(n); g = ms(t0)
            t0 = time.time()
            tabla = catalogo.create_table(("ventas", "py%d" % n), schema=t.schema)
            tabla.append(t)
            total = ms(t0)
            commit = [r for r in CAT["registro"] if r["metodo"] == "POST" and r["ruta"].endswith("/tables/py%d" % n)] or [r for r in CAT["registro"] if r["metodo"] == "POST"]
            fila("PyIceberg append · %s filas" % format(n, ",").replace(",", " "), "%d ms" % total, "pyarrow genera %d ms · commit %d ms · %.1f M filas/s" % (g, commit[-1]["ms"] if commit else -1, n / max(total, 1) / 1000))
        except Exception as e:
            fila("PyIceberg append · %d" % n, "✗", "%s: %s" % (type(e).__name__, str(e)[:90]))


def main():
    for b in (ORE, SERVE, STORE):
        if not os.path.exists(b):
            print("falta", b); sys.exit(2)
    tmp = tempfile.mkdtemp(prefix="ore-escribir-").replace("\\", "/")
    procs = []
    try:
        s3 = subprocess.Popen([PY, RAIZ + "/pruebas-de-fuego/de-mentira.py", "s3", "0"], stdout=open(tmp + "/s3.log", "w"), stderr=subprocess.STDOUT)
        procs.append(s3)
        for _ in range(50):
            if os.path.exists(tmp + "/s3.log") and "listo" in open(tmp + "/s3.log").read():
                break
            time.sleep(0.2)
        s3p = open(tmp + "/s3.log").read().split()[1]
        CAT["s3"] = "http://127.0.0.1:" + s3p
        env = dict(os.environ, ORE_STORE="r2", ORE_R2_S3_ENDPOINT=CAT["s3"], ORE_R2_BUCKET="copia",
                   ORE_R2_ACCESS_KEY_ID="de", ORE_R2_SECRET_ACCESS_KEY="mentira", PATH=BIN + os.pathsep + os.environ["PATH"],
                   FICHEROS_DIR=tmp + "/semilla/datos", LAGO_URL="s3://copia", FORJA_TOKEN="no-hace-falta")
        CAT["env"] = env
        forja = tmp + "/arbol.git"
        git("init", "-q", "--bare", "-b", "main", forja)
        git("clone", "-q", forja, tmp + "/semilla")
        swap.arbol_semilla(tmp + "/semilla")
        git("add", "-A", cwd=tmp + "/semilla"); git("commit", "-qm", "semilla", cwd=tmp + "/semilla"); git("push", "-q", "origin", "HEAD:main", cwd=tmp + "/semilla")
        puerto = puerto_libre()
        base = "http://127.0.0.1:%d" % puerto
        CAT["serve"] = base
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
        cp = puerto_libre()
        CAT["puerto"] = cp
        httpd = ThreadingHTTPServer(("127.0.0.1", cp), Catalogo)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        cat = "http://127.0.0.1:%d" % cp
        fila("catálogo REST de mentira", cat, "delante de ore-serve %s y del S3 de mentira" % base)

        s1(cat, base, tmp)
        s4(base, tmp)
        if not SIN_GCS:
            s2(tmp)
        s3_nodo(tmp)
        print()
        print("peticiones registradas: %d · el log de ore-serve: %s/serve.log" % (len(CAT["registro"]), tmp))
    finally:
        for p in procs:
            p.kill()


if __name__ == "__main__":
    main()
