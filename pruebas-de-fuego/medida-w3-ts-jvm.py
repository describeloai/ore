#!/usr/bin/env python3
"""
MEDIDA · W3.4 · TS y JVM en el puesto (19 de septiembre)

Antes de escribir `puesto-node:1`, `puesto-jvm:1` y sus agentes (0031 W3.4),
cuánto cuesta cada cosa —de punta a punta, con el mismo puesto que hoy corre
Python (rol `puesto`, 2 CPU · 4 GiB, sin internet)— y si el diseño del agente
Python vale igual para los otros dos lenguajes:

  §0  EN LOCAL     Node quita los tipos de TS SIN transpilador (`node:module`
                   stripTypeScriptTypes, 22.13+), y el evaluador del REPL
                   (`repl.start().eval`) da lo que tiene el kernel Python:
                   estado persistente, `await` arriba, valor de la última
                   expresión. Cuánto tarda cada celda.
  §1  LAS IMÁGENES lo que pesan (comprimidas, linux/amd64) las candidatas de
                   Docker Hub — CONSULTADO al registro, no bajado.
  §2  EN EL PUESTO un Job en el inquilino con DOS contenedores desde
                   `mirror.gcr.io` (¿llega la malla?): Node 24 (arranque,
                   TS de un fichero por `import`, celdas por el REPL) y un JDK
                   21 (arranque de la JVM, JShell en proceso: primera celda,
                   segunda, error, salida) — y el pull de cada imagen.

Uso:
  python pruebas-de-fuego/medida-w3-ts-jvm.py [--inquilino victor] [--solo-local]

Crea un Job `ts-jvm-medida` en el namespace del inquilino y lo borra al acabar.
Nada de pago fuera del clúster (`jobs-p` escala 0→1→0).
"""
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
os.environ["MSYS_NO_PATHCONV"] = "1"
os.environ["MSYS2_ARG_CONV_EXCL"] = "*"

NODE = "mirror.gcr.io/library/node:24-slim"
JVM = "mirror.gcr.io/library/eclipse-temurin:21-jdk-alpine"


def fila(k, v, nota=""):
    print("  %-46s %-26s %s" % (k, v, nota))


def sh(*args, entrada=None, cwd=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None, capture_output=True, cwd=cwd)
    return r.stdout.decode("utf-8", "replace") + r.stderr.decode("utf-8", "replace")


def k(*args, ns=None, entrada=None):
    return sh(*(["kubectl"] + (["-n", ns] if ns else []) + list(args)), entrada=entrada)


# ── §0 y §2: lo que corre en Node ──────────────────────────────────────────
NODE_MJS = r'''
import { stripTypeScriptTypes } from "node:module";
import repl from "node:repl";
import { PassThrough } from "node:stream";
import { writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const T0 = performance.now();
const dice = (o) => console.log("MEDIDA " + JSON.stringify({ ...o, t: Math.round(performance.now() - T0) / 1000 }));
dice({ paso: "node-arranca", version: process.version, ms: Math.round(process.uptime() * 1000), tipos: process.features.typescript ?? "no" });
// 1 · quitar tipos sin transpilador
const TS = `
interface Cliente { nombre: string; pais: string }
const clientes: Cliente[] = [{ nombre: "ana", pais: "ES" }, { nombre: "bo", pais: "PT" }];
function porPais(xs: Cliente[]): Record<string, number> {
  const r: Record<string, number> = {};
  for (const c of xs) r[c.pais] = (r[c.pais] ?? 0) + 1;
  return r;
}
porPais(clientes)
`;
let t = performance.now();
const js = stripTypeScriptTypes(TS);
dice({ paso: "strip-ts", ms: Math.round((performance.now() - t) * 100) / 100, bytes: js.length });
// 2 · el evaluador del REPL: estado, await, ultima expresion
const out = new PassThrough(); let capturado = ""; out.on("data", (d) => (capturado += d));
t = performance.now();
const r = repl.start({ input: new PassThrough(), output: out, terminal: false, useGlobal: false, prompt: "" });
dice({ paso: "repl-crea", ms: Math.round((performance.now() - t) * 100) / 100 });
// Un error SINCRONO en la celda no llega al callback del eval: va al dominio del
// REPL (que lo escribiria por output y seguiria). Se le quita ese oido y se le pone
// el nuestro: la celda pendiente acaba con el error. Medido: sin esto, la celda 5 nunca contesta.
let pendiente = null;
r._domain.removeAllListeners("error");
r._domain.on("error", (e) => { const p = pendiente; pendiente = null; if (p) p({ e }); });
const celda = (codigo) => new Promise((ok) => { const t = performance.now(); pendiente = (x) => ok({ ...x, ms: Math.round((performance.now() - t) * 100) / 100 }); r.eval(codigo, r.context, "celda", (e, v) => { pendiente = null; ok({ e, v, ms: Math.round((performance.now() - t) * 100) / 100 }); }); });
let c = await celda(js); dice({ paso: "celda-1-ts", ms: c.ms, valor: JSON.stringify(c.v), error: c.e ? String(c.e) : null });
c = await celda("clientes.length"); dice({ paso: "celda-2-estado", ms: c.ms, valor: c.v });
c = await celda("const x = await Promise.resolve(41); x + 1"); dice({ paso: "celda-3-await", ms: c.ms, valor: c.v });
c = await celda("console.log('hola', x); undefined"); dice({ paso: "celda-4-salida", ms: c.ms, capturado: capturado.trim() });
c = await celda("noExiste + 1"); dice({ paso: "celda-5-error", ms: c.ms, error: c.e ? String(c.e.message ?? c.e) : null });
c = await celda("let y: number = 3; y"); dice({ paso: "celda-6-ts-sin-strip", ms: c.ms, valor: c.v ?? null, error: c.e ? "SyntaxError" : null });
c = await celda(stripTypeScriptTypes("let z: number = 3; z * 2")); dice({ paso: "celda-7-ts-strip", ms: c.ms, valor: c.v });
// 3 · una funcion del arbol: un fichero .ts importado tal cual
const f = `${process.env.TMPDIR ?? "/tmp"}/saludo.ts`;
writeFileSync(f, `export function saludo(n: string): string { return "hola " + n }\n`);
t = performance.now();
try { const m = await import(pathToFileURL(f).href); dice({ paso: "import-ts", ms: Math.round((performance.now() - t) * 100) / 100, valor: m.saludo("ore") }); }
catch (e) { dice({ paso: "import-ts", ms: Math.round((performance.now() - t) * 100) / 100, error: String(e.message).split("\n")[0] }); }
dice({ paso: "fin" });
r.close();
'''

# ── §2: lo que corre en la JVM ─────────────────────────────────────────────
JAVA = r'''
import java.lang.management.ManagementFactory;
import java.util.*;
import jdk.jshell.*;
public class Medida {
    static final long T0 = System.nanoTime();
    static void dice(Object... kv) {
        StringBuilder b = new StringBuilder("MEDIDA {");
        for (int i = 0; i < kv.length; i += 2) {
            if (i > 0) b.append(",");
            b.append('"').append(kv[i]).append("\":");
            Object v = kv[i + 1];
            if (v instanceof Number) b.append(v); else b.append('"').append(String.valueOf(v).replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", " ")).append('"');
        }
        b.append(",\"t\":").append((System.nanoTime() - T0) / 1e9);
        System.out.println(b.append("}"));
    }
    static double ms(long t) { return Math.round((System.nanoTime() - t) / 1e4) / 100.0; }
    static void celda(JShell js, String paso, String codigo) {
        long t = System.nanoTime();
        List<SnippetEvent> evs = js.eval(codigo);
        String valor = null, error = null;
        for (SnippetEvent e : evs) {
            if (e.exception() != null) error = e.exception().toString();
            else if (e.status() == Snippet.Status.REJECTED) {
                StringBuilder d = new StringBuilder();
                js.diagnostics(e.snippet()).forEach(x -> d.append(x.getMessage(Locale.ENGLISH)).append("; "));
                error = d.toString();
            } else if (e.value() != null) valor = e.value();
        }
        dice("paso", paso, "ms", ms(t), "valor", valor == null ? "" : valor, "error", error == null ? "" : error);
    }
    public static void main(String[] a) throws Exception {
        dice("paso", "jvm-arranca", "ms", ManagementFactory.getRuntimeMXBean().getUptime(), "version", Runtime.version().toString());
        long t = System.nanoTime();
        JShell js = JShell.builder().executionEngine("local").build();
        dice("paso", "jshell-crea", "ms", ms(t), "motor", "local");
        celda(js, "celda-1-import", "import java.util.*;");
        celda(js, "celda-2-record", "record Cliente(String nombre, String pais) {}");
        celda(js, "celda-3-estado", "var clientes = List.of(new Cliente(\"ana\", \"ES\"), new Cliente(\"bo\", \"PT\"));");
        celda(js, "celda-4-expr", "clientes.stream().filter(c -> c.pais().equals(\"ES\")).count()");
        celda(js, "celda-5-error", "int y = \"a\";");
        celda(js, "celda-6-excepcion", "1 / 0");
        celda(js, "celda-7-metodo", "String saludo(String n) { return \"hola \" + n; }");
        celda(js, "celda-8-llama", "saludo(\"ore\")");
        celda(js, "celda-9-caliente", "clientes.size() * 21");
        t = System.nanoTime();
        JShell remoto = JShell.create();
        remoto.eval("1 + 1");
        dice("paso", "jshell-remoto", "ms", ms(t), "motor", "jdi (otro proceso)");
        remoto.close();
        js.close();
        dice("paso", "fin");
    }
}
'''


def job_yaml(ns, inq, nombre):
    return f"""apiVersion: batch/v1
kind: Job
metadata:
  name: {nombre}
  namespace: {ns}
  labels:
    kueue.x-k8s.io/queue-name: cola
    ore.dev/tenant: {inq}
    ore.dev/rol: puesto
    ore.dev/medida: w3-ts-jvm
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 600
  activeDeadlineSeconds: 900
  template:
    metadata:
      labels:
        ore.dev/rol: puesto
        ore.dev/tenant: {inq}
    spec:
      restartPolicy: Never
      serviceAccountName: driver
      volumes:
        - name: guion
          configMap: {{ name: {nombre} }}
        - name: tmp-node
          emptyDir: {{}}
        - name: tmp-jvm
          emptyDir: {{}}
      containers:
        - name: node
          image: {NODE}
          imagePullPolicy: Always
          command: ["node", "/guion/medida.mjs"]
          env:
            - {{ name: TMPDIR, value: /tmp }}
            - {{ name: HOME, value: /tmp }}
          volumeMounts:
            - {{ name: guion, mountPath: /guion, readOnly: true }}
            - {{ name: tmp-node, mountPath: /tmp }}
          resources:
            requests: {{cpu: "500m", memory: 1Gi}}
            limits:   {{cpu: "1", memory: 2Gi}}
          securityContext:
            allowPrivilegeEscalation: false
            runAsNonRoot: true
            runAsUser: 65532
            seccompProfile: {{ type: RuntimeDefault }}
            capabilities: {{ drop: [ALL] }}
        - name: jvm
          image: {JVM}
          imagePullPolicy: Always
          command: ["java", "-Xshare:auto", "-XX:TieredStopAtLevel=1", "/guion/Medida.java"]
          env:
            - {{ name: HOME, value: /tmp }}
          volumeMounts:
            - {{ name: guion, mountPath: /guion, readOnly: true }}
            - {{ name: tmp-jvm, mountPath: /tmp }}
          resources:
            requests: {{cpu: "500m", memory: 1Gi}}
            limits:   {{cpu: "1", memory: 2Gi}}
          securityContext:
            allowPrivilegeEscalation: false
            runAsNonRoot: true
            runAsUser: 65532
            seccompProfile: {{ type: RuntimeDefault }}
            capabilities: {{ drop: [ALL] }}
"""


def lineas_medida(texto, vistos, sangria="  "):
    for l in texto.splitlines():
        if l.startswith("MEDIDA ") and l not in vistos:
            vistos.add(l)
            try:
                m = json.loads(l[7:])
            except ValueError:
                continue
            paso = m.pop("paso"); m.pop("t", 0); ms = m.pop("ms", None)
            fila("%s%s" % (sangria, paso), " · ".join("%s %s" % (a, b) for a, b in m.items() if b not in ("", None))[:60], "%s ms" % ms if ms is not None else "")


def local():
    print("§0 · en local: Node %s" % sh("node", "--version").strip())
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "medida.mjs")
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(NODE_MJS)
        salida = sh("node", "--no-warnings", p)
    vistos = set()
    lineas_medida(salida, vistos)
    if not vistos:
        print(salida[-800:])


def imagenes():
    print("§1 · las imágenes (Docker Hub, linux/amd64, comprimidas)")
    for ref in ["node:24-slim", "node:24-alpine", "node:22-slim", "eclipse-temurin:21-jdk-alpine", "eclipse-temurin:21-jre-alpine", "eclipse-temurin:21-jdk-noble", "python:3.12-slim"]:
        repo, tag = ref.split(":")
        try:
            tok = json.load(urllib.request.urlopen(f"https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/{repo}:pull"))["token"]
            def pide(ruta, acepta):
                q = urllib.request.Request(f"https://registry-1.docker.io/v2/library/{repo}/{ruta}", headers={"Authorization": "Bearer " + tok, "Accept": acepta})
                return json.load(urllib.request.urlopen(q))
            lista = pide(f"manifests/{tag}", "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json")
            m = [x for x in lista.get("manifests", []) if x.get("platform", {}).get("architecture") == "amd64" and x["platform"].get("os") == "linux"]
            if not m:
                fila(ref, "sin linux/amd64"); continue
            man = pide("manifests/" + m[0]["digest"], "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json")
            mb = sum(l["size"] for l in man["layers"]) / 1e6
            fila(ref, "%.0f MB" % mb, "%d capas" % len(man["layers"]))
        except Exception as e:  # noqa: BLE001
            fila(ref, "?", str(e)[:60])


def cluster(inq):
    ns = f"t-{inq}"
    nombre = "ts-jvm-medida"
    print("§2 · en el puesto (%s, rol puesto, jobs-p): %s y %s" % (inq, NODE.split("/")[-1], JVM.split("/")[-1]))
    k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
    k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)
    time.sleep(2)
    try:
        with tempfile.TemporaryDirectory() as d:
            for n, txt in [("medida.mjs", NODE_MJS), ("Medida.java", JAVA)]:
                with open(os.path.join(d, n), "w", encoding="utf-8", newline="\n") as f:
                    f.write(txt)
            k("create", "configmap", nombre, f"--from-file=medida.mjs={d}/medida.mjs", f"--from-file=Medida.java={d}/Medida.java", ns=ns)
        k("apply", "-f", "-", ns=ns, entrada=job_yaml(ns, inq, nombre))
        t0 = time.time(); pod = None; vistos = set(); fases = {}
        while time.time() - t0 < 900:
            ps = json.loads(k("get", "pods", "-l", f"job-name={nombre}", "-o", "json", ns=ns) or "{}").get("items", [])
            if ps:
                pod = ps[0]["metadata"]["name"]
                for cs in ps[0]["status"].get("containerStatuses", []):
                    est = cs.get("state", {})
                    if "terminated" in est and cs["name"] not in fases:
                        fases[cs["name"]] = est["terminated"]
                        lineas_medida(k("logs", pod, "-c", cs["name"], ns=ns), vistos, "  %s · " % cs["name"])
                        fila("  %s · contenedor" % cs["name"], est["terminated"].get("reason", ""), "salida %s" % est["terminated"].get("exitCode"))
                    elif "waiting" in est and est["waiting"].get("reason") in ("ErrImagePull", "ImagePullBackOff") and cs["name"] not in fases:
                        fases[cs["name"]] = est["waiting"]
                        fila("  %s · imagen" % cs["name"], est["waiting"]["reason"], (est["waiting"].get("message") or "")[:70])
                if len(fases) == 2 or ps[0]["status"].get("phase") in ("Succeeded", "Failed"):
                    break
            time.sleep(5)
        if pod:
            for ev in json.loads(k("get", "events", "--field-selector", f"involvedObject.name={pod}", "-o", "json", ns=ns) or "{}").get("items", []):
                msg = ev.get("message", "")
                if ev.get("reason") == "Pulled":
                    fila("  pull", msg.split('"')[1].split("/")[-1] if '"' in msg else "", msg.split(" in ")[-1][:40] if " in " in msg else msg[:40])
                elif ev.get("reason") in ("Scheduled", "TriggeredScaleUp", "Failed"):
                    fila("  " + ev["reason"].lower(), "", msg[:70])
        fila("el Job", "%d s en total" % int(time.time() - t0), "(frío si el pool estaba a cero)")
    finally:
        k("delete", "job", nombre, "--ignore-not-found", "--wait=false", ns=ns)
        k("delete", "configmap", nombre, "--ignore-not-found", ns=ns)
        fila("limpieza", "Job y ConfigMap fuera")


def main():
    inq = "victor"
    if "--inquilino" in sys.argv:
        inq = sys.argv[sys.argv.index("--inquilino") + 1]
    print("MEDIDA · W3.4 · TS y JVM en el puesto · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    local()
    imagenes()
    if "--solo-local" not in sys.argv:
        cluster(inq)


if __name__ == "__main__":
    main()
