# Handoff · el motor de vistas

> **Este documento es desechable.** Se borra el día que M6 esté en verde o se declare fuera de
> alcance por escrito. Un plan que sobrevive a su ejecución deja de ser un plan.
>
> Fecha: 2026-09-01 · Derivado de lo que publican Apache Calcite, Substrait, Feldera/DBSP,
> Trino, OpenLineage, Snowflake, Palantir Foundry y Cognite Data Fusion.
>
> **La pieza se diseña libre.** Nada de aquí está condicionado por lo que hoy hay en el árbol.
> Lo que esta pieza le hará a las reglas actuales está en §7, **al final y a propósito**, para
> que se decida a sabiendas y no para que decida el diseño.

---

## 1. Qué es un motor de vistas, técnicamente

Los siete jugadores tienen **los mismos siete órganos**. No se parecen: son el mismo animal.

| | Órgano | Qué hace | Quién lo tiene canónico |
|---|---|---|---|
| **1** | **catálogo** | definiciones versionadas con identidad | CDF (views y data models versionados; containers no) |
| **2** | **IR** | álgebra relacional serializable | **Substrait** · `RelNode` de Calcite |
| **3** | **expansor y reescritor** | mete las vistas en el plan; contesta desde materializaciones | **Calcite** |
| **4** | **capacidades y empuje** | qué sabe hacer el origen, qué queda de residuo | **Trino**, SPI de conector |
| **5** | **ejecución del residuo** | correr lo que el origen no pudo | Velox · DataFusion · DuckDB |
| **6** | **mantenimiento incremental** | mantener lo materializado al día en O(Δ) | **DBSP/Feldera** |
| **7** | **linaje a nivel de columna** | qué columna raíz produce qué columna de salida | **OpenLineage** |

Y ahora el corte que importa:

> **Los órganos 1, 3, 4 y 7 son metadatos. Los órganos 2, 5 y 6 son cómputo — salvo que el 2
> es metadato *sobre* cómputo, y es el que desbloquea a los otros tres.**

De ahí la naturaleza de la pieza, dicha en una línea:

> **Un motor de vistas es un compilador de álgebra relacional con un catálogo versionado, un
> modelo de capacidades y un reescritor. La ejecución es de otro.**

No es una opinión: es la descripción literal de **Apache Calcite**, que no tiene ni
almacenamiento ni ejecución y es el motor de vistas de media industria; y de **Substrait**, que
es una especificación sin motor. Los dos artefactos más usados de esta categoría **no ejecutan
nada**.

---

## 2. El IR es la pieza

Todo lo demás cuelga del órgano 2. Sin un plan que se pueda mirar no hay reescritura, no hay
linaje derivado, no hay empuje y no hay incremental — solo hay cadenas de SQL.

**Substrait** es la respuesta publicada: especificación en protobuf de álgebra relacional,
*«SQL es el lenguaje de consulta para humanos; Substrait es lo que los motores intercambian»*.
La adoptan DuckDB, Apache DataFusion, Velox e Ibis, y permite que **un trozo del plan lo
ejecute un motor y otro trozo otro**, con la frontera expresada en Substrait.

Dos decisiones que hay que tomar y **no están tomadas**:

**¿IR propio o Substrait?** Propio es más simple y encaja con una forma canónica que ya sabemos
hacer determinista. Substrait da portabilidad real a tres motores el primer día. La respuesta
probable es **IR propio con emisión a Substrait**, por la misma razón por la que emitimos a
Cedar y a ODCS sin ser ninguno de los dos.

**¿Hasta dónde llega el álgebra?** Cuanto más expresiva, menos analizable. La expresividad
acotada es lo que hace posible §4. Punto de partida honesto: `Scan`, `Project`, `Filter`,
`Join`, `Aggregate`, `Union`, `Distinct`, `Limit`. Sin recursión, sin ventanas, sin UDF. Lo que
no quepa entra como **expresión opaca declarada**, que cuesta la garantía de análisis y **aun
así declara entradas y salidas** para que las etiquetas fluyan de forma conservadora.

---

## 3. Los tres problemas duros, con su estado del arte real

Ninguno está resuelto del todo por nadie. Conviene saberlo antes de prometer.

### 3.1 · Contestar una consulta desde una materialización

Calcite tiene **tres** implementaciones, lo cual ya dice algo:

- **`SubstitutionVisitor`** — sustituye un trozo del árbol por la materialización, añadiendo un
  predicado residual si hace falta. Y la documentación admite el límite: *«podría necesitar
  enumerar exhaustivamente todas las reescrituras equivalentes posibles, lo que no es escalable
  con vistas complejas, p. ej. vistas con un número arbitrario de joins»*.
- **Retículos y *tiles*** — eficiente cuando las fuentes forman un esquema en estrella.
- **`MaterializedViewRule`**, sobre Goldstein–Larson — cadenas arbitrarias de `Join`, `Filter`,
  `Project`, y `Aggregate` con *roll-up*.

> **Nuestro `cache::consultar` de hoy es el peldaño 0 de esta escalera**: coincidencia exacta de
> entidad más contención de propiedades. No es poco riguroso: es el subconjunto decidible, y lo
> honesto es decir que lo es.

### 3.2 · Mantenimiento incremental

**DBSP** es la respuesta, y es teoría publicada (VLDB'23, VLDB Journal 2025), no un producto con
folleto:

- **Z-sets**: cada fila lleva un **peso**, y el peso puede ser negativo. Con eso, inserciones y
  borrados se tratan **uniformemente**, que es lo que hace que el incremental funcione para
  *cualquier* mezcla de cambios y no solo para *appends*.
- Cubre álgebra relacional completa, conjuntos y multiconjuntos, anidamiento, agregación,
  *flatmap*, recursión monótona y no monótona, y composiciones arbitrarias de todo eso.
- El coste escala con **el tamaño del cambio**, O(Δ), no con el de la tabla.

Es lo que convierte *«las ontologías nacen, crecen y se reproducen en tiempo real»* de premisa
en mecanismo. Y Feldera está escrito en Rust, lo cual no es un detalle menor.

El contraste con la alternativa comercial: las *dynamic tables* de Snowflake son declarativas
—`TARGET_LAG`, mínimo 60 s— y refrescan incrementalmente coordinando el orden de dependencias.
Es DBSP con menos ambición y un SLA declarado.

### 3.3 · Negociación de capacidades

Trino la hace **por intento**: el optimizador llama a `applyFilter`, `applyProjection`,
`applyAggregation`, `applyJoin`, `applyLimit`, `applyTopN`, `applyTableScanRedirect`, y el
conector devuelve `Optional.empty()` si no puede. Se llaman **varias veces** por consulta, y es
crítico devolver vacío cuando la llamada no tuvo efecto **o el optimizador entra en bucle**.

Nosotros lo haríamos **por declaración** —`capabilities:` en la vista—. Las dos son legítimas y
tienen precios opuestos:

| | Ventaja | Precio |
|---|---|---|
| **por intento** | exacto: refleja lo que el conector hace de verdad | necesita el conector delante; no se puede planificar sin conexión |
| **por declaración** | **se rechaza un plan sin abrir una conexión** | la declaración puede mentir |

La resolución honesta: **declarar, y dejar que el driver contradiga.** Una capacidad declarada
que el driver rechaza es una divergencia con nombre, no un fallo en ejecución.

---

## 4. Dónde está la diferencia, y sale de OpenLineage

El único órgano donde no hay competencia es el 7 cruzado con una clasificación.

OpenLineage ya modela el linaje de columna con una precisión que nadie usa para gobernar. Su
`ColumnLineageDatasetFacet` distingue:

- **`DIRECT`** — el valor de la columna de salida se derivó del de la columna de entrada.
- **`INDIRECT`** — el valor de la salida **está influido** por la entrada **sin derivarse de
  ella**. Subtipos: `GROUP_BY`, `FILTER`, `SORT`.

Y cada transformación lleva un campo **`masking`**.

Léelo con ojos de control de flujo de información: **`INDIRECT` es un flujo implícito.** Una
columna que solo aparece en un `WHERE` no sale en el resultado y **decide qué filas salen**. Si
esa columna es `gdpr.sensitivity: critical`, filtrar por ella filtra información crítica hacia
un resultado que nadie clasificó.

> **El retículo tiene que fluir también por las aristas `INDIRECT`, o filtrar por una columna
> crítica es una fuga que el linaje ve y nadie mira.**

OpenLineage lo **registra**. Foundry propaga *markings* pero por dataset y los aplica al
acceder. dbt no sabe qué es una etiqueta. **Nadie se niega a compilar.** Ahí está la pieza que
justifica construir esto en vez de comprarlo.

---

## 5. La superficie

Lo que un motor de vistas de nivel industria **expone**, antes de decidir dónde vive:

| Superficie | Verbo | Qué contesta |
|---|---|---|
| **catálogo** | `registrar`, `listar`, `versión` | qué vistas hay y cuál es esta |
| **plan** | `planificar(vista \| consulta) → plan` | qué se va a hacer, sin hacerlo |
| **linaje** | `linaje(vista) → facet` | qué columna raíz produce qué salida, y por qué arista |
| **gobierno** | `comprobar(vista) → diagnósticos` | por qué esto no compila |
| **empuje** | `repartir(plan, capacidades) → (empujado, residuo)` | qué hace el origen y qué queda |
| **refresco** | `delta(vista, desde) → plan` | qué hay que recomputar y nada más |

Las cuatro primeras **no necesitan ninguna conexión**. Es la misma propiedad que ya se
demuestra hoy con el plan, un piso más abajo.

---

## 6. Los peldaños

> **Desde aquí es desechable.**

### M0 · El IR ✅

`crates/ore-view` — `Lee`, `Proyecta`, `Filtra`, `Une`, `Agrupa`, `Unifica`, `Distingue`,
`Limita`, y la expresión opaca declarada. Forma canónica RFC 8785 —**la del bundle**, no una
segunda— y digest `sha256:` con separación de dominio `OREPLAN1`.

**Listo cuando:** dos escrituras del mismo plan dan el mismo digest, el plan va y vuelve, y un
plan con una expresión opaca **dice que lo es**. ✅ · 19 comprobaciones.

Cuatro cosas que salieron de construirlo y no estaban previstas:

**El digest es del significado, no de la escritura.** La forma canónica reordena lo conmutativo
—operandos de `Y` y `O`, ramas de `Unifica`, columnas de una proyección, pares de una junta— y
deduplica. Lo que **no** se conmuta son los lados de una junta: en una externa no son
conmutables, y tener dos reglas según el tipo de junta sería peor que no tener ninguna.

**La vuelta no reconstruye la escritura: reconstruye la forma canónica.** Lo dijo ejecutarlo. Es
lo correcto, y lo que hay que exigir es lo otro — que **leer la forma canónica sea un punto
fijo**, y eso tiene su comprobación.

**No hay literal `Float`.** `OOS6003` prohíbe los decimales sin comillas para que la forma
canónica no serialice nunca una coma flotante; aquí es la misma regla. Un decimal lleva sus
dígitos tal cual, y comparar contra un campo `Float` no cabe en v1. Mejor que no quepa a que
quepa dando un digest distinto por máquina.

**El tipado caza cuatro cosas que en un almacén no fallan hasta ejecutar** — y una de ellas no
falla nunca: juntar dos tablas con una columna del mismo nombre, unir dos ramas que no producen
lo mismo, **comparar tipos que solo se comparan por conversión implícita** —que no da un error,
da cifras incorrectas— y una expresión opaca que declara leer una columna que no existe. Esa
última es el único control que una opaca tiene, y por eso no se puede saltar: si su superficie
declarada miente, las etiquetas dejan de fluir por donde de verdad pasan.

De paso, `ore_core::types::Type` no se podía escribir de vuelta: había `parse_type` y no había
`Display`. Ahora lo hay, con la invariante `parse_type(t.to_string()) == t` ejercida sobre el
conjunto cerrado entero.

### M1 · El expansor ✅

`catalogo.rs`. Una vista es **un nombre y un cuerpo**, y el cuerpo es un plan cuyas hojas pueden
ser lecturas de una fuente **o [`Nodo::Referencia`] a otra vista**. Expandir es sustituirlas
hasta que no queda ninguna.

**Listo cuando:** una cadena de N vistas produce **un** plan, y un ciclo se rechaza nombrando
las vistas que lo forman. ✅ · `ciclo entre vistas · a → b → c → a`

Tres cosas de construirlo:

**La referencia no es una rareza: es una exploración con otro nombre.** Un `TableScan` de
Calcite se apoya en una tabla que puede ser una vista, y el `ReadRel` de Substrait admite una
`NamedTable`. Con eso, componer no necesita un concepto nuevo — y **no hace falta «pipeline»**.

**El compilador señaló una decisión que no estaba tomada.** Al añadir la referencia, el tipado
dejó de ser exhaustivo, y la respuesta correcta no era «trátala como una hoja vacía»: un plan
sin expandir **no se puede tipar**, porque un esquema sobre medio plan parece bueno. Es
`Desajuste::SinExpandir`, y tiene prueba.

**El camino, no un conjunto de visitados.** Es el error clásico: con un `visitados` global, un
**rombo** —dos vistas apoyadas en una tercera— se confunde con un ciclo, y la primera persona
que reutilizara una vista limpia en dos sitios se encontraría un ciclo que no existe. Hay una
comprobación que falla si alguien lo cambia.

Y dos límites que salen por su nombre en vez de producir algo raro: **incorporar duplica** —una
vista referenciada dos veces se copia dos veces, y lo que lo arregla es M5, no un parche aquí— y
**no hay alias**, así que dos referencias a la misma vista chocan por nombre de columna y lo
dice `ColisionAlUnir`.

### M2 · El linaje, derivado del plan ✅

`linaje.rs`. Calculado del IR, no observado de una ejecución, así que existe antes de que nadie
abra nada. Vocabulario de OpenLineage tal cual —`DIRECT` con `IDENTITY`/`TRANSFORMATION`/
`AGGREGATION`, `INDIRECT` con `JOIN`/`GROUP_BY`/`FILTER`— y emisión del facet `columnLineage`.

**Listo cuando:** cada campo de salida nombra sus campos raíz, y **un campo que solo aparece en
un filtro sale como `INDIRECT`**. ✅ · 39 comprobaciones en el crate.

**La regla de composición es una y es la que hace que la influencia no se pierda:**

> **Derivar es transitivo solo a través de derivaciones. Si cualquiera de los dos pasos es
> influencia, el resultado es influencia.**

Sin ella, proyectar después de filtrar borraría el flujo implícito — y el análisis parecería más
limpio justo donde deja de ser cierto. Hay una prueba con cuatro proyecciones apiladas sobre un
filtro.

Cuatro decisiones que salieron de escribirlo:

| | |
|---|---|
| **`DIRECT` e `INDIRECT` son familias, no un enum plano** | `DIRECT` con subtipo `FILTER` no significa nada, y un enum plano lo dejaría escribir |
| **`Distingue` produce `GROUP_BY`** | quitar duplicados **es** agrupar por todas las columnas, que es como lo reescribe cualquier planificador |
| **`Limita` no inventa aristas** | sin orden, qué filas sobreviven no lo decide ninguna columna. `SORT` llegará con el nodo de orden |
| **`cuenta` sin columna no queda huérfana** | no deriva de ninguna, y aun así su valor lo decide qué filas caen en el grupo: sale por las aristas de agrupación |

Y la opaca sigue pagando su precio y cumpliendo su parte: **su `lee` declarado entra en el
linaje**, así que un trozo que nadie entiende sigue dejando fluir las etiquetas de forma
conservadora.

Dos cosas del facet que **no** se emiten, con su razón: `masking` —saber si algo enmascara es un
juicio de gobierno, y aquí no hay retículo: es de M3— y el array `dataset` de dependencias
indirectas de todo el conjunto —es una compactación, y tener el mismo hecho en dos sitios es lo
que este proyecto no hace—.

### M3 · El retículo fluye por el linaje, y se niega a compilar ✅

`flujo.rs`. **El peldaño que vale dinero:** todo lo anterior lo tienen otros.

**Listo cuando:** una vista que expone por debajo de la clasificación de una entrada no
compila, **y el caso `INDIRECT` falla igual**. ✅ · 52 comprobaciones en el crate.

```text
`cuanto` no compila
  ← lago·ventas.pedidos.nif  por INFLUENCIA (Filtro)
  gdpr.sensitivity del origen    : critical
  gdpr.sensitivity de esta vista : medium
```

`nif` **no sale** en el resultado: solo se filtra por él. En control de flujo de información eso
es un **flujo implícito**, y el tratamiento clásico —Denning— es que la etiqueta de la condición
se une a todo lo que se computa bajo ella. Aquí es literal: una arista `INDIRECTO` clasifica
igual que una `DIRECTO`.

Se podría argumentar que un flujo implícito filtra *menos*. **No tenemos el argumento
cuantitativo**, y aflojar sin él sería aflojar por comodidad en la dirección insegura. Lo que
hace vivible la regla estricta no es relajarla: es **desclasificar explícitamente**, que es lo
que una máscara de `Ruleset` ya hace en OOS.

**Y hay una cosa que no habría acertado solo**, que estaba escrita en `ore_core::flow::Axis`
desde antes que esta pieza:

| Eje | Pregunta | Combina | Viola si |
|---|---|---|---|
| **confidencialidad** | *¿cuánto daño si esto se filtra?* | `max` | la salida sale **por encima** de lo autorizado |
| **integridad** | *¿cuánto daño si esto es falso?* | `min` | la salida queda **por debajo** de lo exigido |

Con `max` en los dos, una vista que junta un dato fiable con uno dudoso **parecería fiable**.
Por eso el retículo se reutiliza entero en vez de redefinirlo: la regla ya estaba escrita, y una
segunda copia habría divergido justo aquí. Hay una prueba que pasa **el mismo par de etiquetas**
por los dos ejes y comprueba que salen al revés.

Tres decisiones más, todas en la dirección de no callar:

| | |
|---|---|
| **una raíz sin etiqueta no participa** | no es el fondo ni lo más alto: es que no está. Es la convención del compilador, y otra aquí clasificaría la misma columna distinto según quién pregunte |
| **un nivel mal escrito no es «estar por debajo»** | confundirlos convertiría una errata en una columna que parece limpia |
| **se dicen todas las fugas y todos los culpables** | un compilador que informa de un error cada vez se ejecuta diez veces; y arreglar una raíz dejando la otra no arregla nada |

Y una medida del propio trabajo: **el tipador de M0 rechazó tres de mis fixtures** por comparar
`Decimal` con `Integer`. Tenía razón.

### M4 · Capacidades y reparto ✅

`capacidades.rs`. `(empujado, residuo)` a partir de las capacidades declaradas.

**Listo cuando:** un plan contra un origen con `fullScan: forbidden` y sin claves se rechaza
**sin abrir una conexión**, y uno con `predicatePushdown: [eq]` empuja el `eq` y deja el resto
de residuo, **diciendo cuál es el residuo**. ✅ · 67 comprobaciones en el crate.

Es lo que convierte un escaneo en una búsqueda por clave — el argumento entero del ADR 0006. Un
predicado se aplana en conyuntos y baja hasta la hoja, **y cada nodo por el que pasa tiene su
regla y su motivo**:

| | Qué baja | Por qué |
|---|---|---|
| **proyección** | lo que nombra columnas copiadas tal cual | reescribir un predicado sobre algo computado es donde un optimizador se equivoca en silencio |
| **junta** | cada conyunto al lado que tiene sus columnas | el que cruza los dos lados cambiaría el resultado si bajara a uno |
| **grupo** | solo sobre claves de grupo | filtrar por un agregado es un `HAVING`, y por debajo del grupo no significa nada |
| **unión** | a **todas** las ramas | la propiedad distributiva, y es lo que evita traer entera la rama que no aporta |
| **límite** | **nada** | filtrar y luego limitar no es limitar y luego filtrar |

Esa última es la trampa, y está encodada con su prueba: equivocarse ahí **devuelve un resultado
plausible** — salen filas, menos de las que debían.

**Dos puertas que se cierran sin abrir nada**, que es toda la gracia de declarar en vez de
intentar: `fullScan: forbidden` sin ningún filtro bajado, y un `requiredFilters` que no llega. Y
**la ausencia de capacidades es una negativa, no una laguna** — P4 aplicada al reparto, y
`05-ejecutor` §5.1 dicho en código.

Y la escapatoria deja de ser solo cara: **una opaca se empuja si el origen habla su dialecto**.
Lo que no hace es cruzar una proyección que renombra — su texto nombra columnas por dentro y
este motor no lo lee, así que traducir solo su `lee` dejaría el texto apuntando a nombres que ya
no existen.

Dos cosas que M4 **no** hace, con su razón: **no poda columnas** —es una segunda optimización
con su propia medida, y hacerla sin medirla sería inventarla— y **no baja agregados ni juntas**,
que son las dos que más cambian el reparto y las dos que más fácil se hacen mal.

Y el aviso que quedó por escrito en la cabecera del módulo: bajar un predicado y quitarlo del
residuo **confía en la declaración**. Para un filtro cualquiera es una optimización; para uno que
restringe **qué puede ver un principal**, confiar es devolver filas de más si el origen lo
ignora. Esta pieza no sabe cuál es cuál, así que deja el predicado escrito en la `Peticion` para
que quien sí lo sepa pueda volver a aplicarlo — y cuando esto se absorba, el campo que los marca
ya existe: es el `ambito` de un filtro.

### M5 · Reescritura con materializaciones

> **Medido en profundidad en el [Anexo](#anexo--m5-y-m6), Parte I.** Lo de aquí es el
> planteamiento; allí están las cuatro condiciones, la frontera de decidibilidad y los seis
> peldaños en que se parte.

Subconjunto honesto primero: misma exploración, contención de proyección, e **implicación de
predicados** para conjunciones de comparaciones simples. La reescritura con un número arbitrario
de joins **queda fuera**, y Calcite documenta por qué.

**Listo cuando:** un plan se contesta desde una materialización cuya proyección lo contiene y
cuyo predicado lo implica; y uno cuyo predicado **no** lo implica va al origen **con el
motivo**.

### M6 · Mantenimiento incremental

> **Medido en profundidad en el [Anexo](#anexo--m5-y-m6), Parte II.** Allí están las reglas
> delta operador por operador, lo que no se puede incrementalizar, y las tres formas del sector
> para el problema del estado.

DBSP: Z-sets con pesos, circuitos, O(Δ).

**Listo cuando:** aplicar un delta a una vista materializada da el mismo estado que
recomputarla, sobre una secuencia generada de inserciones **y borrados** mezclados — que es
justo el caso que los modelos de solo-*append* no cubren.

### Lo que hay que decir de M5 y M6

**Son los dos peldaños de investigación.** M5 es un problema conocido y parcialmente resuelto
por el mejor planificador de código abierto que existe, y M6 es una tesis de VLDB con una
empresa detrás. Ponerlos en una lista al lado de M0 no los hace del mismo tamaño.

**M0–M4 es un motor de vistas útil y completo sin ninguno de los dos**: planifica, empuja,
deriva linaje y se niega a compilar. M5 lo hace rápido. M6 lo hace de tiempo real. **Prometer
M6 con fecha sería la clase de promesa que este proyecto no hace.**

---

## 7. Lo que esta pieza le hace a las reglas de hoy

Deliberadamente al final. No condiciona el diseño; lo paga.

**El veto de dependencias.** Substrait es protobuf, y `prost` es Rust puro: emitirlo **no** rompe
la hermeticidad. Es mejor noticia de la esperada.

**«ORE no opera ninguna base de datos».** M6 la rompe. El mantenimiento incremental **tiene
estado** —los circuitos DBSP guardan el pasado— y ese estado hay que ponerlo en algún sitio. Las
salidas son tres y hay que elegir a sabiendas: que viva en el lago del cliente, que viva en un
programa delegado, o que se reabra el ADR 0006. **No la elijo yo.**

**La ejecución del residuo.** O se empuja todo y se rechaza lo que no se pueda empujar
—coherente con lo que ya hacemos, y limitante—, o hace falta un motor de consulta delegado. Es
una bifurcación real y también es una decisión, no un detalle.

**El `Binding` ya estaba condenado** por `handoff-vistas.md`; esto no añade nada ahí.

---

## 8. Lo que **no** entra

**Escribir un motor de ejecución.** Los dos artefactos más usados de esta categoría no ejecutan
nada. Escribir uno sería competir donde hay tres proyectos maduros y ninguna ventaja.

**Reescritura con joins arbitrarios.** §3.1, con la cita de quien lo intentó.

**Recursión, ventanas y UDF en el álgebra de v1.** §2. Lo que no quepa entra como expresión
opaca y paga el análisis.

**Milisegundos antes de M6.** Y después de M6, solo sobre lo materializado.

---

# Anexo · M5 y M6

> **Este anexo es desechable**, como el resto del documento. Se borra con él.
>
> Fecha: 2026-09-01 · Escrito **después** de cerrar M0–M4, y **antes** de tocar una línea de
> M5 o M6: los dos son peldaños de investigación y merecen medida antes que código.
>
> Derivado de Goldstein–Larson (SIGMOD'01), Halevy (VLDB J. 2001), Oracle, Apache Calcite,
> BigQuery, Databricks Enzyme, Snowflake, DBToaster (VLDB'12), DBSP (VLDB'23), Materialize /
> differential dataflow y Noria (OSDI'18).

---

## Parte I · M5, que lo hace **rápido**

### I.1 · El problema tiene nombre desde hace veinticinco años

No es *«usar la caché»*. Es:

> **¿Puede esta materialización contestar este plan, y con qué compensación?**

Se llama **answering queries using views** y tiene un survey de referencia (Halevy, VLDB J.
10(4), 2001). Nombrarlo bien importa porque decide qué se puede prometer: hay una frontera de
decidibilidad y está estudiada.

### I.2 · Las cuatro condiciones, que son las mismas en todas partes

Oracle las tiene escritas como una lista de comprobaciones, y Calcite y SQL Server implementan
la misma forma bajo otro nombre. Son estas, y **el orden importa**:

| | Condición | Qué falla si se salta |
|---|---|---|
| **1** | **compatibilidad de juntas** | las juntas de la vista tienen que estar en el plan, y una junta **de más** en la vista solo vale si es **sin pérdida**: si no, tira o duplica filas |
| **2** | **suficiencia de datos** | cada columna que el plan necesita tiene que poder derivarse de lo que la vista produce |
| **3** | **subsunción de predicados** | el predicado del plan tiene que **implicar** el de la vista; lo que sobra es la **compensación**, que se aplica encima |
| **4** | **computabilidad de agregados** | la agrupación de la vista tiene que ser igual o **más fina**, y los agregados tienen que poder **enrollarse** |

Dos detalles de la 1 y la 4 que son los que hunden una implementación ingenua:

**La ausencia de pérdida se demuestra con restricciones, no se supone.** Oracle es explícito:
*«las restricciones se usan para determinar juntas sin pérdida»*. Y Calcite lo mismo: se apoya
en *«claves ajenas, claves primarias, claves únicas o `not null`»* para reconocer cuándo una
junta **solo añade columnas sin cambiar la multiplicidad de las tuplas**. Sin una clave
declarada, una junta de más en la vista es una reescritura que devuelve otro número de filas.

**`AVG` no se enrolla.** `SUM`, `COUNT`, `MIN` y `MAX` sí. `AVG` solo si la vista guardó
`SUM` y `COUNT` por separado, y los agregados con `DISTINCT` no se enrollan en general. Es el
error clásico de una implementación que trata todos los agregados igual.

### I.3 · Dónde para lo decidible

Conviene tenerlo delante antes de prometer nada:

- **La contención de consultas conjuntivas es NP-completa** (Chandra–Merlin, por el teorema del
  homomorfismo). Con comparaciones arbitrarias sube más.
- **La determinación** —*¿se puede contestar esta consulta a partir de estas vistas, de alguna
  manera?*— es **indecidible** para consultas conjuntivas. Está probado, y el título del
  artículo es memorable: *The Hunt for a Red Spider*.
- Y la propia documentación de Calcite admite el límite práctico de su sustitución: *«podría
  necesitar enumerar exhaustivamente todas las reescrituras equivalentes posibles, lo que no es
  escalable con vistas complejas, p. ej. vistas con un número arbitrario de juntas»*.

> **Consecuencia:** M5 no es *«implementar la reescritura»*. Es **elegir el subconjunto
> decidible y decir cuál es**. Prometer más sería prometer algo que nadie tiene.

### I.4 · Y cómo se escala a muchas vistas

La aportación de Goldstein–Larson que se cita menos es la que decide si esto sirve en
producción: el **filter tree**, un índice sobre las vistas para que buscar candidatas no sea
recorrerlas todas. Calcite reconoce que no lo tiene: *«la regla intenta emparejar todas las
vistas contra cada consulta. Planeamos implementar técnicas de filtrado más refinadas»*.

**Nosotros ya tenemos la firma para ese índice**: `Nodo::lecturas()` da las hojas
`(datasource, objeto)` de un plan desde M0. Indexar las vistas por ese conjunto es el filtro
barato de primer paso.

### I.5 · Qué hace cada uno, para no inventar

| | Enfoque |
|---|---|
| **Oracle** | coincidencia de texto (exacta y parcial) primero; reescritura general con las cuatro condiciones después |
| **Calcite** | **tres** implementaciones: `SubstitutionVisitor`, retículos y *tiles*, y `MaterializedViewRule` sobre Goldstein–Larson. Reescribe cadenas de `Join`/`Filter`/`Project`, enrolla agregados y produce **respuestas parciales con `Union`** |
| **BigQuery** | *smart tuning*: reescribe **aunque la consulta no nombre la vista**. Condiciones: mismo proyecto, **mismas tablas base**, todas las columnas leídas, todas las filas leídas |
| **Databricks** | `Enzyme` + **modelo de coste**: decide incremental o completo según cuál sale más barato |
| **Snowflake** | *dynamic tables* con `TARGET_LAG` declarado |

Dos cosas que se copian tal cual porque están bien:

**La respuesta parcial con unión.** Cuando la vista cubre un trozo, Calcite la combina con una
consulta al origen por el resto en vez de descartarla. Es la diferencia entre una caché que
sirve el 90 % y una que no sirve.

**Reescribir aunque nadie nombre la vista.** Es lo que hace que la materialización sea una
decisión de operación y no un cambio en cada consulta. Es la forma correcta.

### I.6 · Dónde estamos, dicho en este vocabulario

`ore_core::cache::consultar`, tal y como está hoy:

| Condición | Estado |
|---|---|
| 1 · juntas | **no se mira** — la entrada es por entidad, no por plan |
| 2 · suficiencia | ✅ contención de propiedades |
| 3 · subsunción | **no se mira** — no hay predicados en la entrada |
| 4 · agregados | **no se mira** |

Es el **peldaño 0 de la escalera**, y no es poco riguroso: es el subconjunto trivialmente
decidible. Lo que M5 añade es la 3 primero y la 4 después, sobre planes en vez de sobre
entidades.

### I.7 · Y una condición que es solo nuestra

Ninguno de los cinco la tiene, porque ninguno tiene M3:

> **Contestar desde una materialización no puede bajar la clasificación.**

Y la forma correcta no es recalcular el linaje del plan reescrito —que sería **más corto**,
porque la materialización ya aplicó el filtro, y por tanto **más limpio de lo que es**—. Es:

> **La clasificación de una materialización se hereda, no se recalcula.**

Una vista que filtró por `nif` produce un resultado `critical` aunque `nif` no esté en sus
columnas. Si al reescribir se recalculase el linaje sobre la tabla materializada, esa columna
desaparecería y con ella la etiqueta. **Es exactamente el fallo que M2 y M3 existen para
impedir, entrando por la puerta de M5.**

Y su generalización ya está construida: `cache::ReglaDistinta` — una materialización escrita
bajo otro bundle no vale. M5 lo extiende de *«otro bundle»* a *«otra autorización»*.

### I.8 · Los peldaños de M5

| | Qué | Coste |
|---|---|---|
| **M5.0** | el índice de candidatas por firma de hojas | bajo · la firma ya existe |
| **M5.1** | suficiencia de datos **sobre planes**, con clases de equivalencia de igualdades | medio |
| **M5.2** | **subsunción de predicados** sobre conjunciones de comparaciones simples, y la **compensación** encima | medio · es el corazón |
| **M5.3** | enrollado de agregados, con la regla de `AVG` | medio |
| **M5.4** | juntas, **solo cuando una clave declarada demuestra que no hay pérdida** | alto |
| **M5.5** | la herencia de clasificación de §I.7 | bajo · y es la que nadie tiene |

**Fuera de M5, y con su razón:** reescritura con un número arbitrario de juntas (§I.3, con la
cita de quien lo intentó), subsunción de disyunciones, y cualquier reescritura que toque una
expresión opaca — su texto no se lee, así que no se puede razonar sobre él.

**M5.0 a M5.3 es un reescritor útil.** M5.4 es donde empieza a ser caro.

---

## Parte II · M6, que lo hace **de tiempo real**

### II.1 · Tres generaciones, y la tercera es la buena

| | Idea | Coste de mantener |
|---|---|---|
| **contar** (Gupta–Mumick–Subrahmanian, 1993) | deltas de primer orden y un contador por fila | hay que recomputar juntas |
| **orden superior** (DBToaster, VLDB'12) | la **transformada viewlet**: se materializa la delta, y la delta de la delta, hasta que la k-ésima es constante | *«para un fragmento grande de SQL, el IVM de orden superior **evita el procesamiento de juntas**, reduciendo todo el refresco a sumas»* |
| **algebraico** (differential dataflow 2013, **DBSP** VLDB'23) | Z-sets con pesos con signo; la incrementalización es **mecánica** | O(\|Δ\|) |

### II.2 · Lo que DBSP dice, exactamente

Tres frases, y las tres deciden diseño:

> **`Q^Δ = D ∘ Q ∘ I`.** Diferenciar, aplicar la consulta, integrar. **Cualquier circuito se
> incrementaliza mecánicamente**, sin escribir a mano la regla de cada operador.

> **Aunque `Q` sea una función pura, `Q^Δ` es un sistema con estado, y ese estado vive
> *enteramente* en los operadores de retardo `z⁻¹`.**

> Los **Z-sets** dan a cada fila un peso que puede ser **negativo**, y por eso inserciones y
> borrados se tratan igual. Es lo que hace que funcione para cualquier mezcla de cambios y no
> solo para *appends*.

La segunda es la que contesta la pregunta que quedaba abierta: **el estado no es difuso, es
exactamente los integradores.** Se puede enumerar, medir y decidir dónde vive.

Y el teorema bilineal da la regla de junta clásica sin inventarla:
`Δ(a ⋈ b) = Δa ⋈ Δb + a ⋈ Δb + Δa ⋈ b`.

### II.3 · Nuestra álgebra, operador por operador

Esto es lo que M6.0 tendría que escribir, y sale de la teoría sin margen:

| Operador | Regla delta | Estado que exige |
|---|---|---|
| `Proyecta`, `Filtra` | **lineales**: `Δσ(R) = σ(ΔR)` | **ninguno** |
| `Unifica` | lineal | ninguno |
| `Une` | **bilineal** — la fórmula de §II.2 | **los dos lados, indexados por la clave** |
| `Agrupa` con `SUMA`/`CUENTA` | homomorfismos de grupo: un acumulador por grupo | acumulador por grupo |
| `Agrupa` con `MIN`/`MAX` | **no invertibles bajo borrado** | el multiconjunto del grupo, o volver a derivar |
| `Agrupa` con `PROMEDIO` | no es un homomorfismo | `SUMA` y `CUENTA` por separado — **la misma regla que M5.3** |
| `Distingue` | cuenta por fila | una cuenta por fila distinta |
| `Limita` | **no incrementalizable en general** | borrar dentro del top-N exige conocer el N+1 |

Dos observaciones que valen más que la tabla:

**Los operadores sin estado son exactamente los que M4 empuja al origen.** Lo que se queda
arriba es lo que cuesta mantener. Las dos piezas encajan sin haberlo buscado.

**`MIN`/`MAX` bajo borrado y `Limita` son los dos casos duros**, y son los mismos dos que
complican a todo el mundo. Nombrarlos antes de empezar es la diferencia entre un peldaño y una
sorpresa.

### II.4 · Lo que **no** se puede incrementalizar, y todos dicen lo mismo

La lista de Snowflake es la verdad empírica del sector — lo que su motor **no** mantiene
incrementalmente: juntas laterales, subconsultas fuera del `FROM`, **UDF volátiles**,
`INTERSECT`/`EXCEPT`/`MINUS`, `PIVOT`/`UNPIVOT`, percentiles exactos, `RANDOM()`,
`UUID_STRING()`, `SEQ`, y los operadores `IN`/`ANY`/`ALL`/`EXISTS`. Y cuando aparecen,
**cae a refresco completo**.

De ahí salen dos lecciones, y la primera es un regalo:

> **El determinismo es precondición de la incrementalidad.** Un `RANDOM()` no se mantiene.

Nuestra álgebra **no tiene** funciones volátiles, ni reloj, ni aleatoriedad, ni siquiera
literales `Float` — y esa última la decidimos en M0 por el digest, no por esto.
**El álgebra de M0 es incrementalizable por construcción**, y es un dividendo que no se buscó.

Lo que sí tenemos es la **expresión opaca**, que es exactamente el agujero de Snowflake con
otro nombre. La respuesta honesta ya está escrita en su contrato: declara `lee` y declara
`tipo`, y **no declara determinismo**. Un `Opaca` sin una declaración de pureza no se puede
incrementalizar, y eso es un campo que hay que añadir — o una vista que la contenga cae a
recomputar, y lo dice.

La segunda lección es más incómoda:

> Snowflake documenta que lo incremental gana cuando **cambia menos del 5 %** de la tabla base
> entre refrescos. Por encima, gana recomputar.

Es decir: **un modelo de coste no es opcional.** Databricks lo hace explícito con `Enzyme`, que
elige entre incremental y completo *por coste*. Un motor que siempre incrementa es más lento que
uno que sabe cuándo no hacerlo.

### II.5 · El estado: tres formas, y una encaja con la tesis

Es la decisión que queda pendiente, y ahora se puede tomar con las formas del sector delante:

| | Cómo lo resuelve | Qué exige de nosotros |
|---|---|---|
| **Materialize** | *arrangements*: índices **en memoria** por clave y tiempo, multiversión. Y un *delta join* que no mantiene estructuras adicionales | operar un servicio con memoria |
| **Feldera** | **desborda a disco** sobre NVMe, con checkpoints incrementales cada 60 s y un log de entrada para reproducir | operar un almacén |
| **Noria** (OSDI'18) | **estado parcial**: cada operador mantiene **solo un subconjunto**; los desalojos fluyen hacia delante y las ***upqueries*** hacia atrás repueblan lo que falte | **casi nada** |

> **Noria es la que encaja, y no por casualidad.**

El estado parcial dice: guarda solo lo que se ha pedido, desaloja lo demás, y cuando falte algo
**pregúntaselo a quien lo tiene**. En un sistema que posee sus datos, la *upquery* va a un
operador de más abajo. En el nuestro, **la de más abajo es la fuente del cliente** — y
preguntarle es exactamente lo que M4 ya sabe planificar.

> **Una *upquery* es un plan.** Y un fallo de caché es un `Veredicto::NoMaterializada`, que ya
> existe y ya dice *«leer de la fuente»*.

Con eso, las tres salidas de §7 del documento principal se ordenan solas:

1. **Estado parcial, en el almacenamiento del cliente** — la forma de Noria. Lo que ORE guarda
   sigue siendo metadato: qué claves están calientes y bajo qué identidades. No contradice nada
   de lo escrito.
2. **Un programa delegado** que sostenga los *arrangements* — la forma de Feldera. ORE sigue
   hermético y el estado es de otro proceso.
3. Reabrir el ADR 0006.

**Recomendación: 1, con 2 como ejecutor.** Es la única de las tres que no obliga a retirar una
frase ya publicada, y las dos piezas que necesita —el manifiesto y el planificador— están
construidas. **La decisión sigue siendo tuya**, y conviene tomarla antes de M6.3, no durante.

### II.6 · Los peldaños de M6

| | Qué | Dónde |
|---|---|---|
| **M6.0** | las **reglas delta** por operador, con la prueba que importa: aplicar un delta da lo mismo que recomputar, sobre secuencias generadas de **altas y bajas mezcladas** | puro · `ore-view` |
| **M6.1** | el **veredicto de incrementalizabilidad**: dado un plan, ¿se mantiene incrementalmente, con qué estado, y si no, **por qué no** | puro · `ore-view` |
| **M6.2** | el **modelo de coste**: incremental o recomputar | necesita medidas que no tenemos |
| **M6.3** | el estado | necesita la decisión de §II.5 |

**M6.0 y M6.1 son puros y caben aquí.** Y M6.1 es más valioso de lo que parece: un motor que
dice *«esta vista no se puede mantener incrementalmente porque tiene un `MIN` y una opaca sin
declaración de pureza»* es un motor que se puede usar para **diseñar** vistas mantenibles, antes
de escribirlas. Snowflake solo lo descubre al refrescar.

---

## Lo que este anexo cambia del plan

Tres cosas, y ninguna estaba prevista:

**M5 no es un peldaño: son seis**, y los dos últimos son de otro tamaño. `M5.0`–`M5.3` es un
reescritor útil; `M5.4` —juntas— es donde la literatura se rompe.

**M6.1 vale por sí solo**, sin M6.3. Decir por qué una vista **no** se puede mantener no
necesita ni estado ni decisión, y es lo que permite diseñar vistas mantenibles en vez de
descubrirlo tarde.

**Y hay un campo que falta en el IR desde M0**: `Opaca` declara `lee` y `tipo`, y **no declara
si es determinista**. Sin eso no se puede saber si una vista que la contiene es
incrementalizable — y es el mismo agujero que Snowflake documenta como *«UDF volátiles»*. Es un
campo, y se ve ahora porque hasta ahora no había quien lo preguntara.
