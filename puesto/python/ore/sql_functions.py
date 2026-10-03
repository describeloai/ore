"""Tree functions called from SQL (ORE 0049 B7·2).

ore-serve rewrites each call to a tree `Function` in a query (`sql_calls` in
ore-core): `db.schema.f(x)` becomes `__ore_fn_1(x)`, and one read as rows
(`from`, `join`, `lateral`) becomes `(select unnest(__ore_fn_1(x), max_depth :=
2))`. Here each one is registered in DuckDB under its internal name, typed by
its document —the parameters in the order its `def` declares them, the types
the document says— and called **with its contract**, like `get_function()`:
what works in Python works in SQL.

A function read as rows always gives DuckDB a list of rows: a list of
structs as it is; one struct, a list of one; anything else, rows of one column,
`value`.
"""

import dataclasses
import inspect
import re

__all__ = ["duckdb_type", "register"]

_SIMPLES = {"Integer": "BIGINT", "Float": "DOUBLE", "String": "VARCHAR", "Boolean": "BOOLEAN",
            "Date": "DATE", "DateTime": "TIMESTAMP", "DateTimeTz": "TIMESTAMPTZ", "Time": "TIME",
            "Decimal": "DECIMAL(38, 18)", "Opaque": "BLOB"}


def _partir(s):
    """`a, b<c, d>, e` → `["a", "b<c, d>", "e"]`: commas at depth 0."""
    out, d, ini = [], 0, 0
    for i, ch in enumerate(s):
        if ch == "<":
            d += 1
        elif ch == ">":
            d -= 1
        elif ch == "," and d == 0:
            out.append(s[ini:i].strip())
            ini = i + 1
    if s[ini:].strip():
        out.append(s[ini:].strip())
    return out


def _q(n):
    return '"%s"' % str(n).replace('"', '""')


def duckdb_type(t):
    """The DuckDB type of an OOS type as a `Function` document writes it."""
    from .medios import _CAMPOS_ITEM

    t = (t or "").strip()
    if t in _SIMPLES:
        return _SIMPLES[t]
    m = re.match(r"^(\w+)<(.*)>$", t, re.S)
    if not m:
        raise TypeError("SQL cannot type `%s`" % t)
    ctor, dentro = m.group(1), m.group(2)
    if ctor == "list":
        return duckdb_type(dentro) + "[]"
    if ctor == "Decimal":
        p, s = _partir(dentro)
        return "DECIMAL(%d, %d)" % (int(p), int(s))
    if ctor in ("Money", "Quantity"):
        _, s = _partir(dentro)
        return "DECIMAL(38, %d)" % min(int(s), 18)
    if ctor == "Struct":
        campos = []
        for c in _partir(dentro):
            n, _, tc = c.partition(":")
            campos.append("%s %s" % (_q(n.strip()), duckdb_type(tc)))
        return "STRUCT(%s)" % ", ".join(campos)
    if ctor == "Media":
        return "STRUCT(%s)" % ", ".join("%s %s" % (_q(c), "BIGINT" if c == "size" else "VARCHAR")
                                         for c in _CAMPOS_ITEM)
    raise TypeError("SQL cannot type `%s`" % t)


def _tipo_de_salida(spec):
    """The OOS type of what a function returns: `output: {type: T}` is a value;
    `output: {campo: {type: …}}` is one struct."""
    out = spec.get("output") or {}
    if isinstance(out.get("type"), str):
        return out["type"]
    campos = ["%s: %s" % (n, (c or {}).get("type", "String")) for n, c in out.items()]
    return "Struct<%s>" % ", ".join(campos)


def _a_duckdb(v):
    """What a function returns, as DuckDB takes it: a dataclass or a `MediaRef`
    is a dict, recursively."""
    from .medios import MediaRef, _CAMPOS_ITEM

    if dataclasses.is_dataclass(v) and not isinstance(v, type):
        return {f.name: _a_duckdb(getattr(v, f.name)) for f in dataclasses.fields(v)}
    if isinstance(v, MediaRef):
        return {c: getattr(v, c, None) for c in _CAMPOS_ITEM}
    if isinstance(v, dict):
        return {k: _a_duckdb(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_a_duckdb(x) for x in v]
    return v


def _como_filas(tipo):
    """A function read as rows: its type as a list of structs, and how to
    turn what it returns into one."""
    if tipo.startswith("list<Struct<"):
        return tipo, lambda v: v or []
    if tipo.startswith("Struct<"):
        return "list<%s>" % tipo, lambda v: [] if v is None else [v]
    if tipo.startswith("list<"):
        return "list<Struct<value: %s>>" % tipo[5:-1], lambda v: [{"value": x} for x in (v or [])]
    return "list<Struct<value: %s>>" % tipo, lambda v: [] if v is None else [{"value": v}]


def _con_aridad(n, g):
    """`g` behind a `def` of exactly `n` parameters: DuckDB counts them from the
    signature, and a closure's keyword defaults would count too."""
    nombres = ", ".join("a%d" % i for i in range(n))
    ns = {"g": g}
    exec("def llamar(%s):\n    return g(%s)\n" % (nombres, nombres), ns)
    return ns["llamar"]


def register(con, calls, resolve):
    """Register each call of `calls` (what ore-serve sent: `name`, `internal`,
    `arity`, `table`) in the DuckDB connection `con`. `resolve(name)` gives
    the function, with its contract, and the `spec` of its document."""
    for c in calls:
        f, spec = resolve(c["name"])
        entrada = spec.get("input") or {}
        firma = [p for p in inspect.signature(f).parameters.values()
                 if p.kind in (p.POSITIONAL_ONLY, p.POSITIONAL_OR_KEYWORD)]
        n = int(c.get("arity") or 0)
        if n > len(firma):
            raise TypeError("`%s` takes %d arguments and SQL gives it %d" % (c["name"], len(firma), n))
        faltan = [p.name for p in firma[n:] if p.default is inspect.Parameter.empty]
        if faltan:
            raise TypeError("`%s` needs `%s` and SQL gives it %d arguments"
                            % (c["name"], "`, `".join(faltan), n))
        params = [duckdb_type((entrada.get(p.name) or {}).get("type", "String")) for p in firma[:n]]
        tipo = _tipo_de_salida(spec)
        filas = None
        if c.get("table"):
            tipo, filas = _como_filas(tipo)

        def uno(*args, _f=f, _filas=filas):
            r = _a_duckdb(_f(*args))
            return _filas(r) if _filas else r

        llamar = _con_aridad(n, uno)

        try:
            con.remove_function(c["internal"])
        except Exception:  # noqa: BLE001 — not registered yet
            pass
        con.create_function(c["internal"], llamar, params, duckdb_type(tipo))

