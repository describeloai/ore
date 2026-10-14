# La idempotencia de las escrituras — diseño (2026-10-14)

Documento de trabajo, desechable: cuando esté hecho, la regla pasa al ADR
[0031](decisions/0031-el-puesto.md) (W3.6c, «la clave de operación») y este fichero se borra.
Origen: el fallo A → B → A medido en M0·d (ADR [0064](decisions/0064-notebooks.md)).

## El fallo

`write()` (Python, Node, JVM) pierde escrituras en silencio. Lo medido en victor
(`sandbox.m0d_marimo`), fuera de cualquier cuaderno:

```
write(1000) → repeated=False · la tabla tiene 1000 ✔
write(2000) → repeated=False · la tabla tiene 2000 ✔
write(1000) → repeated=True  · la tabla tiene 2000 ✘  perdida
write(3000) → repeated=False · la tabla tiene 3000 ✔
write(1000) → repeated=True  · la tabla tiene 3000 ✘  perdida
```

**Por qué pasa:**

- La clave de operación es **un hash del contenido** (`operacion: "contenido"` → `ore-store
  escribir` → `sha256(semilla · carga::huella)`, con la semilla `nombre|modo`). Va al resumen del
  snapshot como `ore.operacion`.
- El catálogo la busca **en todos los snapshots de la historia** ([`datasets.rs`](../crates/ore-cli/src/datasets.rs),
  «la clave de operación en la ancestría») y, si aparece en cualquiera, contesta «repetida» sin
  aplicar nada.
- Confunde **«lo acabo de escribir»** (un reintento, una celda reejecutada) con **«lo escribí
  alguna vez»** (volver a un estado anterior, que es una escritura legítima).

**A quién afecta:** a todo lo que escribe por `write()`, que es la sesión, los cuadernos, Build y
los trabajos. Con `append`, anexar un lote igual a uno de hace semanas también se descarta. En una
rama, la historia incluye la de `main`.

## Cómo lo resuelve el estándar

Nadie deduplica por contenido. Se separan tres problemas:

| Problema | Herramienta | Quién la usa |
|---|---|---|
| Dos escritores a la vez | concurrencia optimista: `requirements` (`assert-ref-snapshot-id`) → 409, refrescar y reconstruir | la spec REST de Iceberg; **ya lo tenemos** (código 75) |
| El reintento de la **misma petición** (red, 5xx, un 503 de un proxy **después** de que el commit entró) | **`Idempotency-Key`**: el cliente genera **un UUIDv7 por operación lógica**, lo reutiliza **sólo** en sus reintentos y genera otro para cualquier otra operación. El servidor, si esa clave ya se completó, **devuelve una respuesta equivalente sin ejecutar nada**. El catálogo anuncia `idempotency-key-lifetime` (p. ej. `PT30M`) en `GET /v1/config`. | la spec REST de Iceberg (`updateTable`, `commitTransaction`, `createTable` y 15 más); Apache Polaris 1.7 |
| Exactamente una vez **en un pipeline** (reejecutar el mismo lote) | quién escribe + una secuencia creciente en el commit | Delta (`txnAppId`/`txnVersion`, acción `SetTransaction`); Flink→Iceberg (`flink.job-id`, `flink.max-committed-checkpoint-id` en el resumen) |

La lección: **la identidad de una operación la pone quien escribe, no los datos**. Escribir A otra
vez después de B es una operación nueva.

## El diseño

**La idea.** Separar lo que hoy va mezclado en una clave:

| | Qué es | Dónde vive | Con qué se compara |
|---|---|---|---|
| **la petición** | `Idempotency-Key`, un UUIDv7 por commit que el cliente construye | cabecera HTTP; **ore-serve** la escribe en el resumen del snapshot como `ore.idempotency-key` | con los snapshots **de la ventana** (`idempotency-key-lifetime`): como es un UUID, no reaparece nunca |
| **el contenido** | `ore.operacion`, la huella de hoy (sin cambios) | el resumen del snapshot, como hoy | **sólo con el snapshot vigente**, y **sólo en `overwrite`**: sobrescribir con lo mismo que la tabla ya tiene no deja snapshot. **No es idempotencia: es «nada que hacer»** |

**Cómo se comporta:**

| Caso | Hoy | Con el diseño |
|---|---|---|
| A → B → A (`overwrite`) | la tercera se pierde | **tres snapshots** |
| La misma celda reejecutada sin cambios (`overwrite`) | `repeated: true`, sin snapshot | igual: el contenido es el del vigente → `repeated: true` (sigue verde `el-puesto.sh` 10 y 8) |
| Reintento tras 5xx o 503 cuando el commit sí entró | el SDK hace un `GET` y compara el vigente (y si otro escribió después, falla) | el SDK **reenvía la misma petición con la misma clave**; ore-serve la encuentra y devuelve la respuesta equivalente; **sin snapshot de más** |
| 409 (otro escribió antes) | refrescar y reconstruir | igual, **con una clave nueva** (es otro commit: otra base) |
| `append` del mismo lote dos veces, a propósito | la segunda se pierde | **dos snapshots** (el contenido no se coteja nunca en `append`) |
| Reintento de un `append` que sí entró | `GET` y comparar | la clave lo encuentra: sin duplicado |
| Un cliente estándar (PyIceberg, Spark) que manda `Idempotency-Key` | se ignora | **funciona igual** |
| Un SDK viejo (sin cabecera) contra el servidor nuevo | — | sin el fallo (el contenido sólo se coteja con el vigente) y con los reintentos de antes |

**`repeated` significa** «la tabla ya tenía exactamente esto: no hay snapshot nuevo». Una
repetición por clave devuelve **lo que contestó la primera vez** (un éxito), no `repeated: true`.

**Anunciar en `/v1/config`.** La spec dice que, si el catálogo anuncia `idempotency-key-lifetime`,
los clientes asumen idempotencia en **las 18 operaciones** que llevan la cabecera. Nosotros
sólo escribimos tablas por `updateTable`, `commitTransaction` y `createTable`, así que hay dos
caminos:
- **honrar la cabecera sin anunciarla** hasta cubrir todas las que sirvamos;
- **anunciarla ya** con las que de verdad tenemos.

Propongo la primera: nuestro SDK la manda igual, y anunciarla es una promesa a clientes ajenos.

## Los pasos

| # | Qué | Dónde | Prueba |
|---|---|---|---|
| **I1** ✓ 2026-10-14 | **El catálogo**. La comprobación de la ancestría se sustituye por dos: (1) la clave de la petición en los snapshots de la ventana → respuesta equivalente con el estado vigente; (2) en `overwrite`, `ore.operacion` de la petición = la del **snapshot vigente** → nada que hacer. Al aplicar, el servidor escribe `ore.idempotency-key` en el resumen del snapshot nuevo. `historia` gana la clave y la hora de cada snapshot. | `ore-cli/src/datasets.rs`, `ore-store/src/ciclo.rs` (`historia`) | `cargo test`: A → B → A deja 3 snapshots; la misma clave dos veces deja 1 y contesta igual; `overwrite` igual al vigente → sin snapshot; `append` igual → snapshot; una clave fuera de la ventana no se honra; un snapshot viejo con `ore.operacion` en la historia ya no bloquea |
| **I2** ✓ 2026-10-14 | **ore-serve**. Lee `Idempotency-Key` en `POST …/tables/{t}`, `POST …/tables` y `POST /transactions/commit`, comprueba la forma (36 caracteres; si no la tiene → 400) y la pasa al catálogo. La ventana es una constante (`PT30M`). Se quita el «no entra (todavía)» del encabezado de `catalogo.rs`. | `ore-serve/src/catalogo.rs` | `cargo test` de la ruta: con cabecera, sin ella, mal formada |
| **I3** ✓ 2026-10-14 | **Los tres SDK**. Un UUIDv7 por commit construido (`uuid.uuid7()` en Python 3.14; generador propio de 20 líneas en Node y en Java). Tras un 5xx o un corte, **reenviar la misma petición** con la misma clave (3 veces, con espera). Tras un 409, reconstruir con una clave nueva. Se quita el «`GET` y comparar». `repeated` sólo cuando el servidor contesta con la misma base. | `puesto/python/ore/__init__.py`, `puesto/node/ore/index.mjs`, `puesto/jvm/ore/Ore.java` | `el-puesto.sh` 10, 8 y 9 siguen verdes, y ganan A → B → A en los tres lenguajes |
| **I4** | **En producción**. Desplegar primero el servidor (I1 + I2 sirven a los SDK viejos) y después las imágenes del puesto. Repetir en victor la secuencia de `sandbox.m0d_marimo` y un reintento forzado. | malla (las imágenes), victor | 1000 → 2000 → 1000 → 3000 → 1000: la tabla tiene cada vez lo escrito |
| **I5** | **La regla, escrita**: el ADR 0031 (W3.6c) con la decisión y las fuentes, `docs/sdk.md` (qué significa `repeated`), y este fichero se borra. | docs | — |

**I1, hecho (2026-10-14).** `ore datasets --commit --idempotencia <uuid>`. El catálogo decide con
`ya_esta()`, una función pura y probada sin lago, sobre la historia: `Hecha` (la clave, en la
ventana de 35 min = 30 + 5 de gracia), `SinCambios` (la huella del snapshot vigente) o `Nada`.
`ore-store aplicar` escribe la clave en el resumen del snapshot (`ore.idempotency-key`), e
`historia` la devuelve (`clave`, y también `modo`). La salida gana `reintento` junto a `repetida`.
La historia se mira **antes** del `assert-create`: el reintento de una tabla que nació en el primer
intento es un reintento, no un conflicto.

**Un ajuste al construir.** `ore.modo` sólo se escribe en `upsert`. Si una escritura anexa se sabe
por `operation: append` en el resumen del snapshot, que la spec exige siempre.

**Pruebas.**
- `cargo test`: `volver_a_un_contenido_de_antes_es_escribir`,
  `la_misma_peticion_es_un_reintento_en_su_ventana` y `una_idempotency_key_es_un_uuid`.
- `el-lago.sh` paso **8c**, contra el catálogo y el almacén de verdad: A → B → A escribe la
  tercera; lo vigente → `repetida` sin snapshot; la misma clave anexando → `reintento` sin
  snapshot; sin clave → 75; una clave mal formada → 64; la ficha enseña la clave.
- `el-lago.sh` entero en verde (0–15).

**I2, hecho (2026-10-14).** ore-serve lee `Idempotency-Key` en `updateTable` y `commitTransaction`
(`idempotencia_de`). Si no tiene forma de UUID → **400 `BadRequestException`**, y nada llega a la
línea de órdenes. Si la tiene, la pasa como **un solo argumento** (`--idempotencia=<uuid>`), para que
ni una forma válida pueda leerse como otra opción. La forma la comparten ore-serve y `ore`
(`ore_core::idempotencia::bien_formada`).

**`createTable` sin `stage-create`, fuera.** Nuestro SDK crea con `stage-create` + commit, que ya
está cubierto, y PyIceberg y DuckDB sólo mandarían la cabecera si la anunciáramos. Entra cuando se
anuncie. Pruebas:
- `cargo test`: `la_idempotency_key_se_mira_antes_de_pasarla`;
- `el-lago.sh` paso **13c**: dos `append` de PyIceberg con la misma clave por `/v1` anexan una vez
  (15 filas y no 20), la clave queda en el snapshot, y una cabecera que no es un UUID da 400;
- `el-lago.sh` entero en verde.

**I3, hecho (2026-10-14).**
- **Python, Node y Java** generan un UUIDv7 por commit (`_uuid7`, `uuid7()`), lo mandan en
  `Idempotency-Key` y, ante un 5xx o un corte, reenvían la misma petición con la misma clave
  (`_confirmar`/`confirmar`: 3 veces más, a 0,5, 1 y 2 s). Tras un 409, reconstruyen con otra clave.
  El «`GET` y comparar» desaparece.
- **Python**: `uuid.uuid7()` sólo existe desde 3.14 (la imagen); fuera de ella —el CI corre con el
  Python del sistema— se construye a mano.
- **`el-puesto.sh`** gana A → B → A en los tres lenguajes (casos 10, 8 y 9).
- **Ese caso destapó un fallo anterior**: un ciclo en `derivedFrom` (leer A, escribir B, leer B,
  escribir A) hundía ore-serve por la pila. Arreglado aparte (`02b5b7b7`, `flow.rs` `CARGANDO`).

**Lo que queda fuera:** la idempotencia de **pipelines** (quién escribe + secuencia, al estilo
Delta y Flink). Es para Build y los trabajos programados (0055), y va aparte. Tampoco se toca
`createNamespace`, `dropTable` ni las vistas.

**El orden de despliegue importa** (lección de 2026-09-27: binario antes que malla). El servidor
nuevo es compatible con los SDK viejos, así que va primero. El CI construye los dos desde `main`
y el puesto toma la imagen al arrancar (`Always`): los puestos abiertos siguen con el SDK viejo
hasta que se cierran, y no pasa nada.

## Fuentes

- Iceberg REST catalog OpenAPI — `Idempotency-Key`, `idempotency-key-lifetime`:
  https://raw.githubusercontent.com/apache/iceberg/main/open-api/rest-catalog-open-api.yaml
- Apache Polaris 1.7.0 (idempotencia en `createTable` y `updateTable`):
  https://datalakehousehub.com/blog/apache-polaris-1-7-0
- Lista de desarrollo de Iceberg (la propuesta y el 503 de un proxy tras un commit que entró):
  https://amdatalakehouse.substack.com/p/apache-iceberg-dev-mailing-list-weekly-d25
- Delta Lake, escrituras idempotentes: https://docs.delta.io/kernel/rust/writing/idempotent_writes
- Flink → Iceberg: https://www.mintlify.com/apache/iceberg/engines/flink/writes
