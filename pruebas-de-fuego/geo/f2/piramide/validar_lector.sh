#!/bin/sh
# ORE 0072 F2·3·2·1 · el lector de `ore-teselas` contra los PMTiles de tippecanoe de la prueba de concepto: las
# mismas teselas que la librería oficial (las muestras, de `muestra.py`). Dentro de rust:1.90, con /src = C:\ORE-pg.
#   sh pruebas-de-fuego/geo/f2/piramide/validar_lector.sh [capa…]
set -e
D=/src/pruebas-de-fuego/geo/f2/piramide/pmtiles
for n in ${*:-trip_3m_puntos edificios_densidad trip_3m_lineas_densidad edificios}; do
  printf '%s: ' "$n"
  ORE_PMTILES=$D/$n.pmtiles ORE_PMTILES_MUESTRA=$D/$n.muestra.json \
    cargo test -q -p ore-teselas -- --ignored de_tippecanoe --nocapture 2>&1 | grep -E 'iguales|panicked' | cut -c1-60
done
