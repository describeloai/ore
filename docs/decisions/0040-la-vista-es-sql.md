# 0040 · La vista es SQL (OOS v1alpha14 en ORE)

**Estado:** en curso · pasos 0 y 1 hechos.
**Spec:** `C:\oos` 7d92e6e, `spec/v1alpha14/`.

## Contexto

Desde OOS v1alpha14 una `View` es sólo SQL: `spec.sql`, `spec.dialect: duckdb` y
`spec.columns`, el contrato derivado. Lo que se gobierna de ella —lo que lee, el linaje
por columna, las etiquetas, el canal lateral (`OOS4016`)— se deriva de la consulta. Lo
estructurado se queda en la semántica (`Entity`).

En ORE la forma estructurada se lee en un sitio (`ore-core/src/vistas.rs`, 162 usos en
19 módulos) y llega al motor por una costura (`ore-cli/src/vista.rs::cuerpo`, 6
llamadores). Todo supone **una raíz** por vista (`raiz` 14 usos, `raiz_de_lectura` 8,
`respaldo` 9); una vista SQL puede leer varias fuentes, y ése es el grueso del cambio.

## Decisiones (del usuario, 2026-09-25)

- **A · Una sola View dentro de ORE, la SQL.** Una View v1alpha8–13 se **traduce a SQL al
  cargarla** (la misma traducción que la migración). Las versiones anteriores siguen
  significando lo mismo, como dice la spec (§7), sin dos motores. La conformance
  v1alpha5–13 entera es la puerta.
- **B · Las Views del árbol del usuario no son datos de producción.** Se migran o se
  borran sin ceremonia; no gobiernan ninguna decisión.
- **C · Spark por `/v1` se queda sin vistas hasta la migración de dialecto.** `loadView`
  de una vista `duckdb` pedida por Spark se niega y lo dice (spec §8, PUEDE). La
  alternativa —traducir el texto DuckDB a Spark al servirlo— cambiaría la semántica en
  silencio donde los dialectos difieren; la otra —una consulta por dialecto, como las
  *representations* de Iceberg— es la migración misma, y llega con Spark.
- **D · La copia de una vista es un dataset**, y se rehace entera ejecutando la consulta.
  `ore-view` y `ore-maintain` (mantenimiento incremental) dejan de recibir vistas; su
  destino se decide aparte.
- **E · `diff`:** `OOS5028/5029` (filtro estrechado/ensanchado) se basaban en la forma.
  Pasan a comparar el contrato; cualquier cambio del texto de la consulta es cambio de
  filas (spec §9).

## Pasos

0. **Medir.** Prototipo de `vista_sql` sobre todas las Views propias y de conformance
   traducidas: el linaje derivado del SQL ¿es idéntico al del motor estructurado? Es la
   puerta para quitar la forma. Y los usuarios de `raiz` que se rompen con varias fuentes.
   **HECHO** (`medida-el-linaje-de-la-vista-sql.py`, prototipo
   `crates/ore-core/examples/vista_sql.rs`):
   - L1: de 214 Views del repositorio se traducen 213 (144 de una Table, 48 de una View,
     9 de un Dataset, **12 de la forma v1alpha7 `from: { datasource, object }`**, que no
     tiene nombre del árbol: el paso 3 le da uno sintético); la que no, es un caso
     inválido a propósito (`OOS2034`). El prototipo analiza las 213.
   - L2: **115 de 115** Views válidas dan exactamente el linaje del motor (salida, raíz,
     DIRECT/INDIRECT). Las otras 99 son casos inválidos que `ore view` no sigue; las
     cubre la conformance en el paso 3 (18 de ellas son de flujo: `OOS4001/4002/4011`).
   - Medir corrigió la spec: el motor no deja la arista del `GROUP BY` hacia las propias
     claves, sólo hacia los agregados; y el `HAVING` mira las claves. §5 de
     `01-la-vista-es-sql` lo dice ahora así (una clave no proyectada, hacia todas).
   - L3: 61 usos de la raíz única fuera de `vistas.rs`, en 13 ficheros (`vista.rs` 26,
     `materializar.rs` 7, `flow.rs` 6, `registro.rs` 5, `assets.rs` 5, …).
1. **La spec entra en ORE.** Casos de conformance v1alpha14 en `C:\oos` (válida;
   `OOS1005`; `OOS2038` por `read_parquet`, dos sentencias, `INSERT`; `OOS2039`;
   `OOS4016` con etiqueta y el mismo rango sin ella; `HAVING count(*) >= 8`; `OOS4001`
   por la arista INDIRECT de un `WHERE`). Bump de `vendor/oos`, `ApiVersion::V1Alpha14`,
   claves del spec en `document.rs`.
   **HECHO**: 24 casos en `C:\oos` 510fe10 (9 aceptan, 15 rechazan; README con la tabla),
   `vendor/oos` al día y el marcador `borrador_de_v1alpha14` en `conformance.rs`: **0 / 24**,
   todo pendiente y nada roto. `ApiVersion::V1Alpha14` y las claves de `document.rs` pasan
   al paso 3: aceptar la versión antes de que `comprobar` sepa leer una vista SQL
   convertiría los pendientes en regresiones.
2. **`vista_sql` en ore-core**, pura y sin motor: lo que lee (sin los del `WITH`;
   generadores sí, lectores por función `OOS2038`), lo que proyecta (`*` contra los
   contratos de sus fuentes), linaje por columna (directo, derivado, INDIRECT) y
   predicados clasificados para el canal lateral.
3. **Una sola View en el núcleo.** `vistas.rs` sobre `vista_sql`: `Raiz` pasa a fuentes +
   linaje; la traducción forma→SQL baja a ore-core; `comprobar` (`OOS2018/2019`, `2039`,
   `2011/2022` contra `columns`, `2020`); `flow.rs` propaga por el linaje con varias
   raíces y añade `OOS4016`; se migran los 19 módulos.
4. **Servir.** `ore ask --vista --sql`, `/vistas/…/ejecutar` y `datos_del_puesto` sirven
   `spec.sql` con los nombres resueltos, como una unidad `.sql`, sin `a_sql`. La copia se
   rehace entera. `/v1` `loadView` en `duckdb`; Spark se niega (C).
5. **`CREATE [OR REPLACE] VIEW` en el guion** (ADR 0039): en el puesto DuckDB describe el
   `SELECT` → `columns` → documento en la rama; los códigos OOS vuelven como error de la
   celda; resultado `object/status`. SDK `crear_vista`; `ore view add` y el inductor
   escriben SQL.
6. **Migrar y suprimir.** `ore migrate` reescribe v8–13 → v14 (tipos de sus fuentes) en
   `casos/`, `acme-retail` y el árbol del usuario; lo que no compila se borra. Se quita la
   forma: `cuerpo()` estructurado, emisores de `autoria` y del inductor.
7. **Consola.** `DocumentoView` = `sql/dialect/columns`; la Forge enseña SQL y contrato;
   «As SQL» lee `spec.sql` (fuera `como-sql.ts`); el borrador de vista es un `.sql` con
   `CREATE VIEW`; faceta del catálogo, `ordenDeCampos` y mocks.
8. **Pruebas de fuego y cierre.** Las 15 `.sh` que usan vistas; nueva
   `la-vista-es-sql.sh` de punta a punta (`CREATE VIEW` en el puesto → documento →
   linaje → `OOS4016` → `sql()` → `/v1` → `Entity` con `backedBy` → copia). Cierre.
