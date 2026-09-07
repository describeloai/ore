# El motor de vistas

> **Estado:** construido · **Fecha:** 2026-09-01 · **Crate:** `crates/ore-view`
>
> Este documento es permanente. El plan por el que se construyó era desechable y se borró el
> día que su última pieza se puso en verde, que era su condición. Lo que queda aquí es lo que
> hay que saber para no deshacerlo.

---

## 1. Qué es

Los siete jugadores del sector —Calcite, Substrait, Trino, OpenLineage, Feldera, Snowflake,
Foundry— tienen los mismos siete órganos: catálogo, IR, expansor y reescritor, capacidades y
empuje, ejecución del residuo, mantenimiento incremental, linaje de columna. Cuatro son
metadatos; el IR es metadato **sobre** cómputo y es el que desbloquea al resto.

> **Un motor de vistas es un compilador de álgebra relacional con un catálogo versionado, un
> modelo de capacidades y un reescritor. La ejecución es de otro.**

No es una opinión: es la descripción literal de Apache Calcite —que no tiene ni almacenamiento
ni ejecución— y de Substrait —una especificación sin motor—. Los dos artefactos más usados de
esta categoría no ejecutan nada, y este tampoco.

## 2. Las doce piezas

Nombradas con la terminología del sector, no con nombres propios: *filter tree* es de
Goldstein–Larson, *view matching* de Oracle y Calcite, *partial state* y *upquery* de Noria,
*refresh mode* de Snowflake, *cost model* de Databricks. Un ingeniero de datos tiene que poder
leer esto sin traducir.

| Pieza | Módulo | Qué contesta |
|---|---|---|
| **Plan IR** | `plan.rs` | qué se va a hacer, con identidad determinista |
| **Schema Resolver** | `schema.rs` | qué columnas salen y de qué tipo |
| **View Expander** | `catalog.rs` | una cadena de vistas es un plan |
| **Lineage Analyzer** | `lineage.rs` | de qué columna raíz sale cada salida, y por qué arista |
| **Flow Checker** | `flow.rs` | por qué esto no compila |
| **Pushdown Planner** | `capabilities.rs` | qué hace el origen y qué queda de residuo |
| **Filter Tree** | `filter_tree.rs` | de todas las materializaciones, cuáles podrían servir |
| **View Matcher** | `view_matcher.rs` | si esta la contesta, con qué compensación, y qué hereda |
| **Delta Compiler** | `delta_compiler.rs` | el circuito Δ de un plan, y el estado que exige |
| **Refresh Analyzer** | `refresh_analyzer.rs` | `INCREMENTAL` o `FULL`, y si `FULL`, todos los motivos |
| **Partial State Store** | `state_store.rs` | qué claves están calientes, y la *upquery* de las que no |
| **Cost Model** | `cost_model.rs` | incrementar o recomputar, con todo lo que entró a la vista |

Ninguna sabe qué es un paquete OOS. Ninguna abre una conexión. Todas contestan sin ejecutar.

## 3. Las reglas que no hay que deshacer

Cada una tiene prueba, y varias salieron de que una prueba fallara.

**El digest es del significado, no de la escritura.** La forma canónica —la del bundle, no una
segunda— reordena lo conmutativo: operandos de `Y` y `O`, ramas de unión, columnas de una
proyección, pares de una junta. Los lados de una junta **no** se conmutan.

**No hay coma flotante, en ningún sitio.** No hay literal `Float`; un decimal lleva sus dígitos
tal cual; comparar es exacto; sumar es exacto (mantisa en `i128`); las razones del Cost Model son
racionales enteros. Es `OOS6003` un piso por debajo, y se pagó cuatro veces.

**La opaca declara su superficie.** `lee`, `tipo` y `determinista` —por defecto volátil, que es
P4—. Su cuerpo no se analiza; su superficie sí, y **`lee` entra en el linaje**. Determinista y
analizable son preguntas distintas: una opaca nunca es analizable y puede ser determinista.

**La arista `INDIRECT` clasifica igual que la `DIRECT`.** Una columna que solo está en un
`WHERE` decide qué filas salen. Es un flujo implícito, y el tratamiento es el de Denning. Aflojar
sin argumento cuantitativo sería aflojar en la dirección insegura; lo que hace vivible la regla
es desclasificar explícitamente.

**El eje decide cómo se combina.** Confidencialidad une por arriba (`max`); integridad, por abajo
(`min`). Con `max` en los dos, juntar un dato fiable con uno dudoso parecería fiable. El retículo
es el de `ore_core::flow`, sin copia.

**El *label seal*: la clasificación de una materialización se hereda, no se recalcula.**
Recalcularla sobre la tabla materializada haría desaparecer la columna por la que se filtró, y
con ella su etiqueta. Es el único término de las doce piezas que no tiene estándar, porque
nadie tiene el Flow Checker.

**Las capacidades se declaran, y el driver contradice.** Un plan se rechaza sin abrir una
conexión; la ausencia de capacidades es una negativa. Y **un predicado no baja por debajo de un
límite**: equivocarse ahí devuelve un resultado plausible.

**`AVG` no se enrolla ni se mantiene sin `SUMA` y `CUENTA` aparte**, porque el álgebra no tiene
división. Una regla, dicha por dos piezas, escrita una.

**Una junta de más solo vale con dos restricciones declaradas**: única en el lado de más evita
duplicar; referencial hacia él evita perder. Sin restricciones no se supone ninguna.

**Una *upquery* es un plan.** El de la vista, filtrado a la clave. El Pushdown Planner lo baja a
la hoja, y el *miss* se convierte en la búsqueda por clave que era el argumento del ADR 0006.

**La marca es un ordinal.** LSN, SCN, offset, `snapshot-id`: todos totalmente ordenados. `u64`,
sin reloj y sin fechas.

**Un relleno no pedido se rechaza, y uno bajo otro bundle también.** La regla de la caché
—`ReglaDistinta`— a granularidad de clave.

**El Cost Model no inventa ningún número, y ya tiene los suyos.** El 5 % sigue siendo de
Snowflake y se ofrece con su procedencia. Lo medido está en `crates/ore-view/tests/medidas.rs`,
contando **filas miradas por un operador** y no tiempo — un reloj mide la máquina de quien mide
([ADR 0014](decisions/0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md)).

**El estado se guarda indexado por su clave, y esa es la diferencia entre incrementalizar y
decir que se incrementaliza.** Los integradores de la junta y del agregado eran multiconjuntos
planos: un paso costaba la base. Lo destapó intentar medirlo.

## 4. Lo que no hace, y no por falta de tiempo

**No ejecuta.** El Delta Compiler y el Partial State Store son la **semántica** y el **contrato**
de referencia, sobre Z-sets en memoria, para que lo demás sea comprobable. Correr eso es de un
programa delegado —`crates/ore-maintain`, [ADR 0013](decisions/0013-el-protocolo-del-mantenedor.md)—
que vive fuera de este crate y habla por stdin. Los dos artefactos más usados de esta categoría
tampoco ejecutan.

**No tiene nulos en la semántica de referencia.** `EsNulo` evalúa a falso y una junta externa no
se mantiene incrementalmente. Está dicho antes de que lo descubra una prueba.

**No reescribe con juntas arbitrarias ni atraviesa opacas.** La contención de consultas
conjuntivas es NP-completa y la determinación es indecidible; el View Matcher implementa el
subconjunto decidible y dice cuál es.

**No cronometra.** La unidad es la fila mirada, no el segundo: sirve para comparar dos caminos
sobre los mismos datos, que es la pregunta del Cost Model, y no para prometer latencias. Y mide
la máquina de referencia; un ejecutor sobre otro almacén tendría otros números, y el método para
volver a sacarlos está escrito.

## 5. El documento y el IR

Esta sección existe porque las dos cosas se confunden en la dirección equivocada, y de ahí sale
una lectura falsa del vocabulario entero.

**El documento es el artefacto. El IR es lo derivado.** Una `View` en YAML no es el registro de
algo que vive en otra parte: es la cosa. El plan de `plan.rs` se fabrica desde el árbol de
ficheros en cada invocación de `ore view`, **no se persiste en ningún sitio**, y volver a
fabricarlo desde el mismo documento da el mismo plan con la misma identidad. Es P2 llevado hasta
el final —*lo derivable no se declara*—, y por eso un documento OOS es exactamente el residuo de
decisiones que nadie puede computar por ti: `owner`, `from`, `where`, `materialized`.

**El álgebra ancha del motor no es una segunda clase de vista.** `plan.rs` tiene `Une`, `Agrupa`
y `Limita`; el Delta Compiler los incrementaliza y `tests/medidas.rs` los mide —y medirlos
destapó un defecto real: los integradores de la junta y del agregado eran multiconjuntos planos,
así que la incrementalización estaba escrita y no ocurría—. La máquina se deja lista antes que el
vocabulario, a propósito, y la puerta por la que se cruza es `ore_core::vistas::invertible`, cuyo
defecto es «no».

**Y `Agrupa` ya se cruzó.** v1alpha8 añade `groupBy` y el agregado en `fields`, así que de los
tres el agregado deja de ser inalcanzable: veintiséis funciones de producción de este motor —el
tipo de salida, las dos aristas de linaje, el Check 4 del matcher, diez de mantenimiento
incremental— pasan de estar probadas a mano a alcanzarse desde un documento. `Une` y `Limita`
siguen sin producirse, y eso no es un hueco: el motor no decide qué se puede preguntar.

Lo que se midió al cruzar, y conviene no perder: agrupar **sin** agregados es un `SELECT DISTINCT`
y su linaje sale **directo**, así que no ejerce el flujo implícito; la arista indirecta aparece con
el primer agregado. Un agregado **global** —sin `groupBy`— sale con linaje **vacío**, y por eso
`OOS2033` lo niega: no es la regla de SQL, es la consecuencia de gobernar por linaje.

Y «clase» ya significa otra cosa aquí, que además es **derivada**:
[`02-view`](../vendor/oos/spec/v1alpha8/02-view.md) §5.5 llama **espejo** a la vista sin
escrituras —puede quedarse virtual— y **registro** a la que las tiene —se materializa, y la copia
es el estado—. Lo decide el compilador por vista, no el producto, y las dos conviven en un
paquete.

**Y la asimetría que explica lo que parece un cabo suelto.** Una tabla puede declarar
`joinPushdown: true` y este planificador no lo lee. No es deuda: una clave de `Table` registra un
**hecho del origen** —Workday no sabe juntar, y no sabrá aunque nadie se lo pregunte—, así que
puede preceder a su lector; un campo del planificador **promete un comportamiento**, y leer una
capacidad que no se sabe ejercer sería anunciar un empuje que no ocurre. Lo que faltaba no era el
lector: era escribir el reparto. Está en `CARA_DE_LECTURA` y `OPERADORES_DE_OOS`, con un censo
contra el esquema publicado que impide que crezca sin que alguien decida.

## 6. Dónde está la diferencia

Todo lo anterior lo tiene alguien, pieza a pieza. Lo que no tiene nadie es el cruce:

- **el linaje se comprueba al compilar**, no se observa al ejecutar — y cuenta el flujo
  implícito;
- **la materialización viaja sellada**, y contestar desde ella no puede bajar la clasificación;
- **el modo de refresco se sabe antes de escribir la vista**, con todos los motivos, no al
  refrescarla y por la factura.

## 7. Lo que sigue

- **La absorción, terminada.** Esta pieza sigue sin saber qué es un paquete OOS, y la única
  costura es `crates/ore-cli/src/vista.rs`: lee las `View`, el retículo, las etiquetas efectivas
  y las capacidades, y los convierte en el IR, la clasificación y las capacidades de aquí.
  `ore view` es lo que contesta. Lo que faltaba ya está: `discover` propone vistas —medido contra
  BigQuery— y `kind: Binding` se retiró en v1alpha8 con su `OOS1003`, así que no queda migración
  que describir y los dos documentos que la describían se borraron. Lo que decidieron vive en
  [`spec/v1alpha8`](../vendor/oos/spec/v1alpha8/00-scope.md) —incluido lo que **no** entra— y en
  [`sustrato.md`](sustrato.md). **Las restricciones y el cotejo, conectados** — I2 de
  [`decisions/0016-el-testigo-y-el-rango.md`](decisions/0016-el-testigo-y-el-rango.md): `ore view` dice de cada vista qué
  copias la contestan, con qué compensación y con el sello heredado, y las restricciones bajan
  desde `changes.key`, `primaryKey`/`uniqueKeys` y las relaciones **obligatorias**. Lo que queda
  sin ejercer por ningún fichero del repositorio es la junta de más: hace falta una
  materialización que lea una hoja que la consulta no lee, y ninguna lo hace.
- **El ejecutor, hecho.** `crates/ore-maintain` es el programa delegado que corre el circuito Δ
  y sostiene el estado parcial: una **sesión** por stdin, con el Refresh Analyzer en la puerta,
  la *upquery* saliendo como petición al origen y el dictamen viajando en cada paso
  ([ADR 0013](decisions/0013-el-protocolo-del-mantenedor.md)). Lo que sostiene todo lo demás
  está probado a través del protocolo: **lo que sale de mantener es lo que saldría de
  recomputar**. Lo que queda es persistir entre sesiones — hoy la sesión es el estado, y
  arrancar cuesta una *upquery* por clave caliente.
- **Las medidas, hechas.** `crates/ore-view/tests/medidas.rs` cuenta trabajo —filas miradas— en
  los dos caminos y sobre los mismos datos, con las cifras afirmadas para que un cambio las
  rompa. Salieron tres cosas: que dos integradores no estaban indexados y por tanto **la
  incrementalización no ocurría**; que con eso arreglado **mantener gana siempre salvo en el
  agregado**; y que **dónde se cruza el agregado es dato y no plan** — el mismo documento se
  cruza en el 2 % con veinte grupos y en el 22,3 % con doscientos cincuenta, con el 5 % de
  Snowflake entre medias. De ahí sale `Politica::Trabajo`, que compara medidas en vez de
  extrapolar, y `ore-maintain` la alimenta con lo suyo: la carga inicial de una sesión **es** un
  recómputo, así que de ella sale el coste por fila de recomputar.

  Lo que queda medido a medias es el **almacén real**: estas cifras son de la máquina de
  referencia, sobre Z-sets en memoria.
- **Y una pregunta que este motor todavía no se hace.** Desde el
  [ADR 0018](decisions/0018-la-ontologia-es-el-sistema-de-registro.md) una copia es
  `Q(origen) ⊕ ediciones`, y este crate solo sabe de la primera mitad: sus doce piezas computan
  `Q`, y una edición **no viene de computar nada** — viene de arriba, ya en el vocabulario de la
  vista.

  Que el motor **no ejecute** sigue siendo cierto y no cambia. Lo que hay que decidir es quién
  funde las dos mitades y con qué política cuando se contradicen: hoy `carga::fundir()` funde por
  clave en `ore-store-r2` y no sabe distinguir una fila refrescada de una editada. Es `7.4` de
  [`functions.md`](functions.md), está abierta a propósito, y se dice aquí para que no la
  descubra alguien leyendo el delta compiler y suponiendo que es su trabajo.
