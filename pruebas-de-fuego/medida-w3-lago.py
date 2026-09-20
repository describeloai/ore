#!/usr/bin/env python3
"""
MEDIDA · W3.5b · el lector del lago, EN EL CLÚSTER (20 de septiembre; 0031 §10)

Todo el paradigma «todo es un dataset» cuelga de que DuckDB, dentro de un puesto
—rol `puesto`: sin internet, sólo Google APIs por `private.googleapis.com`—, lea una
tabla Iceberg que está en `gs://` con la identidad del pod. Dos incógnitas que sólo
se resuelven allí, y se resuelven las dos en un Job con TRES contenedores (las tres
imágenes de puesto: python, node, jvm) sobre `jobs-p`:

  §1  LA EXTENSIÓN SIN INTERNET   `INSTALL iceberg` no puede (no hay salida). Se
                                  preinstala: los ficheros de `iceberg`, `avro`,
                                  `httpfs`, `json` e `icu` de la versión EXACTA de
                                  DuckDB de cada enlace (python, node-api, JDBC:
                                  pueden diferir) puestos en `~/.duckdb/extensions/
                                  v<ver>/linux_amd64/`, y `LOAD iceberg` por nombre.
                                  Es lo que la imagen hará al construirse.
  §2  LEER gs:// CON EL POD       tres caminos, sin credencial nueva:
        (b) el token del servidor de metadatos como BEARER_TOKEN de un secreto
            HTTP de DuckDB, y `iceberg_scan('https://storage.googleapis.com/…/
            metadata.json', allow_moved_paths=true)`: DuckDB lee del bucket
            directamente (con poda por estadísticas, sin bajar todo)
        (c) bajar los objetos de la tabla con la API JSON (como hoy con el sobre)
            y `iceberg_scan` en local
        (a) HMAC de la cuenta del puesto — NO se mide: exige crear una credencial;
            queda como alternativa si (b) y (c) no valen
  §3  LOS TIPOS                    la tabla de los 23 tipos leída por (b) desde
                                  cada lenguaje, cotejada con la verdad

Uso:
  python pruebas-de-fuego/medida-w3-lago.py [--inquilino demo] [--filas 10000000] [--solo-preparar] [--solo-informe <job>]

Desde esta máquina: gcloud (sesión propia) para escribir la tabla y las extensiones
en el bucket del inquilino bajo `medida/lago/`, kubectl para el Job. `jobs-p` escala
0 → 1 → 0 solo. Al final se borra el Job y el prefijo del bucket.
"""
import datetime as dt
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
EXTENSIONES = ["iceberg", "avro", "httpfs", "json", "icu"]
VERSIONES = ["1.5.4", "1.5.5"]  # las que hay en extensions.duckdb.org hoy; el Job dice cuál usa cada enlace

espec = importlib.util.spec_from_file_location("leer", os.path.join(RAIZ, "pruebas-de-fuego", "medida-w3-leer.py"))
leer = importlib.util.module_from_spec(espec)
espec.loader.exec_module(leer)


def fila(k, v, nota=""):
    print("  %-38s %-42s %s" % (k, v, nota))


def sh(*args, entrada=None, env=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), capture_output=True, input=entrada, env=env)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


def kubectl(*args, entrada=None):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    return sh("kubectl", *args, entrada=entrada, env=env)


# ── lo que corre en cada contenedor ────────────────────────────────────────
# Los tres hacen lo mismo: instalar las extensiones por nombre, leer por (b) y
# por (c), y enseñar los tipos. Cada uno imprime un JSON por línea con `###`.
COMUN_SQL_TIPOS = "select * from iceberg_scan('%s', allow_moved_paths=true) order by i8 desc nulls last"

PYTHON = r'''
import json, os, sys, time, shutil, urllib.request, platform
import duckdb
def di(k, **v): print("### " + json.dumps(dict(que=k, **v), default=str), flush=True)
def ms(t): return int((time.time() - t) * 1000)
ver = duckdb.__version__
con = duckdb.connect()
plat = con.execute("pragma platform").fetchone()[0]
di("version", duckdb=ver, plataforma=plat, python=platform.python_version())
# §1 · preinstalar por nombre
dest = os.path.expanduser("~/.duckdb/extensions/v%s/%s" % (ver, plat))
os.makedirs(dest, exist_ok=True)
origen = "/ext/v%s/%s" % (ver, plat)
if not os.path.isdir(origen):
    di("extension", ok=False, porque="no hay extensiones para v%s/%s en el bucket" % (ver, plat)); sys.exit(0)
for f in os.listdir(origen): shutil.copy(os.path.join(origen, f), dest)
t = time.time()
try:
    con.execute("load iceberg"); con.execute("load httpfs")
    di("extension", ok=True, ms=ms(t), cargadas=[r[0] for r in con.execute("select extension_name from duckdb_extensions() where loaded").fetchall()])
except Exception as e:
    di("extension", ok=False, ms=ms(t), porque=str(e)[:200]); sys.exit(0)
# ¿y si una celda pide una extensión que NO está? DuckDB sale a extensions.duckdb.org, y el
# pod no tiene salida: lo que importa es CUÁNTO tarda en rendirse (una NetworkPolicy tira el
# paquete, no lo rechaza).
t = time.time()
try:
    con.execute("install spatial")
    di("install_sin_internet", ok=True, ms=ms(t), nota="¿tenía salida?")
except Exception as e:
    di("install_sin_internet", ok=False, ms=ms(t), porque=str(e)[:160])
# el token del pod
req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token", headers={"Metadata-Flavor": "Google"})
tok = json.loads(urllib.request.urlopen(req, timeout=10).read())["access_token"]
bucket, pref = os.environ["BUCKET"], os.environ["PREFIJO"]
https = "https://storage.googleapis.com/%s/%s" % (bucket, pref)
con.execute("create secret gcs_http (type http, bearer_token '%s')" % tok.replace("'", "''"))
# sondas: ¿la API XML de GCS acepta el Bearer desde el pod? ¿y httpfs solo, sin iceberg?
meta_url = "%s/%s" % (https, os.environ["META_TIPOS"])
for metodo in ("HEAD", "GET"):
    try:
        r = urllib.request.urlopen(urllib.request.Request(meta_url, method=metodo, headers={"Authorization": "Bearer " + tok}), timeout=20)
        di("sonda_urllib", metodo=metodo, ok=True, codigo=r.status, bytes=len(r.read()))
    except urllib.error.HTTPError as e:
        di("sonda_urllib", metodo=metodo, ok=False, codigo=e.code, cuerpo=e.read().decode("utf-8", "replace")[:200])
    except Exception as e:
        di("sonda_urllib", metodo=metodo, ok=False, porque=str(e)[:200])
try:
    n = con.execute("select length(content) from read_text('%s')" % meta_url).fetchone()[0]
    di("sonda_httpfs", ok=True, bytes=n)
except Exception as e:
    di("sonda_httpfs", ok=False, porque=str(e)[:600])
try:
    n = con.execute("select length(content) from read_text('%s?x=1')" % meta_url).fetchone()[0]
    di("sonda_httpfs_query", ok=True, bytes=n)
except Exception as e:
    di("sonda_httpfs_query", ok=False, porque=str(e)[:300])
# El puntero del árbol da el metadata.json; a DuckDB se le da LA RAÍZ de la tabla y la
# VERSIÓN (con allow_moved_paths, la raíz es lo que se le pasa): así no lista nada.
def tabla(base, meta):
    raiz, fichero = meta.split("/metadata/")
    return "iceberg_scan('%s/%s', version='%s', allow_moved_paths=true)" % (base, raiz, fichero.replace(".metadata.json", ""))
# §2b · directo del bucket
for nombre, meta in (("grande", os.environ["META_GRANDE"]), ("tipos", os.environ["META_TIPOS"])):
    url = tabla(https, meta)
    t = time.time()
    try:
        n = con.execute("select count(*) from %s" % url).fetchone()[0]
        t_n = ms(t); t = time.time()
        g = con.execute("select pais, count(*) from %s group by 1" % url).fetchall() if nombre == "grande" else []
        t_g = ms(t); t = time.time()
        f = con.execute("select count(*) from %s where cliente = 7" % url).fetchone()[0] if nombre == "grande" else 0
        di("directo", tabla=nombre, ok=True, filas=n, count_ms=t_n, groupby_ms=t_g, grupos=len(g), filtro_ms=ms(t), filtro_filas=f)
    except Exception as e:
        di("directo", tabla=nombre, ok=False, porque=str(e)[:700])
# §2c · bajar y leer en local
def listar(prefijo):
    out, token = [], None
    while True:
        u = "https://storage.googleapis.com/storage/v1/b/%s/o?prefix=%s&fields=items(name,size),nextPageToken" % (bucket, urllib.request.quote(prefijo, safe=""))
        if token: u += "&pageToken=" + token
        r = json.loads(urllib.request.urlopen(urllib.request.Request(u, headers={"Authorization": "Bearer " + tok}), timeout=30).read())
        out += r.get("items", []); token = r.get("nextPageToken")
        if not token: return out
for nombre, meta in (("grande", os.environ["META_GRANDE"]),):
    raiz = pref + "/" + meta.split("/metadata/")[0]
    t = time.time(); objetos = listar(raiz + "/"); t_l = ms(t); t = time.time(); total = 0
    for o in objetos:
        u = "https://storage.googleapis.com/storage/v1/b/%s/o/%s?alt=media" % (bucket, urllib.request.quote(o["name"], safe=""))
        d = "/trabajo/lago/" + o["name"][len(pref) + 1:]
        os.makedirs(os.path.dirname(d), exist_ok=True)
        with urllib.request.urlopen(urllib.request.Request(u, headers={"Authorization": "Bearer " + tok}), timeout=120) as r, open(d, "wb") as f:
            total += f.write(r.read())
    t_b = ms(t); t = time.time()
    try:
        n = con.execute("select count(*) from %s" % tabla("/trabajo/lago", meta)).fetchone()[0]
        di("bajado", tabla=nombre, ok=True, objetos=len(objetos), listar_ms=t_l, bajar_ms=t_b, mb=round(total / 1e6, 1), count_ms=ms(t), filas=n)
    except Exception as e:
        di("bajado", tabla=nombre, ok=False, objetos=len(objetos), bajar_ms=t_b, mb=round(total / 1e6, 1), porque=str(e)[:300], hay=[str(p) for p in __import__("pathlib").Path("/trabajo/lago").rglob("*") if p.is_file()][:8])
# §3 · los tipos por (b)
try:
    tipos = [str(t) for t in con.sql("select * from %s limit 0" % tabla(https, os.environ["META_TIPOS"])).types]
    r = con.execute("select columns(*)::varchar from %s order by 1 desc nulls last" % tabla(https, os.environ["META_TIPOS"]))
    cols = [d[0] for d in r.description]; filas = r.fetchall()
    di("tipos", columnas=cols, tipos=tipos, filas=[[str(v) if v is not None else None for v in f] for f in filas])
except Exception as e:
    di("tipos", ok=False, porque=str(e)[:200])
'''

NODE = r'''
import { DuckDBInstance } from "@duckdb/node-api";
import { mkdirSync, copyFileSync, readdirSync, existsSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import * as api from "@duckdb/node-api";
const di = (que, v) => console.log("### " + JSON.stringify({ que, ...v }, (k, x) => typeof x === "bigint" ? Number(x) : x));
const inst = await DuckDBInstance.create(); const con = await inst.connect();
const ver = api.version().replace(/^v/, ""); const plat = String((await con.runAndReadAll("pragma platform")).getRows()[0][0]);
di("version", { duckdb: ver, plataforma: plat, node: process.version });
const dest = `${homedir()}/.duckdb/extensions/v${ver}/${plat}`; const origen = `/ext/v${ver}/${plat}`;
if (!existsSync(origen)) { di("extension", { ok: false, porque: `no hay extensiones para v${ver}/${plat} en el bucket` }); process.exit(0); }
mkdirSync(dest, { recursive: true }); for (const f of readdirSync(origen)) copyFileSync(`${origen}/${f}`, `${dest}/${f}`);
let t = performance.now();
try { await con.run("load iceberg"); await con.run("load httpfs"); di("extension", { ok: true, ms: Math.round(performance.now() - t) }); }
catch (e) { di("extension", { ok: false, porque: String(e.message).slice(0, 200) }); process.exit(0); }
const tok = (await (await fetch("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token", { headers: { "Metadata-Flavor": "Google" } })).json()).access_token;
const bucket = process.env.BUCKET, pref = process.env.PREFIJO; const https = `https://storage.googleapis.com/${bucket}/${pref}`;
await con.run(`create secret gcs_http (type http, bearer_token '${tok.replaceAll("'", "''")}')`);
// node:24-slim no trae ca-certificates: DuckDB (OpenSSL) no puede verificar a Google. Aquí se le
// da el manojo de la imagen de Python por SSL_CERT_FILE; la imagen tiene que llevarlo.
if (process.env.SSL_CERT_FILE) { try { await con.run(`set ca_cert_file = '${process.env.SSL_CERT_FILE}'`); } catch {} }
const tabla = (base, meta) => { const [raiz, fichero] = meta.split("/metadata/"); return `iceberg_scan('${base}/${raiz}', version='${fichero.replace(".metadata.json", "")}', allow_moved_paths=true)`; };
for (const [nombre, meta] of [["grande", process.env.META_GRANDE], ["tipos", process.env.META_TIPOS]]) {
  const url = tabla(https, meta);
  try {
    t = performance.now(); const n = (await con.runAndReadAll(`select count(*) from ${url}`)).getRows()[0][0]; const t_n = Math.round(performance.now() - t);
    t = performance.now(); const g = nombre === "grande" ? (await con.runAndReadAll(`select pais, count(*) from ${url} group by 1`)).getRows() : []; const t_g = Math.round(performance.now() - t);
    t = performance.now(); const f = nombre === "grande" ? (await con.runAndReadAll(`select count(*) from ${url} where cliente = 7`)).getRows()[0][0] : 0;
    di("directo", { tabla: nombre, ok: true, filas: n, count_ms: t_n, groupby_ms: t_g, grupos: g.length, filtro_ms: Math.round(performance.now() - t), filtro_filas: f });
  } catch (e) { di("directo", { tabla: nombre, ok: false, porque: String(e.message).slice(0, 600) }); }
}
// (c) bajar y leer en local
const listar = async (prefijo) => { let out = [], token = null; while (true) { let u = `https://storage.googleapis.com/storage/v1/b/${bucket}/o?prefix=${encodeURIComponent(prefijo)}&fields=items(name,size),nextPageToken`; if (token) u += "&pageToken=" + token; const r = await (await fetch(u, { headers: { Authorization: "Bearer " + tok } })).json(); out = out.concat(r.items ?? []); token = r.nextPageToken; if (!token) return out; } };
{
  const meta = process.env.META_GRANDE; const raiz = pref + "/" + meta.split("/metadata/")[0];
  t = performance.now(); const objetos = await listar(raiz + "/"); const t_l = Math.round(performance.now() - t); t = performance.now(); let total = 0;
  for (const o of objetos) { const u = `https://storage.googleapis.com/storage/v1/b/${bucket}/o/${encodeURIComponent(o.name)}?alt=media`; const d = "/trabajo/lago-node/" + o.name.slice(pref.length + 1); mkdirSync(d.slice(0, d.lastIndexOf("/")), { recursive: true }); const b = Buffer.from(await (await fetch(u, { headers: { Authorization: "Bearer " + tok } })).arrayBuffer()); writeFileSync(d, b); total += b.length; }
  const t_b = Math.round(performance.now() - t); t = performance.now();
  try { const n = (await con.runAndReadAll(`select count(*) from ${tabla("/trabajo/lago-node", meta)}`)).getRows()[0][0]; di("bajado", { tabla: "grande", ok: true, objetos: objetos.length, listar_ms: t_l, bajar_ms: t_b, mb: Math.round(total / 1e5) / 10, count_ms: Math.round(performance.now() - t), filas: n }); }
  catch (e) { di("bajado", { tabla: "grande", ok: false, objetos: objetos.length, bajar_ms: t_b, porque: String(e.message).slice(0, 200) }); }
}
try {
  const t0 = await con.runAndReadAll(`select * from ${tabla(https, process.env.META_TIPOS)} limit 0`);
  const r = await con.runAndReadAll(`select columns(*)::varchar from ${tabla(https, process.env.META_TIPOS)} order by 1 desc nulls last`);
  di("tipos", { columnas: r.columnNames(), tipos: t0.columnTypes().map(String), filas: r.getRows().map((f) => f.map((v) => v === null || v === undefined ? null : String(v))) });
} catch (e) { di("tipos", { ok: false, porque: String(e.message).slice(0, 200) }); }
'''

JAVA = r'''
import java.net.URI; import java.net.http.*; import java.nio.file.*; import java.sql.*; import java.util.*;
public class Lago {
    static void di(String que, Object... kv) { StringBuilder b = new StringBuilder("### {\"que\":\"" + que + "\""); for (int i = 0; i < kv.length; i += 2) { b.append(",\"").append(kv[i]).append("\":"); Object v = kv[i + 1]; if (v instanceof Number || v instanceof Boolean) b.append(v); else if (v instanceof List<?> l) { b.append("["); for (int j = 0; j < l.size(); j++) { if (j > 0) b.append(","); Object x = l.get(j); b.append(x == null ? "null" : x instanceof List<?> ? j(x) : "\"" + esc(String.valueOf(x)) + "\""); } b.append("]"); } else b.append("\"").append(esc(String.valueOf(v))).append("\""); } System.out.println(b.append("}")); }
    static String j(Object x) { List<?> l = (List<?>) x; StringBuilder b = new StringBuilder("["); for (int i = 0; i < l.size(); i++) { if (i > 0) b.append(","); Object y = l.get(i); b.append(y == null ? "null" : "\"" + esc(String.valueOf(y)) + "\""); } return b.append("]").toString(); }
    static String esc(String s) { return s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", " "); }
    static HttpClient http = HttpClient.newHttpClient();
    static String tabla(String base, String meta) { String[] p = meta.split("/metadata/"); return "iceberg_scan('" + base + "/" + p[0] + "', version='" + p[1].replace(".metadata.json", "") + "', allow_moved_paths=true)"; }
    static String get(String url, String auth) throws Exception { HttpRequest.Builder r = HttpRequest.newBuilder(URI.create(url)); if (auth != null) r.header("Authorization", "Bearer " + auth); else r.header("Metadata-Flavor", "Google"); return http.send(r.build(), HttpResponse.BodyHandlers.ofString()).body(); }
    public static void main(String[] a) throws Exception {
        Class.forName("org.duckdb.DuckDBDriver"); Connection con = DriverManager.getConnection("jdbc:duckdb:"); Statement s = con.createStatement();
        String ver, plat; try (ResultSet r = s.executeQuery("select version(), (select platform from pragma_platform())")) { r.next(); ver = r.getString(1).replaceFirst("^v", ""); plat = r.getString(2); }
        di("version", "duckdb", ver, "plataforma", plat, "java", Runtime.version().toString());
        Path dest = Paths.get(System.getProperty("user.home"), ".duckdb", "extensions", "v" + ver, plat), origen = Paths.get("/ext", "v" + ver, plat);
        if (!Files.isDirectory(origen)) { di("extension", "ok", false, "porque", "no hay extensiones para v" + ver + "/" + plat + " en el bucket"); return; }
        Files.createDirectories(dest); try (var ds = Files.list(origen)) { for (Path p : ds.toList()) Files.copy(p, dest.resolve(p.getFileName()), StandardCopyOption.REPLACE_EXISTING); }
        long t = System.nanoTime();
        try { s.execute("load iceberg"); s.execute("load httpfs"); di("extension", "ok", true, "ms", (System.nanoTime() - t) / 1_000_000); } catch (SQLException e) { di("extension", "ok", false, "porque", e.getMessage().substring(0, Math.min(200, e.getMessage().length()))); return; }
        String tok = get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token", null).replaceAll(".*\"access_token\"\\s*:\\s*\"([^\"]+)\".*", "$1");
        String bucket = System.getenv("BUCKET"), pref = System.getenv("PREFIJO"), https = "https://storage.googleapis.com/" + bucket + "/" + pref;
        s.execute("create secret gcs_http (type http, bearer_token '" + tok.replace("'", "''") + "')");
        for (String[] tm : new String[][] { { "grande", System.getenv("META_GRANDE") }, { "tipos", System.getenv("META_TIPOS") } }) {
            String url = tabla(https, tm[1]);
            try {
                t = System.nanoTime(); long n; try (ResultSet r = s.executeQuery("select count(*) from " + url)) { r.next(); n = r.getLong(1); } long tn = (System.nanoTime() - t) / 1_000_000;
                t = System.nanoTime(); int g = 0; if (tm[0].equals("grande")) try (ResultSet r = s.executeQuery("select pais, count(*) from " + url + " group by 1")) { while (r.next()) g++; } long tg = (System.nanoTime() - t) / 1_000_000;
                t = System.nanoTime(); long f = 0; if (tm[0].equals("grande")) try (ResultSet r = s.executeQuery("select count(*) from " + url + " where cliente = 7")) { r.next(); f = r.getLong(1); }
                di("directo", "tabla", tm[0], "ok", true, "filas", n, "count_ms", tn, "groupby_ms", tg, "grupos", g, "filtro_ms", (System.nanoTime() - t) / 1_000_000, "filtro_filas", f);
            } catch (SQLException e) { String m = String.valueOf(e.getMessage()); di("directo", "tabla", tm[0], "ok", false, "porque", m.substring(0, Math.min(700, m.length()))); try { s.close(); } catch (Exception x) {} s = con.createStatement(); }
        }
        try {
            List<Object> cols = new ArrayList<>(), tipos = new ArrayList<>(), filas = new ArrayList<>();
            try (ResultSet r = s.executeQuery("select * from " + tabla(https, System.getenv("META_TIPOS")) + " limit 0")) { ResultSetMetaData md = r.getMetaData(); for (int i = 1; i <= md.getColumnCount(); i++) tipos.add(md.getColumnTypeName(i)); }
            try (ResultSet r = s.executeQuery("select columns(*)::varchar from " + tabla(https, System.getenv("META_TIPOS")) + " order by 1 desc nulls last")) {
                ResultSetMetaData md = r.getMetaData(); for (int i = 1; i <= md.getColumnCount(); i++) cols.add(md.getColumnLabel(i));
                while (r.next()) { List<Object> f = new ArrayList<>(); for (int i = 1; i <= md.getColumnCount(); i++) { String v = r.getString(i); f.add(v); } filas.add(f); }
            }
            di("tipos", "columnas", cols, "tipos", tipos, "filas", filas);
        } catch (SQLException e) { di("tipos", "ok", false, "porque", e.getMessage().substring(0, Math.min(200, e.getMessage().length()))); }
    }
}
'''


def contenedor(nombre, imagen, mando, extra_env=()):
    env = [
        {"name": "HOME", "value": "/tmp"},
        {"name": "BUCKET", "value": "__BUCKET__"},
        {"name": "PREFIJO", "value": "__PREFIJO__"},
        {"name": "META_GRANDE", "value": "__META_GRANDE__"},
        {"name": "META_TIPOS", "value": "__META_TIPOS__"},
    ] + list(extra_env)
    return {
        "name": nombre,
        "image": imagen,
        "imagePullPolicy": "Always",
        "env": env,
        "volumeMounts": [{"name": "ext", "mountPath": "/ext", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
        "workingDir": "/trabajo",
        "command": ["/bin/sh", "-c"],
        "args": [mando],
        "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "3Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
    }


def job_yaml(inquilino, bucket, prefijo, meta_grande, meta_tipos, nombre):
    ns = "t-" + inquilino
    traer = r'''
set -e
python3 - <<'PY'
import os, time, gzip, shutil
from google.cloud import storage
t = time.time(); cli = storage.Client(); b = cli.bucket(os.environ["BUCKET"]); pre = os.environ["PREFIJO"] + "/ext/"
n = 0
for o in cli.list_blobs(b, prefix=pre):
    d = "/ext/" + o.name[len(pre):]
    os.makedirs(os.path.dirname(d), exist_ok=True)
    o.download_to_filename(d + ".gz")
    with gzip.open(d + ".gz", "rb") as f, open(d, "wb") as g: shutil.copyfileobj(f, g)
    os.remove(d + ".gz"); n += 1
print("### %s" % __import__("json").dumps({"que": "extensiones_bajadas", "n": n, "ms": int((time.time() - t) * 1000)}), flush=True)
PY
cp /etc/ssl/certs/ca-certificates.crt /ext/ca.crt
'''
    job = {
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": inquilino, "ore.dev/rol": "puesto"}},
        "spec": {
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": 1800,
            "activeDeadlineSeconds": 1800,
            "template": {
                "metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": inquilino}},
                "spec": {
                    "restartPolicy": "Never",
                    "serviceAccountName": "driver",
                    "volumes": [{"name": "ext", "emptyDir": {}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                    "initContainers": [
                        {
                            "name": "traer-extensiones",
                            "image": REGISTRO + "/puesto-python:1",
                            "imagePullPolicy": "Always",
                            "env": [{"name": "HOME", "value": "/tmp"}, {"name": "BUCKET", "value": bucket}, {"name": "PREFIJO", "value": prefijo}],
                            "volumeMounts": [{"name": "ext", "mountPath": "/ext"}],
                            "command": ["/bin/sh", "-c"],
                            "args": [traer],
                            "resources": {"requests": {"cpu": "250m", "memory": "512Mi"}, "limits": {"cpu": "1", "memory": "1Gi"}},
                            "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
                        }
                    ],
                    "containers": [
                        contenedor("python", REGISTRO + "/puesto-python:1", "python3 /guiones/lago.py"),
                        contenedor("node", REGISTRO + "/puesto-node:1", "cp /guiones/lago.mjs /trabajo/lago.mjs && ln -s /opt/ore/node_modules /trabajo/node_modules && node --no-warnings /trabajo/lago.mjs", [{"name": "SSL_CERT_FILE", "value": "/ext/ca.crt"}]),
                        contenedor("jvm", REGISTRO + "/puesto-jvm:1", "mkdir -p /trabajo/c && cp /guiones/Lago.java /trabajo/ && javac -d /trabajo/c -cp '/opt/ore/lib/*' /trabajo/Lago.java && java -cp '/trabajo/c:/opt/ore/lib/*' Lago"),
                    ],
                },
            },
        },
    }
    texto = json.dumps(job)
    for k, v in (("__BUCKET__", bucket), ("__PREFIJO__", prefijo), ("__META_GRANDE__", meta_grande), ("__META_TIPOS__", meta_tipos)):
        texto = texto.replace(k, v)
    cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": ns}, "data": {"lago.py": PYTHON, "lago.mjs": NODE, "Lago.java": JAVA}}
    return json.dumps(cm) + "\n---\n" + texto


def preparar(inquilino, filas, prefijo_gs):
    """La tabla Iceberg grande, la de tipos y las extensiones, en el bucket."""
    import gcsfs
    import pyarrow.parquet as pq
    from google.oauth2.credentials import Credentials

    espec2 = importlib.util.spec_from_file_location("ice", os.path.join(RAIZ, "pruebas-de-fuego", "medida-w3-iceberg.py"))
    ice = importlib.util.module_from_spec(espec2)
    espec2.loader.exec_module(ice)
    tok = subprocess.run(["gcloud", "auth", "print-access-token"], capture_output=True, shell=(os.name == "nt")).stdout.decode().strip()
    if not tok:
        raise SystemExit("gcloud auth print-access-token no dio nada")
    fs = gcsfs.GCSFileSystem(token=Credentials(token=tok))
    tmp = tempfile.mkdtemp(prefix="ore-lago-").replace("\\", "/")
    ArbolCatalog = ice.catalogo_arbol()
    cat = ArbolCatalog("lago", tmp + "/arbol", **{"warehouse": prefijo_gs, "py-io-impl": "pyiceberg.io.fsspec.FsspecFileIO", "gcs.oauth2.token": tok, "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000))})
    cat.create_namespace("lago")

    def ultima_metadata(nombre):
        """El metadata.json vigente de una tabla que ya esté en el bucket CON DATOS (un
        snapshot); los restos de una corrida cortada (sólo el `00000` de crear) se borran."""
        fs.invalidate_cache()
        raiz = "%s/lago/%s" % (prefijo_gs[5:], nombre)
        for f in sorted(fs.glob(raiz + "/metadata/*.metadata.json"), reverse=True):
            if json.loads(fs.cat(f)).get("current-snapshot-id", -1) not in (-1, None):
                return "gs://" + f
        if fs.exists(raiz):
            fs.rm(raiz, recursive=True)
        return None

    mg = ultima_metadata("grande")
    if mg:
        fila("tabla grande en el bucket", "ya estaba", mg)
    else:
        t0 = time.time()
        grande = leer.tabla_grande(filas)
        tg = cat.create_table("lago.grande", grande.schema)
        tg.append(grande)
        mg = tg.metadata_location
        fila("tabla grande en el bucket", "%d filas · %d ms" % (filas, (time.time() - t0) * 1000), mg)
    tt = leer.tabla_de_tipos()
    # lo que Iceberg no tiene (medido en medida-w3-iceberg.py): fuera
    tt = tt.drop_columns(["u64", "ts_madrid", "ts_ns", "todo_nulo"])
    mt = ultima_metadata("tipos")
    if not mt:
        tp = cat.create_table("lago.tipos", tt.schema)
        tp.append(tt)
        mt = tp.metadata_location
    fila("tabla de tipos en el bucket", "%d columnas" % tt.num_columns, mt)
    # las extensiones, por versión
    n = 0
    for v in VERSIONES:
        for e in EXTENSIONES:
            url = "https://extensions.duckdb.org/v%s/linux_amd64/%s.duckdb_extension.gz" % (v, e)
            destino = "%s/ext/v%s/linux_amd64/%s.duckdb_extension" % (prefijo_gs[5:], v, e)
            if fs.exists(destino):
                n += 1
                continue
            datos = urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ore-medida/1"}), timeout=120).read()
            with fs.open(destino, "wb") as f:
                f.write(datos)
            n += 1
    fila("extensiones en el bucket", "%d (%s × %s)" % (n, ", ".join(VERSIONES), ", ".join(EXTENSIONES)), "linux_amd64, comprimidas")
    shutil.rmtree(tmp, ignore_errors=True)
    prefijo = prefijo_gs[5:].split("/", 1)[1]
    rel = lambda loc: loc.split(prefijo + "/", 1)[1]
    return leer.verdad(tt), rel(mg), rel(mt)


def texto_duckdb(v):
    """Lo que DuckDB escribe al pasar una columna a VARCHAR → la forma canónica de
    la verdad (base64 para bytes, `NaN`/`inf`, `None` en listas, `{}` = mapa vacío)."""
    import base64
    import re
    if v is None:
        return None
    hexs = re.fullmatch(r"(?:\\x([0-9a-fA-F]{2}))+", v)
    if hexs or v == "":
        return base64.b64encode(bytes(int(h, 16) for h in re.findall(r"\\x([0-9a-fA-F]{2})", v))).decode("ascii") if hexs else v
    if v in ("nan", "-nan"):
        return "NaN"
    if v in ("inf", "-inf"):
        return v
    if v == "-0.0":
        return "0"
    if v == "{}":
        return "[]"
    v = v.replace("NULL", "null")
    return leer.canonico_desde_json(v)


def informe(nombre, ns, verdad):
    c, out, err = kubectl("logs", "-n", ns, "job/" + nombre, "--all-containers", "--prefix", "--tail=-1")
    if c != 0:
        print(err)
        return
    por = {}
    for linea in out.splitlines():
        if "### " not in linea:
            continue
        cont = linea.split("]")[0].rsplit("/", 1)[-1]
        try:
            d = json.loads(linea.split("### ", 1)[1])
        except ValueError:
            continue
        por.setdefault(cont, []).append(d)
    print()
    print("§1 · la extensión sin internet (preinstalada por nombre en ~/.duckdb/extensions/v<ver>/linux_amd64)")
    for cont in ("traer-extensiones", "python", "node", "jvm"):
        for d in por.get(cont, []):
            if d["que"] == "extensiones_bajadas":
                fila("  extensiones bajadas al pod", "%d en %d ms" % (d["n"], d["ms"]))
            if d["que"] == "version":
                fila("  %s" % cont, "duckdb %s · %s" % (d["duckdb"], d["plataforma"]), d.get("python") or d.get("node") or d.get("java", ""))
            if d["que"] == "extension":
                fila("    LOAD iceberg + httpfs", "✓ %d ms" % d["ms"] if d.get("ok") else "✗ %s" % d.get("porque"), ", ".join(d.get("cargadas", [])))
            if d["que"] == "install_sin_internet":
                fila("    INSTALL spatial (no está; sin salida)", ("✗ se rinde en %d ms" % d["ms"]) if not d.get("ok") else "✓ ¿tiene salida? %d ms" % d["ms"], d.get("porque", "")[:110])
    print()
    print("§2 · leer gs:// con la identidad del pod")
    for d in por.get("python", []):
        if d["que"] == "sonda_urllib":
            fila("  sonda · %s de metadata.json con Bearer (urllib)" % d["metodo"], "✓ %s · %s bytes" % (d.get("codigo"), d.get("bytes")) if d.get("ok") else "✗ %s" % d.get("codigo", ""), d.get("cuerpo", d.get("porque", ""))[:120])
        if d["que"] in ("sonda_httpfs", "sonda_httpfs_query"):
            fila("  sonda · read_text por httpfs%s" % (" (con ?query)" if d["que"].endswith("query") else ""), "✓ %s bytes" % d.get("bytes") if d.get("ok") else "✗", d.get("porque", "")[:300])
    for cont in ("python", "node", "jvm"):
        for d in por.get(cont, []):
            if d["que"] == "directo":
                if d.get("ok"):
                    fila("  %s · (b) directo · %s" % (cont, d["tabla"]), "count %d ms · group by %d ms · filtro %d ms" % (d["count_ms"], d["groupby_ms"], d["filtro_ms"]), "%s filas · %s grupos · %s con cliente = 7" % (d["filas"], d["grupos"], d["filtro_filas"]))
                else:
                    fila("  %s · (b) directo · %s" % (cont, d["tabla"]), "✗", d.get("porque", "")[:400])
            if d["que"] == "bajado":
                if d.get("ok"):
                    fila("  %s · (c) bajar y leer" % cont, "listar %d ms · bajar %d ms (%s MB, %d objetos) · count %d ms" % (d["listar_ms"], d["bajar_ms"], d["mb"], d["objetos"], d["count_ms"]), "%s filas" % d["filas"])
                else:
                    fila("  %s · (c) bajar y leer" % cont, "✗ %s objetos · %s MB" % (d.get("objetos"), d.get("mb")), "%s · hay: %s" % (d.get("porque", "")[:200], d.get("hay")))
    print()
    print("§3 · los tipos por (b), cotejados con la verdad (✓ = igual; si no, lo que llegó)")
    for cont in ("python", "node", "jvm"):
        for d in por.get(cont, []):
            if d["que"] != "tipos":
                continue
            if not d.get("columnas"):
                fila("  %s" % cont, "✗", d.get("porque", ""))
                continue
            malas = []
            for i, c in enumerate(d["columnas"]):
                vals = [texto_duckdb(f[i]) for f in d["filas"]]
                # las filas vienen ordenadas por i8 desc nulls last: 127, -128, null → la verdad es 127, null, -128
                esperado = [verdad[c]["valores"][0], verdad[c]["valores"][2], verdad[c]["valores"][1]]
                if vals != esperado:
                    malas.append("%s: %s" % (c, next((x for x, y in zip(vals, esperado) if x != y), "?")))
            fila("  %s" % cont, "%d/%d ✓" % (len(d["columnas"]) - len(malas), len(d["columnas"])), "; ".join(malas)[:160])


def main():
    inquilino = sys.argv[sys.argv.index("--inquilino") + 1] if "--inquilino" in sys.argv else "demo"
    filas = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 10_000_000
    ns = "t-" + inquilino
    bucket = "%s-t-%s-copia" % (PROYECTO, inquilino)
    prefijo = "medida/lago"
    print("MEDIDA · W3.5b · el lector del lago en el clúster · %s · %s" % (inquilino, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    if "--solo-informe" in sys.argv:
        nombre = sys.argv[sys.argv.index("--solo-informe") + 1]
        tt = leer.tabla_de_tipos().drop_columns(["u64", "ts_madrid", "ts_ns", "todo_nulo"])
        informe(nombre, ns, leer.verdad(tt))
        return
    verdad, meta_grande, meta_tipos = preparar(inquilino, filas, "gs://%s/%s" % (bucket, prefijo))
    if "--solo-preparar" in sys.argv:
        return
    nombre = "medida-lago-" + dt.datetime.now().strftime("%H%M%S")
    manifiesto = job_yaml(inquilino, bucket, prefijo, meta_grande, meta_tipos, nombre)
    c, out, err = kubectl("apply", "-f", "-", entrada=manifiesto.encode("utf-8"))
    if c != 0:
        print(err)
        return
    fila("Job", "%s en %s (jobs-p por Kueue)" % (nombre, ns), out.strip().replace("\n", " · "))
    t0 = time.time()
    estado = "?"
    while time.time() - t0 < 1500:
        c, out, err = kubectl("get", "job", "-n", ns, nombre, "-o", "jsonpath={.status.succeeded}/{.status.failed}/{.status.active}")
        estado = out.strip()
        if estado.startswith("1") or "/1/" in estado:
            break
        time.sleep(10)
    fila("estado", estado, "%d s" % (time.time() - t0))
    informe(nombre, ns, verdad)
    if "--conservar" in sys.argv:
        fila("conservado", "el Job, el ConfigMap y el prefijo del bucket quedan para mirarlos", "--solo-informe %s" % nombre)
        return
    # limpieza: el Job (Kueue baja jobs-p solo), el ConfigMap y el prefijo del bucket
    kubectl("delete", "job", "-n", ns, nombre, "--wait=false")
    kubectl("delete", "configmap", "-n", ns, nombre)
    c, out, err = sh("gcloud", "storage", "rm", "-r", "gs://%s/%s" % (bucket, prefijo), "--quiet")
    fila("limpieza", "Job y ConfigMap borrados · prefijo del bucket %s" % ("borrado" if c == 0 else "NO borrado: " + err[-80:]))


if __name__ == "__main__":
    main()
