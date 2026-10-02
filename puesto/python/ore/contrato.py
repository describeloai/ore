"""El contrato de una función (ORE 0050 G3): sus anotaciones, cumplidas.

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
"""

import dataclasses
import datetime
import decimal
import functools
import inspect
import types
import typing

__all__ = ["ErrorDeContrato", "llamada", "convertir", "comprobar_salida"]


class ErrorDeContrato(TypeError):
    """Un valor que no es del tipo que la firma anota."""


_NADA = inspect.Parameter.empty


def _nombre(t):
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
    """`v` como el tipo `t` que anota `que` (un parámetro, o un campo)."""
    if t is _NADA or t is typing.Any or t is object:
        return v
    if v is None:
        if _opcional(t) is not None or t is type(None):
            return None
        raise ErrorDeContrato("`%s` es `%s` y llegó None" % (que, _nombre(t)))
    base = _opcional(t)
    if base is not None:
        return convertir(que, v, base)
    origen = typing.get_origin(t)
    if origen in (typing.Union, types.UnionType):
        for a in typing.get_args(t):
            try:
                return convertir(que, v, a)
            except ErrorDeContrato:
                pass
        raise ErrorDeContrato("`%s` es `%s` y llegó %s" % (que, _nombre(t), _corto(v)))
    if origen in (list, tuple) or t in (list, tuple):
        if not isinstance(v, (list, tuple)):
            raise ErrorDeContrato("`%s` es `%s` y llegó %s, que no es una lista" % (que, _nombre(t), _corto(v)))
        args = typing.get_args(t)
        dentro = args[0] if args else _NADA
        return [convertir("%s[%d]" % (que, i), x, dentro) for i, x in enumerate(v)]
    if origen is not None:  # dict[...] y demás: tal cual
        return v
    mal = lambda porque="": ErrorDeContrato(  # noqa: E731
        "`%s` es `%s` y llegó %s%s" % (que, _nombre(t), _corto(v), porque))
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
        raise mal(", que no es entero")
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
                raise mal(", que no es un número") from None
        raise mal()
    if t is str:
        if isinstance(v, str):
            return v
        raise mal()
    if t is datetime.datetime:
        if isinstance(v, datetime.datetime):
            return v
        if isinstance(v, str):
            try:
                return datetime.datetime.fromisoformat(v.replace("Z", "+00:00"))
            except ValueError:
                raise mal(", que no es una fecha y hora ISO 8601") from None
        raise mal()
    if t is datetime.date:
        if isinstance(v, datetime.date) and not isinstance(v, datetime.datetime):
            return v
        if isinstance(v, str):
            try:
                return datetime.date.fromisoformat(v)
            except ValueError:
                raise mal(", que no es una fecha AAAA-MM-DD") from None
        raise mal()
    if t is datetime.time:
        if isinstance(v, datetime.time):
            return v
        if isinstance(v, str):
            try:
                return datetime.time.fromisoformat(v)
            except ValueError:
                raise mal(", que no es una hora ISO 8601") from None
        raise mal()
    if dataclasses.is_dataclass(t) and isinstance(t, type):
        if isinstance(v, t):
            return v
        if isinstance(v, dict):
            tipos = _tipos_de(t)
            campos = {c.name for c in dataclasses.fields(t)}
            sobran = sorted(set(v) - campos)
            if sobran:
                raise mal(": `%s` no declara %s" % (t.__name__, sobran))
            return t(**{k: convertir("%s.%s" % (que, k), x, tipos.get(k, _NADA)) for k, x in v.items()})
        raise mal()
    if isinstance(t, type) and isinstance(v, t):
        return v
    if isinstance(t, type):
        raise mal()
    return v


def _tipos_de(f):
    try:
        return typing.get_type_hints(f)
    except Exception:  # noqa: BLE001 — una anotación que no resuelve: sin contrato para ella
        return {k: v for k, v in getattr(f, "__annotations__", {}).items() if not isinstance(v, str)}


def comprobar_salida(que, v, t):
    """Lo que `que` devolvió, contra lo que anota. Una dataclass se comprueba
    campo a campo; lo demás, con la misma regla que un parámetro (`3` vale
    para `float` y para `Decimal`, `"3"` no vale para `int`)."""
    if dataclasses.is_dataclass(t) and isinstance(t, type) and isinstance(v, t):
        tipos = _tipos_de(t)
        for c in dataclasses.fields(t):
            try:
                convertir(c.name, getattr(v, c.name), tipos.get(c.name, _NADA))
            except ErrorDeContrato as e:
                raise ErrorDeContrato("`%s` devolvió un `%s` con %s" % (que, t.__name__, e)) from None
        return v
    try:
        return convertir(que, v, t)
    except ErrorDeContrato:
        raise ErrorDeContrato("`%s` devolvió %s y anota `-> %s`" % (que, _corto(v), _nombre(t))) from None


def llamada(f):
    """`f` con su contrato: convierte cada parámetro a lo que anota y
    comprueba lo que devuelve. Un parámetro sin anotar (la fila de una función
    con `over`) pasa tal cual."""
    crudo = inspect.unwrap(f)
    firma = inspect.signature(crudo)
    tipos = _tipos_de(crudo)
    vuelve = tipos.get("return", _NADA)
    nombre = getattr(crudo, "__name__", "la función")

    @functools.wraps(crudo)
    def con_contrato(*args, **kwargs):
        try:
            atado = firma.bind(*args, **kwargs)
        except TypeError as e:
            raise ErrorDeContrato("`%s`: %s" % (nombre, e)) from None
        for k, v in list(atado.arguments.items()):
            atado.arguments[k] = convertir(k, v, tipos.get(k, _NADA))
        r = crudo(*atado.args, **atado.kwargs)
        if vuelve is _NADA:
            return r
        return comprobar_salida(nombre, r, vuelve)

    con_contrato.__ore_contrato__ = True
    return con_contrato
