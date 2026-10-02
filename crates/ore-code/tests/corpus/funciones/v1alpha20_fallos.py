# v1alpha20 `01`: lo que no se deriva (OOS2043), cada uno solo.
from dataclasses import dataclass
from decimal import Decimal
from typing import Annotated

from ore import function
from ore.tipos import Media, Money, Precision, Quantity

UNIDAD = "EUR"


@dataclass
class Nodo:
    valor: int
    hijos: list["Nodo"]


@dataclass
class A:
    b: "B"


@dataclass
class B:
    a: A


class Money2:
    pass


@function
def fuera_de_rango(x: Annotated[Decimal, Precision(39, 2)]) -> int:
    ...


@function
def escala_mayor(x: Annotated[Decimal, Precision(4, 5)]) -> int:
    ...


@function
def precision_de_un_float(x: Annotated[float, Precision(10, 2)]) -> int:
    ...


@function
def precision_no_literal(x: Annotated[Decimal, Precision(UNIDAD, 2)]) -> int:
    ...


@function
def unidad_no_literal(x: Money[UNIDAD, 2]) -> int:
    ...


@function
def sin_precision(x: Quantity["km"]) -> int:
    ...


@function
def precision_negativa(x: Money["EUR", -1]) -> int:
    ...


@function
def unidad_con_coma(x: Money["EUR,USD", 2]) -> int:
    ...


@function
def media_sin_coleccion(x: Media) -> int:
    ...


@function
def media_no_literal(x: Media[UNIDAD]) -> int:
    ...


@function
def recursiva(n: Nodo) -> int:
    ...


@function
def recursiva_indirecta(a: A) -> int:
    ...


@function
def lista_de_listas_de_struct(x: list[list[Nodo]]) -> int:
    ...


@function
def una_clase_que_no_es_dataclass(x: Money2) -> int:
    ...
