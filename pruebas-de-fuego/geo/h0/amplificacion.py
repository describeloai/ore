"""ORE 0072 H0·B4 y B5 · cuánto se reescribe y cuánto choca, con patrones de cliente (modelo, sin datos).

Modelo (validado por `rejilla.py`, correlación 0,98): cada tanda ordenada por su cuenta («run») que cubre toda
la extensión suma ~1 a la profundidad. Datos nuevos repartidos por el espacio (eventos de red, ventas, pólizas):
reescribir «sólo los ficheros que pisan» es reescribir todo. Se comparan tres políticas para 30 días.

  python pruebas-de-fuego/geo/h0/amplificacion.py
"""
import math
import sys

sys.stdout.reconfigure(encoding="utf-8")
FILAS_S = 0.8e6  # filas/s de un proceso que ordena y escribe (0067 V3: 0,81 M; Spark ~1 M en 8 vCPU)
DIAS = 30


def politicas(nombre, base, por_tanda, tandas_dia, umbral=8, razon=10):
    nuevas_dia = por_tanda * tandas_dia
    n = base + nuevas_dia * DIAS / 2  # tamaño medio en el mes
    # (a) reescribirlo todo cuando hay `umbral` tandas sin fundir
    a_dia = tandas_dia / umbral * n
    a_prof = umbral
    # (b) niveles (leveled, razón 10): cada fila se reescribe ~razón/2 veces por nivel
    niveles = max(1, math.ceil(math.log(n / por_tanda, razon)))
    b_dia = nuevas_dia * niveles * razon / 2
    b_prof = niveles + 1
    # (c) escalonado (tiered, razón 10): cada fila una vez por nivel; hasta `razon` tandas por nivel
    c_dia = nuevas_dia * niveles
    c_prof = niveles * razon / 2
    print("| %s | %.0f M/día | todo cada %d tandas: **%s/día**, prof. %d | niveles: **%s/día**, prof. ~%d | escalonado: %s/día, prof. ~%d |" % (
        nombre, nuevas_dia / 1e6, umbral, h(a_dia), a_prof, h(b_dia), b_prof, h(c_dia), c_prof))
    return b_dia


def h(filas):
    horas = filas / FILAS_S / 3600
    return "%.1f mil M filas (%.0f h·proceso)" % (filas / 1e9, horas) if filas >= 1e9 else \
        "%.0f M filas (%.1f h·proceso)" % (filas / 1e6, horas)


def choques(nombre, dura_min, cada_min, rebase_choca):
    sin = 1 - math.exp(-dura_min / cada_min)
    con = 1 - math.exp(-dura_min / cada_min) if rebase_choca else 0.0
    print("| %s | %d min | cada %s | %.0f %% | %.0f %% |" % (
        nombre, dura_min, ("%d min" % cada_min) if cada_min < 120 else ("%d h" % (cada_min // 60)), sin * 100, con * 100))


print("## B4 · lo que se reescribe al día (tabla de 20 000 M filas, 30 días)\n")
print("| patrón | nuevas | (a) todo al pasar el umbral | (b) niveles | (c) escalonado |")
print("|---|---|---|---|---|")
politicas("telco: `append` cada hora", 20e9, 5e6, 24)
politicas("retail: `append` diario (sin particionar)", 20e9, 50e6, 1)
politicas("seguros: `merge` diario 1 % (inserta la mitad)", 20e9, 100e6, 1)
print("\n## B5 · un `replace` que choca con el cliente\n")
print("| caso | dura el `replace` | escribe el cliente | sin rebase (hoy) | con rebase |")
print("|---|---|---|---|---|")
# un trozo (commit parcial) de 50 M filas en 4 procesos ≈ 4 min; un nivel de 5 000 M en 16 procesos ≈ 1,5 h
choques("trozo de 50 M, `append` cada minuto", 4, 1, False)
choques("trozo de 50 M, `append` cada hora", 4, 60, False)
choques("nivel entero de 5 000 M, `append` cada hora", 90, 60, False)
choques("trozo de 50 M, `merge` diario que toca todo", 4, 1440, True)
choques("nivel entero, `merge` diario que toca todo", 90, 1440, True)
print("\n## B4 · particionado por día y el mapa de «todo»\n")
for dias in (7, 30, 365):
    print("- %d particiones diarias, cada una ordenada por dentro: profundidad ≥ %d para una tesela de todo el periodo" % (dias, dias))
