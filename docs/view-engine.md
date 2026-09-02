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

**El Cost Model no inventa ningún número.** El 5 % es de Snowflake y se ofrece con su
procedencia; los coeficientes de uno dicen ser lo que son. Un `if` con un número escondido es lo
que la pieza existe para que no ocurra.

## 4. Lo que no hace, y no por falta de tiempo

**No ejecuta.** El Delta Compiler y el Partial State Store son la **semántica** y el **contrato**
de referencia, sobre Z-sets en memoria, para que lo demás sea comprobable. Correr eso sobre una
tabla del cliente es de un ejecutor delegado. Los dos artefactos más usados de esta categoría
tampoco ejecutan.

**No tiene nulos en la semántica de referencia.** `EsNulo` evalúa a falso y una junta externa no
se mantiene incrementalmente. Está dicho antes de que lo descubra una prueba.

**No reescribe con juntas arbitrarias ni atraviesa opacas.** La contención de consultas
conjuntivas es NP-completa y la determinación es indecidible; el View Matcher implementa el
subconjunto decidible y dice cuál es.

**No mide.** Las medidas del Cost Model las pone quien llama, y el Partial State Store las
cuenta —aciertos, fallos, desalojos, rellenos—. Calibrar los coeficientes es trabajo pendiente y
está nombrado como tal.

## 5. Dónde está la diferencia

Todo lo anterior lo tiene alguien, pieza a pieza. Lo que no tiene nadie es el cruce:

- **el linaje se comprueba al compilar**, no se observa al ejecutar — y cuenta el flujo
  implícito;
- **la materialización viaja sellada**, y contestar desde ella no puede bajar la clasificación;
- **el modo de refresco se sabe antes de escribir la vista**, con todos los motivos, no al
  refrescarla y por la factura.

## 6. Lo que sigue

- **La absorción.** Esta pieza se diseñó libre y no sabe qué es un paquete OOS. Conectarla —el
  retículo real, las etiquetas efectivas, las restricciones desde `primaryKey` y las relaciones,
  el `datasource` real de cada hoja— es el trabajo de `docs/handoff-vistas.md`, que sigue abierto
  hasta que `Kind::Binding` desaparezca.
- **El ejecutor.** Un programa delegado, por stdin, que corra el circuito Δ y sostenga el estado
  parcial en el almacenamiento del cliente. La política ya está escrita; el sitio está decidido
  ([ADR 0012](decisions/0012-el-estado-es-parcial-y-vive-en-el-cliente.md)).
- **Las medidas.** Sin ellas el Cost Model es una forma. Con ellas, deja de serlo.
