# 0059 X5 · Las vistas de C7, en `sandbox`: una base lee a otra por su nombre.
#
# Con el `OntologyConfig` en OOS v1alpha28 el árbol es un catálogo (ADR 0059):
# `sandbox` lee la colección de `s3_stuff` sin que `s3_stuff` la exporte, como
# en Unity.
#
# ANTES, en la consola (una sesión no puede tocar el gobierno del árbol: desde
# un puesto `PUT /arbol/…` es 403, y así tiene que ser):
#   el editor del árbol → `ontology.config.yaml` → la primera línea pasa a
#   `apiVersion: oos.dev/v1alpha28` → commit. En `main` (para todos) y, si tu
#   repositorio Models ya tenía rama, también en ella: una rama nace de `main`
#   y no se actualiza sola.
#
# Después, con Run en el repositorio Models: comprueba el config de tu rama,
# crea las cuatro vistas en `sandbox` (schema `default`) y las lee.
import re

import ore

CONFIG = "ontology.config.yaml"
C = "s3_stuff.nueva_carpeta.contratos"
DESTINO = "sandbox"

# ── 1 · el árbol de tu rama, ¿es un catálogo? ───────────────────────────────
c, r = ore.session.pedir("GET", "/arbol/" + CONFIG)
assert c == 200, (c, r)
version = re.search(r"^apiVersion:\s*(\S+)", r["texto"], re.M).group(1)
if version != "oos.dev/v1alpha28":
    raise SystemExit("✗ el `%s` de tu rama es %s: pásalo a oos.dev/v1alpha28 en la consola (arriba) y vuelve a correr esto"
                     % (CONFIG, version))
print("✓ el árbol de tu rama es un catálogo (", version, ")")

# ── 2 · las vistas, en `sandbox` ─────────────────────────────────────────────
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
