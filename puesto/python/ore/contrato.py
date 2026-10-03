"""A function's contract (ORE 0050 G3): its annotations, enforced.

`llamada(f)` wraps `f` so each parameter is converted to the type it annotates
and the return value is checked against its annotation; a value that does not
fit raises `ContractError` (a `TypeError`). Conversion only goes from what
travels in JSON to what the `def` annotates: `"2026-10-02"` is a `date` and
`12.50` a `Decimal`, but `"3"` is not an `int`. `ErrorDeContrato` is the old
name of `ContractError` (the same class).

---

El contrato de una función (ORE 0050 G3): sus anotaciones, cumplidas.

La firma de un `@function` es su contrato —lo que recibe y lo que devuelve— y
el documento `Function` se deriva de ella (OOS v1alpha18 01 §4). Aquí se hace
cumplir al llamarla: cada parámetro llega **del tipo que anota** y lo que
devuelve **es del tipo que anota**. Es la misma regla en la sesión (el `def`
decorado se llama como cualquier otro, y convierte) y en el arnés que corre
una invocación (`ore-serve`, `funciones.rs`): lo que funciona en tu sesión
funciona invocado.

Medido antes (2026-10-02, t-victor): `importe: Decimal` llegaba como `float`
y `fecha: date` como `str`; la función calculó mal sin ningún error.

Convertir es de lo que viaja en JSON a lo que el `def` anota, y nada más:
`"2026-10-02"` es una `date` y `12.50` un `Decimal`, pero `"3"` no es un
`int` —eso sería adivinar—. Lo que ya es del tipo pasa tal cual. Solo la
biblioteca estándar.

v1alpha20 (G5a): los tipos de `ore.tipos` —`DateTimeTz`, `Money`, `Quantity`,
`Media`, `Annotated[Decimal, Precision(p, s)]`— son un `Annotated` de lo que
Python tiene, y aquí se cumple lo que añaden: un instante lleva zona, un
decimal no pasa de su precisión, una referencia es de su colección. Y `bytes`
viaja en base64.
"""

import base64
import binascii
import dataclasses
import datetime
import decimal
import functools
import inspect
import types
import typing

__all__ = ["ContractError", "llamada", "convertir", "comprobar_salida"]


class ContractError(TypeError):
    """A value that is not of the type the signature annotates."""


def __getattr__(nombre):
    # El nombre de antes: la MISMA clase.
    if nombre == "ErrorDeContrato":
        from . import _avisar

        _avisar("ore.contrato.ErrorDeContrato", "ore.contrato.ContractError")
        return ContractError
    raise AttributeError("module 'ore.contrato' has no attribute %r" % (nombre,))


_NADA = inspect.Parameter.empty


def _nombre(t):
    if typing.get_origin(t) is typing.Annotated:
        base, *metas = typing.get_args(t)
        for m in metas:
            if type(m).__name__ in ("_Unidad", "_DeColeccion"):
                return repr(m)
            if type(m).__name__ == "_ConZona":
                return "DateTimeTz"
            if type(m).__name__ == "Precision":
                return "Decimal<%d, %d>" % (m.precision, m.scale)
        return _nombre(base)
    if t is type(None):
        return "None"
    if typing.get_origin(t) in (typing.Union, types.UnionType):
        return " | ".join(_nombre(a) for a in typing.get_args(t))
    if typing.get_origin(t) in (list, tuple):
        args = typing.get_args(t)
        return "list[%s]" % (_nombre(args[0]) if args else "")
    return getattr(t, "__name__", str(t))


def _corto(v):
    r = repr(v)
    return r if len(r) <= 60 else r[:57] + "..."


def _opcional(t):
    """`X | None` u `Optional[X]` → `X`; si no lo es, `None`."""
    if typing.get_origin(t) in (typing.Union, types.UnionType):
        args = [a for a in typing.get_args(t) if a is not type(None)]
        if len(args) < len(typing.get_args(t)):
            return args[0] if len(args) == 1 else typing.Union[tuple(args)]
    return None


def convertir(que, v, t):
    """`v` as the type `t` that `que` (a parameter, or a field) annotates;
    `ContractError` if it does not fit."""
    if t is _NADA or t is typing.Any or t is object:
        return v
    if typing.get_origin(t) is typing.Annotated:
        return _anotado(que, v, t)
    if v is None:
        if _opcional(t) is not None or t is type(None):
            return None
        raise ContractError("`%s` is `%s` and got None" % (que, _nombre(t)))
    base = _opcional(t)
    if base is not None:
        return convertir(que, v, base)
    origen = typing.get_origin(t)
    if origen in (typing.Union, types.UnionType):
        for a in typing.get_args(t):
            try:
                return convertir(que, v, a)
            except ContractError:
                pass
        raise ContractError("`%s` is `%s` and got %s" % (que, _nombre(t), _corto(v)))
    if origen in (list, tuple) or t in (list, tuple):
        if not isinstance(v, (list, tuple)):
            raise ContractError("`%s` is `%s` and got %s, which is not a list" % (que, _nombre(t), _corto(v)))
        args = typing.get_args(t)
        dentro = args[0] if args else _NADA
        return [convertir("%s[%d]" % (que, i), x, dentro) for i, x in enumerate(v)]
    if origen is not None:  # dict[...] y demás: tal cual
        return v
    mal = lambda porque="": ContractError(  # noqa: E731
        "`%s` is `%s` and got %s%s" % (que, _nombre(t), _corto(v), porque))
    if t is bool:
        if isinstance(v, bool):
            return v
        raise mal()
    if t is int:
        if isinstance(v, bool) or not isinstance(v, (int, decimal.Decimal, float)):
            raise mal()
        if isinstance(v, int):
            return v
        if v == int(v):  # 3.0 o Decimal("3") de un JSON
            return int(v)
        raise mal(", which is not an integer")
    if t is float:
        if isinstance(v, bool) or not isinstance(v, (int, float, decimal.Decimal)):
            raise mal()
        return float(v)
    if t is decimal.Decimal:
        if isinstance(v, bool):
            raise mal()
        if isinstance(v, decimal.Decimal):
            return v
        if isinstance(v, int):
            return decimal.Decimal(v)
        if isinstance(v, float):
            return decimal.Decimal(repr(v))
        if isinstance(v, str):
            try:
                return decimal.Decimal(v.strip())
            except decimal.InvalidOperation:
                raise mal(", which is not a number") from None
        raise mal()
    if t is str:
        if isinstance(v, str):
            return v
        raise mal()
    if t is bytes:
        if isinstance(v, (bytes, bytearray)):
            return bytes(v)
        if isinstance(v, str):  # en JSON, base64 (v1alpha20 `01` §7)
            try:
                return base64.b64decode(v, validate=True)
            except (binascii.Error, ValueError):
                raise mal(", which is not base64") from None
        raise mal()
    if t is datetime.datetime:
        if isinstance(v, datetime.datetime):
            return v
        if isinstance(v, str):
            try:
                return datetime.datetime.fromisoformat(v.replace("Z", "+00:00"))
            except ValueError:
                raise mal(", which is not an ISO 8601 date-time") from None
        raise mal()
    if t is datetime.date:
        if isinstance(v, datetime.date) and not isinstance(v, datetime.datetime):
            return v
        if isinstance(v, str):
            try:
                return datetime.date.fromisoformat(v)
            except ValueError:
                raise mal(", which is not a YYYY-MM-DD date") from None
        raise mal()
    if t is datetime.time:
        if isinstance(v, datetime.time):
            return v
        if isinstance(v, str):
            try:
                return datetime.time.fromisoformat(v)
            except ValueError:
                raise mal(", which is not an ISO 8601 time") from None
        raise mal()
    if dataclasses.is_dataclass(t) and isinstance(t, type):
        if isinstance(v, t):
            return v
        if isinstance(v, dict):
            tipos = _tipos_de(t)
            campos = {c.name for c in dataclasses.fields(t)}
            sobran = sorted(set(v) - campos)
            if sobran:
                raise mal(": `%s` does not declare %s" % (t.__name__, sobran))
            return t(**{k: convertir("%s.%s" % (que, k), x, tipos.get(k, _NADA)) for k, x in v.items()})
        raise mal()
    if isinstance(t, type) and isinstance(v, t):
        return v
    if isinstance(t, type):
        raise mal()
    return v


def _cifras(d):
    """(cifras enteras, decimales) de un `Decimal` finito."""
    signo, digitos, exp = d.normalize().as_tuple() if d != 0 else (0, (0,), 0)
    decimales = max(0, -exp)
    enteras = max(0, len(digitos) + exp) if exp < 0 else len(digitos) + exp
    return enteras, decimales


def _anotado(que, v, t):
    """Un `Annotated` de `ore.tipos` (v1alpha20 `01`): lo de Python, y lo que OOS añade."""
    base, *metas = typing.get_args(t)
    marcas = {type(m).__name__: m for m in metas}
    col = marcas.get("_DeColeccion")
    if col is not None:
        from .medios import MediaRef

        if isinstance(v, dict):
            try:
                v = MediaRef.from_json(v)
            except TypeError as e:
                raise ContractError("`%s` is `%r` and got %s: %s" % (que, col, _corto(v), e)) from None
        if not isinstance(v, MediaRef):
            raise ContractError("`%s` is `%r` and got %s, which is not a reference to an item" % (que, col, _corto(v)))
        if v.collection and v.collection != col.coleccion:
            raise ContractError("`%s` is `%r` and got an item of `%s`" % (que, col, v.collection))
        return v
    try:
        v = convertir(que, v, base)
    except ContractError as e:
        # Con el nombre del tipo que se anotó, no el de su base de Python.
        raise ContractError(str(e).replace("`%s`" % _nombre(base), "`%s`" % _nombre(t), 1)) from None
    if "_ConZona" in marcas and (v.tzinfo is None or v.utcoffset() is None):
        raise ContractError("`%s` is `DateTimeTz` and got %s, without a zone: an instant carries `Z` or `+02:00`"
                            % (que, _corto(v.isoformat())))
    p = marcas.get("Precision")
    if p is not None:
        enteras, decimales = _cifras(v)
        if decimales > p.scale or enteras > p.precision - p.scale:
            raise ContractError("`%s` is `Decimal<%d, %d>` and got %s, which does not fit"
                                % (que, p.precision, p.scale, v))
    u = marcas.get("_Unidad")
    if u is not None and _cifras(v)[1] > u.precision:
        raise ContractError("`%s` is `%r` and got %s: it has more than %d decimals" % (que, u, v, u.precision))
    return v


def _tipos_de(f):
    try:
        # Con los `Annotated`: son lo que `ore.tipos` añade (v1alpha20).
        return typing.get_type_hints(f, include_extras=True)
    except Exception:  # noqa: BLE001 — una anotación que no resuelve: sin contrato para ella
        return {k: v for k, v in getattr(f, "__annotations__", {}).items() if not isinstance(v, str)}


def comprobar_salida(que, v, t):
    """What `que` returned, checked against its annotation `t`. A dataclass is
    checked field by field; anything else with the same rule as a parameter
    (`3` is fine for `float` and `Decimal`, `"3"` is not an `int`)."""
    if dataclasses.is_dataclass(t) and isinstance(t, type) and isinstance(v, t):
        tipos = _tipos_de(t)
        for c in dataclasses.fields(t):
            try:
                convertir(c.name, getattr(v, c.name), tipos.get(c.name, _NADA))
            except ContractError as e:
                raise ContractError("`%s` returned a `%s` with %s" % (que, t.__name__, e)) from None
        return v
    try:
        return convertir(que, v, t)
    except ContractError:
        raise ContractError("`%s` returned %s and annotates `-> %s`" % (que, _corto(v), _nombre(t))) from None


def llamada(f):
    """`f` with its contract: converts each parameter to what it annotates and
    checks what it returns. An unannotated parameter (the row of a function
    with `over`) passes as is."""
    crudo = inspect.unwrap(f)
    firma = inspect.signature(crudo)
    tipos = _tipos_de(crudo)
    vuelve = tipos.get("return", _NADA)
    nombre = getattr(crudo, "__name__", "the function")

    @functools.wraps(crudo)
    def con_contrato(*args, **kwargs):
        try:
            atado = firma.bind(*args, **kwargs)
        except TypeError as e:
            raise ContractError("`%s`: %s" % (nombre, e)) from None
        for k, v in list(atado.arguments.items()):
            atado.arguments[k] = convertir(k, v, tipos.get(k, _NADA))
        r = crudo(*atado.args, **atado.kwargs)
        if vuelve is _NADA:
            return r
        return comprobar_salida(nombre, r, vuelve)

    con_contrato.__ore_contrato__ = True
    return con_contrato
