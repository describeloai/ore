# v1alpha20 `01`: la firma habla OOS. Cada tipo nuevo, con los nombres
# resueltos como Python: con alias, por el módulo y entre comillas.
from dataclasses import dataclass, field
from datetime import date, time
from decimal import Decimal
from typing import Annotated, Optional

import ore.tipos as t
from ore import function
from ore.tipos import DateTimeTz, Media, Money, Precision
from ore.tipos import Quantity as Cantidad


@dataclass
class Linea:
    producto: str
    unidades: int
    precio: Money["EUR", 2]
    notas: Optional[str] = None


@dataclass
class Pedido:
    id: int
    lineas: list[Linea]
    entrega: time
    total: Annotated[Decimal, Precision(12, 2)] = field(default=Decimal("0"))


@dataclass
class Resumen:
    pedidos: int
    peso: Cantidad["kg", 3]
    ultimo: DateTimeTz | None = None


@function
def resumir(pedido: Pedido, pedidos: list[Pedido], firma: bytes, contrato: Media["legal.archivo.contratos"],
            corte: t.DateTimeTz, abre: "time" = time(9)) -> Resumen:
    """Resume pedidos con su dinero, su peso y sus ficheros."""
    ...


@function
def lineas(p: Pedido) -> list[Linea]:
    return p.lineas


@function
def total(p: Pedido) -> Annotated[Decimal, Precision(12, 2), "otra cosa"]:
    return p.total


@function
def solo_v1alpha18(a: int, b: Annotated[str, "no es firma"]) -> date:
    ...


@function
def la_comilla_es_la_unidad(x: t.Money["USD", 0], d: Annotated["Decimal", t.Precision(5, 5)]) -> t.Quantity["m/s", 1]:
    ...
