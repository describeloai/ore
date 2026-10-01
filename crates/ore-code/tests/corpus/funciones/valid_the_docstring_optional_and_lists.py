from datetime import date
from typing import Optional

import ore


@ore.function
def fibonacci(n: int, desde: Optional[date]) -> list[int]:
    """Los n primeros de Fibonacci.

    Lo demás de la docstring no va al documento.
    """
    a, b, serie = 0, 1, []
    for _ in range(n):
        serie.append(a)
        a, b = b, a + b
    return serie
