from dataclasses import dataclass
from decimal import Decimal

from ore import function


@dataclass
class Nivel:
    nivel: str


def riesgo(cliente, umbral: Decimal, moneda: str = "EUR") -> Nivel:
    return Nivel("alto" if umbral > 100 else "bajo")
