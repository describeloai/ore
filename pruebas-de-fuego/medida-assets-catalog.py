"""0034 · LA MEDIDA DEL CATÁLOGO DE ASSETS: qué sirve hoy el backend, cómo, y qué falta.

«Lo que se mide antes de pintar» (0034): contra los inquilinos reales, ítem a ítem,
qué devuelve cada ruta de ②, en cuánto, con qué forma, y qué le falta para
(a) listar todos los ítems de un paquete por carpeta en una llamada,
(b) decir de cada dataset qué lo define y si es identidad,
(c) servir las capas de ③ por ítem, y
(d) Function y Action por `/documentos`.

Un Job por inquilino (como `migrar-a-dataset.py --comprobar`): el agente del
inquilino pide su token y pregunta al ore-serve VIVO —no al binario local—;
aquí se cruza lo que cada ruta sirve con lo que el árbol tiene (`GET /arbol`,
el índice con `kind` por fichero) y se imprime la tabla. Ningún token se
imprime; el clúster queda como estaba.

    python pruebas-de-fuego/medida-assets-catalog.py [demo victor …]
"""
import io
import json
import os
import subprocess
import sys
import time
from collections import Counter, defaultdict

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
PROYECTO = "project-8853a180-450d-47be-b83"


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


# Dentro: pide cada ruta, mide, y vuelca TODO como un JSON por línea
# (`-----MEDIDA-----` … `-----FIN-----`) para cruzarlo aquí.
MEDIR_SH = r"""#!/bin/sh
set -e
CLIENTE=$(cat /puesto/agente-cliente); SECRETO=$(cat /puesto/agente-secreto)
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" \
  -d grant_type=client_credentials -d "client_id=$CLIENTE" -d "client_secret=$SECRETO" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
export TOK
python3 - <<'EOF'
import json, os, time, urllib.request, urllib.error
BASE = "http://ore-serve.t-%s.svc.cluster.local:8080" % os.environ["CELDA"]
TOK = os.environ["TOK"]
def pide(ruta):
    req = urllib.request.Request(BASE + ruta, headers={"authorization": "Bearer " + TOK})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            cuerpo = r.read(); cod = r.status
    except urllib.error.HTTPError as e:
        cuerpo = e.read(); cod = e.code
    except Exception as e:
        cuerpo = str(e).encode(); cod = 0
    ms = int((time.time() - t0) * 1000)
    try:
        j = json.loads(cuerpo)
    except Exception:
        j = {"_texto": cuerpo[:300].decode("utf-8", "replace")}
    return {"ruta": ruta, "codigo": cod, "ms": ms, "bytes": len(cuerpo), "cuerpo": j}
print("-----MEDIDA-----")
salida = []
def anota(r):
    salida.append(r); print(json.dumps(r, ensure_ascii=False))
arbol = pide("/arbol"); anota(arbol)
paquetes = pide("/paquetes"); anota(paquetes)
for ruta in ["/datasets", "/conceptos", "/funciones", "/modelos", "/fuentes"]:
    anota(pide(ruta))
for k in ["Entity", "View", "Table", "Concept", "Interface", "TrainedModel", "Dataset", "Function", "Action", "Model", "Ruleset", "Lattice", "ConduitPolicy"]:
    anota(pide("/documentos/" + k))
for p in (paquetes["cuerpo"].get("packages") or [])[:6]:
    n = p["name"]
    anota(pide("/paquetes/%s/esquema" % n))
    anota(pide("/paquetes/%s/copias" % n))
    anota(pide("/paquetes/%s/decisiones" % n))
ds = (pide("/datasets")["cuerpo"].get("datasets") or [])
for d in ds[:3]:
    ns, n = d["nombre"].split(".", 1)
    anota(pide("/datasets/%s/%s" % (ns, n)))
# la historia de UN documento: la capa History
fs = [f for f in (arbol["cuerpo"].get("ficheros") or []) if f.get("kind") in ("Dataset", "View", "Table")]
if fs:
    anota(pide("/arbol/historia/" + fs[0]["ruta"]))
    anota(pide("/arbol/" + fs[0]["ruta"]))
anota(pide("/arbol/diagnosticos"))
# ── 0034 ⑤ · GET /assets: en frio (calcula) y en caliente (de memoria) ──
frio = pide("/assets"); frio["ruta"] = "/assets (frio)"; anota(frio)
caliente = pide("/assets"); caliente["ruta"] = "/assets (caliente)"; anota(caliente)
print("-----FIN-----")
EOF
"""


def job(ns, celda, nombre):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": nombre, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda, "ore.dev/medida": "assets-catalog"}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": ["set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; done\nchmod 0400 /puesto/*\n" % ns],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "medir", "image": img, "command": ["/bin/sh", "/medida/medir.sh"],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "HOME", "value": "/tmp"},
                                    {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"}, {"name": "REALM", "value": "rubix"}],
                            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "medida", "mountPath": "/medida"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "medida", "configMap": {"name": nombre, "defaultMode": 0o555}}],
        }}},
    }


def medir_dentro(celda):
    ns = "t-" + celda
    nombre = "medida-assets-catalog"
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": nombre, "namespace": ns}, "data": {"medir.sh": MEDIR_SH}}))
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
    salida = kubectl("logs", "-n", ns, "job/" + nombre, "-c", "medir") or ""
    kubectl("delete", "job", nombre, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("delete", "configmap", nombre, "-n", ns, "--ignore-not-found")
    if not st.get("succeeded"):
        print("     MAL el Job no terminó bien:", salida[:600])
        return None
    ini = salida.find("-----MEDIDA-----")
    fin = salida.find("-----FIN-----")
    lineas = salida[ini + len("-----MEDIDA-----"):fin].strip().splitlines()
    return [json.loads(l) for l in lineas if l.startswith("{")]


def fila(*cols):
    print("     " + " · ".join(str(c) for c in cols))


def forma(j, n=8):
    """Las claves de la primera fila de la lista que devuelve, para ver la forma sin volcarla."""
    if isinstance(j, dict):
        for k, v in j.items():
            if isinstance(v, list) and v and isinstance(v[0], dict):
                return "%s[%d]{%s}" % (k, len(v), ",".join(list(v[0].keys())[:n]))
            if isinstance(v, list):
                return "%s[%d]" % (k, len(v))
        return "{" + ",".join(list(j.keys())[:n]) + "}"
    if isinstance(j, list):
        return "[%d]" % len(j)
    return type(j).__name__


def cuenta(j):
    if isinstance(j, dict):
        for k, v in j.items():
            if isinstance(v, list):
                return len(v)
    return "-"


def informe(celda, medidas):
    print("  %s" % celda)
    por_ruta = {m["ruta"]: m for m in medidas}
    arbol = por_ruta.get("/arbol", {}).get("cuerpo", {})
    ficheros = arbol.get("ficheros") or []
    # ── Lo que el árbol tiene: por kind, por paquete y por carpeta ──
    kinds = Counter(f.get("kind") or "(sin kind)" for f in ficheros)
    fila("EL ÁRBOL", "%d ficheros" % len(ficheros), "cabeza %s" % str(arbol.get("cabeza", "?"))[:7])
    fila("   por kind", ", ".join("%s=%d" % kv for kv in sorted(kinds.items(), key=lambda x: -x[1])))
    por_paq = defaultdict(Counter)
    carpetas = defaultdict(set)
    for f in ficheros:
        r = f["ruta"]
        if not r.startswith("packages/"):
            continue
        partes = r.split("/")
        if len(partes) < 3:
            continue
        p = partes[1]
        por_paq[p][f.get("kind") or partes[2]] += 1
        # la carpeta entre el paquete y el fichero: hoy es la del kind (views/, tables/…)
        carpetas[p].add("/".join(partes[2:-1]))
    for p, c in sorted(por_paq.items()):
        fila("   packages/%s" % p, ", ".join("%s=%d" % kv for kv in sorted(c.items())), "carpetas: " + ", ".join(sorted(x for x in carpetas[p] if x)))
    # ── Lo que cada ruta sirve ──
    fila("LAS RUTAS", "código · ms · bytes · forma")
    for m in medidas:
        c = m["cuerpo"]
        err = c.get("error") if isinstance(c, dict) else None
        fila("   %-46s" % m["ruta"], "%3s" % m["codigo"], "%5d ms" % m["ms"], "%7d B" % m["bytes"], (("ERROR: " + str(err)[:90]) if err else forma(c)))
    # ── (a) ítems de un paquete por carpeta en una llamada ──
    print()
    fila("(a) TODOS LOS ÍTEMS DE UN PAQUETE, POR CARPETA, EN UNA LLAMADA")
    for p in sorted(por_paq):
        esq = por_ruta.get("/paquetes/%s/esquema" % p)
        if not esq:
            continue
        c = esq["cuerpo"]
        tablas = len(c.get("tables") or []) if isinstance(c, dict) else 0
        ents = len(c.get("entities") or []) if isinstance(c, dict) else 0
        total = sum(por_paq[p].values())
        fila("   %s" % p, "/esquema da tables=%d entities=%d" % (tablas, ents), "el árbol tiene %d ítems (%s)" % (total, ", ".join("%s=%d" % kv for kv in sorted(por_paq[p].items()))), "faltan %d" % (total - tablas - ents))
    # ── (b) de cada dataset, qué lo define y si es identidad ──
    print()
    fila("(b) DE CADA DATASET, QUÉ LO DEFINE Y SI ES IDENTIDAD")
    ds = (por_ruta.get("/datasets", {}).get("cuerpo") or {}).get("datasets") or []
    claves = Counter()
    for d in ds:
        for k in d:
            claves[k] += 1
    fila("   /datasets", "%d datasets" % len(ds), "claves: " + ", ".join("%s(%d)" % kv for kv in sorted(claves.items())))
    fila("   dice `from`/plan/identidad:", "no" if not any(k in claves for k in ("from", "plan", "identidad", "fields")) else "sí")
    docs_ds = (por_ruta.get("/documentos/Dataset", {}).get("cuerpo") or {})
    fila("   /documentos/Dataset", forma(docs_ds), "(el documento entero con `from`/`fields`: sí, pero aparte del puntero)")
    for m in medidas:
        if m["ruta"].startswith("/datasets/") and m["ruta"].count("/") == 3:
            c = m["cuerpo"]
            fila("   ficha %s" % m["ruta"], m["codigo"], "%d ms" % m["ms"], "claves: " + ", ".join(list(c.keys())[:14]) if isinstance(c, dict) else str(c)[:80])
    # ── (c) capas por ítem ──
    print()
    fila("(c) LAS CAPAS POR ÍTEM (Access, Rules, Links, History)")
    for k in ("Ruleset", "Lattice", "ConduitPolicy"):
        m = por_ruta.get("/documentos/" + k)
        if m:
            fila("   /documentos/%s" % k, m["codigo"], (str(m["cuerpo"].get("error"))[:80] if isinstance(m["cuerpo"], dict) and m["cuerpo"].get("error") else forma(m["cuerpo"])))
    capas_en_arbol = {k: kinds.get(k, 0) for k in ("Ruleset", "Lattice", "ConduitPolicy", "RequestPolicy", "Policy")}
    fila("   en el árbol:", ", ".join("%s=%d" % kv for kv in capas_en_arbol.items()))
    hist = [m for m in medidas if m["ruta"].startswith("/arbol/historia/")]
    for m in hist:
        fila("   History %s" % m["ruta"][16:], m["codigo"], "%d ms" % m["ms"], forma(m["cuerpo"]))
    fila("   Links (linaje/procedencia/backedBy/usos):", "ninguna ruta «qué aplica sobre X» ni «qué usa X»; el linaje sólo por `ore view` (CLI) y la procedencia sólo en el puntero")
    # ── (d) Function y Action por /documentos ──
    print()
    fila("(d) FUNCTION Y ACTION POR /documentos")
    for k in ("Function", "Action", "Model"):
        m = por_ruta.get("/documentos/" + k)
        if m:
            fila("   /documentos/%s" % k, m["codigo"], (str(m["cuerpo"].get("error"))[:80] if isinstance(m["cuerpo"], dict) and m["cuerpo"].get("error") else forma(m["cuerpo"])), "en el árbol: %d" % kinds.get(k, 0))
    m = por_ruta.get("/funciones")
    if m:
        fila("   /funciones", m["codigo"], forma(m["cuerpo"]))


def informe_assets(celda, medidas):
    """0034 ⑤ · lo que GET /assets da frente a lo que el catalogo pedia hasta hoy."""
    por_ruta = {m["ruta"]: m for m in medidas}
    frio, caliente = por_ruta.get("/assets (frio)"), por_ruta.get("/assets (caliente)")
    if not frio:
        return
    print()
    fila("GET /assets (0034 ⑤)")
    for m in (frio, caliente):
        c = m["cuerpo"]
        n = len(c.get("items") or {}) if isinstance(c, dict) else "?"
        fila("   %-20s" % m["ruta"], m["codigo"], "%5d ms" % m["ms"], "%7d B" % m["bytes"], "items=%s" % n, "desde_cache=%s" % (c.get("desde_cache") if isinstance(c, dict) else "?"), "cabeza=%s" % str(c.get("cabeza", "?"))[:7] if isinstance(c, dict) else "")
    # lo que costaba pintar el catalogo hasta hoy: /paquetes + por base (/esquema + /copias) + /datasets
    viejas = [m for m in medidas if m["ruta"] == "/paquetes" or m["ruta"] == "/datasets" or m["ruta"].endswith("/esquema") or m["ruta"].endswith("/copias")]
    fila("   hasta hoy (el catalogo de la consola):", "%d llamadas" % len(viejas), "%d ms" % sum(m["ms"] for m in viejas), "%d B" % sum(m["bytes"] for m in viejas))
    c = frio["cuerpo"]
    if not isinstance(c, dict) or not c.get("items"):
        return
    items = c["items"]
    kinds = Counter(i.get("kind") for i in items.values())
    fila("   items por kind:", ", ".join("%s=%d" % kv for kv in sorted(kinds.items())))
    rel = sum(len(i.get("relaciones") or []) for i in items.values())
    rotas = sum(1 for i in items.values() for r in (i.get("relaciones") or []) if r.get("rota"))
    ident = sum(1 for i in items.values() if (i.get("define") or {}).get("identidad"))
    inducidas = sum(1 for i in items.values() if "vistaInducida" in (i.get("detalle") or {}))
    con_version = sum(1 for i in items.values() if i.get("version"))
    fila("   relaciones (dos direcciones): %d · rotas %d · identidad %d · tablas con vistaInducida %d · con version %d / %d" % (rel, rotas, ident, inducidas, con_version, len(items)))
    for p in c.get("paquetes") or []:
        fila("   paquete %-26s" % p.get("name"), p.get("type"), "scoped=%s" % p.get("scoped"), "items=%s" % p.get("items"), "carpetas=%s" % p.get("carpetas"))
    # los tres hechos del criterio del paso 3, sobre lo que haya
    ds = [i for i in items.values() if i.get("kind") == "Dataset"]
    if ds:
        d = ds[0]
        fila("   un dataset:", d["ref"], "identidad=%s" % (d.get("define") or {}).get("identidad"), "sale_de=%s" % [r["ref"] for r in d.get("relaciones") or [] if r["tipo"] == "sale_de"], "puntero=%s" % {k: (d.get("puntero") or {}).get(k) for k in ("estado", "filas", "motivo")}, "version=%s" % ((d.get("version") or {}).get("commit")))
    ts = [i for i in items.values() if i.get("kind") == "Table" and "vistaInducida" in (i.get("detalle") or {})]
    if ts:
        t = ts[0]
        fila("   una table foreign:", t["ref"], "vistaInducida=%s" % t["detalle"]["vistaInducida"], "produce=%s" % [r["ref"] for r in t.get("relaciones") or [] if r["tipo"] == "produce"])


def main():
    celdas = [a for a in sys.argv[1:] if not a.startswith("--")] or ["demo", "victor"]
    print("0034 · el catálogo de assets, medido contra el ore-serve vivo")
    for c in celdas:
        medidas = medir_dentro(c)
        if medidas:
            informe(c, medidas)
            informe_assets(c, medidas)
            with open(os.path.join(os.environ.get("TEMP", "."), "medida-assets-%s.json" % c), "w", encoding="utf-8") as f:
                json.dump(medidas, f, ensure_ascii=False, indent=1)
    print("  (el clúster, como estaba: el Job y su ConfigMap se retiraron)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
