from ore import function


@function(over="v", extra=1)
def muchos(fila, x, *args, y: dict[str, int], **kw) -> None:
    pass


@function
async def asincrona(x: int) -> int:
    return x


@function("posicional")
def posicional() -> list[list[int]]:
    return []


@function
def lista_de_opcionales(x: list[int | None]) -> int:
    return 0


@function
def union_de_dos(x: int | str) -> int:
    return 0


@function(over="v")
def sin_fila(*, x: int) -> int:
    return x


@function(reads=("a", "b"), models=[MODELO])
def no_literales() -> int:
    return 0


class Clase:
    @function
    def metodo(self) -> int:
        return 0


def externa():
    @function
    def interna() -> int:
        return 0
