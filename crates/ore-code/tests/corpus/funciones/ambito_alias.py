"""Cada nombre se resuelve por lo que el fichero importa, con alias o sin él."""
import datetime as dt
import typing
from typing import Annotated, Union

import ore as o
from ore import function as funcion


@o.function
def por_modulo(dia: dt.date, n: typing.Optional[int] = None) -> str:
    return str(dia)


@funcion(models=["ia.chat"])
def por_alias(x: Union[int, None], nombre: Annotated[str, "el nombre"]) -> typing.List[float]:
    return []


@funcion
def con_comillas(x: "dt.datetime", y: "list[int]") -> "typing.Optional[int]":
    return None


@funcion
def sin_importar(x: Optional[int]) -> int:  # `Optional` no está importado
    return 0
