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

### M2 · El linaje, derivado del plan

Calculado del IR, no observado de una ejecución. `DIRECT` e `INDIRECT` con subtipos. Emisión al
facet de OpenLineage.

**Listo cuando:** cada campo de salida nombra sus campos raíz, y **un campo que solo aparece en
un filtro sale como `INDIRECT`** — que es la mitad que nadie calcula.

### M3 · El retículo fluye por el linaje, y se niega a compilar

**El peldaño que vale dinero.** Todo lo anterior lo tienen otros.

**Listo cuando:** una vista que expone por debajo de la clasificación de una entrada no
compila, **y el caso `INDIRECT` falla igual** — filtrar por una columna `critical` y exponer el
resultado a un conducto `medium` tiene que ser un error, o la mitad de §4 es decorativa.

### M4 · Capacidades y reparto

`(empujado, residuo)` a partir de las capacidades declaradas.

**Listo cuando:** un plan contra un origen con `fullScan: forbidden` y sin claves se rechaza
**sin abrir una conexión**, y uno con `predicatePushdown: [eq]` empuja el `eq` y deja el resto
de residuo, **diciendo cuál es el residuo**.

### M5 · Reescritura con materializaciones

Subconjunto honesto primero: misma exploración, contención de proyección, e **implicación de
predicados** para conjunciones de comparaciones simples. La reescritura con un número arbitrario
de joins **queda fuera**, y Calcite documenta por qué.

**Listo cuando:** un plan se contesta desde una materialización cuya proyección lo contiene y
cuyo predicado lo implica; y uno cuyo predicado **no** lo implica va al origen **con el
motivo**.

### M6 · Mantenimiento incremental

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
