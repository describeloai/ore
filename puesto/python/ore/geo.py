"""Geometry helpers (ORE 0067).

`intersects_box` writes the right filter for "what is in this area": measured
over 34 M buildings (ADR 0067 G2·2), `ST_Intersects` alone reads the whole file,
because DuckDB only skips row groups by their bounding boxes with `&&`. The
filter carries both: `&&` to skip, `ST_Intersects` to be exact.
"""

import math

__all__ = ["intersects_box"]


def _columna(c):
    if not (isinstance(c, str) and c):
        raise TypeError("intersects_box(): the column is a name, like `geom` or `t.geom`")
    return ".".join('"%s"' % p.replace('"', '""') for p in c.split("."))


def intersects_box(column, xmin, ymin, xmax, ymax):
    """The SQL filter for the rows of `column` that touch the box, in the
    column's CRS: `ore.sql(f"select * from x where {intersects_box('geom', -3.75, 40.38, -3.65, 40.46)}")`."""
    caja = []
    for nombre, v in (("xmin", xmin), ("ymin", ymin), ("xmax", xmax), ("ymax", ymax)):
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
            raise TypeError("intersects_box(): `%s` is a finite number, not %r" % (nombre, v))
        caja.append(repr(float(v)))
    if float(xmin) > float(xmax) or float(ymin) > float(ymax):
        raise ValueError("intersects_box(): the box is (xmin, ymin, xmax, ymax), and here min > max")
    c = _columna(column)
    sobre = "ST_MakeEnvelope(%s)" % ", ".join(caja)
    return "(%s && %s AND ST_Intersects(%s, %s))" % (c, sobre, c, sobre)
