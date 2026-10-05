"""EL REPARTO, MEDIDO (ADR 0053 F5·3) · `ore explain` sobre dos corpus.

  §1  LA SINTAXIS DE DUCKDB  las 52 frases de `medida-el-terreno-de-la-regex.py`
                             (la «friendly SQL» y los casos raros), sobre un árbol
                             de prueba donde `hr.t`, `hr.a`, `hr.b` son `Table` de
                             un origen que admite todas las familias: ¿se analiza
                             (o cae a B)?, ¿qué se empuja?, ¿algo se niega?
  §2  CONSULTAS DE VERDAD    sobre el árbol de `main` de victor (sus tablas tal
                             como las dejó la inducción), consultas que un
                             analista escribiría contra Neon, BigQuery y S3.

    ORE=target/debug/ore python pruebas-de-fuego/medida-el-reparto.py [--victor DIR]

`--victor`: el árbol de main de victor (`git archive main` de la forja). Sin él,
sólo §1. No abre ningún origen: `explain` lee el árbol.
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
ORE = os.environ.get("ORE", "target/debug/ore")

spec = importlib.util.spec_from_file_location("terreno", os.path.join(AQUI, "medida-el-terreno-de-la-regex.py"))
terreno = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terreno)
CORPUS = terreno.CORPUS


def explain(raiz, sql):
    p = subprocess.run([ORE, "explain", "--json", "--path", raiz, "--", sql], capture_output=True, text=True)
    try:
        return json.loads(p.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {"ok": False, "codigo": "?", "mensaje": (p.stdout + p.stderr).strip()[:160]}


def arbol_de_prueba():
    d = tempfile.mkdtemp(prefix="ore-reparto-")
    w = lambda f, t: (os.makedirs(os.path.dirname(os.path.join(d, f)), exist_ok=True), open(os.path.join(d, f), "w").write(t))
    w("ontology.config.yaml", "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: m, version: 0.1.0 }\n"
      "datasources:\n  - name: pg\n    type: postgres\n    connectionEnv: PG\n    federation: true\n")
    w("conduits.yaml", "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: m }\nspec:\n  owner: team:m\n"
      "  conduits:\n    federation.read: { oos.maturity: DRAFT }\n")
    w("packages/hr/package.yaml", "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: hr, version: 0.1.0, status: draft, domain: hr }\nspec: { owner: \"team:m\" }\n")
    cols = ["id", "a", "b", "c", "t", "pais", "total", "cuando"]
    for n in ("t", "a", "b"):
        w(f"packages/hr/tables/{n}.yaml",
          "apiVersion: oos.dev/v1alpha22\nkind: Table\n"
          f"metadata: {{ name: {n}, namespace: hr }}\nspec:\n  datasource: pg\n  object: \"public.{n}\"\n  columns:\n"
          + "".join(f"    {c}: {{ type: String }}\n" for c in cols)
          + "  reads:\n    fullScan: cheap\n    predicatePushdown: [eq, neq, in, range, like, isNull]\n  changes:\n    key: [id]\n")
    return d, len(cols)


def resumen(r, total_cols=None):
    if not r.get("ok"):
        return f"NO {r.get('codigo')}: {r.get('mensaje', '')[:90]}"
    if not r["lecturas"]:
        return "sin origen (lago o nada)"
    partes = []
    for l in r["lecturas"]:
        tot = total_cols or "?"
        f = len(l["empujados"])
        m = len(l["enElMotor"])
        lim = f" limit {l['limit']}" if "limit" in l else ""
        partes.append(f"{l['tabla']}: {len(l['columnas'])}/{tot} cols, {f} al origen, {m} al motor{lim}")
    return ("" if r["entendida"] else "[B] ") + "; ".join(partes)


def seccion_1():
    raiz, n = arbol_de_prueba()
    print("§1  LA SINTAXIS DE DUCKDB — 52 frases, `hr.*` como Table de un origen\n")
    cuenta = {"entendida": 0, "B": 0, "sin origen": 0, "no": 0}
    for q, _ in CORPUS:
        r = explain(raiz, q)
        if not r.get("ok"):
            cuenta["no"] += 1
        elif not r["lecturas"]:
            cuenta["sin origen"] += 1
        elif r["entendida"]:
            cuenta["entendida"] += 1
        else:
            cuenta["B"] += 1
        print(f"  {q.replace(chr(10), ' ')[:62]:<62}  {resumen(r, n)}")
    print(f"\n  → analizadas {cuenta['entendida']}, leídas sin empujar (B) {cuenta['B']}, "
          f"sin origen {cuenta['sin origen']}, negadas {cuenta['no']} — de {len(CORPUS)}\n")
    return cuenta


VICTOR = [
    ("un usuario", "SELECT summary_text, generated_at FROM postgresql_20260921_2055.public.ai_insights WHERE user_id = 'user_2z6AxnKAsi72sT810URRFk8MZ3m'"),
    ("los últimos 10", "SELECT id, generated_at FROM postgresql_20260921_2055.public.ai_insights ORDER BY generated_at DESC LIMIT 10"),
    ("un rango de fechas", "SELECT id FROM postgresql_20260921_2055.public.ai_insights WHERE generated_at >= TIMESTAMP '2026-09-01'"),
    ("contar por campaña", "SELECT campaign_id, count(*) FROM postgresql_20260921_2055.public.ai_insights GROUP BY campaign_id"),
    ("clientes de un país", "SELECT id, email FROM bigquery_20260927_1428.ventas.clientes WHERE pais = 'ES'"),
    ("clientes, varios países", "SELECT id FROM bigquery_20260927_1428.ventas.clientes WHERE pais IN ('ES', 'FR')"),
    ("clientes enteros", "SELECT * FROM bigquery_20260927_1428.ventas.clientes"),
    ("la sintética, una fila", "SELECT * FROM bigquery_20260927_1428.ventas.ore_e2e_sintetica WHERE id = 7"),
    ("la sintética, primeras 100", "SELECT id, total FROM bigquery_20260927_1428.ventas.ore_e2e_sintetica LIMIT 100"),
    ("la sintética, total > 500", "SELECT id FROM bigquery_20260927_1428.ventas.ore_e2e_sintetica WHERE total > 500 LIMIT 100"),
    ("pedidos de S3, un cliente", "SELECT id, total FROM s3_demo.nueva_carpeta.pedidos WHERE cliente_id = 'C0010'"),
    ("pedidos de S3, primeros 20", "SELECT * FROM s3_demo.nueva_carpeta.pedidos LIMIT 20"),
    ("BigQuery × S3", "SELECT c.email, p.total FROM bigquery_20260927_1428.ventas.clientes c "
                      "JOIN s3_demo.nueva_carpeta.pedidos p ON p.cliente_id = c.id WHERE c.pais = 'ES' AND p.total > 100"),
    ("Neon × S3, LEFT", "SELECT a.user_id, p.id FROM postgresql_20260921_2055.public.ai_insights a "
                        "LEFT JOIN s3_demo.nueva_carpeta.pedidos p ON p.cliente_id = a.user_id AND p.fecha = DATE '2026-09-01'"),
    ("con un WITH", "WITH es AS (SELECT id, email FROM bigquery_20260927_1428.ventas.clientes WHERE pais = 'ES') "
                    "SELECT email FROM es WHERE id = 'ore-e2e-c1'"),
]


def seccion_2(raiz):
    print("§2  CONSULTAS DE VERDAD — el árbol de main de victor, como lo dejó la inducción\n")
    for nombre, q in VICTOR:
        r = explain(raiz, q)
        print(f"  {nombre:<28} {resumen(r)}")
        if r.get("ok"):
            for l in r["lecturas"]:
                for f in l["empujados"]:
                    print(f"  {'':<28}   al origen: {f['columna']} {f['operador']} {f.get('valor')}")
                for m in l["enElMotor"]:
                    print(f"  {'':<28}   al motor:  {m}")
                for a in l["avisos"]:
                    print(f"  {'':<28}   ⚠ {a}")
    print()


def main():
    seccion_1()
    if "--victor" in sys.argv:
        seccion_2(sys.argv[sys.argv.index("--victor") + 1])
    return 0


if __name__ == "__main__":
    sys.exit(main())
