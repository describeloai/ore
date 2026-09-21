"""0033 · LA MEDIDA DE LA MIGRACIÓN A `kind: Dataset` (ORE 0033, paso 2).

Antes de aplicar `ore migrate v1alpha12` a un árbol real, se mide sobre él:
cuántos documentos cambian, qué se va, qué se reescribe, y que el árbol migrado
compila con los mismos diagnósticos o menos y `ore datasets` lista lo mismo.

Lectura, sin instancias: un Job por inquilino clona el árbol de su forja (con
el token de Secret Manager, como las medidas de 0027) y lo devuelve por los
logs (tar + base64, sin `.git`); aquí se descomprime, se corre `ore migrate
--seco` sobre él, se aplica sobre una copia y se comparan `ore validate` y
`ore datasets --json` antes y después. El clúster no cambia: el Job y su
ConfigMap se retiran al final.

    python pruebas-de-fuego/medida-migrar-dataset.py [demo victor …] [--local <dir>]

`--local <dir>` mide un árbol ya en disco, sin clúster.
"""
import base64
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
ORE = os.path.join(RAIZ, "target", "debug", "ore.exe" if os.name == "nt" else "ore")
PROYECTO = "project-8853a180-450d-47be-b83"


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


def ore(*args, cwd=None):
    r = subprocess.run([ORE, *args], capture_output=True, text=True, encoding="utf-8", cwd=cwd)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


ARBOL_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD) ficheros=$(git ls-files | wc -l)"
echo "-----BEGIN ARBOL-----"
tar czf - --exclude=.git . | base64 -w0
echo
echo "-----END ARBOL-----"
"""


def job(ns, celda, nombre):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda, "ore.dev/medida": "migrar-dataset"}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": ["set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\n" % ns],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "arbol", "image": img, "command": ["/bin/sh", "/medida/arbol.sh"],
                            "env": [{"name": "CELDA", "value": celda}],
                            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "medida", "mountPath": "/medida"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "medida", "configMap": {"name": "medida-migrar", "defaultMode": 0o555}}],
        }}},
    }


def traer_arbol(celda, destino):
    ns = "t-" + celda
    nombre = "medida-migrar-dataset"
    os.makedirs(destino, exist_ok=True)
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "medida-migrar", "namespace": ns}, "data": {"arbol.sh": ARBOL_SH}}))
    kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, nombre)))
    t0 = time.time()
    st = {}
    while time.time() - t0 < 600:
        j = kubectl("get", "job", nombre, "-n", ns, "-o", "json")
        st = json.loads(j)["status"] if j else {}
        if st.get("succeeded") or st.get("failed"):
            break
        time.sleep(5)
    salida = kubectl("logs", "-n", ns, "job/" + nombre, "-c", "arbol") or ""
    kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("delete", "configmap", "medida-migrar", "-n", ns, "--ignore-not-found")
    if not st.get("succeeded"):
        print("     MAL el Job no terminó bien:", (salida or "")[:400])
        return None
    cab = [l for l in salida.splitlines() if l.startswith("(arbol)")]
    ini = salida.find("-----BEGIN ARBOL-----")
    fin = salida.find("-----END ARBOL-----")
    if ini < 0 or fin < 0:
        print("     MAL no llegó el árbol:", salida[:300])
        return None
    b64 = salida[ini + len("-----BEGIN ARBOL-----"):fin].strip()
    tgz = os.path.join(destino, "arbol.tgz")
    with open(tgz, "wb") as f:
        f.write(base64.b64decode(b64))
    dir_ = os.path.join(destino, "arbol")
    os.makedirs(dir_, exist_ok=True)
    import tarfile
    with tarfile.open(tgz) as t:
        t.extractall(dir_)
    print("     " + " · ".join(cab), "· %.1f s" % (time.time() - t0))
    return dir_


def contar(dir_):
    n = {"View": 0, "View+materialized": 0, "Table": 0, "Table lago": 0, "Entity": 0, "Function": 0, "docs": 0, "copias/": 0, "datasets/": 0}
    for raiz, _, fs in os.walk(dir_):
        for f in fs:
            p = os.path.join(raiz, f)
            rel = os.path.relpath(p, dir_).replace("\\", "/")
            if rel.startswith("copias/") and f.endswith(".json"):
                n["copias/"] += 1
            if rel.startswith("datasets/") and f.endswith(".json"):
                n["datasets/"] += 1
            if not f.endswith(".yaml"):
                continue
            try:
                t = open(p, encoding="utf-8").read()
            except Exception:
                continue
            k = next((l.split(":", 1)[1].strip() for l in t.splitlines() if l.startswith("kind:")), "")
            if k:
                n["docs"] += 1
            if k in n:
                n[k] += 1
            if k == "View" and "\n  materialized:" in t:
                n["View+materialized"] += 1
            if k == "Table" and "datasource: lago" in t:
                n["Table lago"] += 1
    return n


def diagnosticos(dir_):
    cod, out = ore("validate", dir_)
    codigos = sorted(set(l.split("[")[1].split("]")[0] for l in out.splitlines() if l.startswith(("error[", "warning[")) and "[" in l))
    n = sum(1 for l in out.splitlines() if l.startswith(("error[", "warning[")))
    return cod, n, codigos, out


def punteros(dir_):
    cod, out = ore("datasets", dir_, "--json")
    try:
        j = json.loads(out.strip().splitlines()[-1])
    except Exception:
        return None
    ds = j.get("datasets") if isinstance(j, dict) else j
    return sorted((d.get("nombre") or d.get("dataset") or json.dumps(d)) for d in (ds or []))


def medir(celda, dir_):
    print("  %s" % celda)
    antes = contar(dir_)
    print("     antes:  %s" % " · ".join("%s=%s" % kv for kv in antes.items() if kv[1]))
    cod, n, codigos, out = diagnosticos(dir_)
    print("     ore validate antes: %s diagnósticos %s" % (n, codigos or ""))
    p_antes = punteros(dir_)
    cod, seco = ore("migrate", "v1alpha12", dir_, "--seco")
    resumen = [l for l in seco.splitlines() if "datasets nuevos" in l or l.startswith("  aviso")]
    for l in resumen:
        print("     " + l.strip())
    if cod != 0:
        print("     MAL migrate --seco salió %s: %s" % (cod, seco[:400]))
        return
    copia = dir_ + "-migrado"
    if os.path.exists(copia):
        shutil.rmtree(copia)
    shutil.copytree(dir_, copia)
    cod, real = ore("migrate", "v1alpha12", copia)
    despues = contar(copia)
    print("     después: %s" % " · ".join("%s=%s" % kv for kv in despues.items() if kv[1]))
    cod2, n2, codigos2, out2 = diagnosticos(copia)
    print("     ore validate después: %s diagnósticos %s %s" % (n2, codigos2 or "", "✓ igual o menos" if n2 <= n else "✗ MÁS QUE ANTES"))
    if n2 > n:
        for l in out2.splitlines()[:12]:
            print("        " + l)
    p_despues = punteros(copia)
    if p_antes is not None and p_despues is not None:
        print("     ore datasets: %s → %s %s" % (len(p_antes), len(p_despues), "✓ la misma lista" if p_antes == p_despues else "✗ DISTINTA: %s" % (set(p_antes) ^ set(p_despues))))
    else:
        print("     ore datasets: sin punteros que comparar (antes=%s después=%s)" % (p_antes, p_despues))
    quedan = sum(1 for raiz, _, fs in os.walk(copia) for f in fs if f.endswith(".yaml") and ("materialized:" in open(os.path.join(raiz, f), encoding="utf-8").read() or "datasource: lago" in open(os.path.join(raiz, f), encoding="utf-8").read()))
    print("     `materialized` / `datasource: lago` que quedan: %s %s" % (quedan, "✓" if quedan == 0 else "✗"))


def main():
    args = sys.argv[1:]
    if not os.path.exists(ORE):
        print("falta", ORE, "· cargo build -p ore-cli")
        return 1
    print("0033 · la migración a `kind: Dataset`, medida")
    if "--local" in args:
        d = args[args.index("--local") + 1]
        medir(os.path.basename(d), os.path.abspath(d))
        return 0
    celdas = [a for a in args if not a.startswith("--")] or ["demo", "victor"]
    base = tempfile.mkdtemp(prefix="medida-migrar-")
    for c in celdas:
        d = traer_arbol(c, os.path.join(base, c))
        if d:
            medir(c, d)
    print("  (los árboles quedan en %s; el clúster, como estaba)" % base)
    return 0


if __name__ == "__main__":
    sys.exit(main())
