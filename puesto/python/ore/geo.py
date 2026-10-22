"""Geometry helpers (ORE 0067).

`intersects_box` writes the right filter for "what is in this area". Over the
lake, DuckDB can only skip row groups by the column's covering, `<col>_bbox`
(ADR 0072 F1·0: a city tile over 20 M points, ~660 ms without it, ~126 ms with
it), so the filter carries three parts: the covering to skip, `&&` and
`ST_Intersects` to be exact. Every geo column read from the lake has its
`<col>_bbox`; on your own relations, pass `covering=False`.
"""

import math

__all__ = ["intersects_box"]


def _columna(c):
    if not (isinstance(c, str) and c):
        raise TypeError("intersects_box(): the column is a name, like `geom` or `t.geom`")
    return ".".join('"%s"' % p.replace('"', '""') for p in c.split("."))


def intersects_box(column, xmin, ymin, xmax, ymax, covering=True):
    """The SQL filter for the rows of `column` that touch the box, in the
    column's CRS: `ore.sql(f"select * from x where {intersects_box('geom', -3.75, 40.38, -3.65, 40.46)}")`.
    With `covering` (the default) it also filters by `<column>_bbox`, which is
    what lets DuckDB skip what is far away; `covering=False` for a relation
    that has no `<column>_bbox`."""
    caja = []
    for nombre, v in (("xmin", xmin), ("ymin", ymin), ("xmax", xmax), ("ymax", ymax)):
        if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
            raise TypeError("intersects_box(): `%s` is a finite number, not %r" % (nombre, v))
        caja.append(repr(float(v)))
    if float(xmin) > float(xmax) or float(ymin) > float(ymax):
        raise ValueError("intersects_box(): the box is (xmin, ymin, xmax, ymax), and here min > max")
    c = _columna(column)
    sobre = "ST_MakeEnvelope(%s)" % ", ".join(caja)
    exacto = "%s && %s AND ST_Intersects(%s, %s)" % (c, sobre, c, sobre)
    if not covering:
        return "(%s)" % exacto
    b = _columna(column + "_bbox")
    x0, y0, x1, y1 = caja
    return "(%s.xmin <= %s AND %s.xmax >= %s AND %s.ymin <= %s AND %s.ymax >= %s AND %s)" % (
        b, x1, b, x0, b, y1, b, y0, exacto)
