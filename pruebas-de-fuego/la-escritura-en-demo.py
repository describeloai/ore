#!/usr/bin/env python3
"""
LA ESCRITURA EN DEMO · W3.6c c5 (0031 §11): `write()` desde los tres puestos, EN EL
CLÚSTER, con la identidad real — lo que la prueba de fuego local (`el-puesto.sh`
10, 8, 9) no puede afirmar:

  · `ore-serve-demo` presta un token acotado a la tabla con el token de SU pod
    (Credential Access Boundary por STS), con el `objectCreator` que el
    aprovisionador le da;
  · el puesto (rol `puesto`: sin internet, sólo Google APIs en privado) escribe
    los ficheros en el bucket de demo con ESE token, por `ore-store-gcs` de su
    propia imagen, y el commit va por `/v1/…` al ore-serve del inquilino, que
    escribe el `metadata.json` y empuja el puntero a la forja de verdad;
  · los tres lenguajes escriben lo suyo y cada uno lee lo de los otros por
    `over()`, en sitio, con el mismo JSON.

`over()` resuelve `<paquete>.<vista>` por el puesto (`/puestos/<id>/datos/…`: en
nombre de la persona que lo abrió y en su rama), y aquí no hay puesto —los abre
una persona por OIDC; esta prueba va con el testigo del agente—, así que la
resolución va por la ficha del dataset (`GET /datasets/<ns>/<n>`, que trae el
mismo `metadata_location`): en Python y Node, `puesto.pedir` reescribe esa
ruta; en Java, `Ore.puesto` no se puede sustituir y `over()` se monta con sus
mismas piezas privadas (`iceberg`, `duckdb`, `exportar`, `filasDe`). Todo lo
demás —el `iceberg_scan` en sitio con el token del pod, los tipos, el JSON— es
el `over()` de la imagen.

Un Job en `t-demo` (Kueue → `jobs-p`, 0 → 1 → 0) con los tres contenedores de
puesto (las imágenes de ESTE commit), la cuenta del puesto (`puesto`: sólo lee
el bucket; lo que escribe va con el token prestado) y el testigo del agente
(como `51`); cada
uno corre un guion que hace lo que una celda haría. Los datasets nacen bajo
`<paquete>.medida_escrito_<lenguaje>` en el primer paquete del árbol y se
retiran al final: los documentos y punteros por `DELETE /arbol/…` (un segundo
Job; ⚠️ desde W3.7 gobierno ① un agente no escribe en `/arbol`: si esto se
vuelve a correr, esa limpieza va por git con el testigo de la forja) y los objetos del bucket, SÓLO bajo `ore/v2/datasets/<paquete>_medida_escrito_*`,
desde esta máquina.

Uso:  python pruebas-de-fuego/la-escritura-en-demo.py [--sha <12 hex>] [--sin-limpiar] [--solo-limpiar]
Necesita kubectl (ore-mesh) y gcloud (sesión propia). No imprime ningún token.
"""
import json
import os
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROYECTO = "project-8853a180-450d-47be-b83"
REGISTRO = "europe-west1-docker.pkg.dev/%s/ore" % PROYECTO
INQUILINO = "demo"
NS = "t-" + INQUILINO
BUCKET = "%s-t-%s-copia" % (PROYECTO, INQUILINO)
SHA = sys.argv[sys.argv.index("--sha") + 1] if "--sha" in sys.argv else subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, cwd=RAIZ).stdout.strip()[:12]
NOMBRE = "escritura-" + SHA[:8]


def fila(a, b="", c=""):
    print("  %-46s %-24s %s" % (a, b, c))


def k(*args, entrada=None, ns=NS):
    env = dict(os.environ, MSYS_NO_PATHCONV="1", MSYS2_ARG_CONV_EXCL="*")
    r = subprocess.run(["kubectl", "-n", ns, *args], input=entrada, capture_output=True, text=True, encoding="utf-8", env=env)
    return r.returncode, r.stdout, r.stderr


# ── el testigo del agente: lo mismo que hace `agente.py` ─────────────────────
TESTIGO_PY = r'''
import json, os, time, urllib.parse, urllib.request
def testigo():
    d = os.environ["DIRECCION"].rstrip("/"); realm = os.environ.get("REALM", "rubix")
    cli = open("/puesto/agente-cliente").read().strip(); sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    with urllib.request.urlopen(d + "/realms/%s/protocol/openid-connect/token" % realm, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}
'''

PYTHON = TESTIGO_PY + r'''
import sys, decimal, datetime as dt
sys.path.insert(0, "/opt/ore")
import ore, pyarrow as pa
ore.puesto._cabeceras = testigo(); ore.puesto.id = ""
_pedir = ore.puesto.pedir
def pedir(metodo, ruta, *a, **kw):
    if ruta.startswith("/puestos//datos/"):
        return _pedir("GET", "/datasets/" + ruta.rsplit("/", 1)[1].replace(".", "/"))
    return _pedir(metodo, ruta, *a, **kw)
ore.puesto.pedir = pedir
def di(que, **kw): print("### " + json.dumps(dict(que=que, **kw)), flush=True)
c, r = ore.puesto.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
open("/trabajo/ns", "w").write(ns)
di("paquete", ns=ns, todos=len(r["namespaces"]))
utc = dt.timezone.utc
t = pa.table({
    "n": pa.array([1, 2, 3], pa.int64()),
    "letra": pa.array(["a", "b", None], pa.string()),
    "cuando": pa.array([dt.datetime(2024, 6, 1, 12, 0, tzinfo=utc), None, dt.datetime(2024, 6, 1, 12, 0, 0, 500000, tzinfo=utc)], pa.timestamp("us", tz="UTC")),
    "importe": pa.array([decimal.Decimal("1.50"), decimal.Decimal("2.25"), None], pa.decimal128(10, 2)),
})
t0 = time.time()
e = ore.write(ns + ".medida_escrito_py", t)
di("write_py", ms=int((time.time() - t0) * 1000), filas=e["filas"], repetida=e["repetida"], metadata_location=e["metadata_location"])
t0 = time.time()
e2 = ore.write(ns + ".medida_escrito_py", t)
di("write_py_repetida", ms=int((time.time() - t0) * 1000), repetida=e2["repetida"])
t0 = time.time()
j = ore.tabla(ore.over(ns + ".medida_escrito_py"))
di("over_py", ms=int((time.time() - t0) * 1000), columnas=[(c["name"], c["type"]) for c in j["columnas"]], filas=j["filas"], total=j["total"])
# lo de los otros, cuando esté
for otro in ("node", "jvm"):
    for _ in range(90):
        try:
            j = ore.tabla(ore.over(ns + ".medida_escrito_" + otro)); break
        except Exception as ex:
            time.sleep(4); j = None
    di("over_" + otro + "_desde_py", filas=j["filas"] if j else None, total=j["total"] if j else None)
c, r = ore.puesto.pedir("GET", "/datasets/%s/medida_escrito_py" % ns)
di("ficha_py", codigo=c, snapshots=len(r.get("snapshots", [])), escrito_por=r.get("escrito_por"), retencion=r.get("retencion"))
'''

NODE = r'''
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import ore from "ore";
const { puesto, over, write, tabla } = ore;
async function testigo() {
  const d = process.env.DIRECCION.replace(/\/+$/, ""), realm = process.env.REALM ?? "rubix";
  const cli = readFileSync("/puesto/agente-cliente", "utf8").trim(), sec = readFileSync("/puesto/agente-secreto", "utf8").trim();
  const r = await fetch(`${d}/realms/${realm}/protocol/openid-connect/token`, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ grant_type: "client_credentials", client_id: cli, client_secret: sec }) });
  return { authorization: "Bearer " + (await r.json()).access_token };
}
const di = (que, o) => console.log("### " + JSON.stringify({ que, ...o }));
puesto._cabeceras = await testigo(); puesto.id = "";
const pedir_ = puesto.pedir.bind(puesto);
puesto.pedir = (metodo, ruta, ...a) => ruta.startsWith("/puestos//datos/") ? pedir_("GET", "/datasets/" + ruta.split("/").pop().replace(".", "/")) : pedir_(metodo, ruta, ...a);
let ns = "";
for (let i = 0; i < 60 && !ns; i++) { if (existsSync("/trabajo/ns")) ns = readFileSync("/trabajo/ns", "utf8").trim(); else await new Promise((r) => setTimeout(r, 2000)); }
if (!ns) { const [, r] = await puesto.pedir("GET", "/v1/namespaces"); ns = r.namespaces.map((n) => n[0]).sort()[0]; }
const filas = [
  { n: 1n, letra: "a", cuando: new Date("2024-06-01T12:00:00Z"), importe: 1.5 },
  { n: 2n, letra: "b", cuando: null, importe: 2.25 },
  { n: 3n, letra: null, cuando: new Date("2024-06-01T12:00:00.500Z"), importe: null },
];
let t0 = Date.now();
const e = await write(`${ns}.medida_escrito_node`, filas);
di("write_node", { ms: Date.now() - t0, filas: e.filas, repetida: e.repetida, metadata_location: e.metadata_location });
t0 = Date.now();
const j = tabla(await over(`${ns}.medida_escrito_node`));
di("over_node", { ms: Date.now() - t0, columnas: j.columnas.map((c) => [c.name, c.type]), filas: j.filas, total: j.total });
for (const otro of ["py", "jvm"]) {
  let jj = null;
  for (let i = 0; i < 90 && !jj; i++) { try { jj = tabla(await over(`${ns}.medida_escrito_${otro}`)); } catch { await new Promise((r) => setTimeout(r, 4000)); } }
  di(`over_${otro}_desde_node`, { filas: jj?.filas ?? null, total: jj?.total ?? null });
}
'''

JAVA = r'''
package ore;
import java.nio.file.*;
import java.net.URI;
import java.net.http.*;
import java.util.*;
public class Medida {
    static void di(String que, Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>(); m.put("que", que);
        for (int i = 0; i + 1 < kv.length; i += 2) m.put(String.valueOf(kv[i]), kv[i + 1]);
        System.out.println("### " + Json.escribir(m)); System.out.flush();
    }
    static Map<String, String> testigo() throws Exception {
        String d = System.getenv("DIRECCION").replaceAll("/+$", ""), realm = System.getenv().getOrDefault("REALM", "rubix");
        String cli = Files.readString(Path.of("/puesto/agente-cliente")).trim(), sec = Files.readString(Path.of("/puesto/agente-secreto")).trim();
        String cuerpo = "grant_type=client_credentials&client_id=" + java.net.URLEncoder.encode(cli, "UTF-8") + "&client_secret=" + java.net.URLEncoder.encode(sec, "UTF-8");
        HttpResponse<String> r = HttpClient.newHttpClient().send(HttpRequest.newBuilder(URI.create(d + "/realms/" + realm + "/protocol/openid-connect/token")).header("content-type", "application/x-www-form-urlencoded").POST(HttpRequest.BodyPublishers.ofString(cuerpo)).build(), HttpResponse.BodyHandlers.ofString());
        return Map.of("authorization", "Bearer " + Json.objeto(r.body()).get("access_token"));
    }
    static java.lang.reflect.Method pieza(String nombre, Class<?>... tipos) throws Exception {
        java.lang.reflect.Method m = Ore.class.getDeclaredMethod(nombre, tipos); m.setAccessible(true); return m;
    }
    /** {@code Ore.over(vista)} con sus piezas, resolviendo por la ficha del dataset (aquí no hay puesto). */
    static Ore.Filas over(String vista) throws Exception {
        String[] p = vista.split("\\.");
        Ore.Respuesta r = Ore.puesto.pedir("GET", "/datasets/" + p[0] + "/" + p[1], null, java.time.Duration.ofSeconds(30));
        if (r.codigo() != 200) throw new IllegalArgumentException("la ficha de `" + vista + "`: " + r.codigo() + " " + r.error());
        String fuente = (String) pieza("iceberg", String.class).invoke(null, String.valueOf(r.cuerpo().get("metadata_location")));
        java.sql.Connection con = (java.sql.Connection) pieza("duckdb").invoke(null);
        long total;
        try (java.sql.Statement s = con.createStatement(); java.sql.ResultSet rs = s.executeQuery("select count(*) from " + fuente)) { rs.next(); total = rs.getLong(1); }
        Object lector = pieza("exportar", java.sql.Statement.class, String.class, int.class).invoke(null, con.createStatement(), "select * from " + fuente, 8192);
        return (Ore.Filas) pieza("filasDe", org.apache.arrow.vector.ipc.ArrowReader.class, int.class, boolean.class, Long.class, String.class).invoke(null, lector, Ore.LIMITE, false, total, "over(" + vista + ")");
    }
    public static void main(String[] a) throws Exception {
        Ore.puesto.cabeceras = testigo();
        String ns = "";
        for (int i = 0; i < 60 && ns.isEmpty(); i++) { if (Files.exists(Path.of("/trabajo/ns"))) ns = Files.readString(Path.of("/trabajo/ns")).trim(); else Thread.sleep(2000); }
        List<Map<String, Object>> filas = new ArrayList<>();
        Map<String, Object> f1 = new LinkedHashMap<>(); f1.put("n", 1L); f1.put("letra", "a"); f1.put("cuando", java.time.Instant.parse("2024-06-01T12:00:00Z")); f1.put("importe", new java.math.BigDecimal("1.50")); filas.add(f1);
        Map<String, Object> f2 = new LinkedHashMap<>(); f2.put("n", 2L); f2.put("letra", "b"); f2.put("cuando", null); f2.put("importe", new java.math.BigDecimal("2.25")); filas.add(f2);
        Map<String, Object> f3 = new LinkedHashMap<>(); f3.put("n", 3L); f3.put("letra", null); f3.put("cuando", java.time.Instant.parse("2024-06-01T12:00:00.500Z")); f3.put("importe", null); filas.add(f3);
        // En orden: un `Map.of` no lo guarda, y la tabla nace con las columnas como llegan.
        Map<String, String> tipos = new LinkedHashMap<>();
        tipos.put("n", "int64"); tipos.put("letra", "string"); tipos.put("cuando", "timestamp[us, tz=UTC]"); tipos.put("importe", "decimal128(10, 2)");
        Ore.Filas fs = new Ore.Filas(tipos, 3L, false);
        fs.addAll(filas);
        long t0 = System.currentTimeMillis();
        Map<String, Object> e = Ore.write(ns + ".medida_escrito_jvm", fs);
        di("write_jvm", "ms", System.currentTimeMillis() - t0, "filas", e.get("filas"), "repetida", e.get("repetida"), "metadata_location", e.get("metadata_location"));
        t0 = System.currentTimeMillis();
        Map<String, Object> j = Ore.tabla(over(ns + ".medida_escrito_jvm"), 200);
        di("over_jvm", "ms", System.currentTimeMillis() - t0, "columnas", j.get("columnas"), "filas", j.get("filas"), "total", j.get("total"));
        for (String otro : new String[] { "py", "node" }) {
            Map<String, Object> jj = null;
            for (int i = 0; i < 90 && jj == null; i++) { try { jj = Ore.tabla(over(ns + ".medida_escrito_" + otro), 200); } catch (Exception ex) { Thread.sleep(4000); } }
            di("over_" + otro + "_desde_jvm", "filas", jj == null ? null : jj.get("filas"), "total", jj == null ? null : jj.get("total"));
        }
    }
}
'''

LIMPIAR = TESTIGO_PY + r'''
import sys
sys.path.insert(0, "/opt/ore")
import ore
ore.puesto._cabeceras = testigo(); ore.puesto.id = ""
c, r = ore.puesto.pedir("GET", "/v1/namespaces")
ns = sorted(n[0] for n in r["namespaces"])[0]
for l in ("py", "node", "jvm"):
    for ruta in ("packages/%s/tables/medida_escrito_%s.yaml" % (ns, l), "datasets/%s_medida_escrito_%s.json" % (ns, l)):
        c, r = ore.puesto.pedir("DELETE", "/arbol/" + ruta)
        print("### " + json.dumps({"que": "retirado", "ruta": ruta, "codigo": c, "commit": (r or {}).get("commit", "")}), flush=True)
print("### " + json.dumps({"que": "paquete", "ns": ns}), flush=True)
'''


def contenedor(nombre, imagen, mando):
    return {
        "name": nombre,
        "image": imagen,
        "imagePullPolicy": "Always",
        "env": [
            {"name": "HOME", "value": "/tmp"},
            {"name": "ORE_SERVE", "value": "http://ore-serve.%s.svc.cluster.local:8080" % NS},
            {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"},
            {"name": "REALM", "value": "rubix"},
            {"name": "ORE_CELDAS", "value": "/trabajo/celdas"},
        ],
        "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guiones", "mountPath": "/guiones", "readOnly": True}, {"name": "trabajo", "mountPath": "/trabajo"}],
        "workingDir": "/trabajo",
        "command": ["/bin/sh", "-c"],
        "args": [mando],
        "resources": {"requests": {"cpu": "500m", "memory": "1Gi"}, "limits": {"cpu": "2", "memory": "3Gi"}},
        "securityContext": {"allowPrivilegeEscalation": False, "runAsNonRoot": True, "runAsUser": 65532, "seccompProfile": {"type": "RuntimeDefault"}, "capabilities": {"drop": ["ALL"]}},
    }


def job(nombre, contenedores, guiones):
    j = {
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": nombre, "namespace": NS, "labels": {"kueue.x-k8s.io/queue-name": "cola", "ore.dev/tenant": INQUILINO, "ore.dev/rol": "puesto"}},
        "spec": {
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": 1800,
            "activeDeadlineSeconds": 1500,
            "template": {
                "metadata": {"labels": {"ore.dev/rol": "puesto", "ore.dev/tenant": INQUILINO}},
                "spec": {
                    "restartPolicy": "Never",
                    "serviceAccountName": "puesto",
                    "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "trabajo", "emptyDir": {}}, {"name": "guiones", "configMap": {"name": nombre}}],
                    "initContainers": [{
                        "name": "traer-el-testigo",
                        "image": REGISTRO + "/ore-drivers:main",
                        "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}],
                        "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}],
                        "command": ["/bin/sh", "-c"],
                        "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=t-%s-agente-$p --out-file=/puesto/agente-$p; chmod 0444 /puesto/agente-$p; done\necho testigo puesto" % INQUILINO],
                        "resources": {"requests": {"cpu": "50m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                        "securityContext": {"allowPrivilegeEscalation": False, "capabilities": {"drop": ["ALL"]}},
                    }],
                    "containers": contenedores,
                },
            },
        },
    }
    cm = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": NS}, "data": guiones}
    return json.dumps(cm) + "\n---\n" + json.dumps(j)


def esperar(nombre, plazo=1500):
    t0 = time.time()
    fase = ""
    while time.time() - t0 < plazo:
        c, out, _ = k("get", "job", nombre, "-o", "jsonpath={.status.succeeded}{' '}{.status.failed}")
        ok, mal = (out.split() + ["", ""])[:2]
        if ok == "1":
            return "ok", int(time.time() - t0)
        if mal and mal != "0":
            return "falló", int(time.time() - t0)
        c, out, _ = k("get", "pods", "-l", "job-name=" + nombre, "-o", "jsonpath={.items[0].status.phase}")
        if out != fase:
            fase = out
            fila("  el pod", fase or "(esperando nodo)", "%ds" % int(time.time() - t0))
        time.sleep(10)
    return "plazo", int(time.time() - t0)


def eventos(nombre):
    out = {}
    for cont in ("python", "node", "jvm"):
        c, log, _ = k("logs", "job/" + nombre, "-c", cont)
        out[cont] = [json.loads(l[4:]) for l in log.splitlines() if l.startswith("### ")]
        raro = [l for l in log.splitlines() if not l.startswith("### ") and l.strip()]
        if raro:
            out[cont + "_texto"] = raro[-6:]
    return out


def main():
    solo_limpiar = "--solo-limpiar" in sys.argv
    print("la escritura en demo · imágenes %s · %s" % (SHA, NOMBRE))
    c, out, err = k("get", "deployment", "ore-serve", "-o", "jsonpath={.spec.template.metadata.annotations.ore\\.dev/commit}")
    fila("ore-serve en %s corre" % NS, (out or "?")[:12], "(tiene que llevar el catálogo REST: c3 o posterior)")
    if not solo_limpiar:
        guiones = {"medida.py": PYTHON, "medida.mjs": NODE, "Medida.java": JAVA}
        conts = [
            contenedor("python", "%s/puesto-python:%s" % (REGISTRO, SHA), "python3 /guiones/medida.py"),
            contenedor("node", "%s/puesto-node:%s" % (REGISTRO, SHA), "cp /guiones/medida.mjs /trabajo/medida.mjs && ln -s /opt/ore/node_modules /trabajo/node_modules && node --no-warnings /trabajo/medida.mjs"),
            contenedor("jvm", "%s/puesto-jvm:%s" % (REGISTRO, SHA), "mkdir -p /trabajo/c/ore && cp /guiones/Medida.java /trabajo/c/ore/ && javac -d /trabajo/c -cp '/opt/ore/clases:/opt/ore/lib/*' /trabajo/c/ore/Medida.java && java --add-opens=java.base/java.nio=ALL-UNNAMED -cp '/trabajo/c:/opt/ore/clases:/opt/ore/lib/*' ore.Medida"),
        ]
        k("delete", "job", NOMBRE, "--ignore-not-found"); k("delete", "configmap", NOMBRE, "--ignore-not-found")
        c, out, err = k("apply", "-f", "-", entrada=job(NOMBRE, conts, guiones))
        if c != 0:
            print(err); sys.exit(1)
        fila("Job", NOMBRE, "en %s (jobs-p por Kueue)" % NS)
        estado, seg = esperar(NOMBRE)
        fila("el Job", estado, "%ds" % seg)
        ev = eventos(NOMBRE)
        for cont in ("python", "node", "jvm"):
            for e in ev.get(cont, []):
                q = e.pop("que")
                fila("%s · %s" % (cont, q), "%s ms" % e.pop("ms") if "ms" in e else "", json.dumps(e, ensure_ascii=False)[:150])
            for l in ev.get(cont + "_texto", []):
                fila("  %s dijo" % cont, "", l[:150])
        with open(os.path.join(tempfile.gettempdir(), NOMBRE + ".json"), "w") as f:
            json.dump(ev, f, indent=1, ensure_ascii=False)
        fila("los eventos", os.path.join(tempfile.gettempdir(), NOMBRE + ".json"))
        if estado != "ok":
            for cont in ("python", "node", "jvm"):
                c, log, _ = k("logs", "job/" + NOMBRE, "-c", cont, "--tail=12")
                print("── %s ──\n%s" % (cont, log[-1500:]))
    if "--sin-limpiar" in sys.argv:
        return
    # ── limpiar: los documentos y punteros por el árbol, los objetos desde aquí ──
    lim = NOMBRE + "-limpiar"
    k("delete", "job", lim, "--ignore-not-found"); k("delete", "configmap", lim, "--ignore-not-found")
    c, out, err = k("apply", "-f", "-", entrada=job(lim, [contenedor("python", "%s/puesto-python:%s" % (REGISTRO, SHA), "python3 /guiones/limpiar.py")], {"limpiar.py": LIMPIAR}))
    estado, seg = esperar(lim, 900)
    c, log, _ = k("logs", "job/" + lim, "-c", "python")
    ns = ""
    for l in log.splitlines():
        if l.startswith("### "):
            e = json.loads(l[4:])
            if e.get("que") == "paquete":
                ns = e["ns"]
            else:
                fila("retirado del árbol", e.get("ruta", ""), "HTTP %s" % e.get("codigo"))
    fila("el Job de limpieza", estado, "%ds" % seg)
    if ns:
        for l in ("py", "node", "jvm"):
            pref = "gs://%s/ore/v2/datasets/%s_medida_escrito_%s/" % (BUCKET, ns, l)
            r = subprocess.run(["gcloud", "storage", "rm", "-r", "-q", pref], capture_output=True, text=True, shell=(os.name == "nt"))
            fila("retirado del bucket", pref.split("/ore/v2/")[1], "ok" if r.returncode == 0 else r.stderr.strip()[:80])
    k("delete", "job", NOMBRE, "--ignore-not-found"); k("delete", "configmap", NOMBRE, "--ignore-not-found")
    k("delete", "job", lim, "--ignore-not-found"); k("delete", "configmap", lim, "--ignore-not-found")
    c, out, _ = k("get", "jobs", "-o", "name")
    fila("jobs que quedan en %s" % NS, out.strip().replace("\n", " ") or "ninguno")


if __name__ == "__main__":
    main()
