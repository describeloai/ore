"""Sintaxis difícil alrededor de funciones de verdad (Python 3.12+)."""
from __future__ import annotations

import functools
from dataclasses import dataclass, field
from datetime import datetime
from decimal import Decimal
from typing import Optional

import ore
from ore import function

TEXTO = """
def falsa(x):
    @function
    def tampoco(y: int) -> int: ...
"""

type Alias = list[int]          # PEP 695: una sentencia `type`


def generica[T](x: T) -> T:     # PEP 695: parámetros de tipo
    return x


def otra(x):
    match x:                     # `match`
        case {"a": 1, **resto}:
            return resto
        case [1, *_]:
            return None
        case _:
            return (y := x)      # morsa


@dataclass
class Linea:
    """Una línea de pedido."""
    producto: str
    cantidad: int
    precio: Decimal = Decimal("0")
    nota: Optional[str] = None
    etiquetas: list[str] = field(default_factory=list)


@functools.lru_cache(maxsize=None)
@function(
    over="ventas.default.pedidos",   # la vista
    reads=[
        "ventas.default.clientes",
        "ventas.default.productos",  # , una coma en un comentario
    ],
    models=["modelo/extractor", "ia.chat"],
    timeout="2m",
)
def evaluar(
    fila,
    umbral: Decimal,
    desde: Optional[datetime] = None,
    *,
    etiquetas: list[str] = ["a,b", "c)"],
    activa: bool = True,
) -> Linea:
    """

    Evalúa una línea: «f"{x!r:>10}"» y nada más.
    """
    nombre = f"{fila['producto']!r:>{umbral}} {'anidada' + f'{1 + 1}'}"
    return Linea(nombre, 1)


@ore.function(models=["extractor"])
def resumir(texto: str, maximo: int | None = None) -> str:
    return texto[:maximo]


class NoEsDelModulo:
    @function
    def metodo(self, x: int) -> int:
        return x


async def asincrona(x: int) -> int:
    return x


@function
def sin_parametros() -> list[Decimal]:
    return [Decimal("1.5")]


@function
def con_lambda(n: int = (lambda: 3)()) -> int:
    return n
