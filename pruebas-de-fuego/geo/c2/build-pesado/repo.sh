#!/usr/bin/env bash
# ORE 0068 C2·3 2b·3 · Dentro del banco: el repositorio Heavyweight de SQL transforms en el árbol
# (`packages/geo/pesado`, clase `transforms-sql`, `motor: heavyweight`), sus dos `.sql`, el
# `Transform` derivado (`ore transforms generate`) y el commit. Lo lanza `medir.sh`.
set -euo pipefail
B=/repo/target/release
A=/tmp/banco/arbol
cd "$A"
git -c user.name=b -c user.email=b@x add -A >/dev/null 2>&1 || true
git -c user.name=b -c user.email=b@x commit -qm "lo que dejó el banco" >/dev/null 2>&1 || true
mkdir -p packages/geo/pesado/transforms
printf -- '---\nnombre: Pesado\nplantilla: transforms-sql\nplantillaVersion: 1\nmotor: heavyweight\n---\nSQL transforms en Spark.\n' \
  > packages/geo/pesado/README.md
# un join espacial que agrega (sin geometría en la salida)
cat > packages/geo/pesado/transforms/conteo.sql <<'SQL'
CREATE OR REPLACE DATASET geo.conteo AS
SELECT z.id, count(*) AS n
FROM geo.zonas z JOIN geo.lugares l ON ST_Contains(z.zona, l.geom)
GROUP BY z.id;
SQL
# una geometría calculada que sale (su CRS lo dice la tabla)
cat > packages/geo/pesado/transforms/centros.sql <<'SQL'
CREATE OR REPLACE DATASET geo.centros AS
SELECT id, ST_Centroid(zona) AS centro, round(ST_Area(zona)) AS area FROM geo.zonas;
SQL
# y un repositorio de Python, también Heavyweight: lo mismo con `over()` y `write()`
mkdir -p packages/geo/pesadopy/transforms
printf -- '---\nnombre: PesadoPy\nplantilla: transforms-python\nplantillaVersion: 1\nmotor: heavyweight\n---\nPython transforms en Spark.\n' \
  > packages/geo/pesadopy/README.md
cat > packages/geo/pesadopy/transforms/anillos.py <<'PY'
from ore import transform, over, write


@transform(inputs=["geo.zonas"], output="geo.anillos")
def anillos():
    # un anillo de 50 m alrededor de cada zona: geometría que entra y que sale
    z = over("geo.zonas")
    return write("geo.anillos", z.selectExpr("id", "ST_Difference(ST_Buffer(zona, 50), zona) AS anillo"))


@transform(inputs=["geo.zonas"], output="geo.fuera")
def fuera():
    # lee lo que NO declara: el catálogo lo niega (lo declarado manda)
    return write("geo.fuera", over("geo.lugares"))
PY
echo "== los datasets que el árbol tiene"
ls packages/geo/ 2>/dev/null | tr '\n' ' '; echo
for f in $(ls packages/geo/*/datasets/*.yaml packages/geo/datasets/*.yaml 2>/dev/null); do
  echo "-- $f"; grep -E "name:|type:" "$f" | head -12
done
echo "== ore transforms generate"
"$B/ore" transforms generate . 2>&1 | tail -12 || true
echo "== ore validate"
"$B/ore" validate . 2>&1 | tail -12 || true
git -c user.name=b -c user.email=b@x add -A && git -c user.name=b -c user.email=b@x commit -qm "geo/pesado: dos SQL transforms Heavyweight"
git log --oneline -1
