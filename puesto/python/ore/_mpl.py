"""El backend de matplotlib de una celda (las salidas de una celda, S3).

El agente del puesto pone `MPLBACKEND=module://ore._mpl`: se dibuja con Agg
(sin ventanas) y `plt.show()` enseña cada figura abierta en ese punto de la
celda, como imagen (`ore.display`), y las cierra —lo que hace el backend
`inline` de Jupyter—. Las que quedan abiertas al acabar la celda las enseña el
agente.
"""
from matplotlib.backend_bases import _Backend
from matplotlib.backends.backend_agg import _BackendAgg


@_Backend.export
class _BackendOre(_BackendAgg):
    @staticmethod
    def show(*args, **kwargs):
        import matplotlib.pyplot as plt

        import ore

        for n in plt.get_fignums():
            ore.display(plt.figure(n))
        plt.close("all")
