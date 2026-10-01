from dataclasses import dataclass

import functools
from ore import function


def registrada(f):
    return f


@dataclass
class Nivel:
    nivel: str


@registrada
@function(over="ventas.clientes")
def riesgo(cliente: dict, *, umbral: float) -> Nivel:
    return Nivel("alto")
