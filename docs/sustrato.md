# El sustrato, y lo que reposa encima

> **Este documento NO es desechable.** Los `handoff-*.md` describen una migración y se borran
> cuando termina. Este describe **una dirección**, y se queda: es el sitio donde se retoma el
> hilo el día que se abra la capa de abstracción.
>
> Fecha: 2026-09-02, al terminar la migracion a tablas y vistas. Los dos handoffs que la
> describian se borraron al cerrarla; esto es lo que se quedo de ellos.
>
> Distingue tres cosas y no las mezcla: **lo decidido** —que es normativo y está en
> `spec/v1alpha8`—, **lo proyectado** —que es esto— y **lo abierto**, que se dice como abierto.

---

## 1. La tesis

`Table` y `View` no son *la capa física de la ontología*. Son **el sustrato**.

La diferencia no es de palabras. Una capa física es una pieza al lado de las demás, y se
justifica por lo que le quita a la entidad. Un sustrato es aquello **sobre lo que las demás se
construyen**, y se justifica por lo que las demás dejan de tener que saber.

> El modelo ontológico —entidad, concepto, interfaz, regla, función, política—, versionado y
> ramificado en plenitud, se construye **sobre** tablas y vistas, y **reposa** en ellas. Y es a
> través de ellas por donde se lee y por donde se escribe: como función sobre la ontología, como
> consulta, o como lo que venga.

> **La coletilla original decía «por donde eventualmente se escribe en el origen».** El
> [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md) la corrigió: se escribe en
> **la copia**, y el origen no se toca nunca. Es la primera frase de este documento y llevaba el
> sesgo dentro.

Todo lo que sigue son consecuencias de tomarse esa frase en serio.

---

## 2. La medida que la sostiene

Al migrar `acme-retail` a v1alpha8 —T4— se contaron los nombres de `hr.empleados` contra los de
`hr.Employee`:

```
campos de la vista : 11
props de la entidad: 10
nombres compartidos:  9
solo en la entidad : nationalId
solo en la vista   : validFrom, validTo
```

**Nueve de once nombres están escritos dos veces**, y no por descuido:
[`v1alpha7/00-scope` §4](../vendor/oos/spec/v1alpha7/00-scope.md) lo convirtió en regla — *«las
propiedades de la entidad se llaman como los campos de su vista»*.

Una regla que dice *«estas dos listas tienen que tener los mismos nombres»* es una regla que dice
que **la segunda lista no debería existir**. Es el mismo defecto que motivó la tabla —el binding
repetía el contrato físico— un piso más arriba: **la entidad repite la forma de la vista.**

Y el residuo delata lo demás. `nationalId` está en la entidad, clasificado `critical`, con
`uniqueKeys: [[nationalId]]`, y **no tiene campo en ninguna vista**: no tiene de dónde salir, y
compila en verde. En el escaparate. Se comprobó aparte con un paquete mínimo:

```yaml
backedBy: empleados        # la vista expone employeeId
properties:
  employeeId: …
  alias: …                 # sin campo en la vista
  baseSalary: …            # sin campo en la vista
```
```
ok · sin errores
```

`OOS2011` exige la clave y los `via`; `normalize::sin_binding` ve entidades sin **ninguna**
fuente. Entre las dos queda el hueco: **una propiedad sin campo en su vista**, que hoy no tiene
código y es un fallo sin síntoma.

---

## 3. Dos caras, y una frontera que resultó ser de otro

> ### ⚠️ Esta sección se llamaba «La tercera cara», y esa es la corrección más grande del documento
>
> Decía: *«si el sustrato tiene que soportar la escritura, la tabla no tiene dos caras: tiene
> **tres**»*, y proyectaba una cara `W` donde el objeto declararía qué escrituras acepta.
>
> **La premisa era falsa**, y el [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md)
> la retiró: una escritura que sale de la ontología aterriza **en la copia**, y al origen no se le
> pide nada. Preguntarle qué acepta no contesta ninguna pregunta que alguien vaya a hacer. La cara
> se llegó a construir —`F0a`— y se retiró entera.
>
> **La tabla tiene dos caras y no va a tener una tercera**: `reads`, qué se le puede pedir, y
> `changes`, qué cambios emite. Las dos decididas en v1alpha8.
>
> Lo que sigue **no se borra**, y no por nostalgia: el análisis es correcto y lo que estaba mal era
> a quién se lo aplicábamos.

| cara | pregunta | estado |
|---|---|---|
| `reads` — `I` | qué se le puede pedir | **decidido**, v1alpha8 |
| `changes` — `D` | qué cambios emite | **decidido**, v1alpha8 |

### 3.0 · La frontera de la invertibilidad, y de quién es

La regla de la versión dice `View = Q(Table)`. Llevar un cambio **hasta el origen** sería `Q⁻¹`: el
problema clásico de actualización de vistas, que solo se resuelve si `Q` es invertible.

**Y el vocabulario de la vista es exactamente el fragmento invertible.**

| operación | invertible | está en la gramática |
|---|---|---|
| renombrar | sí — es una biyección | **sí** |
| recortar por partición | sí — la fila escrita cumple el predicado o se cae de la vista, y las dos son decidibles | **sí** |
| proyectar | parcialmente — faltan columnas, así que la escritura es *parcial*, no ambigua | **sí** |
| unir | **no** — no se sabe a cuál de las dos bases escribir | no |
| agregar | **no** — una fila del resultado no corresponde a una fila de la base | no |
| deduplicar | **no** — la inversa no es una función | no |
| limitar | **no** — qué filas están es un hecho del orden, no del dato | no |

Las cuatro que faltan son **las cuatro que se excluyeron**. `v1alpha8/00-scope` §6 las dejó fuera
razonando sobre el precio en la regla de flujo —una junta trae dos raíces, un agregado puede
desclasificar, un límite impide empujar predicados— y resulta que estaba dibujando **la misma
frontera desde el otro lado**.

> `View = Q(Table)`, con `Q` invertible **por construcción**. Y ahí está, dicha del revés, la
> razón de que unir y agregar estén fuera.

#### Y de quién es esta frontera, que no es de este producto

Escribir aquí **no es `Q⁻¹`**, y el motivo es una medida: la copia guarda **el vocabulario de la
vista** —su esquema sale del plan, así que sus columnas son `employeeId` y no `employee_id`—. Un
edit nombra una propiedad, la propiedad es un campo de la vista, y el campo es una columna de la
copia. **Cae dentro de `Q`, no fuera**, así que no hay nada que deshacer.

Esta frontera es la primera regla del producto que escribe **de vuelta en los sistemas de origen**,
que es otro y no está aquí. Está construida y ejercida —`vistas::invertible` y su censo— y su
código, `OOS7013`, queda **reservado** con el precedente de `OOS2001`.

Que la coincidencia con las cuatro exclusiones de `00-scope` §6 sea exacta **sigue siendo cierta y
sigue valiendo**: dice que el vocabulario de la vista se eligió bien, aunque el motivo que aquí se
le atribuyó no fuera el suyo.

### 3.1 · Lo que eso decidió sobre la federación

Al migrar el árbol apareció que v1alpha8 **no sabe expresar una entidad servida desde N objetos**
— `casos/dos-familias`, que el binding decía sin esfuerzo. Se barajaron tres
salidas: que el binding sobreviviera para eso, que `backedBy` aceptara una lista, o aceptar la
exclusión.

Hay dos razones para la tercera, y el orden importa porque la primera es la que no se arregla
con más gramática.

**Una · federar une por una clave que nadie reconcilió.**

Foundry y Cognite sí pueden. Cognite direcciona cada instancia por `space` + `externalId`, y ese
identificador lo pone quien ingiere; un objeto multi-fuente de Foundry une por la clave primaria
que la tubería dejó consistente. **La reconciliación ocurrió aguas arriba, en la ingesta**, y el
modelo la da por hecha.

Aquí no hay ingesta. Y —esto es lo que cuesta ver— **materializar tampoco la crea**: copiar filas
no reconcilia identidades, y la afirmación *«estas dos filas son la misma cosa»* es exactamente la
misma antes y después de copiar. Dos copias con una clave que colisiona siguen siendo dos copias.

Así que no es del sustrato. Es de `v1alpha2/03-resolution`, cuya estrategia `deterministic` está
descrita allí como **«un `join`»**, con `match` entre fuentes, `normalize` y conducto. El binding
hacía eso sin declarar ninguna de las tres.

**Dos · y por una junta no se sabe escribir.**

Con la tercera cara delante, admitir federación en la vista habría metido en el sustrato justo lo
que lo vuelve de solo lectura para siempre. Es la segunda razón y es suficiente por sí sola; la
primera es la que sigue siendo cierta aunque nunca se escriba nada.

Está escrito en `v1alpha8/00-scope` §6, y la corroboración en §6.1: las *materialized views* de
Snowflake solo consultan **una** tabla y no admiten juntas. Misma frontera, otro camino.

### 3.2 · Y `v1alpha2` llevaba esperando esto

`Function` declara `effects`, y su regla es `I(f) ⊒ I(destino)` — *lo que se puede causar*. Existe
desde v1alpha2 y **nunca ha tenido dónde aterrizar**: no había un sujeto físico al que causarle
nada.

Con `writes` en la tabla, el destino de un efecto es **una vista escribible**, y la regla de flujo
de la escritura es el espejo de la de lectura, ya escrita, sin inventar nada.

---

### 3.3 · Y qué cambia cuando el almacén es nuestro

> **Escrito en futuro y ya ocurrido.** El almacén existe —[ADR 0015](decisions/0015-el-protocolo-del-almacen.md)—
> y el ciclo lo puebla, lo refresca, lo funde y lo recoge. Lo que sigue se deja como se escribió,
> porque acertó; lo que aquello no podía prever está en [§3.4](#34--y-entonces-sí-tenemos-container).

La dirección es traer el almacenamiento de lo materializado **a nuestro lado** —almacenamiento de
objetos— en vez de escribirlo en el almacén del cliente. No es un giro de doctrina: es lo que el
[ADR 0006](decisions/0006-el-artefacto-de-topologia.md) ya decidió para la topología —*«ORE no
opera ninguna base de datos; el índice se construye una vez, se firma, se distribuye y se
mapea»*—, extendido de las aristas a las filas. **Una copia es un artefacto, no una base de
datos.**

Lo que desbloquea, y conviene tenerlo separado de lo que no:

- **La varianza de `reads` desaparece para lo copiado.** Hoy lo que una vista puede servir es lo
  que su origen afronte: un Workday con `fullScan: forbidden` no sirve una búsqueda, y el
  planificador lo rechaza. Sobre una copia nuestra, en un formato que elegimos, las capacidades
  son **las mismas siempre**. Es lo que el índice de objetos le compra a Foundry, dicho en
  nuestros términos.
- **El testigo tiene casa.** Hoy no vive en ninguna pieza del motor. En un artefacto vive en la
  cabecera, como en el `.oretopo`.
- **La copia entra en el grafo de artefactos versionado.** Determinista y firmada ⟹ tiene digest
  ⟹ el lock la puede fijar, `ore diff` la puede comparar y una rama la puede nombrar. Eso es lo
  que hace que *«versionado y ramificado en plenitud»* valga también para lo materializado, y no
  solo para las declaraciones.
- **La topología deja de ser un caso especial.** Mismo almacén, misma familia de formato, mismo
  testigo — que es exactamente lo que I4 de [`decisions/0016-el-testigo-y-el-rango.md`](decisions/0016-el-testigo-y-el-rango.md)
  persigue.

Lo que **no** desbloquea, y es lo importante: **la identidad**. Escribir filas en un almacén
propio no reconcilia nada. Lo que sí abre es la *posibilidad* de reconciliar al copiar — que
sería ingerir, y sería exactamente lo que hacen los otros dos. Y esa posibilidad tiene precio:
quien reconcilia responde de la reconciliación. Por eso seguiría siendo `Resolution` y no una
propiedad de la vista.

Y lo que cuesta, dicho antes de que llegue:

- **ORE pasa a sostener dato del cliente.** El conducto que autoriza una copia deja de autorizar
  un movimiento dentro de su casa y pasa a autorizar **sacarlo de su frontera**.
  `acme.residency: eu_only` deja de ser una etiqueta que se propaga y pasa a ser una pregunta
  sobre dónde está el bucket. La maquinaria para decirlo ya existe; lo que cambia es que ahora
  decide algo caro.
- **Dos frases de la especificación hay que volver a mirarlas.** *«ORE no opera ninguna base de
  datos»* probablemente sobrevive —un artefacto en almacenamiento de objetos no lo es, y ADR 0006
  ya defendió ese límite—. *«La copia es del cliente»*, en `materialized.datasource`, **no
  sobrevive**. No se toca hoy: se toca cuando exista, porque una especificación describe lo que
  es normativo, no lo que se planea.

### 3.4 · Y entonces sí tenemos container

Cuando se tomó la forma de la vista de Cognite, la conclusión fue una frase:

> *«Nosotros no tenemos containers, así que nos queda solo la mitad de arriba — que es toda.»*

**Esa frase dejó de ser cierta.** Y con ella cambia la respuesta a *sobre qué se sienta la capa
ontológica*, que es la pregunta que este documento existe para contestar.

#### Los tres pisos, y de quién es cada palabra

Cognite lo dice así, en su documentación:

| | qué es | ¿guarda dato? |
|---|---|---|
| **container** | *«the physical storage for properties»* | **sí** |
| **view** | *«Data is not queried directly from the containers»* · *«map properties from different containers in a flat object and rename or alias»* | no |
| **data model** | *«group views that belong together for a purpose»* | no |

Lo nuevo es el piso de abajo: **la copia materializada es un container.** Tiene almacenamiento
propio, esquema declarado, clave, testigo, la lee cualquier motor, se funde incrementalmente y se
recoge. No se parece a un container: hace lo que hace un container.

Con **una asimetría que se creyó insalvable y no lo era**: el suyo es almacenamiento **primario**
—se ingiere dentro y se escribe—; el nuestro parecía condenado a ser **solo de lectura**, un plan
sobre un puntero y nada más.

Ya no. Desde el [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md) la copia
**también recibe lo que la ontología escribe** —`Q(origen) ⊕ ediciones`— y sigue siendo derivada,
de dos entradas en vez de una. Lo que se creía que hacía falta para cerrar esa distancia era darle
a la tabla una tercera cara: es [M1](#m1--la-tercera-cara), y la distancia se cerró **por el otro
lado**.

#### Y una equivalencia que este documento afirmó de más

La primera versión de esta sección decía *«nuestra `View` **es** su view»*. **Es demasiado
fuerte**, y se ve al mirar qué declara cada una.

> La view de Cognite hace **dos trabajos**: mapea propiedades de containers —*«map properties from
> different containers in a flat object and rename or alias»*— **y es el tipo lógico**, con
> `implements` para heredar propiedades y aristas de otras views, agrupadas en un data model.
>
> **La nuestra hace uno.** Nuestra `View` es su mitad de mapeo. La mitad de tipo lógico es aquí
> `Entity` —con su propio `implements`—, `Property.is`, `Concept` e `Interface`.

Nosotros **partimos en dos lo que ellos tienen junto**, y la partición no es cosmética: es lo que
permite que una vista exista antes de que nadie modele nada y que varias entidades se respalden de
la misma.

#### La vista no lleva significado. Lo **transporta**

Esto es lo que la hace sustrato y no ontología, y es estructural: **una `View` no admite
`labels`** — el campo no existe, y `document.rs` lo dice donde se decide:

> *«Una vista tampoco admite `labels`, y es la decisión de forma que la define: NO LLEVA
> SIGNIFICADO. Las etiquetas viven en la entidad y en el datasource; si la vista pudiera
> declararlas habría dos sitios diciendo qué es una columna, y el día que discrepen ninguno diría
> cuál manda.»*

Su vocabulario entero lo confirma: `owner`, `from`, `freshness`, `fields`, `where`,
`materialized`. Ni una clave dice qué **son** las cosas. `owner` es lo más cerca que llega, y es
custodia, no semántica.

**Y sin embargo la clasificación la atraviesa, y puede impedir que compile.** Las etiquetas que
una vista carga le llegan por dos vías, y ninguna es ella misma:

| vía | de dónde | qué aporta |
|---|---|---|
| **1** | el `datasource` | procedencia física: etiqueta todo lo que sale de él |
| **2** | la **entidad**, por la cadena | significado: propiedad → campo → columna raíz, subiendo por el retículo |

La vista es **donde las dos se encuentran**, y por eso es el sitio donde se detecta una fuga —lo
enseña `ore view`, y `ore validate` no puede—.

> **Está sujeta al significado sin ser fuente de él.** Esa es la frase, y es la que separa los dos
> pisos.

#### Y lo que hacen los otros dos, verificado

**Databricks.** Su *foreign table* es exactamente nuestro `Table`, hasta en el motivo: *«Queries
are read-only»*, y más fuerte — *«Unity Catalog will not issue write credentials under any
circumstance»*. `01-table` §2 dice *«No se escribe. El puntero es de solo lectura»*. Dos
proyectos llegaron a la misma frase por su cuenta.

**Foundry.** Una *virtual table* puede respaldar un tipo de objeto. Pero servir ese objeto **no
lee del puntero**: lee de un índice separado al que los datos **se sincronizan**, y lo sincroniza
el *Object Data Funnel*. Para los objetos multi-fuente su documentación es explícita —*«Only
Foundry datasets or restricted views can be used for MDOs»*, y solo cuentan *«datasources that
are synced to object storage»*.

> **La ontología de Foundry no se sirve del puntero: se sirve de una copia.**

Eso zanja la duda que quedaba abierta desde [§3.1](#31--lo-que-eso-decidió-sobre-la-federación).
No es que ellos puedan servir ontología desde un puntero y nosotros no: **nadie lo hace**. Lo que
ellos tienen y nosotros acabamos de conseguir es el sitio donde cae la copia.

#### La respuesta, con el recorte exacto

> **La capa ontológica que porta DATO se sienta sobre la vista, y solo sobre la vista.**
> **La que porta SIGNIFICADO no se sienta sobre nada.**

Las dos mitades, porque confundirlas es de donde salía la ambigüedad:

| | ¿sobre qué se sienta? | por qué |
|---|---|---|
| `Concept`, `Interface`, `Lattice`, `Ruleset` | **nada** | un concepto significa lo mismo esté o no instanciado. Ninguno nombra nada físico |
| `Entity` + `Property` | **una vista**, con `backedBy` | prometen filas, y una promesa de filas necesita quién las conteste |

##### Jamás una tabla, y no por convención

`backedBy` **DEBE** resolver a una vista —`OOS2018`—. **No existe sintaxis para nombrar una tabla
desde arriba.** No está desaconsejado: no se puede escribir.

El motivo es el que hizo nacer v1alpha8. Una tabla es **el objeto tal cual está**; una entidad es
**un punto de vista sobre él**. Entre las dos hace falta un sitio donde decir *qué parte, con qué
nombres y con qué recorte* — y si no existiera, eso viviría dentro de la entidad, y el significado
y el contrato físico volverían al mismo documento. La prueba de que no es teórico está en el
corpus: el caso `one-object-many-entities`. Si la entidad nombrara la tabla, **cada una repetiría
el contrato físico**.

##### Ni una vista materializada, y aquí la pregunta cambia de forma

**No existe ese objeto.** `materialized` es un **campo de la vista**, no un `kind`; no hay
`kind: MaterializedView`. Así que la ontología no puede sentarse sobre una vista materializada por
la misma razón por la que no puede sentarse sobre «una vista los martes»: no es una cosa, es un
**estado** de una cosa.

Y la copia —el artefacto del almacén, nombrado por el digest de su plan— tampoco se nombra nunca
desde arriba. La copia es **la respuesta**, no la pregunta. Quién contesta lo decide `raíz de
lectura`, abajo.

##### Y el porqué de verdad, que es uno solo

```text
Table    siempre física   ─┐
copia    siempre física   ─┘ el sustrato

View     ◄── la bisagra: puede ser cualquiera de las dos, y pasar de
             una a otra es una línea que nadie de arriba nota

Entity   siempre lógica   ─── la ontología
```

**La vista es el único piso indiferente a la física**, y de ahí sale todo lo demás:

- **sobre la tabla**, materializar sería *un cambio en el modelo*: habría que tocar la entidad para
  ganar velocidad;
- **sobre la copia**, el modelo no existiría hasta que alguien copiara: una vista virtual no
  tendría dónde sentarse, y sentarse sobre lo que a veces no existe no es sentarse;
- **sobre la vista**, la decisión de rendimiento y frescura se toma abajo y no sube.

Es la misma indirección que hace posible `OOS2020` —*una vista cuya raíz no se deja leer debe
materializarse*—. Esa frase solo se puede escribir si hay un objeto que **es la pregunta con
independencia de quién la conteste**.

##### La excepción, con su número

«Solo sobre la vista» es cierto del paradigma actual, no de todo lo que compila:

```text
v1alpha7   con backedBy:  5  | sin:   0
v1alpha8   con backedBy: 14  | sin:   1
v1alpha1   con backedBy:  3  | sin: 237
```

Las 237 son el mecanismo anterior —`Binding`, con la flecha al revés— y **siguen compilando**,
porque un documento no caduca. La única de v1alpha8 sin `backedBy` es `hr.Department`, del caso
`mixed-versions`, y está ahí a propósito: respaldada por un binding v1alpha1 en el mismo paquete,
para afirmar que la migración no roza.

#### Qué queda para que el piso esté completo, y una asimetría que se cae

La asimetría con el container de Cognite —*el suyo es primario, el nuestro de solo lectura*— **ha
dejado de ser cierta**, y lo decidió el
[ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md): una escritura que sale de la
ontología aterriza **en la copia**, nunca en el origen.

```text
copia  =  Q(origen)  ⊕  ediciones
```

Sigue siendo **derivada** —lo que cambia es de qué: antes de una entrada, ahora de dos, y las dos
declaradas y reproducibles—, y **el puntero sigue siendo de solo lectura sin matices**, que es lo
que este documento llevaba diciendo desde §3.1 y lo que Databricks firma igual.

Lo que eso le da a esta sección es la respuesta que le faltaba, y es del usuario antes que mía:

> **Ser un registro íntegro no impide ser un buen espejo.** Mientras no haya escrituras desde la
> ontología, la vista puede quedarse virtual y refleja el origen exactamente. En cuanto las hay,
> **hay materialización**.

Y por eso se decide **por vista**, no por producto: las dos clases conviven en el mismo paquete y
el compilador dice cuál es cuál, sin que nadie tenga que acordarse.

Lo que sigue sin poder la ontología es **aplicar**, y eso es lo único que queda: `F4` —el runtime
delegado— y `F5` —sellar la copia sucesora—. Sin cuarta delegación: aterriza en `ore-store-r2`,
que ya existe.

## 4. La ramificación sale gratis, y no por suerte

Si la ontología reposa en el sustrato, **una rama es una bifurcación del grafo de vistas**, no de
los datos. Las tablas se comparten; las vistas son declaraciones y no cuestan nada.

Lo único con precio sigue siendo `materialized`, que ya es la única decisión con coste del modelo.
La ramificación «en plenitud» es barata **por construcción**, y solo lo es porque el sustrato
separó *el objeto* de *la consulta*: mientras el puntero vivía dentro de la vista, bifurcar una
vista bifurcaba el contrato físico.

---

## 5. Los movimientos, en orden

Cada uno es el suelo del siguiente. Ninguno está hecho.

### M1 · La tercera cara

`Table.writes` — qué escrituras acepta el objeto: nada, altas, upsert por clave, borrado; con qué
clave y con qué idempotencia. Y **la invertibilidad de la vista derivada, no declarada**: una
vista es escribible si su cadena es selección, renombre y partición, y su raíz acepta escrituras.
Lo derivable no se declara (P2).

Un código nuevo para lo que hoy no tiene nombre: *escribir por una vista que no se puede
invertir*.

**Listo cuando** el compilador rechace una escritura sobre una cadena no invertible sin abrir una
conexión, igual que hoy rechaza `OOS2020`.

> ### ✅ Hecho, en `F0a` y `F0b`
>
> `Table.writes` es un **conjunto** —`insert`, `update`, `delete`— y no los modos que este
> documento imaginó: `information_schema.views` de SQL expone tres columnas separadas, no un modo.
> Sin `upsert` —es la suma de dos— y **sin clave propia**: la fila se identifica con `changes.key`,
> que ya existía, y `OOS2024` la exige a la **tabla** cuando acepta `update` o `delete`.
>
> Que se le exija a la tabla y no a la vista lo decidió una medida en contra de lo que aquí se
> sospechaba: **de 20 vistas v1alpha8 sobre una tabla resuelta, 17 se apoyan en una tabla sin
> clave**, así que exigírsela a la vista habría dejado el 85 % del corpus sin poder escribirse.
>
> El código nuevo *para lo que hoy no tiene nombre* es `OOS7013`, y con una honestidad que hay que
> leer entera: **hoy no puede fallar por ningún documento**, porque el vocabulario de la vista es
> exactamente el fragmento invertible. Lo que tiene dientes es el **censo** que ata la
> clasificación al vocabulario — añadir una clave a `View` sin decir si se invierte no compila la
> suite.
>
> Y `OOS7012`, que este documento no previó: un efecto sobre un objeto que **no acepta que lo
> actualicen**. La ausencia de `writes` cuenta como negativa, igual que `reads: none`.
>
> ### ⚠️ Y retirado a los dos días, por el [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md)
>
> `M1` estaba **bien planteado y mal dirigido**: la tercera cara es una buena simetría, y no sirve
> de nada si al objeto nunca se le va a pedir nada. Si la escritura aterriza en la copia,
> preguntarle a la tabla qué acepta **no responde ninguna pregunta que alguien vaya a hacer**.
>
> Salen `Table.writes` y `OOS7012`; `OOS2024` sobrevive con otro sujeto —la clave la exige **ser
> escribible**, no `writes`—; y `OOS7013` cambia de dueño: pasa a ser la primera regla del producto
> que escribe de vuelta en los orígenes, que no es este.
>
> Se deja escrito arriba en vez de borrado porque **el error no estaba en la forma sino una capa
> más abajo**, y eso es lo que hay que poder volver a leer.

> **Y es el mismo peldaño que `F0` de [`functions.md`](functions.md), no dos.** Aquí se ve desde
> el sustrato —*la tabla gana su tercera cara*— y allí desde arriba —*un efecto necesita un
> destino*—, y es la misma línea de código. Lo que allí se añade es lo que este documento no
> podía saber: **el efecto pierde su `datasourceRef`**, porque el destino se deriva por el mismo
> camino que la lectura.

### M2 · La entidad deja de repetir

No «`Entity` fuera» sino **su `properties` fuera**: pasa a **anotar** campos de una vista —tipo,
`is`, etiquetas, clave, naturaleza— en vez de redeclararlos.

Los nueve nombres duplicados desaparecen. Y `nationalId` sin campo deja de poder existir: anotar
algo que no está es un **error de referencia**, no un silencio. El hueco de §2 se cierra por
construcción en vez de con un código nuevo, que es siempre la mejor de las dos formas.

**Listo cuando** ningún documento del árbol escriba el mismo nombre de campo dos veces.

### M3 · La función aterriza

`effects` apunta a una vista escribible. `I(f) ⊒ I(destino)` deja de ser una regla sin sujeto.

> **La segunda mitad de esta frase decía «escribir en el origen pasa a ser una consulta al
> revés».** La corrige el [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md): no
> hay consulta al revés porque no se sale de la consulta — la copia guarda el vocabulario de la
> vista, así que un edit cae dentro de `Q`, no fuera. *Escribible* pasa a significar
> **materializada y con clave**, no *invertible*.

---

### M4 · La topología es una vista, y lleva serlo desde antes de haber vistas

Lo destapó retirar `ore-exec`, que era su único productor, y dejarla **definida y sin poblar** —
un estado que no teníamos y que obliga a mirarla.

#### Qué es, para que no haya que ir a buscarlo

> Por cada **relación con `via`** de una entidad con clave simple: una proyección de **dos
> columnas** sobre la fuente física de esa entidad — **la clave** y **la columna del enlace**.

Eso es `ore_core::aristas`, y **es exactamente una `View` sobre una `Table`**: un `from`, dos
`fields`, sin `where`. No se parece a una vista: es una vista, escrita en otro sitio y con otro
nombre.

Para qué existe, en la frase que llevaba en su cabecera: *«el índice convierte escaneos en
búsquedas por clave»*. La travesía ocurre en local sobre las aristas, y solo cuando ya sabe qué
claves quiere se abre una conexión.

#### Lo que este árbol ya sabía y no había cobrado

Tres sitios lo dicen, escritos en tres momentos distintos, y ninguno sacó la consecuencia:

| dónde | qué decía |
|---|---|
| `decisions/README` | *«0006 y 0015 son el mismo artefacto con dos cargas»* — aristas en CSR, filas en Parquet |
| `aristas.rs`, al escribir I4 | *«el índice de topología **es** una vista materializada, escrita a mano en el paradigma anterior»* |
| `registro.rs` | lo enumera **como copia**, con su camino de refresco: *«marca de agua propia y ajena al circuito Δ»* |

Esa última es la confesión: formato propio, productor propio, marca de agua propia y refresco
propio, **todo en paralelo** al ciclo que los ADR 0015–0018 construyeron. Era una vista
materializada que se negaba a saberlo.

#### Cómo lo declaran los demás, y por qué la pantalla importa

**Se declara a mano, y esa parte la tenía mal escrita.** El asistente de *link type* de Foundry
obliga a elegir **cómo está respaldado el enlace**, y ofrece exactamente tres formas:

| Foundry | qué es | aquí |
|---|---|---|
| **Object type foreign keys** | el enlace es **una propiedad** de uno de los dos tipos · *one-to-one* y *one-to-many* | **`via`** — `one_to_one` y `many_to_one` |
| **Join table dataset** | el enlace vive en **un dataset aparte** con las dos claves primarias · *many-to-many* | **no existe** |
| **Backing object type** | el enlace **es un objeto** con propiedades propias · *many-to-many con atributos* | **no existe** como relación |

Lo que se declara es **el enlace y dónde se apoya**, no un índice. Y para el primer caso —el
único que nuestro vocabulario tiene, porque `one_to_many` es derivable del inverso y
`many_to_many` no se expresa— **no hay artefacto separado en absoluto**: la clave foránea es una
propiedad del objeto, y el objeto ya está indexado.

#### Entonces, ¿por qué teníamos un índice? Por federar

Y de ahí sale la respuesta, que no es la que este documento escribió primero.

**`via` nombra una PROPIEDAD**, no una columna: `via: [managerId]`, y `aristas` la resuelve por el
mapa de la vista. Y una propiedad de una entidad **tiene que ser campo de su vista** —`OOS2022`,
o declara `derivedFrom`—.

> Así que si la vista está materializada, **la copia ya contiene la arista**: la clave y la columna
> del enlace son dos de sus campos. No hay nada que construir aparte.
>
> **El índice de topología no es una vista que necesite productor: son dos columnas de una copia
> que ya existe** — o la muleta que hace falta exactamente cuando no existe ninguna copia.

Eso explica por qué Foundry no lo tiene y nosotros sí: ellos indexan el objeto entero, así que un
salto es una búsqueda local. Nosotros **federamos**, así que sin copia un salto es una consulta al
origen, y el índice existía para que no lo fuera.

#### Y la medida, que era lo que faltaba

La tesis de arriba se apoya en una cadena —`via` nombra una propiedad, una propiedad es campo de su
vista, luego la copia tiene la arista— y una cadena sin medir es una cadena. Medida contra el
corpus entero:

```text
corpus (vendor/oos)            entidades con `via`          25
                                 relaciones con `via`       27
                                 de ellas, `via` simple     24
                               sin `backedBy` · camino viejo 23
```

**Veintitrés de veinticinco son del paradigma anterior**, así que el corpus de conformidad no
ejercita esto en absoluto. Donde sí es medible es en el único ejemplo completo:

| entidad | `via` | su vista | ¿la vista expone la `via`? |
|---|---|---|---|
| `supply.Shipment` | `supplierId`, `skuCode` | `supply.envios` · **virtual** | **sí** |
| `hr.Employee` | `managerId`, `departmentId` | `hr.empleados` · **materializada** | **sí** |
| `hr.Department`, `customers.Customer`, `customers.Order` | — | **sin `backedBy`** | camino viejo |

**Dos de dos: la propiedad de `via` es campo de su vista.** Y no es suerte — `OOS2022` lo obliga,
así que **no puede no serlo**: una propiedad de una entidad es campo de su vista o declara
`derivedFrom`. La cadena se sostiene por una regla que ya existe, no por costumbre.

De las dos, **una ya está materializada**: `hr.Employee` tiene sus aristas dentro de su copia hoy
mismo, sin que nadie lo haya pretendido.

> **El precio del tercer gemelo, contado:** una entidad —`supply.Shipment`— tendría que declarar
> `materialized` para poder atravesarse. Una, en todo el corpus. Y las tres que no encajan no lo
> hacen por esto: no están migradas.

#### La regla que eso escribe sola

Es el tercer gemelo de una familia que ya tiene dos:

| | |
|---|---|
| `OOS2020` | lo que **no se puede leer** se debe materializar |
| `OOS2025` | lo que **se escribe** se debe materializar |
| **↳ el tercero** | lo que **se atraviesa** se debe materializar |

Con él, el artefacto de topología desaparece en vez de mudarse: atravesar exige copia, y en la
copia las aristas son columnas. La travesía pasa a ser una consulta sobre la copia —una búsqueda
por clave, que es lo que el índice compraba— y deja de necesitar formato propio, marca de agua
propia y refresco propio.

**Y no se decide aquí**, porque tiene un precio que hay que mirar de frente: hoy una entidad se
puede atravesar sin materializar nada, y esta regla lo prohibiría. Es la misma forma de decisión
que `OOS2020` —obligar a materializar lo que no se puede servir de otra manera— y merece el mismo
trato: medirla contra el corpus antes de escribirla.

#### Y el formato, que casi se disuelve con lo anterior

CSR es *«compacto, amable con el ancho de banda de memoria, y adecuado para travesías
eficientes»*. La carga de una copia es **Parquet**, elegido en el ADR 0015 *porque cualquiera la
lee*. No es la misma elección y conviene no fingir que sí.

Pero si las aristas son columnas de una copia, **no hay dos formatos que comparar**: hay una copia
Parquet y una consulta encima. Lo que la industria pone de ese lado, medido: **filtros de Bloom por
grupo de filas** —contestan *«seguro que no»* o *«probablemente sí»* antes de leer una columna— e
**índices de página** con mínimo y máximo, *«particularmente efectivos con datos ordenados»*. Su
límite conocido es que son **por fichero**, y a miles de ficheros abrirlos es el cuello — y una
copia nuestra es **un** artefacto por digest de plan, así que ese límite no nos toca.

Lo que queda es **la travesía multi-salto**, que es donde CSR ganaba por construcción. Y contarlo
en la unidad de este proyecto —**filas miradas**, [ADR 0014](decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md)—
da una cota que conviene tener escrita antes de medir nada:

| | un salto cuesta |
|---|---|
| **CSR** | localizar la clave y leer **su lista de adyacencia**: del orden del **grado del nodo** |
| **Parquet ordenado** | descartar grupos de filas por Bloom y leer **una página**: del orden del **tamaño de página** |

Así que `k` saltos son `k · grado` contra `k · página`, y **quién gana es el cociente entre el
grado medio y el tamaño de página** — que no es una propiedad del plan: **es una propiedad del
dato**.

> Y esa es exactamente la forma del hallazgo que `ore-view/tests/medidas.rs` ya produjo para el
> agregado: *«dónde se cruza el agregado es dato y no plan»* — el mismo documento se cruza en el
> 2 % con veinte grupos y en el 22,3 % con doscientos cincuenta.
>
> Si la respuesta tiene la misma forma, **la decisión también**: no se elige un formato en el
> diseño, se comparan medidas en el momento, que es lo que `Politica::Trabajo` ya hace.

**Y hoy no se puede medir, dicho para que nadie lo confunda con «no se ha medido»:** el CSR se fue
con `ore-exec`, así que no hay línea base; y una travesía **no se puede expresar como plan** porque
el vocabulario de la vista no tiene junta. Medirlo exige construir antes las dos cosas que la
medida tendría que juzgar — y eso, o se acepta, o se decide por la cota de arriba y se mide
después.

#### Lo que esto deja abierto, en una frase cada uno

1. **¿Se escribe el tercer gemelo?** *Lo que se atraviesa se debe materializar.* Prohíbe algo que
   hoy se puede, así que se mide antes.
2. **¿Cuánto cuesta N saltos sobre Parquet?** Filas miradas, no segundos.
3. **¿Y `many_to_many`?** No lo tenemos, y las otras dos casillas de esa pantalla son justamente
   eso. El día que haga falta, el *join table dataset* de Foundry **es una vista de dos columnas**
   — que es exactamente la forma que el índice de topología tenía, con otro dueño.

---

## 6. La costura por la que esto se rompe, si se rompe

*«La vista no lleva significado»* es una decisión que carga peso: si la vista supiera qué significa
una columna habría dos sitios diciéndolo, y el día que discrepen ninguno diría cuál manda.

M2 pone algo semántico **apuntando** a una vista. Creo que sobrevive —anotar desde fuera no es
declarar dentro, y sigue habiendo un solo sitio que manda— pero es exactamente ahí donde hay que
mirar cuando se abra. Si al escribir M2 aparece la tentación de meter una etiqueta *dentro* de la
vista, el diseño se ha torcido y hay que parar.

---

## 7. Lo que este documento **no** decide

- ~~**La forma de `writes`.**~~ **Sin objeto**: el
  [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md) retira la cara. No hay
  vocabulario que elegir porque no hay pregunta que hacerle al origen.
- **Si `Entity` sigue siendo un `kind`.** M2 dice que deja de repetir; no dice si lo que queda se
  llama igual.
- **Cuándo.** Primero se trabaja el sustrato — materializar de verdad, contra fuentes reales. La
  abstracción se abre después, y este documento es donde se retoma.

---

## 8. El claim, y por qué invierte el defecto de la industria

Seis reglas de este documento y de los ADR dicen lo mismo con seis sujetos distintos:

| | lo que se debe materializar |
|---|---|
| `OOS2020` | lo que **no se puede leer** |
| `OOS2025` | lo que se **escribe** |
| `OOS2026` · propuesta | lo que se **atraviesa** |
| *el del residuo* · propuesta | lo que no se puede **empujar** |
| *el de la junta* · propuesta | lo que se **une** — los dos lados |
| *el de servir* · propuesta | lo que se **sirve** |

Y las seis se dejan decir en una:

> ### El sustrato responde desde la copia. El origen se lee **para hacerla**, no para contestar.

### 8.1 · Eso es lo contrario de lo que hace la virtualización, y hay que decirlo

**Denodo** —el referente del sector— es explícito: su módulo de materialización *«está pensado para
**complementar** un enfoque virtual-first, no para reemplazarlo»*, y materializa como excepción,
para *«fuentes lentas, sistemas heredados, servicios web o APIs con límite de peticiones»*.
**Trino** no tiene caché propia. **Cube** tiene las dos cosas: sus *pre-aggregations* son rollups
materializados con *aggregate awareness* que enrutan la consulta *«en vez de ir a la fuente»*, y el
*query pushdown* es un mecanismo **aparte**.

O sea: **virtual por defecto, materializar cuando duele.** Nosotros proponemos lo contrario, y una
inversión así necesita una razón que no sea el gusto.

### 8.2 · La razón, y es estructural

Es una sola frase y se puede comprobar:

> **La virtualización materializa como excepción porque no sabe qué le van a preguntar. Una
> ontología sí lo sabe: una vista ES la pregunta, declarada.**

Denodo recibe SQL arbitrario y no puede materializar lo que no ha visto todavía, así que su defecto
tiene que ser federar. Aquí el conjunto de preguntas está **cerrado y escrito en el paquete**:
`backedBy` nombra una vista, y una vista declara `from`, `fields` y `where`. Materializar lo
declarado no es una limitación — **es lo que tener declaraciones compra**.

Y hay una convergencia que no se buscó: **el mismo recorte del vocabulario que hace posible
escribir** —renombrar, recortar, proyectar, el fragmento invertible de §3.0— **es el que hace
finito el conjunto de consultas**. Una restricción, dos beneficios que no se pidieron juntos.

### 8.3 · Y la ley del ejecutor, dicha por la industria

`05-ejecutor` §2 dijo en v1alpha1 que un motor **no debe compensar** lo que la fuente no sabe hacer,
porque acabaría siendo *«un almacén de datos mediocre además de un motor de ontologías»*. La
industria de la virtualización llegó a la misma frase por su cuenta:

> *«El pushdown es la característica de diseño **más consecuente** de un motor de virtualización;
> sin él, cada consulta degenera en traerse todos los datos de la fuente al motor y filtrarlos allí,
> lo que **no escala más allá de una demostración**.»*

**El principio se valida.** Lo que cambia entre ellos y nosotros es la salida cuando el pushdown no
alcanza: **Denodo compensa** con su propio motor federado —*«las juntas entre fuentes que no se
pueden empujar se ejecutan en el motor federado de Denodo»*— y **nosotros materializamos**. Ellos
pueden compensar porque **son** un motor de consultas; nosotros decidimos no serlo, y
`dependencias.rs` lo hace cumplir leyendo el `Cargo.lock`.

### 8.4 · Y en nuestra propia categoría, la ontología, ya es el estándar

Los dos que hacen esto no son de virtualización:

| | cómo sirve |
|---|---|
| **Palantir Foundry** | **nunca desde el puntero**. Los datos se sincronizan a un índice separado —el *Object Data Funnel*— y la ontología se sirve de ahí. Una *virtual table* puede respaldar un tipo de objeto y aun así las ediciones y las lecturas van al índice |
| **Cognite** | desde el **container**, que es almacenamiento primario. *«Data is not queried directly from the containers»* se refiere a la **view**, que mapea; el dato sale del container |
| **Cube** | construyó **Cube Store**, un motor OLAP propio en Rust, para servir sus materializaciones |

> **Nadie sirve una ontología desde un puntero.** Los tres tienen almacenamiento propio para
> servir, y los tres lo construyeron a propósito. Nuestro `ore-store-r2` es la misma pieza.

### 8.5 · El precio, contado — y el sexto gemelo **no se puede escribir**

`pruebas-de-fuego/medida-servir.py`, sobre el corpus entero:

```text
vistas que SIRVEN a una entidad     23
  YA se sirven de una copia         12   ← 52 %, sin que ninguna regla lo obligue
    (de una copia mas abajo)         4
  VIRTUALES, dejarian de compilar   11   ← 48 %
    y ademas necesitan conducto      5
```

**Once de veintitrés**, y cinco de ellas necesitarían además un `ConduitPolicy` entero para
autorizar `materialization.payload` — que es `OOS4011`, y no es una línea. Frente a las **dos
entidades** que cuesta `OOS2026`.

> **No es asequible.** Se dice con el mismo criterio con el que se aceptó el precio de `OOS2026`:
> allí eran dos casos y aquí es la mitad del corpus.

#### Y la medida corrigió cómo estaba formulada la regla

La primera versión preguntaba *«¿esta vista declara `materialized`?»* y daba **15**. Cuatro de esos
quince eran falsos: una vista **virtual sobre una materializada** ya se sirve de una copia — lo
decide `raíz de lectura`, y el caso `materialized-view-over-table-within-clearance` lo tenía en el
nombre.

> La regla no es *«esta vista se materializa»*: es **«su raíz de lectura es una copia»**. Y esa
> pregunta ya la contesta el árbol, así que el sexto gemelo no habría necesitado maquinaria nueva
> — solo un sujeto bien elegido.

#### Lo que se queda, entonces

**El claim se queda como dirección y como defecto, no como regla.** Cinco de los seis gemelos son
asequibles y cuatro ya están escritos o medidos; el sexto describe **lo que ya hace el 52 % del
corpus sin que nadie se lo pida**, y esa es la forma de validación que importa: la gente lo hace.

Y con eso volvemos a la misma postura que Cube y Denodo en el único punto donde íbamos a divergir:
**la vuelta al origen se queda como derecho.** Lo que sí cambia respecto a ellos, y es el claim
entero, es el **defecto**: ellos federan salvo que duela; aquí se materializa lo declarado, y
federar es lo que queda para quien no lo declaró.

**Lo que sigue abierto**, y ahora con número: si el 48 % baja —porque `discover` proponga
`materialized`, o porque los casos se migren— el sexto gemelo vuelve a la mesa. Hoy no.

---

### Fuentes

- [Cube · using pre-aggregations](https://docs.cube.dev/docs/pre-aggregations/using-pre-aggregations) ·
  [query pushdown](https://cube.dev/blog/query-push-down-in-cubes-semantic-layer)
- [Denodo · expert trail: materialization](https://community.denodo.com/expert-trails/view/document/Expert%20Trail:%20Materialization) ·
  [best practices: caching](https://community.denodo.com/kb/en/view/document/Best%20Practices%20to%20Maximize%20Performance%20III%3A%20Caching)
- [Palantir · object indexing](https://www.palantir.com/docs/foundry/object-indexing/overview) ·
  [Cognite · containers, views, data models](https://docs.cognite.com/cdf/dm/dm_concepts/dm_containers_views_datamodels)

