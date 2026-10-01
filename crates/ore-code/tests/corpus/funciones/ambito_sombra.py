from datetime import date

from ore import function


class date:  # tapa a `datetime.date`: ya no es una fecha
    pass


@function
def con_una_clase_propia(dia: date) -> str:
    return "?"


@function
def bien_antes(n: int) -> int:
    return n


from otra.biblioteca import function  # noqa: E402 · tapa a `ore.function`


@function
def no_es_de_ore(n: int) -> int:
    return n


int = float  # noqa: A001 · tapa al `int` del lenguaje


@ore_no_importado.function
def tampoco(n: int) -> int:
    return n
