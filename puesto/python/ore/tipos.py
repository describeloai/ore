"""The OOS types Python cannot write on its own (ORE 0050 G5a, OOS v1alpha20 `01`).

    from ore.tipos import DateTimeTz, Money, Quantity, Media, Precision

`DateTimeTz` is a `datetime` with a zone; `Money["EUR", 2]` and
`Quantity["km", 1]` a `Decimal` with its unit and precision;
`Annotated[Decimal, Precision(p, s)]` is `Decimal<p, s>`;
`Media["db.schema.collection"]` a reference to an item (`MediaRef`). They are
annotations for `@function` signatures, enforced by `ore.contrato`.

---

Los tipos de OOS que Python no escribe solo (ORE 0050 G5a, OOS v1alpha20 `01`).

    from ore.tipos import DateTimeTz, Money, Quantity, Media, Precision

    @function
    def cobrar(importe: Money["EUR", 2], cuando: DateTimeTz,
               tasa: Annotated[Decimal, Precision(5, 4)],
               contrato: Media["legal.archivo.contratos"]) -> ...

Son **anotaciones**: el documento `Function` se deriva de ellas leyendo el
fichero, sin ejecutarlo, y por eso sus argumentos son literales. Al ejecutarse
son un `Annotated` de lo que Python sí tiene —`datetime`, `Decimal`, la
referencia a un ítem— con lo que OOS añade, y el contrato (`ore.contrato`) lo
hace cumplir: un instante lleva zona, un `Money<EUR, 2>` no tiene más de dos
decimales, una referencia es de su colección. Solo la biblioteca estándar.
"""

import datetime
import decimal
import typing

__all__ = ["DateTimeTz", "Money", "Quantity", "Media", "Precision"]


class _ConZona:
    """La marca de `DateTimeTz`: un `datetime` que lleva su zona."""

    def __repr__(self):
        return "ConZona"


#: Un instante: un `datetime` **con** zona (`DateTimeTz` de OOS). `datetime` a
#: secas es `DateTime`, una fecha y hora sin zona.
DateTimeTz = typing.Annotated[datetime.datetime, _ConZona()]


class Precision:
    """`Annotated[Decimal, Precision(p, s)]` is `Decimal<p, s>`: at most `p`
    digits, `s` of them after the decimal point."""

    def __init__(self, precision, scale=None, **kw):
        if "escala" in kw:  # el nombre de antes
            if scale is not None:
                raise TypeError("Precision() got both `scale` and its old name `escala`")
            from . import _avisar

            _avisar("Precision(escala=…)", "Precision(scale=…)")
            scale = kw.pop("escala")
        if kw:
            raise TypeError("Precision() got an unexpected keyword argument %r" % next(iter(kw)))
        if not (isinstance(precision, int) and isinstance(scale, int) and 1 <= precision <= 38
                and 0 <= scale <= precision):
            raise ValueError("Precision(%r, %r): 1 ≤ p ≤ 38 and 0 ≤ s ≤ p" % (precision, scale))
        self.precision, self.scale = precision, scale

    @property
    def escala(self):
        """El nombre de antes de `scale`."""
        return self.scale

    def __repr__(self):
        return "Precision(%d, %d)" % (self.precision, self.scale)


class _Unidad:
    def __init__(self, ctor, unidad, precision):
        self.ctor, self.unidad, self.precision = ctor, unidad, precision

    def __repr__(self):
        return "%s<%s, %d>" % (self.ctor, self.unidad, self.precision)


class _ConUnidad:
    _ctor = ""

    def __class_getitem__(cls, args):
        if not (isinstance(args, tuple) and len(args) == 2 and isinstance(args[0], str) and args[0]
                and isinstance(args[1], int) and args[1] >= 0):
            raise TypeError('`%s[...]` takes the unit and the precision: `%s`'
                            % (cls._ctor, 'Money["EUR", 2]' if cls._ctor == "Money" else 'Quantity["km", 1]'))
        return typing.Annotated[decimal.Decimal, _Unidad(cls._ctor, args[0], args[1])]


class Money(_ConUnidad):
    """`Money["EUR", 2]` is `Money<EUR, 2>`: a `Decimal` with its currency in the type."""

    _ctor = "Money"


class Quantity(_ConUnidad):
    """`Quantity["km", 1]` is `Quantity<km, 1>`: a `Decimal` with its unit in the type."""

    _ctor = "Quantity"


class _DeColeccion:
    def __init__(self, coleccion):
        self.coleccion = coleccion

    def __repr__(self):
        return "Media<%s>" % self.coleccion


if typing.TYPE_CHECKING:
    # Para el editor (pyright): `Media["…"]` es un `MediaRef`, con sus campos.
    from .medios import MediaRef as _DeMedia
else:
    _DeMedia = object


class Media(_DeMedia):
    """`Media["db.schema.collection"]` is `Media<db.schema.collection>`: a
    reference to an item of that collection (`ore.medios.MediaRef`), not its
    bytes. To read them, `ore.collection(ref.collection)`."""

    def __class_getitem__(cls, coleccion):
        if not (isinstance(coleccion, str) and coleccion):
            raise TypeError('`Media[...]` names a collection: `Media["db.schema.collection"]`')
        from .medios import MediaRef

        return typing.Annotated[MediaRef, _DeColeccion(coleccion)]
