# Preview P1c · EL PREVIEW DE UN ACTIVO ES SU SELECT — una celda de un puesto
# Python en `main` (victor). Cada página de `GET /preview/{kind}/{b}/{s}/{n}`
# tiene que ser lo que `ore.sql("select * from b.s.n limit L offset D")` da en
# el mismo puesto: las mismas columnas en el mismo orden, el mismo número de
# filas y los mismos valores (comparados por lo que valen: `10.50` y
# `Decimal('10.5')` son el mismo número, `2026-09-26` y un `Timestamp` el
# mismo día). El tipo OOS de cada columna se enseña al lado del de pandas.
#
# Una Table de BigQuery puede devolver las filas en otro orden en cada
# lectura (Storage Read en varios flujos, sin `order by`): ahí se compara el
# conjunto y se dice si el orden coincide. Lo demás, fila a fila.
import datetime
import decimal
import math
import time

import pandas as pd

import ore

# (kind, base, schema, nombre, [desde…], limite) — los activos de victor.
ACTIVOS = [
    ("dataset", "bq", "ventas", "ore_e2e_sintetica", [0, 1_500_000], 100),
    ("dataset", "standard_test", "public", "prueba_de_pk", [0, 900], 100),
    ("dataset", "s3_pedidos", "nueva_carpeta", "pedidos", [0, 1400], 100),
    ("dataset", "standard_test", "public", "brain_items", [0], 100),
    ("table", "postgresql_20260918_1920", "olist", "orders", [0, 500], 100),
    ("table", "bigquery_20260927_1428", "ventas", "clientes", [0], 100),
]


def preview(kind, b, s, n, desde, limite, snapshot=None):
    ruta = f"/preview/{kind}/{b}/{s}/{n}?desde={desde}&limite={limite}"
    if snapshot:
        ruta += f"&snapshot={snapshot}"
    t = time.time()
    codigo, d = ore.session.pedir("GET", ruta, plazo=120)
    return codigo, d, time.time() - t


def texto(v):
    """Un valor de pandas como lo escribiría el preview, o None si es nulo."""
    if v is None or v is pd.NaT:
        return None
    if isinstance(v, float) and math.isnan(v):
        return None
    if isinstance(v, bool):
        return "true" if v else "false"
    return v


def iguales(a, b):
    """`a` del preview (texto) y `b` de sql() (lo que pandas trae)."""
    if a is None or b is None:
        return a is None and b is None
    if isinstance(b, str):
        return a == b
    if isinstance(b, (int, decimal.Decimal)) and not isinstance(b, bool):
        try:
            return decimal.Decimal(a) == decimal.Decimal(b)
        except decimal.InvalidOperation:
            return False
    if isinstance(b, float):
        try:
            return math.isclose(float(a), b, rel_tol=1e-9, abs_tol=1e-12)
        except ValueError:
            return False
    if isinstance(b, (pd.Timestamp, datetime.datetime, datetime.date, datetime.time)):
        try:
            x, y = pd.Timestamp(a), pd.Timestamp(b)
            if (x.tzinfo is None) != (y.tzinfo is None):
                x, y = x.tz_localize(None) if x.tzinfo else x, y.tz_localize(None) if y.tzinfo else y
            return x == y
        except (ValueError, TypeError):
            return str(a) == str(b)
    return str(a) == str(b)


def fila_de(df, i):
    return {c: texto(df.iloc[i][c]) for c in df.columns}


malos = 0
for kind, b, s, n, desdes, limite in ACTIVOS:
    nombre = f"{b}.{s}.{n}"
    snapshot = None
    for desde in desdes:
        codigo, d, t_prev = preview(kind, b, s, n, desde, limite, snapshot)
        if codigo != 200:
            print(f"✗ {kind} {nombre} desde {desde}: {codigo} {d}")
            malos += 1
            continue
        snapshot = snapshot or d.get("snapshot")
        t = time.time()
        try:
            df = ore.sql(d["sql"])
        except Exception as e:
            print(f"✗ {kind} {nombre} desde {desde}: sql() falló: {type(e).__name__}: {str(e)[:200]}")
            malos += 1
            continue
        t_sql = time.time() - t
        cols = [c["nombre"] for c in d["columnas"]]
        tipos = {c["nombre"]: c["tipo"] for c in d["columnas"]}
        problemas = []
        if cols != list(df.columns):
            problemas.append(f"columnas {cols} ≠ {list(df.columns)}")
        if len(d["filas"]) != len(df):
            problemas.append(f"{len(d['filas'])} filas ≠ {len(df)}")
        orden = True
        if not problemas:
            de_sql = [fila_de(df, i) for i in range(len(df))]
            mal = [
                (i, c, d["filas"][i].get(c), de_sql[i][c])
                for i in range(len(df))
                for c in cols
                if not iguales(d["filas"][i].get(c), de_sql[i][c])
            ]
            if mal:
                # ¿El mismo conjunto en otro orden? (BigQuery sin `order by`)
                clave = lambda f: tuple(str(f.get(c)) for c in cols)
                orden = False
                restantes = list(de_sql)
                for f in d["filas"]:
                    j = next(
                        (j for j, g in enumerate(restantes) if all(iguales(f.get(c), g[c]) for c in cols)),
                        None,
                    )
                    if j is None:
                        problemas.append(f"fila {clave(f)[:4]}… del preview no está en sql(); p.ej. {mal[0]}")
                        break
                    restantes.pop(j)
        estado = "✓" if not problemas else "✗"
        malos += bool(problemas)
        extra = "" if orden else " · mismo conjunto, otro orden"
        print(
            f"{estado} {kind} {nombre} desde {desde}: {len(d['filas'])} filas · "
            f"preview {t_prev:.2f}s ({d.get('origen')}, {d.get('bytes', '-')} B) · sql() {t_sql:.2f}s{extra}"
        )
        for p in problemas:
            print(f"    {p}")
        if desde == desdes[0]:
            print("    tipos: " + ", ".join(f"{c} {tipos[c]}/{df[c].dtype}" for c in cols if c in df.columns))

print("✓ el preview es su select" if malos == 0 else f"✗ {malos} páginas no casan")
