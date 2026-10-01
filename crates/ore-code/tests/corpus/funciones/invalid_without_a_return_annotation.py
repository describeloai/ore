from dataclasses import dataclass
from decimal import Decimal

from ore import function


@dataclass
class Nivel:
    nivel: str


@function(over="ventas.clientes", reads=["ventas.pedidos"])
def riesgo(cliente, umbral: Decimal, moneda: str = "EUR"):
    return Nivel("alto" if umbral > 100 else "bajo")
