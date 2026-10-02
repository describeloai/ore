#!/usr/bin/env python3
"""LA FUENTE AL DÍA (ORE 0051 P3′): un árbol que ya existía, puesto al paradigma de hoy.

Un árbol nuevo nace con lo que el inductor escribe hoy —el `required` de cada
columna que el origen garantiza (0051 P3), el puntero de todo lo catalogado
(0045 E5′)—. Uno que ya existía se catalogó con el de antes, y nadie lo vuelve a
inducir hasta que alguien vuelva a catalogar. Esto lo pone al día: un Job por
inquilino clona el árbol de su forja (con el token de Secret Manager, como
`migrar-a-dataset.py`), y corre `ore source induce <fuente>` por cada fuente
declarada que tiene su catálogo guardado (`packages/<fuente>/discover.catalog.json`).
**No abre ningún origen**: re-induce desde lo que el catálogo ya dijo.

Antes de nada, M1: que el `ore` de la imagen `ore-drivers` escribe `required`
(si es de antes de P3, para). Y no empuja nada sin comprobar, M2:

  · `ore validate` no da más diagnósticos que antes;
  · `ore datasets` lista lo mismo;
  · `ore view` enseña las mismas vistas, y lo único nuevo son líneas `nunca nula`;
  · sólo cambian ficheros de los paquetes de fuente: `Table` y `Schema` que llevan
    la marca del inductor, y en un `package.yaml` sólo su `spec:` (los `exports`).
    Un fichero escrito a mano no se toca;
  · y dice cuántas tablas existentes ganan `required` (0051), en cuántas
    columnas, y cuántos punteros y schemas nuevos llegan (E5′).

    python pruebas-de-fuego/la-fuente-al-dia.py [victor demo …]            # ensayo
    python pruebas-de-fuego/la-fuente-al-dia.py victor --empujar           # M3
    python pruebas-de-fuego/la-fuente-al-dia.py victor --comprobar         # M4, antes y después
    python pruebas-de-fuego/la-fuente-al-dia.py victor --copia             # M4, la copia en seco

Con `--empujar`, un solo commit `ore migrate` en `main`. Vuelta atrás: `git revert`
de ese commit; el lago y los datos no se tocan. El Job y su ConfigMap se retiran
al final, y ningún token se imprime.
"""
import io
import json
import os
import subprocess
import sys
import time

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
PROYECTO = "project-8853a180-450d-47be-b83"
NOMBRE = "la-fuente-al-dia"
# Las fuentes de cada celda cuyo esquema mira `--comprobar` (las que tienen catálogo).
FUENTES = {
    "victor": "postgresql_20260918_1920 postgresql_20260918_2038 postgresql_20260921_2055 bigquery_20260927_1428",
    "demo": "postgresql_20260910_1719 postgresql_20260910_1924 postgresql_20260910_2024 postgresql_20260910_2146 postgresql_20260912_0928",
}


def kubectl(*args, entrada=None):
    r = subprocess.run(["kubectl", *args], input=entrada, capture_output=True, text=True, encoding="utf-8",
                       env={**os.environ, "MSYS_NO_PATHCONV": "1", "MSYS2_ARG_CONV_EXCL": "*"})
    if r.returncode != 0 and "NotFound" not in (r.stderr or ""):
        print("     kubectl", " ".join(args[:3]), "→", (r.stderr or "").strip()[:200])
    return r.stdout


# El guion que corre dentro. `EMPUJAR` vacío = ensayo.
GUION_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"

# ── M1 · el binario escribe `required` ───────────────────────────────────────
mkdir -p /tmp/m1 && cd /tmp/m1
printf '%s' '{"source":"pg","tables":[{"name":"public.t","columns":[{"name":"id","type":"Integer","required":true}]}]}' > c.json
ore discover --from c.json --out p > /dev/null 2>&1 || true
if grep -rq 'required: true' p; then echo "(M1) ore escribe required · $(ore --version 2>/dev/null | head -1)"
else echo "(M1) EL ore DE LA IMAGEN NO ESCRIBE required: es de antes de 0051 P3"; exit 1; fi

# ── M2 · el ensayo ───────────────────────────────────────────────────────────
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
echo "(arbol) commit=$(git rev-parse --short HEAD) $(git log -1 --format=%ci) ficheros=$(git ls-files | wc -l)"
ore validate . > /tmp/v0.txt 2>&1 || true
ore view . > /tmp/w0.txt 2>&1 || true
ore datasets . --json > /tmp/d0.json 2>/dev/null || true
FUENTES=$(sed -n 's/^ *- *{\{0,1\} *name: *\([A-Za-z0-9_-]*\).*/\1/p' ontology.config.yaml)
for f in $FUENTES; do
  [ -f "packages/$f/discover.catalog.json" ] || continue
  if ore source induce "$f" > /tmp/i.txt 2>&1; then echo "(induce) $f · $(tail -1 /tmp/i.txt | sed 's/^ *//')"
  else echo "(induce) $f FALLO"; cat /tmp/i.txt; exit 1; fi
  echo "$f" >> /tmp/fuentes.txt
done
ore validate . > /tmp/v1.txt 2>&1 || true
ore view . > /tmp/w1.txt 2>&1 || true
ore datasets . --json > /tmp/d1.json 2>/dev/null || true
git add -A
git diff --cached --name-status > /tmp/cambios.txt
git diff --cached -U0 > /tmp/diff.txt
python3 /guion/comprobar.py
if [ -z "$EMPUJAR" ]; then echo "(ensayo) nada empujado"; exit 0; fi
git -c user.name='ore migrate' -c user.email=migrate@invalido \
  commit -q -m "ore migrate · la fuente al dia (0051 P3' y 0045 E5'): las fuentes, re-inducidas desde su catalogo con el inductor de hoy"
git push -q origin HEAD:main
echo "(empujado) $(git rev-parse --short HEAD)"
"""

# Las comprobaciones de M2, sobre lo que el guion dejó en /tmp.
COMPROBAR_PY = r'''
import json, re, subprocess, sys
MARCA = "# ore discover: lo escribe la inducción"
mal = []
def lee(p):
    try: return open(p, encoding="utf-8").read()
    except OSError: return ""
def cuenta(t): return len(re.findall(r"^(error|warning)\[", t, re.M))
d0, d1 = cuenta(lee("/tmp/v0.txt")), cuenta(lee("/tmp/v1.txt"))
print(f"(M2) ore validate · diagnósticos {d0} → {d1}")
if d1 > d0:
    mal.append("más diagnósticos que antes")
    print("\n".join(lee("/tmp/v1.txt").splitlines()[:20]))
def datasets(p):
    try:
        j = json.loads(lee(p).strip().splitlines()[-1]); ds = j.get("datasets") if isinstance(j, dict) else j
        return sorted((x.get("nombre") or x.get("dataset") or json.dumps(x)) for x in (ds or []))
    except Exception: return None
a, b = datasets("/tmp/d0.json"), datasets("/tmp/d1.json")
print(f"(M2) ore datasets · {len(a or [])} → {len(b or [])} · {'iguales' if a == b else 'DISTINTOS'}")
if a != b: mal.append("ore datasets cambió")
w0, w1 = lee("/tmp/w0.txt").splitlines(), lee("/tmp/w1.txt").splitlines()
vistas = lambda w: [l for l in w if l and not l.startswith(" ")]
print(f"(M2) ore view · {len(vistas(w0))} → {len(vistas(w1))} vistas · líneas nunca nula: "
      f"{sum(l.startswith('  nunca nula') for l in w0)} → {sum(l.startswith('  nunca nula') for l in w1)}")
if vistas(w0) != vistas(w1): mal.append("ore view enseña otras vistas")
# `nunca nula` es 0051 P4; el resumen de `restricciones` crece con E5′ (cada
# puntero nuevo trae la clave de su tabla). Se enseñan; no son otra cosa.
esperado = lambda l: l.startswith("  nunca nula") or l.startswith("  restricciones")
for x, y in zip([l for l in w0 if l.startswith("  restricciones")], [l for l in w1 if l.startswith("  restricciones")]):
    if x != y:
        print(f"(M2) ore view · {x.strip()} → {y.strip().split('restricciones', 1)[1].strip()} (E5′: las claves de los punteros nuevos)")
sin = lambda w: [l for l in w if not esperado(l)]
if sin(w1) != sin(w0):
    import difflib
    otras = [l for l in difflib.unified_diff(sin(w0), sin(w1), lineterm="", n=0) if l[:1] in "+-" and l[:3] not in ("+++", "---")]
    print("(M2) ore view, lo que cambia además (%d líneas):" % len(otras))
    for l in otras[:30]:
        print("(M2)   " + l[:160])
    mal.append("ore view cambia algo más que `nunca nula`")
fuentes = lee("/tmp/fuentes.txt").split()
nuevas_t = nuevas_s = mod_t = mod_p = cols = cols_nuevas = 0
for linea in lee("/tmp/cambios.txt").splitlines():
    est, ruta = linea.split("\t", 1)
    partes = ruta.split("/")
    if len(partes) < 3 or partes[0] != "packages" or partes[1] not in fuentes:
        mal.append(f"fuera de un paquete de fuente: {est} {ruta}"); continue
    nuevo = lee(ruta)
    kind = (re.search(r"^kind:\s*(\w+)", nuevo, re.M) or [None, None])[1]
    if est == "A":
        if kind not in ("Table", "Schema") or MARCA not in nuevo:
            mal.append(f"nuevo y no es del inductor: {ruta} ({kind})"); continue
        if kind == "Table":
            nuevas_t += 1; cols_nuevas += nuevo.count("required: true")
        else:
            nuevas_s += 1
    elif est == "M":
        viejo = subprocess.run(["git", "show", f"HEAD:{ruta}"], capture_output=True, text=True).stdout
        if kind == "Package":
            dif = [(x, y) for x, y in zip(viejo.splitlines(), nuevo.splitlines()) if x != y]
            if len(viejo.splitlines()) != len(nuevo.splitlines()) or any(not x.startswith("spec:") for x, _ in dif):
                mal.append(f"un package.yaml cambia algo más que `spec:`: {ruta}")
            mod_p += 1
        elif kind == "Table" and MARCA in viejo:
            mod_t += 1; cols += nuevo.count("required: true") - viejo.count("required: true")
        else:
            mal.append(f"se modifica algo que no es del inductor: {ruta} ({kind})")
    else:
        mal.append(f"{est} {ruta}: la migración no borra ni renombra")
print(f"(M2) 0051 · tablas existentes que ganan required: {mod_t} · columnas: {cols}")
print(f"(M2) E5′  · punteros nuevos: {nuevas_t} (con {cols_nuevas} columnas required) · schemas nuevos: {nuevas_s} · package.yaml con exports nuevos: {mod_p}")
if mal:
    print("(M2) ✗ " + "\n(M2) ✗ ".join(mal)); sys.exit(1)
print("(M2) ✓ todo lo que cambia es del inductor, y nada de lo que se ve cambia salvo `nunca nula`")
'''


# M4 · lo que el ore-serve vivo dice: el agente del inquilino pregunta.
COMPROBAR_SH = r"""#!/bin/sh
set -e
CLIENTE=$(cat /puesto/agente-cliente); SECRETO=$(cat /puesto/agente-secreto)
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" \
  -d grant_type=client_credentials -d "client_id=$CLIENTE" -d "client_secret=$SECRETO" \
  | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
S="http://ore-serve.t-$CELDA.svc.cluster.local:8080"
C=$(curl -s -o /tmp/d.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$S/datasets")
echo "(M4) GET /datasets → $C"
python3 /guion/m4.py datasets /tmp/d.json
for f in $FUENTES; do
  C=$(curl -s -o /tmp/e.json -w '%{http_code}' -H "authorization: Bearer $TOK" "$S/paquetes/$f/esquema")
  python3 /guion/m4.py esquema /tmp/e.json "$f" "$C"
done
"""

M4_PY = r'''
import json, sys
que, ruta = sys.argv[1], sys.argv[2]
try:
    j = json.load(open(ruta))
except Exception:
    print("(M4) %s · sin JSON" % " ".join(sys.argv[3:] or [que])); sys.exit(0)
if que == "datasets":
    ds = (j.get("datasets") if isinstance(j, dict) else j) or []
    print("(M4) datasets=%d" % len(ds))
    for d in sorted(ds, key=lambda x: str(x.get("nombre") or x.get("dataset"))):
        print("(M4)   %s · %s · snapshot %s" % (d.get("nombre") or d.get("dataset"), d.get("estado", "?"),
                                              d.get("snapshot") or d.get("snapshot_id") or "-"))
else:
    f, c = sys.argv[3], sys.argv[4]
    ts = j.get("tables") or []
    cols = [x for t in ts for x in (t.get("columns") or [])]
    print("(M4) esquema de %s → %s · tablas %d · columnas %d · required %d"
          % (f, c, len(ts), len(cols), sum(1 for x in cols if x.get("required") is True)))
'''


# M4 · la copia, en seco: con el árbol migrado, ¿recalcularía alguna? La cabecera
# de una copia no lleva la nulabilidad (0051 P4), así que todas deberían seguir
# al día. Corre como la copia de verdad: la imagen ore-drivers, su identidad.
COPIA_SH = r"""#!/bin/sh
set -e
export GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=http.extraheader GIT_CONFIG_KEY_1=safe.directory GIT_CONFIG_VALUE_1='*' GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_VALUE_0="Authorization: token $(cat /puesto/forja)"
cd /tmp && git clone --quiet "http://forja.t-$CELDA.svc.cluster.local:3000/t-$CELDA/ontologia.git" arbol && cd arbol
export ORE_STORE=gcs ORE_GCS_BUCKET="project-8853a180-450d-47be-b83-t-$CELDA-copia"
echo "(M4) copia en seco sobre $(git rev-parse --short HEAD) · almacén $ORE_STORE, $ORE_GCS_BUCKET"
ore materialize . --seco > /tmp/s.txt 2>&1 || true
echo "(M4) ya está: $(grep -c 'ya está' /tmp/s.txt) · haría falta calcular: $(grep -c 'haría falta calcularla' /tmp/s.txt) · el puntero lo dijo sin leer: $(grep -c 'sin leer una sola fila' /tmp/s.txt) · fuente sin credencial aquí: $(grep -c 'no está definida' /tmp/s.txt) · el manifiesto no se pudo leer: $(grep -c 'no se pudo leer el manifiesto' /tmp/s.txt)"
grep -B1 'haría falta calcularla' /tmp/s.txt | sed 's/^/(M4)   /' | head -40
tail -1 /tmp/s.txt | sed 's/^/(M4)   /'
"""


def job(ns, celda, empujar, comprobar=False, copia=False):
    img = "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO
    secretos = ("set -e\nfor p in cliente secreto; do gcloud secrets versions access latest --secret=%s-agente-$p --out-file=/puesto/agente-$p; done\nchmod 0400 /puesto/*\n"
                if comprobar else
                "set -e\ngcloud secrets versions access latest --secret=%s-forja-token --out-file=/puesto/forja\nchmod 0400 /puesto/*\n") % ns
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
                            "command": ["/bin/sh", "/guion/comprobar.sh" if comprobar else "/guion/copia.sh" if copia else "/guion/guion.sh"],
                            "env": [{"name": "CELDA", "value": celda}, {"name": "EMPUJAR", "value": "1" if empujar else ""}, {"name": "HOME", "value": "/tmp"},
                                    {"name": "FUENTES", "value": FUENTES.get(celda, "")},
                                    {"name": "DIRECCION", "value": "http://idp-service.identidad.svc.cluster.local:8080"}, {"name": "REALM", "value": "rubix"}],
                            "resources": {"requests": {"cpu": "200m", "memory": "512Mi"}, "limits": {"cpu": "1000m", "memory": "1Gi"}},
                            "volumeMounts": [{"name": "puesto", "mountPath": "/puesto", "readOnly": True}, {"name": "guion", "mountPath": "/guion"}]}],
            "volumes": [{"name": "puesto", "emptyDir": {"medium": "Memory"}}, {"name": "guion", "configMap": {"name": NOMBRE, "defaultMode": 0o555}}],
        }}},
    }


def al_dia(celda, empujar, comprobar=False, copia=False):
    ns = "t-" + celda
    print("  %s%s" % (celda, " (lo que dice el ore-serve vivo)" if comprobar else "" if empujar else " (ensayo)"))
    datos = {"guion.sh": GUION_SH, "comprobar.py": COMPROBAR_PY, "comprobar.sh": COMPROBAR_SH, "m4.py": M4_PY, "copia.sh": COPIA_SH}
    kubectl("apply", "-f", "-", entrada=json.dumps({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": NOMBRE, "namespace": ns}, "data": datos}))
    kubectl("delete", "job", NOMBRE, "-n", ns, "--ignore-not-found", "--wait=true")
    kubectl("apply", "-f", "-", entrada=json.dumps(job(ns, celda, empujar, comprobar, copia)))
    t0 = time.time()
    st = {}
    while time.time() - t0 < 900:
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


def main():
    args = sys.argv[1:]
    empujar = "--empujar" in args
    comprobar = "--comprobar" in args
    copia = "--copia" in args
    celdas = [a for a in args if not a.startswith("--")] or ["victor"]
    print("0051 P3′ · la fuente al día%s" % (" · comprobación" if comprobar else "" if empujar else " · ENSAYO (sin --empujar nada cambia)"))
    bien = all([al_dia(c, empujar, comprobar, copia) for c in celdas])
    print("  (el clúster, como estaba: el Job y su ConfigMap se retiraron)")
    return 0 if bien else 1


if __name__ == "__main__":
    sys.exit(main())
