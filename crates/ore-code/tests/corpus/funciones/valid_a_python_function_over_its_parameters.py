from ore import function


@function
def repetir(texto: str, veces: int = 1) -> str:
    return " ".join([texto] * veces)
