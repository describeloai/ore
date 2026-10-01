from decimal import Decimal

from ore import function


class Reglas:
    @function(over="ventas.clientes", reads=["ventas.pedidos"])
    def riesgo(self, cliente, umbral: Decimal, moneda: str = "EUR") -> str:
        return "alto"
