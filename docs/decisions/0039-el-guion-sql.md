# 0039 · El guion SQL: varias sentencias en la misma ejecución

**Estado:** en curso (pasos 1 a 5 hechos) · **Fecha:** 2026-09-25 ·
**Decide:** qué es un `.sql` con varias sentencias —`create schema`, `create dataset`, un
`insert`, un `select`— y cómo corre. Sigue a [`0038`](0038-los-tres-niveles.md) (los tres
niveles) y al SQL del árbol (`ore_core::sql_del_arbol`, 0037).

## El problema

En el editor de SQL de Databricks un fichero es un guion: las sentencias corren en orden en la
misma sesión, se para en la primera que falla, y el panel de resultados tiene un selector
(«Result 3 of 4») con lo que dejó cada una. Hasta hoy un `.sql` del árbol era **una**
sentencia. Medido (`pruebas-de-fuego/medida-el-guion-sql.sh`, con el guion de ejemplo de la
persona sobre un puesto de verdad):

- el guion entero, en una celda: se niega («UNA sentencia»);
- cada sentencia en su celda: `create schema` se iba a DuckDB («Catalog "ventas" does not
  exist»), `create table … (cols)` y el `insert` con lista de columnas se negaban;
- `createNamespace` de `/v1` no está (404); `createTable` con columnas desde una celda **sí**
  (crea el `Dataset` en su schema y su puntero, 409 si ya está);
- anexar sobre ese dataset vacío da **500** en `ore-store-r2`: hay que arreglarlo (paso 2);
- DuckDB corre el guion entero de forma nativa, salvo `current_timestamp()` con paréntesis
  (dialecto de Spark).

## La decisión

**Un guion es una lista de unidades.** No hay una semántica nueva de «fichero con varias
salidas»: cada sentencia que lee o escribe datos es la unidad de siempre (lo que lee, lo que
escribe, su modo, su procedencia, su puerta), y cada una que crea algo del catálogo es un verbo
que ya existe. En nuestro vocabulario, no en el de Databricks:

| la frase | es | sobre |
|---|---|---|
| `create [standard] database [if not exists] b` | una standard database vacía | `ore package new` |
| `create standard database b from origin o include (s.t, s.*)` | una standard database sobre un origen: se copia al lago | el alta de `POST /paquetes` |
| `create foreign database b from origin o include (…)` | una foreign database: se lee en el origen | el mismo alta |
| `create schema [if not exists] b.s` | un schema declarado | `ore package schema new` |
| `create dataset [if not exists] b.s.d (col tipo, …)` | un Dataset vacío con su esquema | `createTable` de `/v1` |
| `create or replace dataset b.s.d as select …` | escribe (sobrescribe) | `write()` |
| `insert into b.s.d [(cols)] select …` / `values (…)` | escribe (anexa) | `write()` |
| `insert or replace into b.s.d select …` | escribe (upsert) | `write()` |
| `select …` | lee | `sql()` |

- **Lo que se escribe es un Dataset** (0033), la unidad de almacenamiento del lago. Una
  **Table** es un puntero a un objeto de un origen: nace del descubrimiento y no guarda bytes.
  `create table` se niega con esa frase, y `create or replace table … as` —la forma de antes—
  también: es `create or replace dataset … as`.
- Una base es una **standard** o una **foreign database**, y de donde sale es un **origen**
  (`from origin`); los objetos que entran, `include (schema.objeto, schema.*)`. Una base sobre
  un origen dice su clase; sin origen, es standard.
- **El dialecto es el de DuckDB**, el motor que corre la frase. El de Spark no se traduce (por
  ahora): `current_timestamp()` falla y lo dice DuckDB.

## Los pasos

1. **El guion en el núcleo — HECHO.** `sql_del_arbol::guion` parte el texto con el
   tokenizador (un `;` en una cadena o un comentario no corta) y analiza cada sentencia con lo
   de alrededor en blanco, así que cada fallo lleva la línea y la columna del fichero.
   `dataset` se cambia por `table  ` —el mismo largo— antes de pasar por `sqlparser`.
   `cotejar_guion` coteja en orden: lo que crea la sentencia 1 existe para la 2. `analizar`
   sigue siendo UNA unidad (un trabajo) sobre lo mismo. `ore sql` enseña el guion sentencia a
   sentencia (`sentencias` en `--json`). La semilla de `transforms-sql` (v5) escribe con
   `CREATE OR REPLACE DATASET`.
2. **Cada sentencia sobre su verbo — HECHO.** En la sesión, una celda `sql` con una sentencia
   que crea corre como el verbo del SDK (`crear_base`, `crear_schema`, `crear_dataset`), con lo
   que la frase dice (`celda_de_sentencia`), en nombre de quien abrió el puesto y en su rama:
   - `create schema` → **`createNamespace` de `/v1`** (nuevo: `POST /v1/{base}/namespaces`, el
     que llamarán Spark y DuckDB) → `ore package schema new`; 409 si ya está.
   - `create [standard] database b` → el alta `POST /paquetes` **sin origen** (nuevo): `ore
     package new`, con el dueño de las bases. Con origen, el alta de siempre, e `include (s.*)`
     se expande contra el catálogo del origen. Desde un puesto entra por la puerta del agente.
   - Una base sin `discover.*` (sin origen) es **standard** en el índice y en `/paquetes`: lo que
     tenga sólo vive en el lago. Antes, sin nada que lo dijera, era `foreign`.
   - `create dataset (cols)` → `createTable` de `/v1` (ya estaba).
   - **Anexar sobre un dataset vacío** daba 500: `escribir` pasaba sus `requirements` por el
     `Json` del núcleo, que no modela `null`, y el `assert-ref-snapshot-id` de una tabla sin
     snapshot llegaba como la cadena `"null"`. Ahora van tal cual (`ore-store`).
   - **Un `insert` lleva cada valor al tipo de su columna**, como en SQL (`_como_la_tabla`):
     `current_timestamp` es TIMESTAMPTZ y la columna del ejemplo, TIMESTAMP. Un `create or
     replace` no: sus tipos son los de su consulta. Lo que no convierte sin perder, lo dice
     `write()` como siempre.
   Medido en `medida-el-guion-sql.sh` G7, sentencia a sentencia sobre un puesto de verdad.
3. **Un guion es un lote de celdas en el puesto — HECHO.** `POST /puestos/{id}/ejecutar` con
   un `.sql` de varias sentencias que cotejan en orden encola **una celda por sentencia**,
   seguidas, cada una con su `lote` (`{primera, i, n}`, también en su ficha), y contesta con
   `celdas` y `sentencias` (`{celda, que, texto, linea, columna}`): lo que la consola necesita
   para el selector. El agente las corre de una en una en la misma sesión, así que lo que crea
   o escribe una lo ve la siguiente.
   - **Se para en el primer error**, como Databricks: cuando una celda del lote sale con
     `error`, las de detrás salen de la cola con `{tipo: vacia, saltada: true, por: <la que
     falló>}`. Lo que ya corrió, corrió: no hay transacción entre sentencias.
   - **Un guion que no coteja no corre nada** —ni la sentencia 1—: una celda de error con los
     diagnósticos de todas, en su línea. Mejor que dejarlo a medias sabiendo que falla.
   - Lo que no es del árbol (`create schema tmp; create table tmp.t …`) sigue yendo **entero**
     a DuckDB, como siempre: el guion es para lo que el árbol sabe correr.
   - Una sola sentencia, como antes.
   Probado en `el-puesto.sh` 10d con el guion del ejemplo.
4. **Todo resultado es una tabla, como en Databricks — HECHO.** Medido antes (G8): cada
   sentencia que escribía o creaba dejaba un TEXTO, y el `insert` de una fila decía «3
   filas» —el total del dataset, no lo insertado—; un `insert or replace` no tenía clave que
   usar (desde SQL no había cómo declararla); la consola no sabía de `saltada`.
   | la sentencia | su resultado (una fila) |
   |---|---|
   | `select` | su tabla |
   | `insert`, `create or replace dataset … as` | `num_affected_rows`, `num_inserted_rows`: las filas que llegaron |
   | `insert or replace` (upsert) | `num_affected_rows`, `num_updated_rows`, `num_inserted_rows` (como el `MERGE` de Databricks) |
   | `create … database`, `create schema`, `create dataset` | `object`, `status` (`created` · `already exists`) |
   - `ore-store escribir` cuenta las filas que llegan (`anadidas`) y las que había (`antes`), y
     `write()` las devuelve; `filas` sigue siendo el total. Actualizadas = antes + llegan −
     después (el upsert es copy-on-write). La misma escritura otra vez: ceros.
   - **`primary key` en `create dataset`** (de la tabla, `primary key (id)`, o de la columna,
     `id bigint primary key`) declara la clave del dataset (`ore.clave`, lo que
     `write(modo="upsert", clave=…)` ya guardaba). Es la que usa un `insert or replace`.
   - El texto de antes (`hr.x · anexar · 6 filas`) sigue en la salida, bajo la tabla.
   Probado en `el-puesto.sh` 10c y 10d.
5. **La consola: «Result i of N» — HECHO** (rubix-platform). `ejecutarCelda` trae el lote
   (`celdas`, `sentencias`); `correr` (`puesto.tsx`) espera cada celda EN ORDEN y da cada
   resultado en cuanto llega (`alLote`), y devuelve la salida de la que falló —o de la
   última— con el lote entero. En el panel de resultados, una ejecución con varias sentencias
   lleva `resultados`, y encima del cuerpo hay un **selector**: flechas, un desplegable
   «Result 3 of 4 · insert» (con `· failed`, `· skipped`, `· running`), la sentencia en una
   línea y «Line N». Cada resultado se pinta como una ejecución suelta —la tabla con su filtro
   y su CSV, el rechazo con «Go to line» a SU sentencia—, y el que no corrió dice «Skipped:
   statement 2 failed» con un botón a ella. Si nadie elige, se ve el que falló, el que corre,
   o el último.
6. Pruebas de fuego (el guion del ejemplo en `el-puesto.sh`) y el cierre de este ADR.
7. Después: un guion como trabajo; `main`, `memory`, `system` y `temp` negados como nombres de
   base (DuckDB los reserva); `update`, `delete`, `merge`, `drop`.
