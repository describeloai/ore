# -*- coding: utf-8 -*-
"""La cuota del tier compartido, en `iam` y en la plantilla: la MISMA, o rojo.

`026-el-tier.sql` dice lo que el compartido promete —10 vCPU, 36Gi, 50 jobs—
para que la consola lo pinte desde el plano de control. `malla/11-el-inquilino.yaml`
dice lo que el compartido HACE CUMPLIR: la `ResourceQuota` del namespace.

Son dos descripciones de un hecho, y eso es lo que este arbol no permite sin
una medida al lado: el dia que alguien suba la cuota en el malla y no en la
026 —o al reves—, la consola prometeria una cosa y el cluster otra, y las dos
parecerian ciertas. Esto lo pone rojo.

Sin red, sin base: lee los dos ficheros y compara.
"""
import pathlib
import re
import sys

for f in (sys.stdout, sys.stderr):
    try:
        f.reconfigure(errors="replace")
    except AttributeError:
        pass

RAIZ = pathlib.Path(__file__).resolve().parent.parent
SQL = (RAIZ / "iam/migraciones/026-el-tier.sql").read_text(encoding="utf-8")
YAML = (RAIZ / "malla/11-el-inquilino.yaml").read_text(encoding="utf-8")

# ── lo que la 026 dice del compartido ───────────────────────────────────────
m = re.search(r"\('compartido',.*?'([^']*)',\s*'([^']*)',\s*'([^']*)'\)", SQL, re.S)
assert m, "la 026 no trae la fila de `compartido` con tres cuotas"
sql_cpu, sql_mem, sql_jobs = m.groups()

# ── lo que la plantilla hace cumplir ────────────────────────────────────────
bloque = re.search(r"kind: ResourceQuota.*?spec:\s*hard:(.*?)(?:\n---|\Z)", YAML, re.S)
assert bloque, "la plantilla no trae una ResourceQuota"
hard = dict(re.findall(r'^\s*([\w./-]+):\s*"?([^"\n]+)"?\s*$', bloque.group(1), re.M))
yaml_cpu = hard.get("requests.cpu")
yaml_mem = hard.get("requests.memory")
yaml_jobs = hard.get("count/jobs.batch")

print("== la cuota del compartido, en dos sitios ==")
print()
print("   %-18s %-10s %-10s" % ("", "iam (026)", "malla (11)"))
fallos = 0
for etiqueta, a, b in (("requests.cpu", sql_cpu, yaml_cpu),
                       ("requests.memory", sql_mem, yaml_mem),
                       ("count/jobs.batch", sql_jobs, yaml_jobs)):
    igual = a == b
    fallos += 0 if igual else 1
    print("   %-18s %-10s %-10s %s" % (etiqueta, a, b, "ok" if igual else "x  DIVERGEN"))

# ── y lo que la consola NO debe tener: una copia propia ─────────────────────
consola = pathlib.Path(r"C:\rubix-platform\lib\cloud\cluster.ts")
if consola.exists():
    t = consola.read_text(encoding="utf-8", errors="replace")
    if re.search(r"36Gi|'10'|\"10\"|requests\.cpu", t):
        print()
        print("   x  la consola lleva una copia de la cuota en lib/cloud/cluster.ts — tiene que salir de /celdas")
        fallos += 1

print()
if fallos:
    print("== %d divergencia(s): la consola prometeria una cosa y el cluster otra ==" % fallos)
    sys.exit(1)
print("== todo verde: una cuota, dicha en dos sitios que coinciden ==")
