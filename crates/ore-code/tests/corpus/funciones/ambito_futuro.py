from __future__ import annotations

from dataclasses import dataclass

from ore import function


@function(over="ventas.default.pedidos", timeout="30s")
def adelantada(fila, minimo: int = 0) -> Despues:
    """

       La docstring empieza tarde.
    """
    return Despues(1)


@dataclass
class Despues:
    valor: int | None
