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

## 3. La tercera cara, y por qué el vocabulario ya estaba elegido

Si el sustrato tiene que soportar la escritura, la tabla no tiene dos caras: tiene **tres**.

| cara | pregunta | estado |
|---|---|---|
| `reads` — `I` | qué se le puede pedir | **decidido**, v1alpha8 |
| `changes` — `D` | qué cambios emite | **decidido**, v1alpha8 |
| `writes` — `W` | qué escrituras acepta | **proyectado** |

Y aquí está lo que cierra el círculo. La regla de la versión dice `View = Q(Table)`. Escribir a
través de una vista es `Q⁻¹`: el problema clásico de actualización de vistas, que solo se resuelve
si `Q` es invertible.

**El vocabulario de la vista es exactamente el fragmento invertible.**

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

### 3.1 · Lo que eso decidió sobre la federación

Al migrar el árbol apareció que v1alpha8 **no sabe expresar una entidad servida desde N objetos**
— `crates/ore-exec/casos/dos-familias`, que el binding decía sin esfuerzo. Se barajaron tres
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

Con **una asimetría que no se tapa**: el suyo es almacenamiento **primario** —se ingiere dentro y
se escribe—; el nuestro es **derivado** —es el resultado de un plan sobre un puntero, y es de solo
lectura—. Esa diferencia tiene nombre y es [M1](#m1--la-tercera-cara).

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

## 5. Los tres movimientos, en orden

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
