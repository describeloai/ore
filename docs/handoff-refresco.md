# Handoff · del poblar al mantener

> **Desechable.** Cuando sus peldaños estén, este documento sobra: lo que decidan vive en
> [ADR 0016](decisions/0016-el-testigo-y-el-rango.md) y en la especificación.

`ore materialize` corre entero —[ADR 0015](decisions/0015-el-protocolo-del-almacen.md), y el ciclo
está medido contra un R2 de verdad— y aun así **la copia no se refresca nunca**. Este plan cierra
esa distancia.

Lo que hay que hacer sale de [ADR 0016](decisions/0016-el-testigo-y-el-rango.md), que no lo
razonó desde cero: lo leyó en Debezium, Iceberg, Delta, Snowflake, BigQuery y Airbyte, y los seis
coinciden. Aquí solo se ordena en peldaños.

---

## 1. El problema, medido

Tres cosas, y **la primera no la vio nadie hasta que se fue a mirar fuera**.

### 1.1 · Hoy el árbol recomienda incrementar lo que se duplica

Un paquete con una tabla `{ mode: append, witness: field }` y una vista `materialized`:

```
ore validate   ok · sin errores
ore view       caras     changes: append · witness: field · field: ocurrio_en
               refresco  REFRESH_MODE = INCREMENTAL
               flujo     materializada · `materialization.payload` compila
```

**Compila, y además se recomienda incrementar.** Y esa pareja es exactamente la que Airbyte
documenta —para su *cursor field*, que es el mismo mecanismo— como **at-least-once**: el solape se
re-entrega en cada refresco, y sin clave con la que deduplicar **se acumula para siempre**. Airbyte
además admite que se pueden **perder** filas si la columna no se mantiene.

No hace falta gramática nueva para cerrarlo: la pareja `(mode, witness)` **ya está declarada**.

### 1.2 · El testigo va vacío, así que la cabecera nunca cambia

El valor del testigo lo escribe hoy una persona —`ore-exec/src/main.rs:305`, `--marca`— y en el
ciclo va vacío. Consecuencia: la cabecera del sobre es **idéntica** cada vez, el recibo dice *«ya
está»*, y no se vuelve a leer el origen. `ore materialize` puebla **una vez**.

### 1.3 · El horizonte está declarado y nadie lo mira

`changes.retention` existe en v1alpha8 desde el primer día, diciendo *«cuánto guarda el origen su
changelog, si se sabe. Informativo: quien planifique un refresco lo usa para saber si puede llegar
tarde»*. **Ningún consumidor lo lee.** Es el `STALE_AFTER` de Snowflake, ya declarable.

---

## 2. Qué se toca y qué no

| pieza | hoy | después |
|---|---|---|
| `ore-core` · forma de `Table` | admite cualquier `(mode, witness)` | **rechaza** la pareja que no puede mantenerse |
| `ore-view/refresh_analyzer` | ve la cara `D` desde I3 | ve además la **garantía**, no solo el modo |
| `ore-driver` · `Peticion` | `url, objeto, proyeccion, claves, filtros` | **+ `desde`, `hasta`** |
| `ore-read-<tipo>` | dos verbos | **+ `testigo`**, y honrar o negarse ante un rango |
| `ore-cli/materializar` | el testigo va vacío | lo pregunta, y refresca |
| `ore-cli/registro` | `Testigo { marca, valor: None }` | el valor deja de ser `None` |
| `ore-exec/topologia` | marca propia en su cabecera | **el mismo campo** que el sobre |
| la gramática | — | **nada nuevo**: `mode`, `witness` y `retention` ya están |

La última fila es la que hace que esto entre: **lo que hay que declarar ya se puede declarar.** Lo
que falta es leerlo.

---

## 3. Los peldaños

> Cada uno dice qué es y **cuándo está listo** con algo que se puede medir.

| | qué | dónde | cuesta |
|---|---|---|---|
| **R0** | la pareja `(mode, witness)` decide la garantía | `ore-core`, `ore-view` | nada de protocolo |
| **R1** | `retention` deja de ser decorativa | `ore-core`, `ore-cli` | nada de protocolo |
| **R2** | el verbo `testigo` — **0016 A** | `ore-driver`, los lectores, el ciclo | enmienda a 0008 |
| **R3** | `desde` / `hasta` — **0016 B** | los mismos | enmienda a 0008 |
| **R4** | un solo sitio para el testigo | `ore-exec`, el sobre | ninguno |
| **R5** | la recogida de basura | `ore-store-r2`, el registro | ninguno |

**R0 y R1 no dependen de R2 ni de R3**, y cierran agujeros que están abiertos **hoy**. Van primero
por eso, no por ser fáciles.

### R0 · la pareja decide la garantía

**Qué.** `(mode, witness)` pasa a tener una lectura normativa, y una de las cuatro combinaciones se
rechaza:

| `witness` | `mode` | garantía | veredicto |
|---|---|---|---|
| `snapshot` | cualquiera | exacta | ✅ |
| `log` | cualquiera | exacta, con el solape re-entregado | ✅ |
| `field` | `upsert` / `retract` | at-least-once, **idempotente por clave** | ✅ |
| **`field`** | **`append`** | at-least-once **sin con qué deduplicar** | **❌** |

El rechazo es sobre **materializar**, no sobre la tabla: una tabla `{field, append}` es legal y
existe —un log de eventos con una columna de tiempo es eso— y lo que no se puede es **mantener una
copia suya incrementalmente**. Cae al lado de `OOS2020` y `OOS2021`, que son las otras dos reglas
que miran las dos caras a la vez.

**Y el analizador de refresco deja de mentir.** Hoy dice `INCREMENTAL` de esa pareja; después dice
`FULL`, con el motivo — que es la forma que `refresh_analyzer` ya tiene de contestar.

**Listo cuando.** El caso de §1.1 **no compila**, con un código propio y un mensaje que nombra la
pareja; y una prueba comprueba que `{field, upsert}` sí compila, para que el rechazo sea de la
combinación y no del modo.

**No hace.** No toca la gramática. `mode`, `witness` y `key` ya están declarados.

### R1 · `retention` deja de ser decorativa

**Qué.** Un consumidor para el campo que lleva desde v1alpha8 sin ninguno, y el código que falta:
*tu testigo cayó fuera de la retención; esta copia no se puede refrescar incrementalmente, hay que
rehacerla entera.*

Es literalmente el `STALE` de Snowflake y el re-snapshot de Debezium, y los cuatro sistemas que
0016 miró hacen lo mismo: **se niegan en voz alta en vez de degradar en silencio a un recorrido
completo.** Degradar en silencio es lo caro: una copia que se rehace entera sin decirlo es una
factura que aparece sin explicación.

**Listo cuando.** `ore view` dice, por copia, si su testigo cabe en la retención declarada; una
copia fuera de plazo se marca y se dice qué hacer; y **la ausencia de `retention` no se inventa** —
sin dato, no se afirma nada, que es lo que el propio campo pide.

### R2 · el verbo `testigo` · [0016](decisions/0016-el-testigo-y-el-rango.md) A

**Qué.** El tercer verbo. `ore-read-postgres` lo contesta con `pg_current_wal_lsn()`;
`ore-read-jsonl` **se niega** —un directorio no tiene versiones, y `none` es la respuesta cierta—;
la receta de BigQuery lo saca del historial de cambios cuando está encendido.

Y el ciclo lo usa: el paso ③ deja de ir vacío, la cabecera cambia cuando el origen cambia, y el
recibo deja de decir *«ya está»* para siempre.

**Listo cuando.** Dos `ore materialize` con el origen movido entre medias producen **dos**
artefactos distintos, el segundo con el testigo mayor — y un tercero sin mover nada no sube ni un
byte. Es la misma medida que la prueba de fuego del almacén ya hace, con el origen moviéndose.

**No hace.** No lee un rango: eso es R3. Aquí la copia sigue siendo entera cada vez.

### R3 · `desde` / `hasta` · [0016](decisions/0016-el-testigo-y-el-rango.md) B

**Qué.** `Peticion` gana los dos campos, y **un driver que no sepa honrarlos falla**. Ignorarlos
devolvería otras filas de las que se pidieron, y eso no falla: se sirve.

Con `hasta` desaparece el desfase entre preguntar el testigo y leer las filas, y el modo del
testigo pasa a contestar **si la copia puede ser atómica** — que es la frase que 0016 quería dejar
escrita.

**Y una ventana puede no caber.** BigQuery topa `CHANGES` a **un día por consulta**. Así que un
refresco puede necesitar **varias lecturas encadenadas**, y el protocolo tiene que permitir que el
driver diga *«ese rango no lo sirvo de una vez»*. Encadenarlas es del planificador y **no entra
aquí**.

**Listo cuando.** Un refresco lee **solo el rango**, contado en filas miradas al modo de
`ore-view/tests/medidas.rs` — no en tiempo; y un driver sin soporte se niega, y hay una prueba que
lo provoca.

### R4 · un solo sitio para el testigo

**Qué.** La cabecera del `.oretopo` lleva su marca de agua y el sobre de 0015 lleva su testigo.
**Son el mismo campo en dos formatos**, y 0016 lo dice al contestar dónde se guarda: fuera del
origen, en el artefacto.

Es la misma operación que ya se hizo dos veces —la tabla con el puntero, `ore_core::aristas` con
la derivación— y por la misma razón: dos sitios divergen en el que ninguna prueba ejerce.

**Listo cuando.** Borrar el campo de uno de los dos **no pierde información**, y una prueba lo
enseña.

### R5 · la recogida de basura

**Qué.** [ADR 0015](decisions/0015-el-protocolo-del-almacen.md) la dejó abierta y **R2 la vuelve
urgente**: en cuanto el testigo se rellena, cada refresco escribe **otro** artefacto con otro
nombre. Que esté bien —una copia vieja sigue siendo cierta hasta su marca— no quita que el almacén
crezca sin freno.

Y parte de partida buena, que el propio ADR ya nombró: **nada referencia una copia por nombre
mutable**, así que borrarla no puede romper a quien la estuviera usando por casualidad.

**Listo cuando.** Se puede enumerar qué artefactos no nombra ningún bundle, y borrarlos deja el
árbol verde.

---

## 4. Lo que **no** entra

**Cada cuánto se refresca.** El testigo dice hasta dónde está el origen; **cuándo** volver a
preguntar es de `freshness` y de quien programe el refresco.

**El encadenado de ventanas** cuando el origen limita el rango. Es del planificador, y hasta que no
exista un origen que lo exija de verdad, escribirlo sería inventarlo.

**La cara `writes`.** Materializar escribe en una copia. Escribir en el **origen** es M1 de
[`sustrato.md`](sustrato.md).

**Y lo que sigue bloqueado por fuera:** `ore-exec` compila desde que hay `gcc`, y BigQuery responde
desde que hay sesión — pero la rama `{retract, log}` de la receta de BigQuery sigue sin poder
medirse en vivo, porque **ninguna tabla del dataset tiene el historial encendido** y encenderlo es
modificar el dataset de otro.
