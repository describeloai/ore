# 0059 X6 · Las vistas de C7, en `sandbox`: una base lee a otra por su nombre.
#
# Un árbol es un catálogo (ADR 0059, OOS v1alpha28): `sandbox` lee la colección
# de `s3_stuff` por su nombre, sin que `s3_stuff` la exporte, como en Unity.
# Vale para todo árbol y toda rama: no hay nada que cambiar antes.
#
# Con Run en el repositorio Models: crea las cuatro vistas en `sandbox` (schema
# `default`) y las lee.
import ore

C = "s3_stuff.nueva_carpeta.contratos"
DESTINO = "sandbox"

# ── las vistas, en `sandbox` ─────────────────────────────────────────────
VISTAS = {
    # Inventario: cuántos ficheros de cada tipo, cuánto ocupan y desde cuándo.
    f"{DESTINO}.contratos_inventario": f"""
        SELECT content_type, count(*) AS ficheros, sum(size) AS bytes,
               min(modified) AS el_primero, max(modified) AS el_ultimo
        FROM {C}
        GROUP BY content_type""",
    # Por mes: lo que entró cada mes.
    f"{DESTINO}.contratos_por_mes": f"""
        SELECT date_trunc('month', modified) AS mes, count(*) AS ficheros, sum(size) AS bytes
        FROM {C}
        GROUP BY date_trunc('month', modified)""",
    # Duplicados: el mismo contenido (la misma huella) con más de un nombre.
    f"{DESTINO}.contratos_duplicados": f"""
        SELECT digest, count(*) AS copias, string_agg(path, ', ') AS rutas
        FROM {C}
        GROUP BY digest
        HAVING count(*) > 1""",
    # Los escaneados: por su nombre, con su tamaño.
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
