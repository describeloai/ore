# 0043 · El flujo es Arrow: del driver al lago sin pasar por la memoria

**Estado:** decidido y hecho (2026-09-26, Fase B de BigQuery: B1 y B2) · **Decide:** en qué forma
viajan las filas de la fase ③ y quién las tiene en memoria. Amplía
[`0008`](0008-el-protocolo-del-driver.md) (la petición gana `formato`) y
[`0015`](0015-el-protocolo-del-almacen.md) (el almacén gana `sellar-flujo`). Sigue a
[`0042`](0042-bigquery-por-rest.md).

## Lo medido

Con una tabla de BigQuery de 2 M de filas y 5 columnas (`ventas.ore_e2e_sintetica`), desde casa
(~7 MB/s de línea):

| | tiempo | pico de memoria |
|---|---|---|
| `materialize` en texto (hasta hoy) | 180 s | `ore-store` **2 017 MB** · driver 600 MB · `ore` 465 MB |
| el driver solo, por `jobs.query` | 157 s | 600 MB |
| el driver solo, por Storage Read (Arrow, 1 stream) | 21,7 s | 12 MB |

El muro no era BigQuery: era **el camino**. `ore` guardaba toda la salida del driver en un
`String` antes de pasársela al almacén, y el almacén la analizaba entera a filas de texto antes
de tiparla. La memoria crecía con la tabla: hacia los 10 M de filas un pod no llega.

La pila gRPC se midió aparte: solo compila sin compilador de C `tonic` **sin** sus features de
TLS sobre `hyper-tls` (`native-tls`, la misma pila que `ureq`); con `ring` o `aws-lc-sys` hace
falta `gcc`. El código de la API viene generado (`googleapis-tonic-…`), sin `protoc`.

## Lo decidido

1. **La petición lleva `formato: arrow`**, y es una **preferencia**. Un driver que sabe contesta
   un flujo Arrow IPC por stdout, con los nombres de las **propiedades** (no de las columnas).
   Uno que no sabe —o que para esta petición no puede— contesta en texto como siempre. Quien lee
   distingue por los cuatro primeros bytes: `0xFFFFFFFF` es IPC, `{` es una fila.
2. **`ore` no lee el flujo: lo encauza.** La salida del driver va a `ore-store sellar-flujo` por
   un pipe, con la cabecera delante. Si el driver falla, el almacén se corta; si el almacén falla
   (un lote que no casa con el contrato), el driver se corta.
3. **El flujo solo se sella con su marca de fin.** `StreamReader` trata un corte limpio entre dos
   mensajes como el final. Un driver que muere a mitad dejaría una tabla corta que se sellaría
   entera. `sellar-flujo` exige los ocho bytes de la marca que escribe `finish()`; sin ellos, no
   hay snapshot.
4. **Al contrato, sin modo seguro** (`carga::al_contrato`): una columna que falta o sobra se dice
   con su nombre, una conversión que falla es un error y no un nulo, y **un decimal que estrecha
   se convierte y se vuelve a convertir**: si no sale idéntico, se niega. La Storage Read da todo
   NUMERIC como `decimal(38, 9)`, y un contrato `Decimal<10, 2>` es legítimo; el cast de Arrow
   redondearía `0.005` sin decirlo.
5. **Se escribe lote a lote** (`Lago::instantanea_flujo`): cada lote va al Parquet en curso y se
   suelta, y un fichero se sube al llegar a **128 MiB** (antes 512: cada fichero se acumula entero
   antes de subirse, y eso es lo que cuesta en memoria una copia de cualquier tamaño). Fundir por
   clave sí necesita lo que había: ese camino sigue siendo el de siempre, sobre un incremento.
6. **BigQuery lee las tablas por la Storage Read API**, con la proyección y el filtro empujados
   (el `WHERE` de `ore-sql`, con sus parámetros escritos como literales **validados por tipo**: la
   API no admite parámetros) y hasta 8 streams en paralelo con LZ4. **Declina a REST** —y lo dice
   por stderr— cuando no puede servir lo mismo: una vista, una columna `STRUCT`/`ARRAY`/
   `BIGNUMERIC`/`BYTES`/`JSON`/`GEOGRAPHY`, un literal que no se sabe escribir sin riesgo, o una
   cuenta sin `bigquery.readsessions.create`. Declinar pasa antes de escribir un byte.

## Resultado

| 2 M de filas | antes | ahora |
|---|---|---|
| `materialize` | 180 s | **17,9 s** |
| `ore-store` | 2 017 MB | **90 MB** |
| driver | 600 MB | **12 MB** |
| `ore` | 465 MB | **6 MB** |

Mismas cuentas por columna que el camino de texto (1 714 286 no nulos en `nombre`).
`bigquery-real.sh` entero en verde por el camino nuevo. `ore` no gana ni un crate (guardián de
dependencias); `ore-read-bigquery` gana unos 140.

## Lo que queda fuera

- **Postgres y JSONL siguen en texto** (y con el texto en memoria de `ore`). Emitir Arrow desde
  ellos es el mismo contrato; se hará cuando se mida una tabla grande de Postgres.
- **La cuenta del driver en el clúster necesita `roles/bigquery.readSessionUser`** para la
  Storage Read. Sin él funciona, pero declina a REST: la vía lenta.
- **No está compilado en musl**: razonado desde los build scripts (el único C es `openssl-sys`, el
  mismo que ya se enlaza). Lo comprueba la imagen.
