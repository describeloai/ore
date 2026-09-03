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
> través de ellas por donde eventualmente se escribe en el origen: como función sobre la
> ontología, como consulta, o como lo que venga.

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

Nuestra `View` **es** su view, y eso ya estaba medido. Lo nuevo es el piso de abajo: **la copia
materializada es un container.** Tiene almacenamiento propio, esquema declarado, clave, testigo,
la lee cualquier motor, se funde incrementalmente y se recoge. No se parece a un container: hace
lo que hace un container.

Con **una asimetría que no se tapa**: el suyo es almacenamiento **primario** —se ingiere dentro y
se escribe—; el nuestro es **derivado** —es el resultado de un plan sobre un puntero, y es de solo
lectura—. Esa diferencia tiene nombre y es [M1](#m1--la-tercera-cara).

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

#### La respuesta, sin ambigüedad

> **La capa ontológica se sienta sobre la VISTA. Ni sobre la tabla, ni sobre la copia.**
>
> La vista se apoya en la tabla **o** en su copia, y **cuál de las dos es una decisión de
> rendimiento y de frescura, tomada abajo, que la ontología no ve.**

No es una aspiración: está construido y se ve en dos sitios del árbol.

- **`backedBy` nombra una vista**, nunca una tabla ni una copia. Una entidad no sabe si lo que la
  respalda está copiado.
- **`raíz de lectura` decide** cuál de las dos contesta, y `OOS2020` la obliga cuando el origen no
  se deja leer.

La ontología nombra **la pregunta**; el sustrato decide **quién la contesta**. Esa indirección es
la pieza entera: sin ella, mover una vista de virtual a materializada sería un cambio en el
modelo, y con ella es un cambio en una línea que nadie de arriba nota.

#### Qué queda para que el piso esté completo

Uno solo, y es el de la asimetría: **la copia es de solo lectura**. Con eso la ontología puede
**responder** sobre el sustrato y no puede **actuar** sobre él. Escribir en el origen a través de
la vista es [M1](#m1--la-tercera-cara), y es lo único que separa nuestro container del suyo.

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

`effects` apunta a una vista escribible. `I(f) ⊒ I(destino)` deja de ser una regla sin sujeto, y
escribir en el origen pasa a ser lo que siempre debió ser: **una consulta al revés**.

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

- **La forma de `writes`.** El vocabulario cerrado se elegirá midiendo contra orígenes reales, no
  aquí. Lo único decidido es que la cara existe y que la invertibilidad se deriva.
- **Si `Entity` sigue siendo un `kind`.** M2 dice que deja de repetir; no dice si lo que queda se
  llama igual.
- **Cuándo.** Primero se trabaja el sustrato — materializar de verdad, contra fuentes reales. La
  abstracción se abre después, y este documento es donde se retoma.
