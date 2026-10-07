# 0057 C7 · Vistas sobre una colección del lago, en vivo (victor), desde Python.
#
# Lo mismo que `vistas.sql`, con el verbo del SDK que hay detrás de `create view`:
# cuatro vistas sobre el LISTADO de `s3_stuff.nueva_carpeta.contratos` (una
# fila por fichero, sin leer un byte; OOS v1alpha17 `04`), junto a ella.
#
# ⚠️ En la misma base que la colección: una vista de OTRA base (`sandbox`) sólo
#   lee lo que `s3_stuff` exporta (OOS2028), y hoy no exporta nada. Para
#   tenerlas en `sandbox`, exporta antes la colección en el `package.yaml` de
#   `s3_stuff` (`exports: [s3_stuff.nueva_carpeta.contratos]`) y cambia DESTINO.
#
# Cada vista queda como documento del árbol, en tu rama, y se calcula al leerla:
# aquí con `sql()`, y en el catálogo su preview la hace ore-motor.
#
# Se corre con Run en un repositorio que ejecuta (no `transforms-*`, donde el
# botón es Build y sólo construye lo que escribe datos).
import ore

C = "s3_stuff.nueva_carpeta.contratos"
DESTINO = "s3_stuff.nueva_carpeta"

VISTAS = {
    # 1 · Inventario: cuántos ficheros de cada tipo, cuánto ocupan y desde cuándo.
    f"{DESTINO}.contratos_inventario": f"""
        SELECT content_type, count(*) AS ficheros, sum(size) AS bytes,
               min(modified) AS el_primero, max(modified) AS el_ultimo
        FROM {C}
        GROUP BY content_type""",
    # 2 · Por mes: lo que entró cada mes.
    f"{DESTINO}.contratos_por_mes": f"""
        SELECT date_trunc('month', modified) AS mes, count(*) AS ficheros, sum(size) AS bytes
        FROM {C}
        GROUP BY date_trunc('month', modified)""",
    # 3 · Duplicados: el mismo contenido (la misma huella) con más de un nombre.
    f"{DESTINO}.contratos_duplicados": f"""
        SELECT digest, count(*) AS copias, string_agg(path, ', ') AS rutas
        FROM {C}
        GROUP BY digest
        HAVING count(*) > 1""",
    # 4 · Los escaneados: por su nombre, con su tamaño.
    f"{DESTINO}.contratos_escaneados": f"""
        SELECT path, size, modified
        FROM {C}
        WHERE lower(path) LIKE '%escaneado%'""",
}

for nombre, consulta in VISTAS.items():
    r = ore.create_view(nombre, consulta.strip(), or_replace=True)
    print("✓", r["view"], "·", r["status"], "·", ", ".join(r["columns"]))

print()
for nombre in VISTAS:
    t = ore.sql(f"select * from {nombre}", format="arrow")
    print("──", nombre, "·", t.num_rows, "fila(s)")
    for fila in t.to_pylist()[:5]:
        print("  ", fila)
