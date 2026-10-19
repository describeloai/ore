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
# y uno de Java (C2·3 pieza 4): los mismos nombres con `Dataset<Row>`, en la JVM de Spark
mkdir -p packages/geo/pesadojava/transforms/geo
printf -- '---\nnombre: PesadoJava\nplantilla: transforms-java\nplantillaVersion: 1\nmotor: heavyweight\n---\nJava transforms en Spark.\n' \
  > packages/geo/pesadojava/README.md
cat > packages/geo/pesadojava/transforms/geo/Medidas.java <<'JAVA'
package geo;

/** De apoyo: otra clase del repositorio, que javac encuentra por el -sourcepath. */
final class Medidas {
    static final int ANCHO = 50;

    static String anillo(String col) {
        return "ST_Difference(ST_Buffer(" + col + ", " + ANCHO + "), " + col + ")";
    }
}
JAVA
cat > packages/geo/pesadojava/transforms/geo/AnillosJava.java <<'JAVA'
package geo;

import static ore.Ore.*;

import ore.Transform;
import org.apache.spark.sql.Dataset;
import org.apache.spark.sql.Row;

public class AnillosJava {
    /** Un anillo de 50 m alrededor de cada zona, en Java. */
    @Transform(inputs = {"geo.zonas"}, output = "geo.anillos_java")
    public static Object anillos() throws Exception {
        Dataset<Row> z = over("geo.zonas");
        return write("geo.anillos_java", z.selectExpr("id", Medidas.anillo("zona") + " AS anillo"));
    }
}
JAVA
cat > packages/geo/pesadojava/transforms/geo/ConteoJava.java <<'JAVA'
package geo;

import static ore.Ore.*;

import ore.Transform;

public class ConteoJava {
    /** El join espacial de conteo.sql, con sql() desde Java. */
    @Transform(inputs = {"geo.zonas", "geo.lugares"}, output = "geo.conteo_java")
    public static Object conteo() throws Exception {
        return write("geo.conteo_java", sql(
            "SELECT z.id, count(*) AS n FROM geo.zonas z JOIN geo.lugares l ON ST_Contains(z.zona, l.geom) GROUP BY z.id"));
    }
}
JAVA
cat > packages/geo/pesadojava/transforms/geo/FueraJava.java <<'JAVA'
package geo;

import static ore.Ore.*;

import ore.Transform;

public class FueraJava {
    /** Lee lo que NO declara: se niega. */
    @Transform(inputs = {"geo.zonas"}, output = "geo.fuera_java")
    public static Object fuera() throws Exception {
        return write("geo.fuera_java", over("geo.lugares"));
    }
}
JAVA
cat > packages/geo/pesadojava/transforms/geo/RompeJava.java <<'JAVA'
package geo;

import static ore.Ore.*;

import ore.Transform;

public class RompeJava {
    /** Un error del código: su línea llega al build. */
    @Transform(inputs = {"geo.zonas"}, output = "geo.rompe_java")
    public static Object rompe() throws Exception {
        String nada = null;
        return write("geo.rompe_java", over("geo.zonas").limit(nada.length()));
    }
}
JAVA
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
