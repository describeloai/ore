import json, pyarrow.parquet as pq, duckdb, geopandas
f = "/salida/ore-geo.parquet"
md = pq.ParquetFile(f).metadata
geo = json.loads(md.metadata[b"geo"])
print("V5|pyarrow|clave geo|-|", geo["version"], geo["columns"]["geom"]["geometry_types"], "crs:", geo["columns"]["geom"]["crs"]["id"])
print("V5|pyarrow|tipo lógico|-|", md.schema.column(1).logical_type)
print("V5|pyarrow|campo|-|", pq.read_schema(f).field("geom").metadata)
c = duckdb.connect(); c.execute("install spatial; load spatial")
print("V5|duckdb|tipo|-|", c.execute(f"select typeof(geom), ST_AsText(geom) from '{f}' limit 1").fetchone())
print("V5|duckdb|geo_bbox por grupo|-|", c.execute(f"select count(*), any_value(geo_bbox) from parquet_metadata('{f}') where path_in_schema='geom'").fetchone())
g = geopandas.read_parquet(f)
print("V5|geopandas|read_parquet|-|", len(g), g.crs.to_string() if g.crs else None, g.geometry.iloc[0].geom_type)
