# ADR 0067 G2·5 · El arnés local de los lectores de Python y GDAL (para iterar; el veredicto es en victor).
# Necesita /c/tmp/v5 de `cargo run -p ore-store --example v5-fijos` y `g24/promover.py` sobre cada tabla.
#   bash pruebas-de-fuego/geo/v5/local-python.sh "$(cygpath -w pruebas-de-fuego/geo/v5)"
M=$1
for c in EPSG:25830 OGC:CRS84; do d=${c/:/-}
 v0=$(ls /c/tmp/v5/$d/tabla/ore/v2/v5/metadata/00000-*.json | xargs -n1 basename | sed 's/.metadata.json//')
 v1=$(ls /c/tmp/v5/$d/tabla/ore/v2/v5/metadata/00001-*.json | xargs -n1 basename | sed 's/.metadata.json//')
 ARGS="A=/v5/$d/ore-geo.parquet B=/v5/$d/tabla/ore/v2/v5@$v0 C=/v5/$d/tabla/ore/v2/v5@$v1"
 MSYS_NO_PATHCONV=1 docker run --rm --network none -v "C:/tmp/v5:/v5" -v "$M:/m" g235-puesto-python sh -c "for l in duckdb duckdb-spatial pyarrow geopandas; do python /m/lectores.py \$l /m/fijos.json $ARGS; done" > /c/tmp/v5/py-$d.txt 2>&1
 MSYS_NO_PATHCONV=1 docker run --rm --network none -v "C:/tmp/v5:/v5" -v "$M:/m" ghcr.io/osgeo/gdal:ubuntu-full-3.13.3 python3 /m/lectores.py gdal /m/fijos.json $ARGS >> /c/tmp/v5/py-$d.txt 2>&1
 python /c/ORE-pg/pruebas-de-fuego/geo/v5/juzgar.py /c/ORE-pg/pruebas-de-fuego/geo/v5/fijos.json $c < /c/tmp/v5/py-$d.txt | cut -c1-260
done
