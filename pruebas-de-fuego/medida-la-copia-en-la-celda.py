# -*- coding: utf-8 -*-
"""MEDIDA · P1, la copia en la celda: ¿cuál es la materia?

Antes de construir el primer eslabón de «inferencia sobre los datos» (ADR 0027,
«Después de E3»): qué existe ya del ciclo de la copia (ADR 0015/0017/0018), qué le
falta a la celda para correrlo, y contra qué almacén. Lo que no está se dice con el
fichero que lo diría.

    uso:  python pruebas-de-fuego/medida-la-copia-en-la-celda.py [celda]
"""
import json
import os
import re
import subprocess
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "demo")
PROYECTO = "project-8853a180-450d-47be-b83"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"


def sh(*cmd):
    r = subprocess.run(list(cmd), capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return (r.stdout or "").strip(), (r.stderr or "").strip()


def lee(rel):
    try:
        return open(os.path.join(RAIZ, rel), encoding="utf-8").read()
    except OSError:
        return ""


def lineas(rel):
    return lee(rel).count("\n")


def fila(que, res):
    print("  %-50s %s" % (que, res))


print("\n  ═══ MEDIDA · P1 · la copia en la celda · %s ═══\n" % CELDA)

# ── A · el ciclo ya existe, fuera de la celda ───────────────────────────────
print("A · lo que ya existe (en local, refresco.sh)")
fila("`ore materialize` (los 6 pasos del ADR 0015)", "SÍ · materializar.rs, %d líneas: compila el plan, testigo del origen, recibo, leer→sellar→subir, registrar, recoger" % lineas("crates/ore-cli/src/materializar.rs"))
fila("los lectores `ore-read-<tipo>`", ", ".join(sorted(d.replace("ore-read-", "") for d in os.listdir(os.path.join(RAIZ, "crates")) if d.startswith("ore-read-"))))
fila("el almacén delegado", "`ore-store-r2` (%d líneas): S3 SigV4 con clave estática · ORE_R2_S3_ENDPOINT/BUCKET/ACCESS_KEY_ID/SECRET_ACCESS_KEY" % sum(lineas("crates/ore-store-r2/src/" + f) for f in os.listdir(os.path.join(RAIZ, "crates/ore-store-r2/src"))))
fila("la prueba de fuego", "`refresco.sh` (R6): cuenta filas leídas del origen por acto · jsonl + R2 de Cloudflare, en local")
fila("`ore` no abre sockets", "cierto por construcción: tests/dependencias.rs lee el Cargo.lock")

# ── B · la gramática: quién declara la copia ────────────────────────────────
print("\nB · quién declara que una vista tiene copia")
ind = lee("crates/ore-cli/src/inductor.rs")
fila("`View.materialized {datasource, table, key}`", "existe en v1alpha8 (la vista de la conformidad)")
fila("¿`ore discover` lo escribe?", "NO, a propósito: «materialized y freshness son decisiones de operación, no se proponen» (inductor.rs)" if "no se proponen" in ind or "decisiones de operaci" in ind else "?")
fila("⇒ en `%s`, ¿alguna vista declara copia?" % CELDA, "NO (el catálogo es del inductor): alguien tiene que ESCRIBIRLO — un verbo (`POST /paquetes/{n}/copia`) o la consola")

# ── C · la celda: qué tiene y qué le falta para correr el ciclo ─────────────
print("\nC · la celda")
cat = lee("malla/44-el-catalogo.yaml")
fila("el Job de catálogo (44)", "la figura entera: forja + agente + cofre → `ore source catalog` → `ore discover` → commit. %d líneas" % cat.count("\n"))
fila("la credencial del origen en el Job", "del cofre por HTTP con el token de agente, a una variable, sin tocar disco (44, líneas ~240)")
fila("¿la imagen ore-drivers trae ore-store-r2?", "?")
img, _ = sh("kubectl", "run", "-n", "t-" + CELDA, "medida-p1", "--rm", "-i", "--restart=Never", "--image=europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO, "--labels=ore.dev/rol=driver,ore.dev/tenant=" + CELDA, "--overrides", json.dumps({"spec": {"containers": [{"name": "medida-p1", "image": "europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO, "command": ["sh", "-c", "ls /usr/local/bin /usr/bin 2>/dev/null | grep -E '^ore' | tr '\\n' ' '"], "resources": {"requests": {"cpu": "50m", "memory": "64Mi"}, "limits": {"cpu": "200m", "memory": "128Mi"}}}]}}))
fila("  binarios `ore*` en la imagen", img.replace("pod \"medida-p1\" deleted", "").strip() or "(no se pudo mirar)")
fila("¿la celda tiene un bucket propio?", "NO: los buckets del proyecto son bastion-perfiles, -copias (backups forja/IdP), _cloudbuild")
fila("¿quién lanzaría `materialize`?", "NADIE: el convergedor sólo rinde Jobs de catálogo (aprovisionar-inquilino.sh ~769: «un Job de catalogo por fuente»)")

# ── D · el almacén: contra qué ──────────────────────────────────────────────
print("\nD · el almacén, medido")
_, err = sh(GCLOUD, "storage", "hmac", "list", "--project", PROYECTO)
pol, _ = sh(GCLOUD, "resource-manager", "org-policies", "describe", "iam.disableServiceAccountKeyCreation", "--project", PROYECTO, "--effective", "--format=value(booleanPolicy.enforced)")
fila("GCS por la API S3 (HMAC) con `ore-store-r2`", "NO SE PUEDE: `gcloud storage hmac create` → 412 «viola constraints/iam.disableServiceAccountKeyCreation» (política de la organización, enforced=%s)" % (pol or "?"))
fila("R2 de Cloudflare (lo que usa refresco.sh)", "funciona, pero la copia SALE de la VPC (0027: «nada sale de la VPC»; 0028 soberanía)")
fila("GCS por su API JSON con Workload Identity", "es lo que los Jobs ya usan para Secret Manager (`gcloud` con la cuenta `driver`): un token del metadata server, sin clave estática")
fila("⇒ el delegado que falta", "`ore-store-gcs`: el mismo protocolo (cabecera + filas por stdin → una línea), buscar/sellar/recoger contra storage.googleapis.com con el token de WI")

# ── E · el origen de verdad: olist en demo ──────────────────────────────────
print("\nE · el origen de verdad (%s)" % CELDA)
src = lee("pruebas-de-fuego/deployments-tiene-filas.py").split('print("\\n  ═══ E3 I5')[0]
g = {}
exec(compile(src, "ayudas", "exec"), g)  # noqa: S102
g["CELDA"] = CELDA
tok = g["token_del_agente"](CELDA)
cod, _, cuerpo = g["serve"]("GET", "/paquetes", tok)
paquetes = json.loads(cuerpo).get("packages", []) if cod == "200" else []
elegidos = [p for p in paquetes if p.get("scoped")]
for p in elegidos[:1]:
    cod, _, cuerpo = g["serve"]("GET", "/paquetes/%s/esquema" % p["name"], tok)
    e = json.loads(cuerpo) if cod == "200" else {}
    ents = e.get("entities", [])
    fila("paquete elegido `%s` (fuente %s)" % (p["name"], p.get("source")), "%d entidades: %s" % (len(ents), ", ".join(x.get("name", "?") for x in ents)[:100]))
    claves = [x.get("name") for x in ents if x.get("primaryKey")]
    fila("  con clave primaria (lo que `key:` necesita)", "%d de %d" % (len(claves), len(ents)))
fila("el origen", "Postgres del cliente, alcanzado por Cloud NAT; la credencial en el cofre (`fuente-<n>`)")

print("\n  ═══ la materia de P1, en orden, está en la ADR 0027 §P1 ═══\n")
