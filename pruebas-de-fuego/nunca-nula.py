#!/usr/bin/env python3
"""
NUNCA NULA (ORE 0051 P4): la propiedad que la derivación no puede romper.

    lo que `ore view` dice «nunca nula» NUNCA sale nulo al ejecutar la consulta

Una regla de más miente —un motor se apoya en la marca y da cifras falsas, como
midió E6 con Spark—; una de menos sólo deja de decir algo. Así que esto no mide
cuánto deriva, sino que no derive de más, sobre consultas que nadie escribió a
mano:

  · tablas al azar, con columnas `required` (v1alpha22) y datos que lo cumplen:
    nulos en las demás, y alguna tabla vacía (un agregado sobre nada);
  · vistas SQL al azar que mezclan lo que la derivación trata: lectura tal cual,
    renombre, literales, NULL, `COALESCE`, `CASE` con y sin `ELSE`, funciones,
    aritmética y las tres divisiones (`//` y `%` dan nulo por cero), `COUNT`/`SUM`/`MAX` con `GROUP BY`, los cuatro
    `JOIN`, `WHERE` con `IS NOT NULL`, comparaciones y `OR`, `UNION ALL`;
  · `ore view` dice qué columnas de cada vista nunca son nulas;
  · DuckDB ejecuta cada consulta sobre los mismos datos, y cada columna «nunca
    nula» se cuenta: un solo nulo es un fallo, con la consulta.

Uso (necesita el módulo `duckdb` y un `ore` de este commit):

    ORE=/ruta/a/ore python3 pruebas-de-fuego/nunca-nula.py [--semilla 7] [--vistas 300]

En esta máquina, en Docker (como se compila ORE):

    docker run --rm -v <este repo>:/src -v <target>:/target python:3.14 sh -c \
      'pip install -q duckdb && ORE=/target/debug/ore python3 /src/pruebas-de-fuego/nunca-nula.py'
"""

import argparse
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

import duckdb

P = "p"  # el paquete
TIPOS = {"Integer": "BIGINT", "String": "VARCHAR"}


def tablas_al_azar(rnd):
    tablas = {}
    for t in range(rnd.randint(2, 3)):
        nombre = f"t{t}"
        cols = [("id", "Integer", True)]
        for c in range(rnd.randint(2, 4)):
            cols.append((f"c{c}", rnd.choice(["Integer", "String"]), rnd.random() < 0.5))
        filas = [] if rnd.random() < 0.15 else [
            [None if (not req and rnd.random() < 0.3) else valor(rnd, tipo, i)
             for (_, tipo, req) in cols]
            for i in range(rnd.randint(1, 25))
        ]
        tablas[nombre] = (cols, filas)
    return tablas


def valor(rnd, tipo, i):
    if tipo == "Integer":
        return rnd.choice([0, 1, 2, 3, 5, 10, -1, i])
    return rnd.choice(["a", "b", "zz", "", f"v{i}"])


def q(n):
    return '"' + n + '"'


def ref(t):
    return f'{q(P)}."default".{q(t)}'


def expresion(rnd, cols, alias, agregado):
    """Una expresión sobre `cols` (lista de (alias, col, tipo))."""
    a, c, tipo = rnd.choice(cols)
    col = f"{alias(a)}{q(c)}"
    ints = [x for x in cols if x[2] == "Integer"]
    strs = [x for x in cols if x[2] == "String"]
    otra = rnd.choice(cols)
    col2 = f"{alias(otra[0])}{q(otra[1])}"
    opciones = [
        lambda: col,
        lambda: "1",
        lambda: "'k'",
        lambda: "NULL",
        lambda: f"coalesce({col}, {col2})",
        lambda: f"coalesce({col}, {'0' if tipo == 'Integer' else repr('x')})",
        lambda: f"CASE WHEN {col} IS NULL THEN {col2} END",
        lambda: f"CASE WHEN {col} IS NULL THEN {col2} ELSE {col} END",
        lambda: f"{col} IS NULL",
    ]
    if strs:
        s = rnd.choice(strs)
        opciones.append(lambda: f"upper({alias(s[0])}{q(s[1])})")
        opciones.append(lambda: f"nullif({alias(s[0])}{q(s[1])}, 'a')")
    if ints:
        i1, i2 = rnd.choice(ints), rnd.choice(ints)
        e1, e2 = f"{alias(i1[0])}{q(i1[1])}", f"{alias(i2[0])}{q(i2[1])}"
        opciones += [lambda: f"{e1} + {e2}", lambda: f"{e1} / {e2}", lambda: f"{e1} // {e2}",
                     lambda: f"{e1} % {e2}", lambda: f"cast({e1} as varchar)"]
    if agregado:
        opciones = [lambda: "count(*)", lambda: f"count({col})", lambda: f"max({col})",
                    lambda: f"min({col})", lambda: f"coalesce(max({col}), {'0' if tipo == 'Integer' else repr('x')})"]
        if ints:
            i1 = rnd.choice(ints)
            opciones.append(lambda: f"sum({alias(i1[0])}{q(i1[1])})")
    return rnd.choice(opciones)()


def consulta_al_azar(rnd, tablas):
    nombres = list(tablas)
    t1 = rnd.choice(nombres)
    unir = rnd.random() < 0.5 and len(nombres) > 1
    if unir:
        t2 = rnd.choice([n for n in nombres if n != t1])
        tipo = rnd.choice(["JOIN", "LEFT JOIN", "RIGHT JOIN", "FULL JOIN"])
        desde = f"FROM {ref(t1)} a {tipo} {ref(t2)} b ON a.\"id\" = b.\"id\""
        cols = [("a", c, ty) for (c, ty, _) in tablas[t1][0]] + [("b", c, ty) for (c, ty, _) in tablas[t2][0]]
        alias = lambda x: f"{x}."
    else:
        desde = f"FROM {ref(t1)}"
        cols = [("", c, ty) for (c, ty, _) in tablas[t1][0]]
        alias = lambda x: ""
    agrupa = rnd.random() < 0.25
    items = []
    k = None
    if agrupa:
        clave = rnd.choice(cols)
        k = f"{alias(clave[0])}{q(clave[1])}"
        items.append(f"{k} AS k")
        for i in range(rnd.randint(1, 3)):
            items.append(f"{expresion(rnd, cols, alias, True)} AS x{i}")
    else:
        for i in range(rnd.randint(2, 5)):
            items.append(f"{expresion(rnd, cols, alias, False)} AS x{i}")
    donde = ""
    r = rnd.random()
    if r < 0.4:
        a, c, _ = rnd.choice(cols)
        donde = f" WHERE {alias(a)}{q(c)} IS NOT NULL"
    elif r < 0.55:
        ints = [x for x in cols if x[2] == "Integer"]
        if ints:
            a, c, _ = rnd.choice(ints)
            donde = f" WHERE {alias(a)}{q(c)} > 0"
    elif r < 0.65:
        (a1, c1, _), (a2, c2, _) = rnd.choice(cols), rnd.choice(cols)
        donde = f" WHERE {alias(a1)}{q(c1)} IS NOT NULL OR {alias(a2)}{q(c2)} IS NULL"
    sql = f"SELECT {', '.join(items)} {desde}{donde}"
    if agrupa:
        sql += f" GROUP BY {k}"
    elif rnd.random() < 0.15:
        # El mismo número de columnas, del mismo tipo que no se sabe: texto.
        n = len(items)
        sql = (f"SELECT {', '.join(f'cast(x{i} as varchar) AS x{i}' for i in range(n))} FROM ({sql}) "
               f"UNION ALL SELECT {', '.join(rnd.choice(['NULL', repr('u'), 'cast(1 as varchar)']) for _ in range(n))}")
    salida = ["k"] + [f"x{i}" for i in range(len(items) - 1)] if agrupa else [f"x{i}" for i in range(len(items))]
    return sql, salida


def escribir_arbol(dir_, tablas, vistas):
    def w(ruta, texto):
        p = os.path.join(dir_, ruta)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8", newline="\n") as f:
            f.write(texto)

    w("ontology.config.yaml",
      "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: nunca-nula, version: 0.1.0 }\n"
      "datasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n")
    w(f"packages/{P}/package.yaml",
      f"apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {P}, version: 0.1.0, status: active, domain: d }}\n"
      "spec: { owner: team:datos }\n")
    for t, (cols, _) in tablas.items():
        lineas = "".join(
            f"    {c}: {{ type: {ty}{', required: true' if req else ''} }}\n" for (c, ty, req) in cols)
        w(f"packages/{P}/tables/{t}.yaml",
          f"apiVersion: oos.dev/v1alpha22\nkind: Table\nmetadata: {{ name: {t}, namespace: {P} }}\n"
          f"spec:\n  datasource: pg\n  object: \"public.{t}\"\n  columns:\n{lineas}"
          "  reads: {}\n  changes: { mode: none, witness: none }\n")
    for i, (sql, salida) in enumerate(vistas):
        cuerpo = "\n".join("    " + l for l in sql.splitlines())
        contrato = "".join(f"    {c}: {{ type: String }}\n" for c in salida)
        w(f"packages/{P}/views/v{i}.yaml",
          f"apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: {{ name: v{i}, namespace: {P} }}\n"
          f"spec:\n  owner: team:datos\n  dialect: duckdb\n  sql: |\n{cuerpo}\n  columns:\n{contrato}")


def lo_que_dice_ore(ore, dir_):
    s = subprocess.run([ore, "view", "."], cwd=dir_, capture_output=True, text=True)
    dice, actual = {}, None
    for linea in (s.stdout + s.stderr).splitlines():
        m = re.match(rf"^{P}\.(?:default\.)?(v\d+)\s*$", linea)
        if m:
            actual = m.group(1)
            dice.setdefault(actual, set())
            continue
        m = re.match(r"^\s+nunca nula\s+(.*)$", linea)
        if m and actual:
            dice[actual] = {c.strip() for c in m.group(1).split(",")}
    return dice, s.stdout + s.stderr


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--semilla", type=int, default=7)
    ap.add_argument("--vistas", type=int, default=300)
    ap.add_argument("--rondas", type=int, default=5)
    args = ap.parse_args()
    ore = os.environ.get("ORE", "ore")
    fallos, afirmadas, nulas_de_verdad, sin_decir, descartadas, vistas_total = [], 0, 0, 0, 0, 0
    for ronda in range(args.rondas):
        rnd = random.Random(args.semilla * 1000 + ronda)
        tablas = tablas_al_azar(rnd)
        vistas = [consulta_al_azar(rnd, tablas) for _ in range(args.vistas // args.rondas)]
        dir_ = tempfile.mkdtemp(prefix="nunca-nula-")
        try:
            escribir_arbol(dir_, tablas, vistas)
            dice, salida = lo_que_dice_ore(ore, dir_)
            if not dice:
                print(salida[-2000:])
                sys.exit(f"ronda {ronda}: `ore view` no dijo nada de ninguna vista")
            con = duckdb.connect()
            con.execute(f"ATTACH ':memory:' AS {q(P)}")
            con.execute(f'CREATE SCHEMA {q(P)}."default"')
            for t, (cols, filas) in tablas.items():
                con.execute(f"CREATE TABLE {ref(t)} ({', '.join(f'{q(c)} {TIPOS[ty]}' for (c, ty, _) in cols)})")
                for f in filas:
                    con.execute(f"INSERT INTO {ref(t)} VALUES ({', '.join('?' for _ in f)})", f)
            for i, (sql, salida_cols) in enumerate(vistas):
                nunca = dice.get(f"v{i}", set())
                try:
                    filas = con.execute(sql).fetchall()
                except duckdb.Error:
                    descartadas += 1  # una consulta que DuckDB no acepta no prueba nada
                    continue
                vistas_total += 1
                for j, c in enumerate(salida_cols):
                    nulos = sum(1 for f in filas if f[j] is None)
                    if c in nunca:
                        afirmadas += 1
                        if nulos:
                            fallos.append((ronda, i, c, nulos, sql))
                    elif filas and not nulos:
                        sin_decir += 1
                    if nulos:
                        nulas_de_verdad += 1
        finally:
            shutil.rmtree(dir_, ignore_errors=True)
    print(f"vistas ejecutadas: {vistas_total} (descartadas por DuckDB: {descartadas})")
    print(f"«nunca nula» afirmadas: {afirmadas} · columnas con algún nulo de verdad: {nulas_de_verdad} "
          f"· sin nulos y sin afirmar (lo conservador): {sin_decir}")
    if fallos:
        for (ronda, i, c, nulos, sql) in fallos[:20]:
            print(f"✗ ronda {ronda} · v{i}.{c}: {nulos} nulo(s) y se dijo «nunca nula»\n    {sql}")
        sys.exit(f"{len(fallos)} fallo(s)")
    print("✓ ninguna columna «nunca nula» sale nula")


if __name__ == "__main__":
    main()
