from dataclasses import KW_ONLY, InitVar, dataclass, field
from typing import TYPE_CHECKING, ClassVar

from ore import function

if TYPE_CHECKING:
    from decimal import Decimal


@function
def entre_comillas(importe: "Decimal") -> "Resultado":
    return Resultado("x", importe)


@function
def antes_de_tiempo(n: int) -> Resultado:  # en 3.12 se evalúa al definir: aún no existe
    return Resultado("x", n)


@dataclass
class Base:
    id: str


@dataclass(frozen=True)
class Resultado(Base):
    total: "Decimal"
    UNIDAD: ClassVar[str] = "EUR"
    semilla: InitVar[int] = 0
    _: KW_ONLY
    notas: list[str] = field(default_factory=list)
    puntos: int = field(metadata={"x": 1})
    peso: float = field(default=1.0)


@function
def despues(n: int) -> Resultado:
    return Resultado("x", n)


class NoEsDataclass:
    a: int


@function
def devuelve_una_clase_normal(n: int) -> NoEsDataclass:
    return NoEsDataclass()


@dataclass
class Rara(dict):
    a: int


@function
def hereda_de_fuera(n: int) -> Rara:
    return Rara()
