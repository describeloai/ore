"""EL TERRENO DE QUITAR LA REGEX · despues de arreglar las Views, medido otra vez.

`medida-la-regex-fuera.py` dejo dos rutas: (A) el motor pregunta al catalogo por
ATTACH; (B) ore-serve analiza el texto y resuelve de una vez. El arreglo de las
Views cambio el terreno: una View llega al motor como SQL (`ore ask --sql`), y
eso solo sirve por ATTACH si DuckDB lee vistas de Iceberg REST.

  §1  LAS VIEWS POR ATTACH   `medida-las-vistas-por-el-catalogo.py`: ¿pide
                             DuckDB `/views`?
  §2  EL TECHO DE (B)        lo que ore-serve no sepa analizar no se resuelve.
                             45 frases de la sintaxis propia de DuckDB (la
                             «friendly SQL» de su documentacion) y los 7 casos
                             donde la regex falla, contra: DuckDB
                             (¿la acepta? es la verdad), sqlparser 0.63 con el
                             dialecto DuckDB (¿la analiza? ¿saca los nombres?) y
                             la regex de hoy.
  §3  LO QUE NO ES DEL ARBOL un `a.b` puede ser un esquema de la sesion (`create
                             schema tmp`), no un paquete: ¿que hace hoy la regex?

    python pruebas-de-fuego/medida-el-terreno-de-la-regex.py [--sonda <sqlp.exe>]

`--sonda`: un binario que lee frases JSON por stdin y contesta `OK\\t<nombres>` o
`ERR\\t<motivo>` con sqlparser 0.63 (dialecto DuckDB, sin default-features, como
en ore-core). Sin el, se imprime lo anotado de la corrida del 2026-09-24.
"""
import json
import os
import re
import subprocess
import sys

# La regex que los tres SDK usaban hasta que `sql()` paso a `POST /puestos/{id}/sql`
# (sin regex): se deja aqui, tal cual era, para que la medida siga comparando lo mismo.
REGEX_DE_ANTES = r"(?i)\b(?:from|join)\s+([a-z_][a-z0-9_]*)\.([a-z_][a-z0-9_]*)\b"

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)

# (frase, nombres del arbol que lee de verdad)
CORPUS = [
    ("select * exclude (a) from hr.t", "hr.t"),
    ("select * replace (a + 1 as a) from hr.t", "hr.t"),
    ("select columns('^a') from hr.t", "hr.t"),
    ("select min(columns(*)) from hr.t", "hr.t"),
    ("from hr.t", "hr.t"),
    ("from hr.t select a", "hr.t"),
    ("select a, count(*) from hr.t group by all", "hr.t"),
    ("select a from hr.t order by all", "hr.t"),
    ("select * from hr.t qualify row_number() over (partition by a order by b) = 1", "hr.t"),
    ("pivot hr.t on pais using sum(total)", "hr.t"),
    ("unpivot hr.t on a, b into name k value v", "hr.t"),
    ("select list_transform([1, 2, 3], x -> x + 1)", ""),
    ("select [x * 2 for x in [1, 2, 3]]", ""),
    ("select {'a': 1, 'b': 2} as s", ""),
    ("select s.a from (select {'a': 1} as s)", ""),
    ("select * from hr.a positional join hr.b", "hr.a,hr.b"),
    ("select * from hr.a asof join hr.b on hr.a.t >= hr.b.t", "hr.a,hr.b"),
    ("select * from hr.a anti join hr.b using (id)", "hr.a,hr.b"),
    ("select * from hr.a semi join hr.b using (id)", "hr.a,hr.b"),
    ("summarize hr.t", "hr.t"),
    ("describe hr.t", "hr.t"),
    ("select * from hr.t using sample 10%", "hr.t"),
    ("select * from hr.t tablesample 10 percent", "hr.t"),
    ("with recursive r(n) as (select 1 union all select n + 1 from r where n < 3) select * from r", ""),
    ("select a::varchar from hr.t", "hr.t"),
    ("select 'x' || a from hr.t", "hr.t"),
    ("select * from hr.t where a ilike '%x%'", "hr.t"),
    ("select * from hr.t where a similar to 'x.*'", "hr.t"),
    ("select struct_pack(a := 1)", ""),
    ("select a from hr.t limit 10 offset 5", "hr.t"),
    ("select * from hr.a union by name select * from hr.b", "hr.a,hr.b"),
    ("select arg_max(a, b) from hr.t", "hr.t"),
    ("select date_trunc('month', cuando) from hr.t", "hr.t"),
    ("select * from hr.t where cuando > now() - interval 1 day", "hr.t"),
    ("select a, sum(b) filter (where c > 0) from hr.t group by a", "hr.t"),
    ("select * from hr.t order by a nulls last", "hr.t"),
    ("select distinct on (a) * from hr.t", "hr.t"),
    ("select greatest(a, b) from hr.t", "hr.t"),
    ("select * from hr.t, lateral (select a + 1 as a1)", "hr.t"),
    ("select * from hr.t where a in (select a from hr.a)", "hr.t,hr.a"),
    ("select * from read_parquet('x.parquet')", ""),
    ("create or replace table hr.x as select * from hr.t", "hr.t"),
    ("insert into hr.x by name select * from hr.t", "hr.t"),
    ("set threads = 4", ""),
    ("select a from hr.t; select b from hr.a", "hr.t,hr.a"),
    # los de `medida-el-sql-del-arbol.py` §2, donde la regex falla
    ("select * from hr.a, hr.b where a.id = b.id", "hr.a,hr.b"),
    ("-- antes: from viejo.tabla" + chr(10) + "select * from hr.a", "hr.a"),
    ("select 'from hr.b' as s from hr.a", "hr.a"),
    ('select * from "hr"."t"', "hr.t"),
    ("select * from lago.hr.t", ""),
    ("with t as (select * from hr.a) select * from t join hr.b using (id)", "hr.a,hr.b"),
    ("select * from hr.a, unnest([1, 2]) as u(x)", "hr.a"),
]

# Lo que dio la sonda (sqlparser 0.63, DuckDbDialect) el 2026-09-24; se reescribe
# al correr con --sonda.
ANOTADO = {}


def verdad_duckdb():
    """¿La acepta DuckDB? Con tablas vacias `hr.t`, `hr.a`, `hr.b`, `hr.x`."""
    import duckdb

    out = []
    for q, _ in CORPUS:
        con = duckdb.connect()
        con.execute("create schema hr")
        for t in ("t", "a", "b", "x"):
            con.execute("create table hr.%s (id int, a varchar, b int, c int, t timestamp, pais varchar, total decimal(18,2), cuando timestamp)" % t)
        try:
            for s in [x for x in q.split(";") if x.strip()]:
                con.execute(s)
            out.append(True)
        except Exception as e:
            out.append("✗ " + str(e).splitlines()[0][:70])
        con.close()
    return out


def sonda(exe):
    p = subprocess.run([exe], input="\n".join(json.dumps(q) for q, _ in CORPUS) + "\n",
                       capture_output=True, text=True, encoding="utf-8")
    return [l.split("\t", 1) for l in p.stdout.splitlines()]


def main():
    print("=== el terreno de quitar la regex")
    print()
    print("  §1 LAS VIEWS POR ATTACH (medida-las-vistas-por-el-catalogo.py, 2026-09-24)")
    print("     duckdb 1.5.4 (Python) y @duckdb/node-api 1.5.5 (Node): leen la TABLA por el catalogo")
    print("     y NUNCA piden /views —ni anunciandolas en /v1/config, ni con dialecto duckdb,")
    print("     spark o trino—; `ore.hr.ventasES` es «Table … does not exist». La JVM (JDBC 1.5.5)")
    print("     es el mismo motor que Node: deducido, no medido.")
    print("     ⇒ por ATTACH solo llegan tablas: una View necesita que alguien sepa que el")
    print("       texto la nombra. La ruta (A) sola no cubre lo que hoy funciona.")

    print()
    print("  §2 EL TECHO DE (B): %d frases (la sintaxis propia de DuckDB y los casos de la regex)" % len(CORPUS))
    sdk = open(os.path.join(RAIZ, "puesto", "python", "ore", "__init__.py"), encoding="utf-8").read()
    regex = re.compile(REGEX_DE_ANTES)
    duck = verdad_duckdb()
    exe = sys.argv[sys.argv.index("--sonda") + 1] if "--sonda" in sys.argv else None
    sp = sonda(exe) if exe else [ANOTADO.get(q, ["?", ""]) for q, _ in CORPUS]
    acepta = analiza = nombres_bien = regex_bien = 0
    malos = []
    for (q, esperado), d, (estado, resto) in zip(CORPUS, duck, sp):
        esp = sorted(x for x in esperado.split(",") if x)
        r = sorted(set(".".join(m) for m in regex.findall(q)) - {"hr.x"})
        acepta += d is True
        regex_bien += r == esp
        if estado == "OK":
            analiza += 1
            got = sorted(x for x in resto.split(",") if x)
            if got == esp:
                nombres_bien += 1
            else:
                malos.append((q, "nombres %s, esperados %s" % (got, esp)))
        else:
            malos.append((q, "NO ANALIZA · " + resto[:90]))
    print("     DuckDB las acepta: %d de %d%s" % (acepta, len(CORPUS),
          "" if acepta == len(CORPUS) else "  (" + "; ".join("%s → %s" % (q[:30], d) for (q, _), d in zip(CORPUS, duck) if d is not True) + ")"))
    print("     sqlparser (DuckDB) las analiza: %d de %d · saca bien los nombres: %d de %d" % (analiza, len(CORPUS), nombres_bien, len(CORPUS)))
    print("     la regex saca bien los nombres: %d de %d" % (regex_bien, len(CORPUS)))
    print("     lo que sqlparser no da:")
    for q, m in malos:
        print("       · %-58s %s" % (q[:58], m))
    # (B') el TOKENIZADOR de sqlparser + el arbol como filtro: cada `a.b` fuera de
    # comentarios y cadenas, y solo si `a.b` es un dataset o una View del arbol.
    arbol = {"hr.t", "hr.a", "hr.b"}
    if exe:
        p = subprocess.run([exe, "--tokens"], input="\n".join(json.dumps(q) for q, _ in CORPUS) + "\n",
                           capture_output=True, text=True, encoding="utf-8")
        tk = [l.split("\t", 1) for l in p.stdout.splitlines()]
    else:
        tk = [["?", ""]] * len(CORPUS)
        print("     (sin --sonda; la corrida del 2026-09-24 dio: sqlparser analiza 38 de 45 y el")
        print("      tokenizador + el arbol acierta 45 de 45, y 52 de 52 con los casos de la regex)")
    tk_bien, tk_malos = 0, []
    for (q, esperado), (estado, resto) in zip(CORPUS, tk):
        esp = sorted(x for x in esperado.split(",") if x)
        got = sorted(x for x in resto.split(",") if x in arbol) if estado == "OK" else None
        if got == esp:
            tk_bien += 1
        else:
            tk_malos.append((q, got, esp))
    print("     el TOKENIZADOR + el arbol como filtro: bien en %d de %d" % (tk_bien, len(CORPUS)))
    for q, got, esp in tk_malos:
        print("       · %-58s %s, esperados %s" % (q[:58], got, esp))

    print()
    print("  §3 LO QUE NO ES DEL ARBOL")
    q = "create schema tmp; create table tmp.t as select 1 as x; select * from tmp.t"
    print("     `%s`" % q)
    print("     la regex resuelve %s por ore-serve → 404 y la celda muere: un esquema de" % sorted(set(".".join(m) for m in regex.findall(q))))
    print("     la sesion no es un paquete. (B) tiene que resolver solo lo que el ARBOL nombra")
    print("     (el primer trozo es un paquete) y dejar lo demas al motor.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
