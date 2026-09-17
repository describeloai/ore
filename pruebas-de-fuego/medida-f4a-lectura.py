#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
medida-f4a-lectura.py — ¿qué falta para F4a con una FUNCIÓN DE LECTURA?

ADR 0029, orden revisado, paso 2: una `Function` con `runtime: model`, `over` (la
vista copiada), `output` y SIN `effects`: lee la copia, llama al modelo real,
devuelve. Sin entidad, sin clave, sin L0, sin wasm. Esto mide, antes de escribir
nada, cuánto de cada mitad del Job (0029 ③: traer · invocar · devolver) ya existe:

  §A  la gramática: ¿`ore validate` acepta hoy esa función sobre una vista
      copiada de una base estándar (sin entidades)? Local, con el `ore` del árbol.
  §B  traer: ¿la copia se puede RELEER? El almacén (`ore-store-gcs`) y lo que hay
      en el bucket de demo (sólo listar).
  §C  invocar: ¿qué ve demo del gateway hoy? (`GET /modelos`; la máquina
      `modelos-e0` está parada: nada se enciende aquí).
  §D  devolver y quién manda: las piezas del Job que existen o no (`ore-invoke`,
      `49-la-invocacion.yaml`, `/funciones`, `ore verify`).

No enciende nada de pago. Necesita `gcloud` y `kubectl` sólo para §B/§C; sin
ellos mide §A y §D y lo dice.

  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-f4a-lectura.py [--sin-demo]
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIN_DEMO = "--sin-demo" in sys.argv
PROYECTO = "project-8853a180-450d-47be-b83"
CELDA = "demo"
NS = "t-" + CELDA
BUCKET = "gs://%s-%s-copia" % (PROYECTO, NS)
IDP = "https://login.paladio.io/realms/rubix"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"
GSUTIL = "gsutil.cmd" if os.name == "nt" else "gsutil"

FILAS = []


def fila(que, medido, veredicto):
    FILAS.append((que, medido, veredicto))
    print("  %-46s %-58s %s" % (que, medido[:58], veredicto))


def sh(cmd, cwd=None):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8", cwd=cwd)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def ore():
    for c in ("release", "debug"):
        for n in ("ore.exe", "ore"):
            p = os.path.join(RAIZ, "target", c, n)
            if os.path.isfile(p):
                return p
    sys.exit("no hay binario de `ore`: cargo build -p ore-cli")


# ── §A · la gramática ───────────────────────────────────────────────────────
ARBOL = {
    "ontology.config.yaml": """apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
datasources:
  - { name: pg, type: postgres, connectionEnv: PG_URL }
  - { name: copia, type: iceberg, connectionEnv: COPIA_URL }
""",
    "conduits.yaml": """apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: demo }
spec:
  owner: team:data
  conduits:
    materialization.payload: { oos.maturity: DRAFT }
""",
    "modelos/v2-lite.yaml": """apiVersion: oos.dev/v1alpha9
kind: Model
metadata: { name: v2-lite }
spec:
  profile: g1/deepseek-v2-lite
  tier: shared
  task: chat
""",
    "packages/olist_copia/package.yaml": """apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: olist_copia, version: 0.1.0, status: active, domain: sales }
spec: { owner: team:data }
""",
    # Lo que el inductor deja para una tabla SIN modelar de una base estándar
    # (0027 C1): Table + View trivial con `materialized`, ninguna Entity.
    "packages/olist_copia/tables/product_category_name_translation.yaml": """apiVersion: oos.dev/v1alpha8
kind: Table
metadata: { name: olist_product_category_name_translation, namespace: olist_copia }
spec:
  datasource: pg
  object: "olist.product_category_name_translation"
  columns:
    product_category_name: { physicalType: text }
    product_category_name_english: { physicalType: text }
  reads: { fullScan: cheap }
  changes: { mode: append, witness: log }
""",
    "packages/olist_copia/views/productCategoryNameTranslation.yaml": """apiVersion: oos.dev/v1alpha8
kind: View
metadata: { name: productCategoryNameTranslation, namespace: olist_copia }
spec:
  owner: team:data
  from: { table: olist_copia.olist_product_category_name_translation }
  fields:
    productCategoryName: product_category_name
    productCategoryNameEnglish: product_category_name_english
  materialized: { datasource: copia, table: "copia.productCategoryNameTranslation" }
""",
}

FUNCION_LECTURA = """apiVersion: oos.dev/v1alpha10
kind: Function
metadata: { name: traducirCategoria, namespace: olist_copia }
spec:
  runtime: model
  model: modelo/v2-lite
  over: olist_copia.productCategoryNameTranslation
  prompt: "Traduce la categoría al español en una o dos palabras."
  output:
    categoriaEs: { type: String }
"""


def arbol(extra):
    d = tempfile.mkdtemp(prefix="f4a-")
    for p, c in list(ARBOL.items()) + list(extra.items()):
        f = os.path.join(d, p)
        os.makedirs(os.path.dirname(f), exist_ok=True)
        with open(f, "w", encoding="utf-8", newline="\n") as h:
            h.write(c)
    return d


def valida(extra):
    d = arbol(extra)
    rc, out = sh('"%s" validate .' % ore(), cwd=d)
    shutil.rmtree(d, ignore_errors=True)
    codigos = sorted(set(re.findall(r"OOS\d{4}", out)))
    return rc, codigos, out


def seccion_a():
    print("\n§A · la gramática (ore validate, local)")
    rc, cod, _ = valida({})
    fila("A1 · la base estándar sin entidades compila", "rc %d %s" % (rc, cod), "✓" if rc == 0 else "✗")
    rc, cod, out = valida({"packages/olist_copia/functions/traducirCategoria.yaml": FUNCION_LECTURA})
    fila("A2 · + Function lectura (model, over, output, sin effects)", "rc %d %s" % (rc, cod), "✓ la gramática la admite" if rc == 0 else "✗ " + out.strip().splitlines()[0][:40])
    sin_output = FUNCION_LECTURA.replace("  output:\n    categoriaEs: { type: String }\n", "")
    rc, cod, _ = valida({"packages/olist_copia/functions/f.yaml": sin_output})
    fila("A3 · la misma sin `output` (sólo over)", "rc %d %s" % (rc, cod), "admitida: leer y no devolver nada" if rc == 0 else "rechazada")
    sin_over = FUNCION_LECTURA.replace("  over: olist_copia.productCategoryNameTranslation\n", "")
    rc, cod, _ = valida({"packages/olist_copia/functions/f.yaml": sin_over})
    fila("A4 · sin `over` ni reads ni effects", "rc %d %s" % (rc, cod), "✓ OOS1004 (no toca nada)" if "OOS1004" in cod else "⚠ admitida")
    sin_modelo = FUNCION_LECTURA.replace("modelo/v2-lite", "modelo/no-existe")
    rc, cod, _ = valida({"packages/olist_copia/functions/f.yaml": sin_modelo})
    fila("A5 · `model` que no resuelve a un Model", "rc %d %s" % (rc, cod), "✓ lo coteja" if rc != 0 else "⚠ NO lo coteja: el Job lo descubriría")
    sin_copia = ARBOL["packages/olist_copia/views/productCategoryNameTranslation.yaml"].replace('  materialized: { datasource: copia, table: "copia.productCategoryNameTranslation" }\n', "")
    rc, cod, _ = valida({"packages/olist_copia/views/productCategoryNameTranslation.yaml": sin_copia,
                         "packages/olist_copia/functions/f.yaml": FUNCION_LECTURA})
    fila("A6 · `over` una vista SIN copia (foránea)", "rc %d %s" % (rc, cod), "la gramática no lo distingue: es del runtime (0029 ②)" if rc == 0 else "rechazada")
    rc, out = sh('"%s" invoke --help' % ore())
    fila("A7 · `ore invoke`", ("existe" if rc == 0 else out.strip().splitlines()[0][:50]), "—" if rc != 0 else "✓")


# ── §B · traer: releer la copia ─────────────────────────────────────────────
def seccion_b():
    print("\n§B · traer: la copia se puede releer")
    src = open(os.path.join(RAIZ, "crates/ore-store/src/ciclo.rs"), encoding="utf-8").read()
    verbos = re.findall(r'^\s+"([a-z-]+)" =>', src, re.M)
    fila("B1 · verbos de ore-store-gcs", ", ".join(verbos), "no hay `leer`: `anterior` relee para fundir, no para servir filas")
    carga = open(os.path.join(RAIZ, "crates/ore-store/src/carga.rs"), encoding="utf-8").read()
    fila("B2 · Parquet → filas en el almacén", "carga::leer %s" % ("existe" if "pub fn leer(parquet" in carga else "NO"), "✓ la mitad difícil está")
    if SIN_DEMO:
        return
    rc, out = sh('%s ls -l "%s/ore/v1/"' % (GSUTIL, BUCKET))
    arte = [l for l in out.splitlines() if "/ore/v1/" in l and not l.strip().endswith("/") and "plan/" not in l]
    rc2, out2 = sh('%s ls -r "%s/ore/v1/plan/"' % (GSUTIL, BUCKET))
    recibos = [l for l in out2.splitlines() if l.strip() and not l.endswith(":") and not l.endswith("/")]
    tam = sum(int(l.split()[0]) for l in arte if l.split() and l.split()[0].isdigit())
    fila("B3 · el bucket de demo", "%d artefactos (%.1f KB) · %d recibos" % (len(arte), tam / 1024.0, len(recibos)), "✓ hay qué leer" if arte else "✗ vacío")


# ── §C · invocar: la puerta vista desde demo ────────────────────────────────
def token_del_agente():
    _, cli = sh("%s secrets versions access latest --secret=%s-agente-cliente --project=%s" % (GCLOUD, NS, PROYECTO))
    _, sec = sh("%s secrets versions access latest --secret=%s-agente-secreto --project=%s" % (GCLOUD, NS, PROYECTO))
    cli, sec = cli.strip(), sec.strip()
    if not cli or not sec:
        return None
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials",
                        "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)["access_token"]
    except Exception:
        return None


def serve(camino, tok):
    r = subprocess.run(["curl", "-s", "-m", "60", "-o", "-", "-w", "\n%{http_code} %{time_total}",
                        "https://%s.ore.paladio.io%s" % (CELDA, camino), "-H", "authorization: Bearer " + tok],
                       capture_output=True, text=True, encoding="utf-8")
    cuerpo, _, cod = (r.stdout or "").rpartition("\n")
    p = cod.split()
    return (p[0] if p else "000"), (p[1] if len(p) > 1 else "?"), cuerpo.strip()


def seccion_c():
    print("\n§C · invocar: la puerta vista desde demo")
    _, vm = sh("%s compute instances describe modelos-e0 --zone europe-west1-b --project %s --format=value(status)" % (GCLOUD, PROYECTO))
    fila("C1 · la máquina del gateway (`modelos-e0`, e2-micro)", vm.strip() or "?", "parada: nada se enciende sin go" if "TERMINATED" in vm else "")
    if SIN_DEMO:
        return
    tok = token_del_agente()
    if not tok:
        fila("C2 · token del agente de demo", "no se pudo acuñar", "✗")
        return
    cod, t, cuerpo = serve("/modelos", tok)
    try:
        ms = json.loads(cuerpo).get("modelos") or json.loads(cuerpo).get("models") or []
    except Exception:
        ms = []
    resumen = "; ".join("%s %s" % (m.get("name") or m.get("nombre"), (m.get("estado") or {}).get("fase")) for m in ms) if isinstance(ms, list) else cuerpo[:60]
    fila("C2 · GET /modelos en demo", "%s en %s s · %s" % (cod, t, resumen or cuerpo[:60]), "sin Model en demo: el alta es un paso" if not ms else "")
    cod, t, cuerpo = serve("/paquetes/olist_copia/copias", tok)
    try:
        cs = json.loads(cuerpo).get("copias", [])
        det = ", ".join("%s %s" % (c["view"], (c.get("copia") or {}).get("filas")) for c in cs)
    except Exception:
        det = cuerpo[:60]
    fila("C3 · las copias de olist_copia", "%s · %s" % (cod, det), "la de 71 filas es la aceptación")


# ── §D · devolver, y quién manda ────────────────────────────────────────────
def seccion_d():
    print("\n§D · devolver, y quién manda: las piezas del Job")
    def hay(patron, *fs):
        for f in fs:
            p = os.path.join(RAIZ, f)
            if os.path.isfile(p) and re.search(patron, open(p, encoding="utf-8").read()):
                return True
        return False
    fila("D1 · `ore-invoke` (0029 ⑤, el delegado)", "crates/ore-invoke: %s" % ("existe" if os.path.isdir(os.path.join(RAIZ, "crates/ore-invoke")) else "NO"), "el que habla con la puerta: ureq como ore-store-gcs")
    fila("D2 · `49-la-invocacion.yaml` en la malla", "existe" if os.path.isfile(os.path.join(RAIZ, "malla/49-la-invocacion.yaml")) else "NO", "el Job con el agente de la celda; salida-al-modelo ya está")
    fila("D3 · `POST /funciones/{ns}/{n}/invocar` en ore-serve", "existe" if hay(r"/funciones", "crates/ore-serve/src/rutas.rs") else "NO", "encola; Cedar sólo si la función declara authorization")
    fila("D4 · `ore verify` (la Propuesta)", "existe" if hay(r"Command::Verify", "crates/ore-cli/src/main.rs") else "NO", "no aplica a una lectura: no hay Propuesta")
    fila("D5 · dónde aterriza el `output` de una lectura", "nadie lo dice (0029 ③ sólo habla de la Propuesta)", "⚠ decisión: bucket + informe en el árbol")
    fila("D6 · GET /modelos/{n} → {url, model}", "existe" if hay(r"fn modelo\(", "crates/ore-serve/src/modelos.rs") else "NO", "el Job lo pregunta con su token")
    fila("D7 · la consola: Data › Jobs con tipo `invocar`", "informar.sh: tipo por prefijo %s" % ("catalogar|copiar" if hay(r"copiar", "malla/informar.sh") else "?"), "un prefijo más")


if __name__ == "__main__":
    t0 = time.time()
    print("F4a · lectura — lo que hay y lo que falta (%s)" % time.strftime("%Y-%m-%d %H:%M"))
    seccion_a()
    seccion_b()
    seccion_c()
    seccion_d()
    print("\n%d medidas en %.0f s" % (len(FILAS), time.time() - t0))
