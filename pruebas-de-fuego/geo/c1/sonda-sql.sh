#!/usr/bin/env bash
# ORE 0068 C1·1 · El previo de SQL en un python:3.14 con DuckDB 1.5.6 (la versión del puesto) y sus
# extensiones bajadas una vez a un volumen (c1-ext).
set -euo pipefail
aqui="$(cd "$(dirname "$0")" && pwd -W 2>/dev/null || pwd)"
sdk="$(cd "$(dirname "$0")/../../../puesto/python" && pwd -W 2>/dev/null || pwd)"
MSYS_NO_PATHCONV=1 docker run --rm -v "$sdk":/sdk:ro -v "$aqui":/c1:ro -v c1-ext:/opt/ore/duckdb -v c1-pip:/root/.cache/pip python:3.14 bash -c '
  pip install -q duckdb==1.5.6 pyarrow shapely 2>&1 | grep -v "notice\|WARNING" || true
  python -c "
import duckdb; c = duckdb.connect(); c.execute(\"set extension_directory = '"'"'/opt/ore/duckdb'"'"'\")
c.execute(\"install spatial\"); c.execute(\"install h3 from community\")"
  python /c1/sonda-sql.py'
