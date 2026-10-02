#!/usr/bin/env python3
"""NUNCA NULA, EN VIVO (ORE 0051 P6d/P6e): encender la imposición en una celda.

La copia impone lo que nunca es nulo cuando el árbol lo enciende con un fichero,
`.arbol/nulos.yaml` (`imponer: true`; `ore materialize`, `NULOS`). Esto lo hace
celda a celda, con los mismos pasos que P3′ (`la-fuente-al-dia.py`): un Job en
el inquilino con la imagen `ore-drivers`, el token de la forja de Secret
Manager, y nada que se imprima de él.

    python pruebas-de-fuego/nunca-nula-en-vivo.py demo                # ensayo
    python pruebas-de-fuego/nunca-nula-en-vivo.py demo --encender     # el commit
    python pruebas-de-fuego/nunca-nula-en-vivo.py demo --rehacer      # la pasada
    python pruebas-de-fuego/nunca-nula-en-vivo.py demo --comprobar    # Iceberg

· ENSAYO. M1: el `ore` de la imagen impone (un árbol mínimo dentro del Job, con
  un origen de ficheros, dice `impone nunca nula`). Y la copia EN SECO del árbol
  vivo, sin y con el fichero (puesto sólo en el clon): qué copias impondrían
  qué columnas, y que **sólo esas** dejan de estar al día —las demás conservan su
  cabecera byte a byte—. Nada se empuja.
· `--encender`: lo mismo, y si el ensayo sale bien, UN commit con
  `.arbol/nulos.yaml` en `main`. Vuelta atrás: `git revert` de ese commit (y la
  pasada siguiente afloja: apagar es aflojar).
· `--rehacer`: la pasada de la copia, la de verdad. El Job `copiar-<resumen>`
  no se reintenta solo con la plantilla igual (`malla/48-la-copia.yaml`): se
  borra y Flux lo recrea. Espera a que termine y enseña su final.
· `--comprobar`: cada puntero contra el `metadata.json` que nombra, leído del
  bucket de la celda: las columnas `required` de Iceberg tienen que ser las
  `obligatorias` de la cabecera de su snapshot, ni una más ni una menos.
"""
import io
import json
import os
import subprocess
import sys
import time

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
PROYECTO = "project-8853a180-450d-47be-b83"
NOMBRE = "nunca-nula-en-vivo"


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


GUION_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"

# ── M1 · el binario impone ───────────────────────────────────────────────────
mkdir -p /tmp/m1/arbol/packages/p/tables /tmp/m1/arbol/packages/p/datasets /tmp/m1/arbol/.arbol /tmp/m1/datos
cd /tmp/m1/arbol
printf '{"id":"1"}\n' > ../datos/t.jsonl
printf 'apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: m1, version: 0.1.0 }\ndatasources:\n  - { name: f, type: jsonl, connectionEnv: M1_DIR }\n' > ontology.config.yaml
printf 'apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: m1 }\nspec:\n  owner: team:x\n  conduits:\n    materialization.payload: { oos.maturity: DRAFT }\n' > conduits.yaml
printf 'apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: p, version: 0.1.0, status: active, domain: x }\nspec: { owner: team:x }\n' > packages/p/package.yaml
printf 'apiVersion: oos.dev/v1alpha22\nkind: Table\nmetadata: { name: t, namespace: p }\nspec:\n  datasource: f\n  object: "t.jsonl"\n  columns:\n    id: { type: String, required: true }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n' > packages/p/tables/t.yaml
printf 'apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: d, namespace: p }\nspec:\n  owner: team:x\n  from: { table: p.t }\n  fields: { id: id }\n' > packages/p/datasets/d.yaml
printf 'imponer: true\n' > .arbol/nulos.yaml
M1_DIR=/tmp/m1/datos ORE_STORE=gcs ORE_GCS_BUCKET=nadie ore materialize . --seco > /tmp/m1.txt 2>&1 || true
if grep -q 'impone nunca nula · id' /tmp/m1.txt; then echo "(M1) el ore de la imagen impone"
else echo "(M1) EL ore DE LA IMAGEN NO IMPONE: es de antes de 0051 P6"; cat /tmp/m1.txt; exit 1; fi

cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD) $(git log -1 --format=%ci)"
if [ -f .arbol/nulos.yaml ]; then echo "(arbol) ya lleva .arbol/nulos.yaml:"; sed 's/^/(arbol)   /' .arbol/nulos.yaml; fi
export ORE_STORE=gcs ORE_GCS_BUCKET="project-8853a180-450d-47be-b83-t-$CELDA-copia"

# ── el ensayo: la copia en seco, sin y con el fichero ───────────────────────
rm -f .arbol/nulos.yaml.ensayo
[ -f .arbol/nulos.yaml ] && mv .arbol/nulos.yaml .arbol/nulos.yaml.ensayo
ore materialize . --seco > /tmp/s0.txt 2>&1 || true
mkdir -p .arbol && printf 'imponer: true\n' > .arbol/nulos.yaml
ore materialize . --seco > /tmp/s1.txt 2>&1 || true
python3 /guion/ensayo.py
rm -f .arbol/nulos.yaml; [ -f .arbol/nulos.yaml.ensayo ] && mv .arbol/nulos.yaml.ensayo .arbol/nulos.yaml
if [ -z "$ENCENDER" ]; then echo "(ensayo) nada empujado"; exit 0; fi

# ── encender ────────────────────────────────────────────────────────────────
if [ -f .arbol/nulos.yaml ]; then echo "(encender) ya estaba encendido: nada que empujar"; exit 0; fi
mkdir -p .arbol
cat > .arbol/nulos.yaml <<'Y'
# ORE 0051 P6 · las copias de este árbol imponen lo que nunca es nulo: lo que el
# origen garantiza (`required: true` en su Table) o lo que la consulta deriva.
# La copia lo escribe `required` en Iceberg y niega la fila que traiga un nulo.
# Apagar (borrar este fichero, o un revert) es aflojar: la copia se rehace.
imponer: true
Y
git add .arbol/nulos.yaml
git -c user.name='ore 0051' -c user.email=nulos@invalido \
  commit -q -m "0051 P6 · las copias imponen lo que nunca es nulo (.arbol/nulos.yaml)"
git push -q origin HEAD:main
echo "(encendido) $(git rev-parse --short HEAD)"
"""

ENSAYO_PY = r'''
import re, sys
def bloques(p):
    """`<vista>` → (lo que impone, la línea de lo que haría). La salida de `ore
    materialize` es una vista por línea sin sangría y lo suyo debajo."""
    out, actual = {}, None
    for l in open(p, encoding="utf-8", errors="replace").read().splitlines():
        if l and not l.startswith(" "):
            if l.startswith("error:") or l.startswith("sin copias"):
                continue
            actual = l.strip(); out[actual] = {"impone": "", "dice": []}
        elif actual:
            m = re.match(r"\s+impone nunca nula · (.*)", l)
            if m: out[actual]["impone"] = m.group(1)
            else: out[actual]["dice"].append(l.strip())
    return out
def estado(b):
    t = " ".join(b["dice"])
    if "ya está" in t: return "ya está"
    if "haría falta" in t: return "por rehacer"
    return "no pudo preguntar: " + (b["dice"][0][:90] if b["dice"] else "?")
s0, s1 = bloques("/tmp/s0.txt"), bloques("/tmp/s1.txt")
mal = []
cuenta = {}
for v in sorted(s1):
    e0, e1 = estado(s0.get(v, {"dice": []})), estado(s1[v])
    imp = s1[v]["impone"]
    if imp: print(f"(ensayo) {v} · impone {imp} · {e0} → {e1}")
    cuenta[(e0, e1, bool(imp))] = cuenta.get((e0, e1, bool(imp)), 0) + 1
    if s0.get(v, {}).get("impone"): mal.append(f"{v}: sin el fichero ya impone")
    if not imp and e0 == "ya está" and e1 != "ya está": mal.append(f"{v}: no impone nada y deja de estar al día")
    if imp and e0 == "ya está" and e1 == "ya está": mal.append(f"{v}: impone {imp} y sigue «ya está»: la cabecera no cambió")
print(f"(ensayo) {len(s1)} copias · imponen algo: {sum(1 for v in s1 if s1[v]['impone'])}")
for (e0, e1, imp), n in sorted(cuenta.items(), key=lambda x: -x[1]):
    print(f"(ensayo)   {n:3d} · {'impone' if imp else 'nada  '} · {e0} → {e1}")
if mal:
    print("(ensayo) ✗ " + "\n(ensayo) ✗ ".join(mal)); sys.exit(1)
print("(ensayo) ✓ sólo dejan de estar al día las copias que imponen algo")
'''

REHACER_SH = ""  # se hace desde fuera: borrar el Job y esperar al de Flux

COMPROBAR_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD) $(git log -1 --format='%ci %an') · nulos.yaml: $([ -f .arbol/nulos.yaml ] && grep -v '^#' .arbol/nulos.yaml | tr -d '\n' || echo no)"
export ORE_STORE=gcs ORE_GCS_BUCKET="project-8853a180-450d-47be-b83-t-$CELDA-copia"
python3 /guion/comprobar.py
"""

COMPROBAR_PY = r'''
import glob, json, subprocess, sys
mal, n, con = [], 0, 0
for p in sorted(glob.glob("datasets/**/*.json", recursive=True)):
    try: d = json.load(open(p, encoding="utf-8"))
    except Exception: continue
    if d.get("estado") == "error":
        print(f"(comprobar) {p[9:]} · error · {str(d.get('motivo'))[:400]}")
    ml = d.get("metadata_location")
    if not ml or not d.get("cabecera"): continue   # lo escrito no es una copia
    r = subprocess.run(["gcloud", "storage", "cat", ml], capture_output=True, text=True)
    if r.returncode != 0:
        print(f"(comprobar) {p} · no se lee {ml}: {r.stderr.strip()[:120]}"); continue
    m = json.loads(r.stdout)
    s = next(s for s in m["schemas"] if s["schema-id"] == m["current-schema-id"])
    req = sorted(f["name"] for f in s["fields"] if f.get("required"))
    snap = next((x for x in m.get("snapshots", []) if x["snapshot-id"] == m.get("current-snapshot-id")), {})
    cab = (snap.get("summary") or {}).get("ore.cabecera")
    obl = sorted(json.loads(cab).get("obligatorias", [])) if cab else []
    n += 1; con += bool(req)
    linea = f"(comprobar) {p[9:]} · {d.get('estado')} · required: {', '.join(req) or '-'}"
    if req != obl:
        mal.append(f"{p}: Iceberg dice required {req} y la cabecera obligatorias {obl}"); linea += f" ✗ cabecera {obl}"
    if req or d.get("estado") not in ("al-dia", "copiada"): print(linea)
print(f"(comprobar) {n} copias leídas del bucket · con columnas required: {con}")
# Y cada puntero contra el de la pasada anterior que lo movió: si la cabecera
# cambió sin imponer nada, la huella no es la de antes y algo se rehizo de más.
for p in sorted(glob.glob("datasets/**/*.json", recursive=True)):
    try: d = json.load(open(p, encoding="utf-8"))
    except Exception: continue
    if not d.get("cabecera"): continue
    cs = subprocess.run(["git", "log", "--format=%h", "-n", "2", "--", p], capture_output=True, text=True).stdout.split()
    if len(cs) < 2: continue
    try: a = json.loads(subprocess.run(["git", "show", f"{cs[1]}:{p}"], capture_output=True, text=True).stdout)
    except Exception: continue
    t = lambda x: json.dumps(x.get("testigo"), sort_keys=True)
    print(f"(antes) {p[9:]} · {cs[1]}→{cs[0]} · cabecera {'igual' if a.get('cabecera') == d.get('cabecera') else 'OTRA'}"
          f" · plan {'igual' if a.get('plan') == d.get('plan') else 'OTRO'} · testigo {t(a)} → {t(d)}")
if mal:
    print("(comprobar) ✗ " + "\n(comprobar) ✗ ".join(mal)); sys.exit(1)
print("(comprobar) ✓ en cada copia, lo required en Iceberg es exactamente lo que su cabecera impone")
'''


def job(ns, celda, guion, encender=False):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    secretos = ("set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\n"
                "chmod 0400 /puesto/*\n") % ns
    return {
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": NOMBRE, "namespace": ns, "labels": {"kueue.x-k8s.io/queue-name": "cola"}},
        "spec": {"backoffLimit": 0, "ttlSecondsAfterFinished": 600, "template": {"metadata": {"labels": {"ore.dev/rol": "driver", "ore.dev/tenant": celda, "ore.dev/medida": NOMBRE}}, "spec": {
            "restartPolicy": "Never", "serviceAccountName": "driver",
            "initContainers": [{"name": "puesto", "image": img, "command": ["/bin/sh", "-c"],
                                "env": [{"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"}, {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                                "args": [secretos],
                                "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}},
                                "volumeMounts": [{"name": "puesto", "mountPath": "/puesto"}]}],
            "containers": [{"name": "guion", "image": img, "imagePullPolicy": "Always",
                            "command": ["/bin/sh", "/guion/" + guion],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "ENCENDER", "value": "1" if encender else ""},
                                    {"name": "HOME", "value": "/tmp"}, {"name": "CLOUDSDK_CONFIG", "value": "/tmp/.gcloud"},
                                    {"name": "CLOUDSDK_CORE_PROJECT", "value": PROYECTO}],
                            "resources": {"requests": {"cpu": "200m", "memory": "512Mi"}, "limits": {"cpu": "1000m", "memory": "1Gi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guion", "mountPath": "/guion"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "guion", "configMap": {"name": NOMBRE, "defaultMode": 0o555}}],
        }}},
    }


def correr(celda, guion, encender=False):
    ns = "t-" + celda
    datos = {"guion.sh": GUION_SH, "ensayo.py": ENSAYO_PY, "comprobar.sh": COMPROBAR_SH, "comprobar.py": COMPROBAR_PY}
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": NOMBRE, "namespace": ns}, "data": datos}))
    kubectl("delete", "job", NOMBRE, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, guion, encender)))
    t0, st = time.time(), {}
    while time.time() - t0 < 1200:
        j = kubectl("get", "job", NOMBRE, "-n", ns, "-o", "json")
        st = json.loads(j)["status"] if j else {}
        if st.get("succeeded") or st.get("failed"):
            break
        time.sleep(5)
    salida = kubectl("logs", "-n", ns, "job/" + NOMBRE, "-c", "guion") or ""
    kubectl("delete", "job", NOMBRE, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("delete", "configmap", NOMBRE, "-n", ns, "--ignore-not-found")
    for l in salida.splitlines():
        print("     " + l)
    ok = bool(st.get("succeeded"))
    print("     %s · %.0f s" % ("✓" if ok else "✗ el Job no terminó bien", time.time() - t0))
    return ok


def copias(ns):
    """Los Jobs de la pasada de `main` (no los de las ramas): `copiar-<resumen>`."""
    j = json.loads(kubectl("get", "jobs", "-n", ns, "-o", "json") or '{"items": []}')
    return [x for x in j["items"] if x["metadata"]["name"].startswith("copiar-")
            and not x["metadata"]["name"].startswith("copiar-rama-")]


def rehacer(celda):
    ns = "t-" + celda
    viejos = copias(ns)
    if not viejos:
        print("     no hay ningún Job copiar-… en %s: Flux lo creará en su próxima pasada" % ns)
    for v in viejos:
        print("     se borra %s (%s)" % (v["metadata"]["name"], "terminó" if v["status"].get("succeeded") else "sin terminar"))
        kubectl("delete", "job", v["metadata"]["name"], "-n", ns, "--wait=true")
    nombres = {v["metadata"]["name"] for v in viejos}
    uids = {v["metadata"]["uid"] for v in viejos}
    t0, nuevo, aviso = time.time(), None, 0
    while time.time() - t0 < 3600:
        nuevo = next((x for x in copias(ns) if x["metadata"]["uid"] not in uids), None)
        if nuevo and (nuevo["status"].get("succeeded") or nuevo["status"].get("failed")):
            break
        if time.time() - t0 > aviso + 120:
            aviso += 120
            print("     %s… (%.0f s)" % ("la pasada corre" if nuevo else "esperando a que Flux recree el Job", time.time() - t0))
        time.sleep(5)
    if not nuevo:
        print("     ✗ Flux no recreó el Job en una hora"); return False
    n = nuevo["metadata"]["name"]
    print("     %s · %s (%s)" % (n, "terminó" if nuevo["status"].get("succeeded") else "FALLÓ", "el mismo nombre" if n in nombres else "otro nombre"))
    log = kubectl("logs", "-n", ns, "job/" + n, "-c", "copiar") or kubectl("logs", "-n", ns, "job/" + n, "--all-containers") or ""
    lineas = log.splitlines()
    imp = [l for l in lineas if "impone nunca nula" in l]
    print("     la pasada: %d líneas · «impone nunca nula» %d · «ya está» %d · copiadas/sobrescritas %d · error %d"
          % (len(lineas), len(imp), sum("ya está" in l for l in lineas),
             sum(("sobrescrita" in l or "creada" in l or "refrescada" in l) for l in lineas),
             sum(l.strip().startswith("error") or "nunca es nula" in l for l in lineas)))
    for l in [l for l in lineas if "nunca es nula" in l or l.startswith("error")][:20]:
        print("       " + l[:200])
    for l in lineas[-6:]:
        print("       │ " + l[:200])
    return bool(nuevo["status"].get("succeeded"))


def main():
    args = sys.argv[1:]
    celdas = [a for a in args if not a.startswith("--")] or ["demo"]
    bien = True
    for c in celdas:
        if "--rehacer" in args:
            print("0051 P6 · %s · la pasada de la copia (se borra el Job, Flux lo recrea)" % c)
            bien &= rehacer(c)
        elif "--comprobar" in args:
            print("0051 P6 · %s · cada copia contra su metadata.json" % c)
            bien &= correr(c, "comprobar.sh")
        else:
            enc = "--encender" in args
            print("0051 P6 · %s · %s" % (c, "ENCENDER" if enc else "ENSAYO (sin --encender nada cambia)"))
            bien &= correr(c, "guion.sh", enc)
    print("  (el clúster, como estaba: el Job y su ConfigMap se retiraron)")
    return 0 if bien else 1


if __name__ == "__main__":
    sys.exit(main())
