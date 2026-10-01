from dataclasses import dataclass

import ore
from ore import function

LIMITE = 20


def _ayuda(x):
    return x


@dataclass
class Resumen:
    resumen: str


@function(reads=["ventas.pedidos"], models=["extractor"], timeout="60s")
def resumir(pais: str) -> Resumen:
    return Resumen(pais)
