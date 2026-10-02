"""Los tipos de OOS que Python no escribe solo (ORE 0050 G5a, OOS v1alpha20 `01`).

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
    """`Annotated[Decimal, Precision(p, s)]` es `Decimal<p, s>`: como mucho `p`
    cifras, `s` detrás de la coma."""

    def __init__(self, precision, escala):
        if not (isinstance(precision, int) and isinstance(escala, int) and 1 <= precision <= 38
                and 0 <= escala <= precision):
            raise ValueError("Precision(%r, %r): 1 ≤ p ≤ 38 y 0 ≤ s ≤ p" % (precision, escala))
        self.precision, self.escala = precision, escala

    def __repr__(self):
        return "Precision(%d, %d)" % (self.precision, self.escala)


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
            raise TypeError('`%s[...]` lleva la unidad y la precisión: `%s`'
                            % (cls._ctor, 'Money["EUR", 2]' if cls._ctor == "Money" else 'Quantity["km", 1]'))
        return typing.Annotated[decimal.Decimal, _Unidad(cls._ctor, args[0], args[1])]


class Money(_ConUnidad):
    """`Money["EUR", 2]` es `Money<EUR, 2>`: un `Decimal` con su moneda en el tipo."""

    _ctor = "Money"


class Quantity(_ConUnidad):
    """`Quantity["km", 1]` es `Quantity<km, 1>`: un `Decimal` con su unidad en el tipo."""

    _ctor = "Quantity"


class _DeColeccion:
    def __init__(self, coleccion):
        self.coleccion = coleccion

    def __repr__(self):
        return "Media<%s>" % self.coleccion


class Media:
    """`Media["base.schema.coleccion"]` es `Media<base.schema.coleccion>`: la
    referencia a un ítem de esa colección (`ore.medios.MediaRef`), no sus bytes.
    Para leerlos, `ore.coleccion(ref.collection)`."""

    def __class_getitem__(cls, coleccion):
        if not (isinstance(coleccion, str) and coleccion):
            raise TypeError('`Media[...]` nombra una colección: `Media["base.schema.coleccion"]`')
        from .medios import MediaRef

        return typing.Annotated[MediaRef, _DeColeccion(coleccion)]
