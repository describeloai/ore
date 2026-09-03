# Handoff · del poblar al mantener

> **Desechable.** Cuando sus peldaños estén, este documento sobra: lo que decidan vive en
> [ADR 0016](decisions/0016-el-testigo-y-el-rango.md) y en la especificación.

`ore materialize` corre entero —[ADR 0015](decisions/0015-el-protocolo-del-almacen.md), y el ciclo
está medido contra un R2 de verdad— y aun así **la copia no se refresca nunca**. Este plan cierra
esa distancia.

Lo que hay que hacer sale de [ADR 0016](decisions/0016-el-testigo-y-el-rango.md), que no lo
razonó desde cero: lo leyó en Debezium, Iceberg, Delta, Snowflake, BigQuery y Airbyte, y los seis
coinciden. Aquí solo se ordena en peldaños.

## 0. Cuándo está listo esto

**No cuando los seis peldaños estén escritos: cuando [R6](#r6--el-ciclo-cerrado-y-medido-en-filas)
pase.** R6 no añade nada — es **la definición de listo del plan entero**, escrita como una sola
prueba con números afirmados.

Y «funcionando óptimamente» aquí tiene una definición y solo una, que es la que
[ADR 0014](decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md) ya fijó para todo el
proyecto:

> **El trabajo es proporcional al cambio, no al tamaño.** Y se cuenta en **filas miradas**, no en
> segundos.

Un refresco que funciona pero relee el origen entero **no está listo**: está poblando otra vez con
otro nombre. Por eso la prueba de R6 no dice «verde»: dice **cuántas filas se leyeron en cada
paso**, y falla si son otras.

Los cinco peldaños intermedios tienen su propio criterio, todos falsables. R6 los ata.

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
| **R6** | **el ciclo cerrado, y medido en filas** | una prueba de fuego | **es la definición de listo** |

**R0 y R1 no dependen de R2 ni de R3**, y cierran agujeros que están abiertos **hoy**. Van primero
por eso, no por ser fáciles.

**R6 se puede escribir el primero**, y conviene: roja en su primer acto, es la lista de trabajo del
plan entero, y cada peldaño la va poniendo verde por partes.

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

**Listo cuando.** Tres afirmaciones, y las tres falsables:

1. `ore view` dice, **por copia**, si su testigo cabe en la retención declarada;
2. una copia cuyo testigo cae fuera se rechaza **nombrando la retención y el testigo**, y **no**
   degrada a un recorrido completo sin decirlo — hay una prueba que lo provoca y comprueba el
   texto, no solo el código de salida;
3. **sin `retention` declarada no se afirma nada**: ni que cabe ni que no. Una prueba con el campo
   ausente comprueba que la salida no menciona plazo ninguno.

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

#### Hecho a medias · el protocolo, y lo que faltaba debajo

`Peticion` lleva **`start`, `end` y `cursor`**, y los nombres son los de la industria: Iceberg lee
con `start-snapshot-id`, Delta con `startingVersion`, BigQuery con `start_timestamp`, y a la
columna que ordena el avance medio sector la llama *cursor field*. La regla queda escrita donde se
declaran:

> **Donde la industria tiene un nombre, se usa el suyo; donde no, el nuestro.** Es la misma que la
> gramática de OOS ya sigue —`witness`, `mode`, `retention`— y la misma por la que `changes.mode`
> habla con el vocabulario de Flink.

`start` es **exclusivo** y `end` **inclusivo**: lo que fue `end` de un refresco es `start` del
siguiente, y una fila cae en exactamente uno de los dos. Y la mitad que hace que el campo valga
está en `rango_servible`, escrita **una vez** en el protocolo y no en cada driver:

> Un driver que reciba un rango y no sepa servirlo **debe fallar**. Ignorarlo devolvería las filas
> de ahora en vez del incremento, y eso **no falla: se sirve** — la copia sale con filas de más y
> nadie ve nada.

`ore-read-postgres` y `ore-read-jsonl` declaran los dos lo mismo: saben recortar por columna, no
saben leer un changelog. Para el SQL, un rango resulta ser **dos condiciones más en el `WHERE`** —
que es otra vez la prueba de que la petición estaba cortada por el sitio correcto.

#### Y lo que R3 destapó, que no es de R3

**Un refresco incremental no puede sellar solo el incremento.** Una vista materializada tiene que
contener el resultado **entero**; si el driver devuelve 10 filas y el almacén sella esas 10, la
copia queda con 10 y **responde mal**. Leer menos exige, debajo, **fundir** el delta con la copia
anterior.

R6 no lo veía: medía `leidas` y no cuántas filas quedaban en la copia, así que un refresco que
leyera 10 y sellara 10 habría pasado en verde. **Ya lo mide**, y de ahí sale la cuarta invariante:

> **④ Y la copia, entera.** Trabajo proporcional al cambio **y** resultado completo son dos cosas,
> y hacen falta las dos.

Y la fusión **ya tiene dueño en este árbol**: es el circuito Δ —[ADR 0013](decisions/0013-el-protocolo-del-mantenedor.md)—
que existe justamente para aplicar un delta a un estado. Lo que R3 deja visto es que **la copia es
ese estado**, y que `ore-maintain` y `ore materialize` están resolviendo dos mitades del mismo
problema sin saberlo. Juntarlas es un peldaño propio y no cabe aquí.

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

**Listo cuando.** Se puede enumerar qué artefactos **no nombra ningún bundle**; borrarlos deja el
árbol verde; y **el almacén queda acotado**: tras `N` refrescos de una vista, el número de objetos
bajo `ore/v1/` es el que se declare y no `N`.

---

### R6 · el ciclo cerrado, y medido en filas

**Qué.** Nada nuevo. **Es la definición de listo del plan entero**, escrita como una prueba de
fuego con números afirmados, al modo de
[`pruebas-de-fuego/almacen-r2.sh`](../pruebas-de-fuego/almacen-r2.sh) y con la unidad de
[ADR 0014](decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md): **una fila mirada**.

#### El escenario

Un origen con **1.000 filas**, un R2 de verdad, y cinco actos:

| | qué pasa | filas leídas del origen | objetos nuevos en el almacén |
|---|---|---|---|
| 1 | primera materialización | **1.000** | 1 artefacto + 1 recibo |
| 2 | `materialize` **sin tocar el origen** | **0** | **0** |
| 3 | se añaden **10** filas · `materialize` | **10** | 1 artefacto + 1 recibo |
| 4 | se modifican **3** · `materialize` | **3** | 1 artefacto + 1 recibo |
| 5 | recogida de basura | — | quedan **los declarados**, no siete |

**El acto 2 es el que separa «funciona» de «está listo».** Cero filas leídas es lo que dice que el
recibo hace su trabajo antes de abrir nada. Y **el 3 y el 4 son los que separan «refresca» de
«refresca óptimamente»**: 10 y 3, no 1.010 y 1.013.

#### Y las negativas, que valen igual

Una prueba que solo comprueba el camino bueno no define «listo»: define «anduvo una vez».

| | qué se provoca | qué tiene que pasar |
|---|---|---|
| a | `{ witness: field, mode: append }` con `materialized` | **no compila** · R0 |
| b | un testigo fuera de `changes.retention` | se niega **nombrando la retención**, y no relee entero en silencio · R1 |
| c | un driver que no sabe `desde` recibe un rango | **falla**, y no devuelve las filas de ahora · R3 |
| d | el origen retrocede — testigo menor que el de la copia | se niega, y **no** escribe una copia que dice ser más nueva |

#### Lo que la prueba afirma, y que no es «verde»

1. **Filas leídas por acto**, exactas. Si el acto 3 lee 1.010, el peldaño **no está**.
2. **Objetos en el almacén por acto**, exactos.
3. **El testigo de cada artefacto es mayor que el del anterior**, leído del sobre.
4. **Las cuatro negativas fallan por su motivo**, no por otro.
5. **El bucket queda como se encontró.**

#### Qué es exactamente «óptimamente»

Tres invariantes, y las tres son afirmaciones sobre trabajo, no sobre tiempo:

> **① Sin cambio, cero trabajo.** Ni una fila leída, ni un byte subido.
> **② Con cambio, trabajo proporcional al cambio.** No al tamaño de la tabla.
> **③ Con el tiempo, almacén acotado.** No una copia por refresco, para siempre.

Cualquiera de las tres que falle deja el plan sin cerrar, aunque las otras dos pasen y la salida
sea verde.

#### Por qué esto es un peldaño y no un apéndice

Porque **se puede escribir antes que R0**, y conviene: la prueba roja en su primer acto es la
lista de trabajo del plan entero, y cada peldaño la va poniendo verde por partes. Es lo mismo que
`medidas.rs` hizo por el motor — y aquello destapó que *la incrementalización estaba escrita y no
ocurría*, que es exactamente la clase de cosa que esta prueba existe para encontrar.

**Listo cuando.** Los cinco actos dan los números de arriba, las cuatro negativas fallan por su
motivo, y las tres invariantes se sostienen. **Entonces el plan está cerrado y este documento se
borra.**

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
