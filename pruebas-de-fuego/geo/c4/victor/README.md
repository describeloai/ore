# C4 en victor · un enriquecimiento de punta a punta en el puesto pesado

Lo hace una persona desde la consola, en su árbol. Sustituye `geo` por un paquete suyo si no tiene ese.

## 1 · Los datos (una celda de Python del puesto pesado)

Los mismos de C2·3 (`c2/build-pesado/sembrar.py`): 30 puntos y 4 zonas de 150 m, en EPSG:25830.

```python
import ore
from ore import spark as pesado

s = pesado.sesion()
ore.write("geo.lugares", s.sql(
    "select id, concat('p', id) nombre, "
    "ST_SetSRID(ST_Point(440000 + id * 40, 4474000 + (id % 3) * 40), 25830) geom from range(30)"))
ore.write("geo.zonas", s.sql(
    "select id, ST_SetSRID(ST_Buffer(ST_Point(440000 + id * 400, 4474000), 150), 25830) zona from range(4)"))
```

## 2 · El repositorio y el enriquecimiento

Un repositorio de SQL transforms con `motor: heavyweight` (su `README.md` lo dice), y en él
`transforms/lugares.enrich.yaml`:

```yaml
output: geo.lugares_enriquecidos
target: { dataset: geo.lugares }
layers:
  - dataset: geo.zonas
    relation: within
    take:
      - { as: zona, column: id }
  - dataset: geo.zonas
    relation: intersects
    around: 300
    take:
      - { as: zonas_300m, count: true }
  - dataset: geo.zonas
    relation: { nearest: 1, maxDistance: 2000 }
    take:
      - { as: zona_cercana, column: id }
      - { as: dist_zona, distance: true }
```

Commit (ORE deriva el `Transform` v1alpha33 `runtime: enrich`) y **Build** de `geo.lugares_enriquecidos`.

## 3 · Lo que debe salir

- 30 filas, una por lugar, con `id`, `nombre`, `geom` y las cinco columnas nuevas.
- `zona`: el id de la zona que contiene el punto, o null. Las zonas están en x = 440000, 440400,
  440800 y 441200 con radio 150; los puntos, cada 40 m desde 440000.
- `zonas_300m`: 0, 1 o 2.
- `zona_cercana` y `dist_zona`: nunca null (la más lejana está a menos de 2 km).
- La ficha: `producedBy.columns` con las cinco columnas y su capa.
