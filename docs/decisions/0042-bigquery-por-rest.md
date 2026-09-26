# 0042 · BigQuery por REST: el driver habla la API, y el catálogo vive en él

**Estado:** decidido y hecho (2026-09-26, Fase A: A0–A5 en `main`) · **Decide:** cómo lee
`ore-read-bigquery`, dónde vive su catálogo, qué credencial lleva y cuál es su frontera. Sigue a
[`0008`](0008-el-protocolo-del-driver.md) (el protocolo del driver) y
[`0032`](0032-el-contrato-de-tipos.md) (el contrato de tipos, T5: `Decimal<p, s>`). Vara:
`pruebas-de-fuego/bigquery-real.sh`.

## Lo medido

Contra un dataset de verdad (`ventas`, semilla acotada `ore-e2e-*`, 5 + 8 filas):

- **El CLI `bq` perdía en silencio.** Su salida `prettyjson` convertía el texto `'null'` en NULL,
  quitaba los microsegundos de un TIMESTAMP y lo daba sin zona, así que la columna acababa como
  `string` en Iceberg mientras la cabecera decía `DateTimeTz`. Cada llamada tardaba 8–11 s en
  arrancar el intérprete de Python de `bq`.
- **REST no pierde nada** si se le pide bien: `jobs.query` tarda 0,55–0,9 s, NULL es `null` de
  JSON (distinto de `"null"`), NUMERIC llega como texto exacto y TIMESTAMP es exacto **solo** con
  `formatOptions.useInt64Timestamp` (sin él, un float que pierde µs). Las páginas van por
  `pageToken` a ~0,5 s cada una.
- **El catálogo vivía en `ore`**, que es una imagen `scratch` sin TLS (0008, y el guardián
  `ore-cli/tests/dependencias.rs`). Por REST no puede seguir ahí.
- **BigQuery no aplica Credential Access Boundaries**: un token acotado por STS —a GCS o a
  BigQuery— sigue consultando cualquier tabla que la cuenta pueda leer. Acotar el token no es
  una frontera.
- **La cuenta del driver del clúster (`ore-driver-demo`) no tiene hoy ningún rol de BigQuery**:
  ni en el proyecto ni en la ACL del dataset `ventas` (medido 2026-09-26). En la malla, el driver
  no podría leer BigQuery.
- Escala (para lo que viene, no para esta decisión): `tabledata.list` es gratis y lo limita el
  servidor (~3,5 k filas/s por petición, escala con hilos); la Storage Read API da Arrow con el
  físico exacto de 0032 y la limita la línea.

## Lo decidido

1. **D1 · El driver habla REST, sin `bq`.** `jobs.query` con `useInt64Timestamp`,
   `JOB_CREATION_OPTIONAL` y paginación por `pageToken`; lo contado tiene que ser el `totalRows`
   del servidor o la lectura se niega (una página perdida no es una tabla más corta). Los valores
   se traducen en `valores.rs` desde `serde_json`, no desde el analizador de `ore-core`, porque
   aquí importa distinguir `null` de `"null"`.
2. **D2 · El catálogo vive en el driver**: el verbo `catalogo` de 0008 (URL por stdin, nombre de
   la fuente por argumento). `ore` deja de traer recetas de ningún origen: ejecuta el driver.
3. **La credencial es la de la cuenta que corre**, en una sola crate (`ore-gcp`): del servidor de
   metadatos (Workload Identity) o de `ORE_GCP_TOKEN` (alias `ORE_GCS_TOKEN`), **renovada antes
   de caducar** (`expires_in`). La comparten `ore-read-bigquery` y `ore-store-gcs`. Antes vivía en
   el proceso de `bq`, en el mismo pod; ahora vive en este. No se pierde nada que hubiera.
4. **La frontera es el IAM de la cuenta del driver, y solo eso.** Como CAB no se aplica, lo que
   la cuenta puede leer es lo que el driver puede leer. El mínimo:
   - `roles/bigquery.jobUser` en el proyecto que **paga** (crear el job de la consulta);
   - `roles/bigquery.dataViewer` **en cada dataset** que la celda declara como origen, nunca en
     el proyecto.
   Un proyecto que paga distinto del de los datos funciona (medido), así que la factura puede
   ser del inquilino.
5. **Tipos (0032 T5)**: NUMERIC → `Decimal<38, 9>` (con `(P, S)`, los suyos); BIGNUMERIC sin
   precisión o con más de 38 cifras → `String` exacto (no cabe en el decimal de Iceberg);
   TIMESTAMP → `DateTimeTz` en UTC; `ARRAY<escalar>` → `list<…>`. Un STRUCT no tiene tipo en OOS y
   el catálogo lo **dice** en vez de inventarlo; su valor viaja como JSON con los nombres de sus
   campos.
6. **Una lectura de 0 filas es una tabla de 0 filas** (A5): con su esquema, su cabecera y su
   puntero. Antes era «nada que escribir» en cada pasada. Y los filtros viajan con la forma de
   0008 (`{columna, operador, valor}`): los triples de `materialize` solo los leía
   `ore-store copiar`, así que toda copia con `where` desde un origen externo fallaba.

## Lo que queda fuera

- **REQUIRED → `required` en Iceberg**: Fase B. Pide gramática en OOS, análisis de nulabilidad en
  las vistas, y que Iceberg no deja endurecer una tabla que ya existe.
- **La Storage Read API** (Arrow, en paralelo): cuando una tabla no quepa en el tiempo de un Job.
- **Conceder los roles del punto 4 a `ore-driver-*`**: es un cambio de IAM y se hace a mano, por
  inquilino y por dataset. Hasta entonces BigQuery solo se ha ejercido fuera del clúster.
- **La imagen `drivers` sigue sobre el SDK de Google Cloud**, pero ya no por `bq`: la usa
  también el Job del aprovisionador, que necesita `gcloud`. Separarlas daría a los drivers una
  base `alpine` con las CA; queda anotado, no decidido.

## Aceptación

`BQ_URL=bigquery://<proyecto>/ventas bash pruebas-de-fuego/bigquery-real.sh` todo en verde
(2026-09-26): 13/13 filas exactamente iguales a la semilla (el texto `'null'`, los µs, el
instante UTC, el NUMERIC de 29 cifras, NULL, UTF-8), `decimal(38, 9)` y `timestamptz` en
Iceberg, un dataset con `where` sobre BigQuery (2 filas) y uno vacío (0 filas, con esquema).
`materialize` pasa de 45 s a 3 s.
