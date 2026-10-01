from ore import function
from datetime import date, timedelta
from typing import Optional


# You can test your code in real time using the Live Preview feature.

@function
def example_addition_function(a: int, b: int) -> str:
    return f"The sum of {a} and {b} is {a + b}."


@function
def example_date_function(fecha: date, days_ahead: int = 3) -> str:
    return f"{days_ahead} days after {fecha} is {fecha + timedelta(days=days_ahead)}."


@function
def example_fibonacci_function(n: Optional[int] = None) -> list[int]:
    n = n if n is not None else 10
    a, b, seq = 0, 1, []
    for i in range(n):
        seq.append(a)
        a, b = b, a + b
    return seq
