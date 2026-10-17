# D · Borradores para upstream (ADR 0067 G2·4·4) — SIN PUBLICAR

Nada de esto se publica sin permiso explícito, comentario a comentario.

## 1. apache/iceberg-rust — PR 3341 (geospatial read support)

> Thanks for this series. We tested `main` (697abcee, after #2933) against v3 tables with
> `geometry` written by three different writers, and it deserializes all three `metadata.json`:
> Iceberg Java 1.12 via Spark 4.1 (`geometry(OGC:CRS84)`), a table promoted from `binary` to
> `geometry(EPSG:25830)` in metadata only, and a schema-only table from the Java API. `0.10.1` fails
> on all of them with `data did not match any variant of untagged enum TableMetadataEnum`, i.e.
> before reaching the schema, so any reader on 0.10 cannot even list such a table.
>
> Two things that may be useful for the read path:
> - Iceberg Java 1.12 writes data files with the Parquet GEOMETRY logical type but **without** the
>   GeoParquet `geo` key; readers relying only on the key will see a plain binary.
> - Files written with `parquet` 58.4 (`geospatial` feature) carry the CRS of the logical type as a
>   JSON-quoted string (`crs="\"EPSG:25830\""`); DuckDB and GDAL treat it as unknown. 59.x writes it
>   unquoted. Worth a test if 3341 reads the CRS from the logical type.
>
> We can share the three `metadata.json` (and small data files) as fixtures if that helps.

## 2. apache/arrow-rs — `parquet` 58.4 (solo si no hay ya un issue cerrado)

> In 58.4, `logical_type_for_binary` (`arrow/schema/extension.rs`) builds the GEOMETRY/GEOGRAPHY CRS
> with `crs.to_string()` on a `serde_json::Value`, which keeps the quotes. The resulting logical
> type is `Geometry(crs="\"EPSG:25830\"")`; DuckDB 1.5.6 and GDAL 3.13 do not recognise it. 59.3.1
> fixes this (uses the raw string). Filing only for the record / a 58.x backport if one is planned.

Antes de publicar: comprobar si ya existe el issue o el commit de arreglo en arrow-rs y citarlo en vez
de abrir uno nuevo.

## 3. apache/iceberg — el lector genérico de Java

Con 1.12 **ya lee** los ficheros GEOMETRY nativos (con 1.11 fallaba, G2·0). **No hay nada que
reportar**; se deja constancia aquí y no se publica.
