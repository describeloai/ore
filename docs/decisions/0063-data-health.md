# 0063 · Data Health — la salud, la frescura y la calidad de los activos del catálogo

**Estado:** propuesto (2026-10-13). Se construye en este orden:

1. **Los fundamentos** (escritos): qué son la frescura y la calidad, de la literatura y la evidencia
   empírica, sin mirar ORE.
2. **Las buenas prácticas** (escritas): cómo lo resuelve la industria.
3. **ORE medido contra los fundamentos** (M1–M8, **medidos** el 2026-10-13): qué hay de verdad en nuestro stack,
   medido en producción. **Nada de lo que creíamos saber se da por bueno**; lo que salga de aquí
   es lo único que cuenta.
4. **El producto**: qué construir, cómo y dónde. Sale de cruzar 1 y 2 con lo que 3 demuestre que
   falta, está incompleto o está mal. **La visión está escrita**, y **el plan por capas propuesto, con S1 hecho** (la spec v1alpha30) (capa 0 la verdad, siempre; 1 vigilancia y 2 perfilado, que se encienden; S0–S15 y siete decisiones antes de S1; 2026-10-13). Nada construido.

Una primera versión de esta ADR (H0, `a56185e8`) derivó un producto de lo que creíamos que ORE
tenía. Una de sus premisas, que el testigo dice si una copia va atrás, era falsa (`241f287a`). Por
eso esa versión se retira entera: está en el historial y no en este documento.

## Contexto

Al cerrar el origen PostgreSQL (PG1) se vio que una copia de una tabla **sin testigo** contesta «ya
está» para siempre y no se refresca nunca, y que no hay ninguna pasada periódica que vuelva a
mirarla. Tirando de ese hilo apareció un producto que no existe: **la salud de los datos**, es
decir, si lo que se lee está al día y si es lo que se cree que es.

La spec declara una parte (la `freshness` de un `Dataset`, que superada «no se sirve como fresco»,
y las aserciones de un `Ruleset`). Lo que ORE hace de verdad con ello no se sabe hasta medirlo.

## Los fundamentos — independientes de ORE

Esto es la base de la ADR. No menciona ORE a propósito: son las definiciones con las que se mide ORE
(M1–M8) y con las que después se decide qué construir. Salen de la literatura, no de los vendedores. Lo marcado **[sin verificar]** se tomó de fuentes
secundarias y se comprueba antes de citarlo como norma.

### F1 · La frescura de una copia

**Tres relojes y una posición.** Todo número de frescura dice de qué reloj es:

- **τ_origen**: cuándo se confirmó un cambio en el origen (la hora del commit, `last-modified`).
- **τ_lectura**: cuándo la copia observó el origen, es decir, cuándo tomó su posición o su
  snapshot. No es cuándo terminó la copia, ni cuándo se publicó.
- **ahora**: cuándo alguien lee.
- **P**, la posición del origen (un LSN, un snapshot, el hash de un listado): no es un reloj, pero
  es el único orden exacto.

**Las magnitudes** (Cho y Garcia-Molina, SIGMOD 2000; Bouzeghoub y Peralta, IQIS 2004; Ballou et
al., *Management Science* 1998):

| Magnitud | Definición | Qué hace falta para medirla |
|---|---|---|
| **Punto de sincronía** (P\*, τ\*) | Hasta dónde está completa la copia: una **marca de agua** | Exacto con un log o un snapshot consistente. **Heurístico** con una columna cursor: lo que llega tarde con un cursor menor cae por debajo (Akidau et al., *The Dataflow Model*, VLDB 2015) |
| **Retraso** (*currency*) | `ahora − τ*` | Guardar τ\*. Siempre se puede |
| **Edad** (la *A* de Cho) | `ahora −` el primer cambio del origen que la copia no tiene; **0 si el origen no cambió** | Saber si el origen cambió **después de P\***, con una señal **del ámbito del activo**. Edad ≤ retraso |
| **Obsolescencia** | Cuántos cambios del origen hay después de P\* | Sólo con un log |
| **Puntualidad** (*timeliness*) | Si la edad o el retraso caben en la tolerancia **de quien consume** | Una tolerancia declarada. Es un contrato, no una propiedad del dato (Wang y Strong, JMIS 1996) |
| **Frescura derivada** | `retraso(nodo) = max(retraso(entrada) + espera) + coste` | El linaje. Es una cota superior: lo derivado está tan viejo como su camino más viejo (Peralta y Bouzeghoub, 2004) |

**Retraso y edad son dos números distintos**, y el honesto es la edad: «copiada hace 6 h, pero el
origen no ha cambiado desde entonces» es edad 0. El retraso se mide siempre; la edad sólo si hay una
señal de cambio con el ámbito del activo.

**Resultados clásicos que se pueden usar** (Cho y Garcia-Molina, con cambios de Poisson):

- Refrescar **tan a menudo como cambia** el origen sólo da un 63 % del tiempo fresco. Para un 80 %
  hace falta refrescar unas 2 veces más a menudo de lo que cambia.
- Un intervalo fijo es mejor que uno irregular.
- Con un presupuesto fijo, refrescar **en proporción** a lo que cambia cada cosa es peor que
  refrescar todo igual. Si el objetivo es la fracción fresca, lo que cambia demasiado deja de
  refrescarse. Si el objetivo es la edad, se queda cerca de lo uniforme. **El objetivo decide la
  política**, así que hay que elegirlo.
- Sondear «¿cambió?» ve varios cambios entre dos sondeos como uno. Estimar con qué frecuencia cambia
  un origen así tiene sesgo, y hay estimadores corregidos (Cho y Garcia-Molina, TOIT 2003)
  **[sin verificar]**.

**Qué garantiza una copia** (Terry, CACM 2013; Zhuge et al., SIGMOD 1995 **[sin verificar]**):

- Una copia tomada de **un snapshot consistente** da un **prefijo consistente** y una
  ***staleness* acotada** por el intervalo de refresco más lo que tarda en extraerse.
- Tablas copiadas **por separado** no comparten instante: cada una es un prefijo distinto.
- Una copia nunca da *read-your-writes* de lo que se escribe en el origen.

**Cómo se detecta un cambio en el origen**, y qué no ve cada forma:

| Mecanismo | Ve | No ve |
|---|---|---|
| Log (LSN, binlog, SCN) | Todo insert, update y delete, en orden | **La posición es del log entero, no de una tabla**. Hace falta filtrar por tabla (decodificación lógica) para saber si cambió **ésta** |
| Columna cursor (`MAX(ts)`) | Inserts y updates que mueven la columna | Borrados, versiones intermedias, llegadas tardías con un cursor menor |
| Diferencia de snapshots | El cambio neto | Los estados intermedios. Cuesta leerlo todo (Labio y Garcia-Molina, VLDB 1996) |
| Listado, ETag, `last-modified` | Objetos creados, reemplazados o borrados | Lo que hay dentro, salvo por el objeto entero. `last-modified` da más información que el ETag |
| Metadatos del motor (la hora de última modificación de una tabla, contadores de filas tocadas) | Que la tabla cambió | Depende del motor: hay contadores que se reinician y metadatos que no se actualizan con todo |

**Las averías de frescura**, en el orden en que pasan:

1. **No corrió** (el programador cayó, una credencial caducó). Se ve en la hora de la última copia
   buena.
2. **Corrió y no trajo nada** (el origen devolvió vacío, la partición no había llegado). Por la hora
   de escritura parece fresca; se ve en el volumen o en el dato más reciente de dentro.
3. **Corrió bien, pero el origen se paró aguas arriba.** La copia es fiel a un origen muerto. Lo
   arregla el dueño del origen, no quien copia.
4. **Llegó una parte.** Unas particiones o tablas sí y otras no.
5. **Llegó tarde por debajo de la marca.** El cursor ya había pasado: sólo un log o una diferencia
   lo ven.

### F2 · La calidad de un dato

**Definición operativa.** Para una versión D de un activo y la tarea T de quien lo usa, la calidad es
**el conjunto de aserciones declaradas sobre D**.
- Cada aserción es un predicado sobre una métrica con un ámbito.
- Cada resultado es `pass`, `fail` o `unknown`, con su evidencia y su hora.
- «Bueno para T» significa que toda aserción bloqueante pasa y ninguna es `unknown`.
- Las dimensiones (completitud, validez…) son **etiquetas** para agrupar aserciones, no medidas.
  Nadie se pone de acuerdo en cuáles son (Wand y Wang, CACM 1996), y una métrica mal definida lleva
  a decisiones malas (Heinrich et al., JDIQ 2018).

**La calidad es de quien consume** (*fitness for use*, Wang y Strong 1996). El estudio que lo
fundó salió de preguntar a consumidores, y **la mitad de lo que llamaron calidad no se calcula del
dato**: el contexto y la representación, es decir, qué significa, en qué unidad y si está
documentado. Eso es metadato del catálogo, y es tan de primera como un check.

**Los cuatro tipos de comprobación:**

1. **Restricción**: un predicado por fila o por conjunto (tipo, no nulo, rango, patrón, clave,
   referencia). Binaria, determinista y sin falsos positivos si la regla es cierta. Su base formal
   son las dependencias, funcionales y condicionales (Fan y Geerts, 2012).
2. **Métrica y umbral**: `m(D) ∈ [lo, hi]`. Toda restricción se reduce a esto (Deequ, VLDB 2018).
3. **Test estadístico** entre lotes (χ², KS). **A escala, todo sale significativo**: en Google, con
   100 M de filas y un 0,01 % de ruido, χ² dio falsa alarma 7 de cada 10 veces (Breck et al.,
   MLSys 2019). Se usan distancias legibles, como el mayor cambio de probabilidad de un valor (L∞),
   que además nombran el valor que se movió.
4. **Anomalía aprendida** sobre la historia de una métrica (Holt-Winters en Uber, Deequ). **Sin
   historia no hay nada que aprender.**

**Lo que cuesta cada una** (de la arquitectura de Deequ):

| Clase | Ejemplos | Coste |
|---|---|---|
| Por fila | Tipo, rango, patrón, no nulo | Una pasada, en flujo |
| Agregado de una columna | Completitud, media, filas, distintos aproximados | Una pasada compartida; estado pequeño que se suma por partes |
| Con agrupación | Unicidad, distintos exactos, dependencias | Barajar los datos; estado del tamaño de un histograma |
| Entre tablas | Referencias, conciliar origen y copia | Un join o dos lecturas, y el linaje |
| Con historia | Anomalías de volumen o de frescura, deriva, cambio de unidad | Una historia guardada; **inútil hasta tenerla** |

**Lo que se rompe de verdad**, ordenado por la evidencia de frecuencia e impacto:

| # | Avería | Evidencia | La señal más barata | Lo que tiene que declarar una persona |
|---|---|---|---|---|
| 1 | **Esquema que cambia** (columna nueva, quitada, otro tipo) | Google: columna nueva 10 % de los pipelines en 30 días, columna perdida 6 % (Breck 2019); tipos, el 33 % de las causas (Foidl et al., JSS 2024) | Comparar con el esquema declarado o el anterior: gratis | El esquema, y si se admiten columnas nuevas |
| 2 | **Dato tarde o que falta** | Breck; Uber; Microsoft (Zhou et al., ICSE 2015) | F1 | La tolerancia |
| 3 | **Volumen anómalo** (filas que caen o se disparan) | Auto-Validate-by-History, Microsoft (Tu et al., KDD 2023); Uber | Filas frente a la historia | Al principio nada; después la tolerancia y la estacionalidad |
| 4 | **Ola de nulos o centinelas** (−1, `""`) | Breck; AVH (*timeouts* de RPC que dejan valores por defecto) | % de nulos y peso del valor más frecuente | Qué centinelas cuentan como falta; dónde se admite nulo |
| 5 | **Valores nuevos en una categoría** | Breck: 6 % | El conjunto de distintos, L∞ | El dominio, o «dominio abierto» |
| 6 | **Duplicados o clave rota** | Uber; Foidl | `1 − distintos(clave) / filas` | La clave |
| 7 | **Codificación o formato** | Foidl: 17 % | % que cumple el patrón | El formato |
| 8 | **Cambio de unidad o de convención** | AVH: días que pasan a horas, `en-gb` que pasa a `en-GB` | Desplazamiento de media o cuantiles | La unidad, como metadato |
| 9 | **Origen y copia que divergen** | Uber (completitud entre etapas) | Conciliar filas o claves | El linaje |
| 10 | **Una regla de negocio rota** | Uber; dependencias condicionales | Un predicado SQL | La regla: no se puede inferir |

Las cinco primeras **pasan en silencio**: nada lanza un error (AVH). Y la evidencia más fuerte es
de producción (Google, más de 700 pipelines); la de Foidl son 100 incidencias de proyectos abiertos,
una muestra pequeña. Las encuestas de vendedores (Monte Carlo: 67 incidencias al mes, el 74 % las
descubre el negocio) son indicativas y nada más.

**Las trampas de medir:**

- **La tasa base.** Con miles de columnas, una tasa de falsos positivos baja da muchas falsas
  alarmas al día. Las alertas ignoradas son el modo de fallo documentado (Breck). Se presupuestan los
  falsos positivos y se agrega antes de avisar (Uber agrega por tabla).
- **La cascada.** Un lote tarde dispara a la vez frescura, volumen y completitud. Se ordenan los
  requisitos, **primero la frescura**, y se exige que el fallo se sostenga en el tiempo (Uber: aviso
  durante 3 h antes de escalar).
- **Sin historia, «historia insuficiente», nunca `pass`.** Y la estacionalidad (el día de la
  semana, el fin de mes) se compara con lo mismo de antes.
- **`unknown` no es `pass`.** Una comprobación que no corrió, no tenía métrica o no aplica se ve
  como desconocida. Y **la cobertura** (qué está comprobado y qué no) se enseña junto a los
  resultados: en Google, sólo el 45 % de los pipelines podía disparar algunos tipos de anomalía.

### F3 · Lo que los fundamentos obligan, antes de mirar ORE

1. **Guardar el punto de sincronía de cada copia**, (P\*, τ\*), con τ\* = τ_lectura. Sin él no hay
   ni retraso. Es lo primero.
2. **Retraso y edad son dos números.** El retraso, siempre. La edad, cuando haya una señal de
   cambio **con el ámbito del activo**, que hay que encontrar **por cada tipo de origen**.
3. **Cada señal dice su garantía**: exacta (un log filtrado por tabla, un snapshot) o heurística (un
   cursor), y qué no ve (borrados, llegadas tardías).
4. **La tolerancia la declara alguien**: quien produce (un SLA) o quien consume (su tarea). Sin
   ella, la frescura es `unknown` con el retraso a la vista.
5. **Una política de refresco elige su objetivo**, fracción fresca o edad, porque las dos llevan a
   políticas distintas.
6. **El esquema es un artefacto revisado que puede cambiar**, y comparar con él es la comprobación
   que más valor da.
7. **Las reglas las pone una persona.** Inferir sólo propone (la clave, el dominio, la unidad).
8. **Primero lo que se mide al escribir**, en una pasada. Lo que necesita historia, sólo cuando la
   haya.
9. **`unknown` y la cobertura, visibles.** La frescura se comprueba antes que el volumen.
10. **Lo derivado acota su frescura por su camino más viejo** (máx-más).

## El estado del arte (investigado el 2026-10-13)

Cuatro familias, con documentación oficial de cada una. Al final está lo que todas comparten, que es
de donde sale nuestro producto.

### 1. Las plataformas integradas

| | Dónde se declara | Cuándo corre | Qué da | Qué pasa si falla |
|---|---|---|---|---|
| **Palantir Foundry** | Dos cosas: **Data Expectations** en el código del transform (`Check(E.primary_key('id'), on_error='FAIL'\|'WARN')`) y **health checks** en la app Data Health (28 tipos: estado, tiempo, tamaño, contenido, esquema) | Las expectations, **dentro del build**. Los checks, después; *time since last updated* en modo automático **en cada escritura y otra vez al vencer el plazo** | Pasa o falla, con severidad Moderate o Critical; escala a Critical tras N fallos seguidos; abre un *Issue* | FAIL aborta el job (lo malo no baja). Un check sólo avisa a quien lo vigila. **Monitoring views** aplican reglas a una carpeta, un proyecto o un linaje, y lo nuevo entra solo |
| **Databricks** | Expectations del pipeline (`EXPECT … ON VIOLATION DROP ROW \| FAIL UPDATE`). La **detección de anomalías**, **sin reglas**: un clic por schema | En el pipeline. Las anomalías, a la cadencia de cada tabla (más a menudo las de más impacto) | `Healthy · Unhealthy · Training · Error · Excluded`, en `system.data_quality_monitoring.table_results`. Indicador en el catálogo y en el linaje | Warn (por defecto), drop, o fail con *rollback*. Lo de reposo sólo avisa |
| **Snowflake** | **Data Metric Functions** (`FRESHNESS`, `ROW_COUNT`, `NULL_COUNT`…) sobre la tabla, y una **expectation** aparte que juzga (`VALUE < 10`) | `DATA_METRIC_SCHEDULE`: cada N minutos, cron, o **`TRIGGER_ON_CHANGES`** | Una tabla de eventos (`DATA_QUALITY_MONITORING_RESULTS`) | Nada se bloquea; notificaciones. Y `TARGET_LAG` en las dynamic tables, medido **contra las raíces del grafo**: un objetivo que el programador intenta cumplir, no una promesa |
| **Google Dataplex** | Un *scan* de calidad por tabla: reglas por fila (con % que pasa) y agregadas (SQL que no debe devolver filas); **cada regla con su dimensión** | Programado o a demanda; completo o incremental | Puntuación = % de reglas que pasan, por dimensión; publicada en el catálogo como *aspect* | Avisa; nada se bloquea |
| **AWS Glue Data Quality** | **DQDL** (sobre Deequ): `RowCount`, `Completeness`, `DataFreshness "col" <= 24 hours`, `CustomSql`; reglas dinámicas (`RowCount > avg(last(5))`) | **En reposo**, sobre el catálogo, o **en el ETL** | Puntuación, métricas guardadas 2 años, resultados en tablas Iceberg | En el ETL: no cargar, o cargar y fallar; filas marcadas para cuarentena |
| **Purview / Fabric** | Reglas por columna, seis dimensiones; la frescura, un SLA de tabla. En Fabric, `CONSTRAINT … ON MISMATCH DROP\|FAIL` | Programado | Puntuación que sube de activo a producto y a dominio | Avisa (Purview); falla o tira filas (Fabric) |

Fuentes: [Foundry Data Health](https://www.palantir.com/docs/foundry/data-health/overview),
[sus checks](https://www.palantir.com/docs/foundry/data-health/checks-reference),
[cuándo se evalúan](https://www.palantir.com/docs/foundry/data-health/check-evaluation),
[Data Expectations](https://www.palantir.com/docs/foundry/transforms-python/data-expectations-getting-started/),
[monitoring views](https://www.palantir.com/docs/foundry/monitoring-views/overview/);
[Databricks expectations](https://docs.databricks.com/aws/en/ldp/expectations),
[anomalías](https://docs.databricks.com/aws/en/data-governance/unity-catalog/data-quality-monitoring/anomaly-detection/);
[Snowflake DMF](https://docs.snowflake.com/en/user-guide/data-quality-intro),
[expectations](https://docs.snowflake.com/en/user-guide/data-quality-expectations),
[target lag](https://docs.snowflake.com/en/user-guide/dynamic-tables-target-lag);
[Dataplex](https://docs.cloud.google.com/dataplex/docs/auto-data-quality-overview);
[Glue DQDL](https://docs.aws.amazon.com/glue/latest/dg/dqdl.html);
[Purview](https://learn.microsoft.com/en-us/purview/unified-catalog-data-quality).

### 2. Los marcos declarativos

- **dbt**: `freshness: {warn_after, error_after}` sobre fuentes y modelos. El tiempo sale de
  `loaded_at_field` (un `MAX()`), de `loaded_at_query`, o de los metadatos del almacén si no se dice.
  Los tests tienen `severity: warn|error` y umbrales (`error_if: ">1000"`). En `dbt build`, un error
  **salta lo de abajo** y un warn nunca bloquea.
  ([freshness](https://docs.getdbt.com/reference/resource-properties/freshness),
  [build](https://docs.getdbt.com/reference/commands/build))
- **Dagster**: *asset checks* con `severity` WARN o ERROR, que no bloquean salvo `blocking=True`.
  *Freshness policies* (`time_window(fail_window, warn_window)` o `cron(deadline)`). Y la **salud
  del activo**, un solo estado: **`Unknown · Healthy · Warning · Degraded`, el peor gana**, y los
  avisos sólo cuando el estado cambia. Hay una trampa documentada: un check que no da resultado
  deja pasar lo de abajo (*fail-open*).
  ([asset health](https://docs.dagster.io/guides/observe/asset-health-status),
  [checks](https://dagster.io/docs/guides/test/asset-checks))
- **Great Expectations**: suites de expectations, *checkpoints* y acciones; severidad `critical ·
  warning · info`. **No bloquea nada**: decide el orquestador.
- **Soda**: SodaCL (`freshness(col) < 1d`, `row_count between …`, `anomaly detection for
  row_count`) y los contratos v4 con `must_be*` y `level: warn|fail`. Resultados `pass · warn · fail
  · error`, y **«no evaluado» hasta tener historia**.
  ([contratos v4](https://docs.soda.io/soda-v4/reference/contract-language-reference))
- **Deequ** (debajo de Glue): un `VerificationSuite` que **mide** y un repositorio de métricas, y la
  **anomalía se calcula sobre la historia guardada** (tasa de cambio, Holt-Winters…).
  ([deequ](https://github.com/awslabs/deequ))

### 3. Los estándares

- **ODCS v3.2.0** (Bitol, Linux Foundation; 3.1 en diciembre, 3.2 en septiembre). Es el que ya cita
  nuestro `Ruleset`. Tiene dos partes:
  - **`quality[]`**: tipos `text · library · sql · custom`; métricas de biblioteca (`nullValues`,
    `missingValues`, `invalidValues`, `duplicateValues`, `rowCount`); operadores `mustBe*`;
    `dimension` ∈ {accuracy, completeness, conformity, consistency, coverage, timeliness,
    uniqueness}; `severity` es texto libre.
  - **`slaProperties[]`**: `latency` (preferido a «freshness»), `frequency`, `retention`,
    `availability`, `timeOfAvailability`, `timeToDetect`, `timeToNotify`, `timeToRepair`, cada uno
    con `element` (a qué columna) y `driver`.
  - **No define resultados**, ni una regla con aviso y fallo a la vez.
  - El *Data Contract Specification* se ha fundido en él: `datacontract-cli` escribe ODCS 3.2 y
    OpenMetadata lo importa y exporta.
  ([calidad](https://bitol-io.github.io/open-data-contract-standard/latest/data-quality/),
  [SLA](https://bitol-io.github.io/open-data-contract-standard/latest/service-level-agreement/))
- **OpenLineage**: dos *facets* de **resultado**. **`DataQualityMetrics`** lleva `rowCount`,
  `bytes`, `lastUpdated` y por columna `nullCount`, `distinctCount`, `min`, `max`…;
  **`DataQualityAssertions`** lleva `assertion`, `success`, `column`, `severity`, `expected` y
  `actual`. Es el formato de resultados que falta en ODCS.
  ([assertions](https://openlineage.io/docs/spec/facets/dataset-facets/data_quality_assertions))
- **DataHub** y **OpenMetadata**, catálogos: aserciones `FRESHNESS · VOLUME · FIELD · SQL · SCHEMA`.
  Resultados `SUCCESS · FAILURE · ERROR · INIT`; **error y fallo, distintos**. Gestor de incidentes
  `New → Ack → Assigned → Resolved`. **Observan y abren incidencias; nunca bloquean.** El `sla` del
  contrato de OpenMetadata todavía es sólo documentación.
- **Write-Audit-Publish**: se escribe en una rama de Iceberg (o de Nessie o lakeFS), se comprueba,
  y `main` sólo avanza si pasa. **Es el único patrón que garantiza que un consumidor no ve lo que
  falló.** ([Iceberg branching](https://iceberg.apache.org/docs/latest/branching/))

### 4. La observabilidad y su teoría

- **Monte Carlo**: cinco pilares, **frescura, volumen, distribución, esquema y linaje**, contra el
  «*data downtime*». La frescura tiene dos monitores: «tiempo desde la última escritura» y «desde
  el último cambio de filas». Umbrales aprendidos o explícitos. Añadió *circuit breakers* porque
  los avisos solos no cambiaban nada. ([freshness](https://docs.getmontecarlo.com/docs/freshness))
- **Metaplane** (de Datadog desde abril de 2025) y **Anomalo** (dos niveles): **metadatos para todo**
  (`information_schema`, casi gratis) y escaneo o ML de muestras **sólo para lo crítico**. Mozilla
  midió Bigeye: los umbrales aprendidos, sólo para frescura y volumen; el resto, explícitos.
- **Sifflet**: estado por incidencias abiertas (rojo, naranja, verde) y **gris = sin monitores,
  que no es verde**. El rojo baja por el linaje.
- **Elementary** (sobre dbt): anomalías con estacionalidad (`day_of_week`…) y 14 días de
  entrenamiento.
- **Tres relojes de la frescura**: el del evento, el de la ingesta y el de la publicación. Y el
  hallazgo más útil para nosotros: **ninguna herramienta distingue bien «la copia se ha quedado
  atrás» de «el origen ha dejado de cambiar».** Fivetran se pone verde copiando cero filas.
  Lo que se hace es tener dos señales, el retraso de la copia y la recencia del origen, con
  dueños distintos.
- **SLO de datos** ([Google SRE](https://sre.google/workbook/data-processing/)): la frescura se
  enuncia como «el dato más viejo no pasa de Y» o «el X % llega antes de Y». Se mide de punta a
  punta, con presupuesto de error.
- **Dimensiones**: las seis de DAMA (exactitud, completitud, consistencia, puntualidad, unicidad,
  validez); ISO/IEC 25012 (15 características, inherentes o del sistema); ISO 8000-120 (la
  procedencia del valor).
- **Los agentes** (2025–2026): Unity Catalog certifica (`system.certification_status`) y Genie
  prefiere lo certificado. dbt, Monte Carlo, Elementary y DataHub exponen la salud por **MCP**.
  **No hay un protocolo** para que un agente se abstenga o avise de lo que no está sano: cada agente
  se lo inventa.

### Lo que comparten: las buenas prácticas

1. **Medir no es juzgar.** Snowflake separa la DMF de la expectation, Glue los analizadores de las
   reglas, Deequ las métricas de los checks. Guardar lo medido es lo que permite después umbrales
   dinámicos y anomalías.
2. **Dos capas en todos.** Lo que corre **dentro de la escritura** puede bloquear: Foundry,
   Databricks, Glue ETL, Fabric, dbt build. Lo que mira **en reposo** sólo observa y avisa: Foundry
   checks, Snowflake, Dataplex, Glue catálogo, Purview, los catálogos. **Sólo bloquea quien es dueño
   de la escritura.**
3. **La frescura es relativa a una expectativa**, declarada (un SLA o un *target lag*) o aprendida
   (la cadencia). Sin ella, «hace 3 días» no dice nada. Se evalúa **al escribir y al vencer el
   plazo** (Foundry automático), no por cron.
4. **Cuatro estados, no dos**: sano, aviso, degradado y **desconocido**. Gris no es verde, y un
   check sin resultado no pasa (la trampa de Dagster). Por check, **el error (no pudo correr) no es
   el fallo**.
5. **El peor gana**, y **baja por el linaje**: lo derivado no es más fresco que su raíz más vieja
   (Snowflake mide contra las raíces).
6. **Metadatos para todo, escaneo para lo crítico.** Lo barato da cobertura total; lo caro se
   elige.
7. **Bloquear en la frontera de publicación; avisar en todo lo demás.** WAP, circuit breakers,
   `FAIL` en el ETL. Y en la lectura, servir con aviso (como el *stale data* de Tableau).
8. **Empujar a los dueños, que los consumidores tiren.** Avisos e incidencias a quien es dueño; el
   estado, en el catálogo, en la API y en el MCP.
9. **ODCS para declarar, OpenLineage para los resultados**: es donde converge el mundo abierto.

## ORE medido contra los fundamentos (M1–M8)

Cada hipótesis se mide en producción, contra orígenes de prueba, y se anota aquí lo que salga, sea
lo que sea. La columna «Lo que se cree» es sólo el punto de partida: lo que dice el código o lo que
se ha dado por supuesto.

| | Pregunta | Lo que se cree (sin medir) | Cómo se mide | Fundamento |
|---|---|---|---|---|
| **M1** | ¿Qué hora guarda una copia, y de qué reloj es? | `cuando` sale del commit del puntero: sería la publicación, no τ_lectura | Una copia de una tabla de Postgres: la hora del commit, la del arranque del Job y la de lectura del testigo | F3·1 |
| **M2** | ¿Qué dice cada modo de testigo del cambio **de un activo**? | El código: `log` es el LSN del servidor entero; `field`, el `MAX` del cursor; `listing`, el `sha256` de `(clave, ETag, tamaño)`, donde el ETag es la generación en GCS, el ETag en S3 y Azure, y `mtime-tamaño` en SFTP | Cambiar la tabla, cambiar **otra**, y no cambiar nada; actualizar sin mover el cursor; reescribir un objeto igual y cambiarlo con el mismo tamaño | F1 · detectar un cambio |
| **M3** | ¿Qué señal de cambio **con el ámbito del activo** da cada origen, cuánto cuesta y cuánto se puede fiar? | — | Postgres: contadores por tabla y decodificación lógica filtrada. BigQuery: la hora de última modificación de la tabla. Objetos: `last-modified` por objeto | F3·2–3 |
| **M4** | ¿El informe de la copia dice la verdad? | Cuenta filas y no nulos por columna; marca `esquema_cambiado` | `filas` y no nulos contra el origen; cambiar el esquema del origen | F2 · averías 1 y 4 |
| **M5** | ¿Una copia que falla deja servir la anterior? | Sí (0031 §10) | Provocar un fallo a mitad y leer por SQL, Python y la vista previa | F3·8 |
| **M6** | ¿Acierta «ya está», modo por modo? | Con `none`, nunca se refresca (visto en PG1) | Con los cambios de M2 | F1 |
| **M7** | ¿Cuáles de las cinco averías de frescura ve ORE hoy? | — | Reproducir cada una y mirar el puntero, la consola y una lectura | F1 · averías |
| **M8** | ¿Qué se sabe de la frescura de un activo en vivo (foreign database)? | — | Las señales de M3, sin copia | F1 |

### M2 · medido (2026-10-13): qué dice cada testigo del cambio de un activo

Se midió en orígenes reales, con objetos propios que se borraron al terminar:
- **Postgres**: Neon, la fuente `postgresql_20260918_1920` de `victor`, con `wal_level=logical`. Un
  esquema propio (`ore_salud_m2`), desde un Job de la celda, y el testigo tomado con **el binario del
  driver** (`/opt/ore/conectores/ore-read-postgres testigo`).
- **BigQuery**: un dataset propio en el proyecto de ORE (EU), con la **misma consulta del driver**
  (`SELECT CAST(MAX(ts) AS STRING)`).
- **Objetos**: un prefijo propio en el bucket de GC (`…-origen-gcs`, con versionado) y en el de R2
  (`ore-origen-r2`). El testigo se **recalculó como lo hace el código** (`sha256` de
  `clave\tetag\ttamaño` ordenado), con la generación como «etag» en GCS y el ETag en R2.

Guiones en [`pruebas-de-fuego/salud/`](../../pruebas-de-fuego/salud/) (`m2pg.sh` como Job con `hacer-job.py`, `m2bq.py`, `m2obj.py`).

| Lo que pasa en el origen | `log` · Postgres | `field` · BigQuery | `listing` · GCS | `listing` · R2 |
|---|---|---|---|---|
| Se actualiza el activo | se mueve ✓ | **no se mueve** ✗ | se mueve ✓ | se mueve ✓ |
| Se borra una fila u objeto del activo | se mueve ✓ | **no se mueve** ✗ | se mueve ✓ | se mueve ✓ |
| Llega una fila tarde (cursor menor) | se mueve ✓ | **no se mueve** ✗ | — | — |
| Llega una fila nueva (cursor mayor, por DML o en *streaming*) | se mueve ✓ | se mueve ✓ | — | — |
| Cambia **otro** activo | **se mueve** ✗ | no se mueve ✓ | no se mueve ✓ | no se mueve ✓ |
| Nada cambia | **se mueve solo** ✗: en 4 min de reposo avanzó en cada intervalo de 60 s (≈ 260–470 bytes de WAL), sin ninguna escritura nuestra; quieto en una ventana de 20 s | quieto ✓ | quieto ✓ | quieto ✓ |
| Se reescribe un objeto con **el mismo** contenido | — | — | **se mueve** ✗ (la generación cambia en cada escritura) | quieto ✓ (el ETag es el MD5) |
| Cambian sólo los metadatos de un objeto | — | — | quieto ✓ | no se pudo probar |
| Una lectura (`SELECT`) | quieto ✓ | quieto ✓ | — | — |
| **Coste** de preguntarlo | **1,1–1,3 s** por llamada (proceso, TLS y conexión) | 0,45–0,85 s y **10 MB facturados** por llamada (el mínimo); 0 B si la caché de consultas acierta, que sólo pasa si la tabla no cambió | 160–200 ms (1–2 objetos) | 290–470 ms (1–2 objetos), por la API de Cloudflare |

Lo que esto quiere decir:

- **`log` (Postgres) sólo garantiza una dirección**: si es igual, el activo no cambió. Pero se mueve
  con cualquier commit del servidor. En Neon, además, **se movió solo**, sin escrituras nuestras,
  minuto a minuto; no se sabe si fue el propio Neon u otro cliente de esa base. En la práctica, dos
  pasadas separadas por minutos casi nunca lo verán igual: **cada pasada creerá que hay cambios**.
- **`field` (BigQuery) sólo garantiza la otra**: si se mueve, hay filas nuevas. Pero **no ve
  actualizaciones, borrados ni filas tardías**. Un activo con cursor puede cambiar sin que el
  testigo se entere. Si «ya está» se fía de él, **la copia queda vieja en silencio**: se confirma
  en M6. Cuesta además 10 MB facturados en cada pregunta.
- **`listing` es el único con ámbito exacto** (el prefijo) y que ve altas, bajas y cambios. En GCS
  la generación da **falsos «cambió»** al reescribir lo mismo. El listado de GCS trae también el
  `md5Hash`, que no los daría. En R2, el ETag de una subida simple es el MD5 y no los da; la subida
  multiparte no se probó. Su coste crece con el número de objetos, y eso no se midió a escala.
- **No medidos**:
  - Azure: no hay credencial de escritura; los contenedores de prueba son públicos y de solo lectura.
  - El S3 de AWS: la credencial de prueba caducó.
  - SFTP: la VM se borró.
  - JSONL: es `field`, como BigQuery, según el código.

### M3 · medido (2026-10-13): las señales de cambio con el ámbito del activo

| Origen | Señal | Ámbito | Qué ve | Qué no ve, o qué cuesta |
|---|---|---|---|---|
| Postgres | Contadores por tabla (`pg_stat_user_tables`: `n_tup_ins/upd/del`) | **La tabla** ✓: cambiar `b` no tocó los de `a` | Inserts, updates y deletes ✓ | Sólo dicen **cuántos**, no cuándo. Y tras suspenderse el cómputo: **sobrevivieron** (el cómputo se suspendió y volvió a arrancar a las 11:52 con los mismos valores; se creía que se perdían, y no). Un reinicio sucio sí los borra, según la documentación de Postgres; eso no se midió |
| Postgres | Decodificación lógica (una ranura `test_decoding`) filtrada por tabla | **La tabla** ✓: 0 cambios de `a` entre 8 cuando sólo cambió `b`; 1 de 11, con la fila, cuando cambió `a` | Todo, en orden, con la fila | Hace falta `wal_level=logical`, el privilegio `REPLICATION` y **una ranura que retiene WAL en el origen** mientras no se consume: disco del cliente. Lee los cambios de todo el servidor para filtrar (718 ms para 11) |
| Postgres | Hora de commit por fila (`track_commit_timestamp`) | — | — | **Apagado en Neon** |
| Postgres | Hora de última modificación de una tabla | — | — | **No existe en Postgres** |
| BigQuery | `lastModifiedTime` (`tables.get`, o `__TABLES__`) | **La tabla** ✓: cambiar o truncar `u` no lo movió | Update, delete, insert tardío ✓; **0 B facturados**, unos 0,5 s | **No se movió con una inserción en *streaming*** (`insertAll`), y `numRows` tampoco (5 s después). No se midió si se mueve cuando se vacía el búfer |
| GCS | `updated` por objeto | El objeto ✓ | Cualquier escritura | También cambia **sólo con metadatos**: dice «se tocó», no «cambió el contenido». Para eso, `md5Hash` (o `crc32c`) |
| R2 | `last-modified` por objeto | El objeto ✓ | Cualquier escritura | También cambia al **reescribir lo mismo**. Para el contenido, el ETag de una subida simple |

Lo que esto quiere decir:

- **Ningún origen da gratis y exacta la edad de un activo**, pero todos dan algo con su ámbito.
  - BigQuery: `lastModifiedTime` es gratis y tiene el ámbito correcto, menos el *streaming*.
  - Postgres: los contadores por tabla tienen el ámbito correcto, sobreviven a la suspensión, pero dicen cuántos cambios y no cuándo: hay que guardarlos y comparar,, o la
    decodificación lógica, que es exacta y cuesta una ranura en el origen.
  - Objetos: el listado con el hash de contenido.
- **Lo que hoy usa ORE como testigo no es lo mejor que cada origen ofrece** para saber si un activo
  cambió:
  - En BigQuery, `lastModifiedTime` ve lo que `field` no ve, y no cuesta.
  - En Postgres, los contadores o la decodificación tienen el ámbito que el LSN no tiene.
  - En GCS, `md5Hash` evita los falsos cambios de la generación.
- Y el testigo sigue sirviendo para lo que se diseñó, **retomar** una lectura (0016). Saber si un
  activo cambió es otra pregunta, y necesita su propia señal.

### M1 y M4–M8 · medido (2026-10-13): lo que ORE hace con una copia y con una lectura en vivo

**Cómo se midió.**
- **Orígenes de prueba.** La misma tabla en dos sitios: 1000 filas, 100 nulos en `cliente`, valores
  conocidos.
  - Postgres: `ore_salud.pedidos` en Neon, fuente `pg_test`, testigo `log`.
  - BigQuery: `ventas.ore_salud_pedidos`, fuente `bq_test`, testigo `none`.
- **Bases.** Cuatro, creadas por la persona desde la consola (el camino real del producto): una
  standard y una foreign por origen (`pg_test_s`, `pg_test_f`, `bq_test_s`, `bq_test_f`).
- **Pasadas.** Cinco pasadas de copia **normales**, sin `REHACER`, que es lo que haría un refresco
  periódico, con un cambio en el origen entre una y otra. Son Jobs con la plantilla de la malla
  (`copiar-*`).
- **Lecturas.** SQL y Python desde la consola y un puesto, la vista detallada del catálogo, y los
  punteros del árbol.
- **Antes**, el árbol de `victor` tal como estaba: 75 punteros y sus snapshots en el lago.

**Las pasadas:**

| | Cambio en el origen | `pg_test_s` (`log`) | `bq_test_s` (`none`) |
|---|---|---|---|
| P1 | Ninguno | **Recopia entera**: 1000 leídas. El driver no sabe leer el changelog, y el LSN se mueve solo (M2) | «ya está», sin leer una fila |
| P2 | `UPDATE` de 9 filas, `DELETE` de 1, `INSERT` tardío de 1 (filas: 1000 → 1000; no nulos de `cliente`: 900 → 892) | Recopia; el informe dice 1000 y 892 ✓ | **«ya está»: la copia sigue en 900** |
| P3 | Columna nueva `nota` | Recopia, **la ignora y no lo dice** (`esquema_cambiado: false`) | «ya está» |
| P4 | `cliente` pasa a llamarse `cliente_x` | `estado: error`, con el motivo justo; el puntero **conserva** el snapshot anterior | **«ya está»** |
| P5 | `TRUNCATE`: el origen se vacía | **`copiada` con 0 filas**: de 1000 a 0, sin un aviso | — |

**Las lecturas tras P4** (el origen ya con `cliente_x`):

| Activo | SQL y Python | La vista detallada del catálogo |
|---|---|---|
| `pg_test_s` (copia en `error`) | **Se niega**: «la copia no está hecha: el informe dice `error`». La copia buena de P3 está en el lago y nadie la puede leer | `State: error`; el esquema se sirve |
| `bq_test_s` (copia «al-dia») | **1000 filas y 900 no nulos, sin aviso**. El origen tiene 892, y la columna ya no existe | `State: al-dia`; esquema y filas, «con normalidad» |
| `pg_test_f`, `bq_test_f` (en vivo) | **Fallan**: «`ore_salud.pedidos` no tiene la columna `cliente`» | **«Not found»** con ese mismo texto, y **sin estado** |

**Lo que dice cada M:**

- **M1 · La hora.** La que se ve es la del **commit del puntero**, y cambia cuando cambia el puntero.
  También cuando sólo pasa de `copiada` a `al-dia`. **τ_lectura no se guarda en ningún sitio.**
  En el árbol de `victor`:
  - `bq.ventas.pedidos` tenía su puntero de las 09:08 de hoy y sus datos son del **2 de octubre**.
  - `s3_pedidos`, del **29 de septiembre**.
  - La hora del snapshot de Iceberg es la cota más cercana que hay a τ_lectura, y nadie la enseña.
- **M4 · El informe.**
  - Cuando copia, **dice la verdad**: las filas y los no nulos por columna coinciden con el origen
    (P2), y con el lago (`total-records` = `filas` en los 8 punteros revisados).
  - Pero **sólo mide lo declarado**: una columna nueva del origen no se ve (P3).
  - `esquema_cambiado` habla de la copia, no del origen. En los punteros de `victor` sigue en `true`
    en pasadas que no copiaron nada.
- **M5 · ¿Se sirve la copia anterior si la nueva falla? No.** El puntero conserva el snapshot, pero
  `Puntero::hecho` (`materializar.rs:1287`) y el SDK sólo leen `copiada` o `al-dia`.
  - **Una pasada fallida apaga el activo** hasta que otra salga bien. Es lo contrario de
    *write-audit-publish*, que no publica lo malo y **sigue sirviendo lo bueno**.
  - La consola sí enseña `State: error`.
- **M6 · ¿Acierta «ya está»?**
  - **Con `none`, no mira el origen.** Dice «al-dia» siempre: con datos cambiados (P2), con el
    esquema roto (P4) y con el origen muerto. En `victor`, a las 09:08, `sftp_s.tablas.us_states`
    dijo «al-dia» en la misma pasada en la que otra tabla del mismo SFTP falló con «no responde»
    (la VM estaba borrada). Lo mismo `s3_pedidos`, con su credencial de S3 inexistente.
  - **Con `log`, no acierta nunca «ya está»**: recopia entero en cada pasada, haya o no cambios
    (P1).
  - Un puntero de `minst_standard11` dice `al-dia` y lleva el motivo «la credencial no está
    definida».
- **M7 · Las averías de frescura.**

  | Avería | ¿La ve ORE? |
  |---|---|
  | 1 · No corrió | **No.** No hay pasada periódica. Y el primer intento de P1 se colgó **5 min** resolviendo el DNS del IdP y murió; el Job lo reintentó y en ningún sitio queda rastro |
  | 2 · Corrió y no trajo nada | **No.** Publica 0 filas como `copiada` (P5) |
  | 3 · El origen se paró o murió | **No**, con `none` (SFTP y S3 de `victor`). Con `log`, se ve como error de lectura sólo si el origen no contesta |
  | 4 · Llegó una parte | No medido: una tabla por dataset |
  | 5 · Llegó tarde, por debajo de la marca | Con `log` y `none` no aplica (se copia entero). Con `field` no se vería (M2) |

  Y el **cambio de esquema** (la avería de calidad n.º 1 de F2):
  - una columna **nueva** pasa sin verse;
  - una **renombrada** rompe la copia con `log`, pasa sin verse con `none` y rompe la lectura en
    vivo.
- **M8 · Lo que se sabe en vivo.**
  - Una lectura en vivo es fresca por construcción: lee el origen al consultar.
  - Pero el catálogo de la fuente **no se entera de que el origen cambió**: la `Table` sigue
    declarando `cliente`.
  - La consola lo enseña como **«Not found»**, aunque la tabla existe; lo que pasa es que su
    esquema cambió.
  - La lectura en vivo no tiene estado.

**En una frase:** ante el mismo cambio del origen, la copia con testigo **se apaga**, la copia sin
testigo **sirve lo viejo como fresco** y la lectura en vivo **se rompe al consultar**. Y en los tres
casos **nadie se entera antes que el consumidor**.

## El producto, como debería ser (la visión, 2026-10-13)

Esta parte describe el producto **ideal**, mirado desde quien lo usa. No dice todavía en qué pieza vive
cada cosa ni en qué orden se construye: eso viene después, y se recorta desde aquí. Cada rasgo sale
de un fundamento (F), de una buena práctica, o de un fallo medido (M).

### La promesa, en una frase

**Cada activo del catálogo dice la verdad sobre sí mismo.** Dice de cuándo es lo que muestra, si es lo
que dice ser y qué le pasa. Lo dice **antes de que nadie se tropiece con ello**. Y cuando algo se
rompe, quien consume sigue trabajando con la última versión buena, sabiendo exactamente lo vieja que
es, mientras su dueño ya está avisado.

Hoy ORE hace lo contrario en los tres casos medidos. La copia con testigo se apaga (M5); la copia sin
testigo sirve lo viejo como fresco (M6); la lectura en vivo se rompe al consultar (M8). Y en los tres
casos el primero en enterarse es el consumidor.

### Para quién

| Quién | Qué necesita | Hoy (medido) |
|---|---|---|
| **Quien consume** (SQL, Python, un notebook, un dashboard) | «¿Me puedo fiar de esto ahora? ¿De cuándo es?», sin tener que preguntar a nadie | Un error, o un dato viejo sin aviso |
| **Un agente** | La salud en lo que recibe, legible por máquina, para abstenerse o decirlo | Nada |
| **Quien es dueño** de una base o de un dataset | Enterarse el primero, saber por qué, arreglarlo rápido y sin ruido | Se entera por el consumidor |
| **Quien es dueño del origen** | Saber que su sistema dejó de cambiar, cambió de forma o dejó de contestar | Nadie se lo dice |
| **Quien gobierna** la organización | Cuánto está vigilado, qué se cumple y qué falla a menudo | Nada |

### Los principios

1. **La verdad antes que la comodidad.** Nunca verde sin haberlo comprobado. Lo que no se sabe se
   enseña como **desconocido**, en gris, con lo que sí se sabe al lado (F2: `unknown` no es `pass`;
   M6).
2. **Nunca apagar al consumidor.** Si la versión nueva no llega o no pasa, se sigue sirviendo la
   última buena, marcada con su edad y su motivo (*write-audit-publish*; M5).
3. **Saberlo antes que el consumidor.** El dueño se entera cuando el estado cambia, no cuando alguien
   consulta (M7, M8).
4. **La verdad, siempre y gratis; la vigilancia, se enciende por ámbito.** Lo que ORE ya sabe en la
   propia pasada de la copia lo dice siempre, sin configurar nada: la edad, la última buena, el
   esquema del origen, las filas y los nulos (F2: las averías 1–4 salen de lo que ya se mide al
   copiar). Lo que cuesta, porque toca el origen entre pasadas o escanea datos, **se enciende** por
   base, por schema o por activo, como en Databricks. Y sobre todo eso, el dueño **promete** lo que
   importa. Ver «Las tres capas».
5. **Un solo vocabulario en todas partes.** El mismo estado, con las mismas palabras, en el catálogo,
   en SQL, en Python, en la vista previa, en el linaje, en los avisos y en lo que recibe un agente.
6. **Explicar, no sólo marcar.** Todo estado que no es sano dice:
   - qué pasa;
   - desde cuándo;
   - por qué;
   - a quién afecta aguas abajo;
   - quién lo lleva;
   - qué se puede hacer.
7. **Silencioso por diseño.** Se avisa cuando el estado cambia, y una causa es **una** incidencia,
   no cuarenta alertas (F2, las trampas de medir: la tasa base y la cascada).
8. **La salud es parte de la spec.** Las promesas son documentos del árbol: se revisan, se proponen
   y tienen versión, como el resto (0044, 0056).

### Las tres capas: la verdad siempre, la vigilancia cuando se pide

Lo que la plataforma **debe** al consumidor no es lo mismo que lo que **ofrece** vigilar.

Databricks deja apagadas sus dos piezas de calidad por defecto: la detección de anomalías se activa
por schema y el *profiling* por tabla. Lo hace porque las dos **gastan cómputo del cliente**. En
cambio, una tabla Delta siempre dice cuándo se modificó, y las *expectations* de un pipeline
siempre cuentan lo que falla. Lo mismo aquí:

| Capa | Qué es | Cuesta | Por defecto |
|---|---|---|---|
| **0 · La verdad** | Todo lo que ORE ya sabe en la propia pasada de la copia: *data as of*, la última versión buena, el resultado de cada pasada, *Unverified* cuando no se puede comprobar, el esquema del origen comparado al copiar, las filas y los nulos del informe, las guardas que retienen, y las promesas que se comprueban al copiar | Nada que no cueste ya la copia | **Siempre**, sin interruptor: es que la plataforma no mienta |
| **1 · Vigilancia** | **Entre pasadas**: preguntar al origen si cambió, si contesta y si la credencial vale; *Refresh overdue*; el esquema de lo que se lee en vivo antes de que alguien consulte; incidencias y avisos al dueño | Poco, pero **toca el origen del cliente** | **Apagada.** Se enciende **por base o por schema**, con un clic que dice qué hará y qué costará |
| **2 · Perfilado y promesas caras** | Perfiles por columna y su historia, promesas SQL, y más adelante monitores aprendidos | **Escanea datos** | **Apagada.** Se enciende **por activo** |

En la ficha de un activo:

```
 Health
   Data as of Oct 13, 13:37 · last refresh failed · serving last good version   ← capa 0, siempre
 Monitoring        Disabled   [Enable for database pg_test_s]                   ← capa 1
   Checks the source between refreshes, alerts the owner, opens issues.
 Profiling         Disabled   [Enable]                                          ← capa 2
```

**La honestidad no depende de un interruptor; el gasto, sí.**
- Sin vigilancia, una copia sin señal de cambio dice *Unverified · as of Oct 2*: es honesta y
  no sabe más.
- Con vigilancia, dice además que el origen cambió dos horas después, y el dueño ya tiene el aviso.

### El vocabulario

| Concepto | Qué es para quien lo ve |
|---|---|
| **Health** | El estado del activo: **Healthy** (verde), **Warning** (ámbar), **Degraded** (rojo), **Unknown** (gris). Gana el peor de sus señales, y baja por el linaje: un activo con una raíz degradada lo dice |
| **Data as of** | La hora del mundo que refleja: cuándo se leyó el origen (τ_lectura, F1). Es la hora principal, nunca la de un commit (M1) |
| **Checked** | Cuándo se comprobó por última vez que seguía igual. «Data as of 09:00 · checked 2 min ago · source unchanged since» es edad 0 (F1: retraso ≠ edad) |
| **Last good version** | La versión que se sirve mientras la nueva no llega o no pasa |
| **Promise** | Lo que el dueño promete del activo. Tipos: *Fresh within* (la tolerancia de F1), *Schema* (las columnas y sus tipos), *Volume* (filas esperadas), *Valid* (no nulo, único, rango, valores admitidos, una consulta SQL). Cada una, **Blocking** o **Warning** |
| **Monitor** | Lo que se vigila. Siempre (capa 0): la pasada (si corrió y si acabó), el esquema del origen al copiar, el volumen y los nulos. Con la vigilancia encendida (capa 1): el origen entre pasadas (si cambió, si contesta, si la credencial vale) |
| **Monitoring · Profiling** | Los dos interruptores: la capa 1 por base o schema, la capa 2 por activo. Los dos, apagados por defecto |
| **Issue** | Una causa con su historia: abierta, reconocida o resuelta, con dueño, línea de tiempo y alcance. Agrupa todos los síntomas de esa causa |
| **Coverage** | Qué tiene promesas, qué sólo tiene la vigilancia de base y qué no se puede comprobar (y por qué) |

### Las experiencias

**1 · En el catálogo, en cada fila.** Un punto de color y «as of» al lado del nombre:

```
  ●  pedidos              Dataset   as of Oct 13, 13:37 · 2 h ago   ⚠ Degraded
  ●  ore_salud_pedidos    Dataset   as of Oct 2 · 11 days ago       ◌ Unverified
  ●  pedidos (live)       Table     live · schema drift             ⚠ Degraded
```

Bases y schemas suman a sus hijos («3 of 41 degraded»). Hay un filtro «Needs attention». Y el hover
dice lo mismo, en corto.

**2 · La pestaña Health de un activo.**

```
 pedidos · pg_test_s.ore_salud                                       ⚠ DEGRADED since 13:45
 ─────────────────────────────────────────────────────────────────────────────────────────────
 Serving the last good version · data as of Oct 13, 13:37 (2 h ago)
 Why: the last refresh failed — the source renamed column `cliente` to `cliente_x` (13:44).
 Impact: 2 downstream datasets, 1 dashboard · 14 reads in the last hour got this version.
 Owner: victor · Issue #41 · acknowledged 13:52
 [Accept the new schema]   [Refresh now]   [Open issue]

 Freshness   ██████████████████▒▒▒▒░  copied · unchanged (verified) · failed · not run
 Promises    Fresh within 1h ........ ✗ failing since 14:37
             Schema ................. ✗ source changed (1 rename)
             Valid: total ≥ 0 ....... ✓ 13:37
 Monitors    Volume 1000 rows (±0 %) ✓ · Nulls `cliente` 10.8 % (was 10.0 %) ✓
 History     schema changes · refreshes · issues
```

**3 · Al leer: el aviso viaja con el dato.**
- **SQL y la vista previa**: una franja sobre el resultado, como esta:
  «⚠ Degraded · data as of Oct 13, 13:37 (2 h ago) · last refresh failed: source column renamed ·
  showing the last good version».
- **Python**: el resultado lleva su salud (`df.attrs["health"]`) y un aviso de Python. Quien quiere
  rigor lo pide al leer: `ore.sql(q, fresh_within="1h")` o `strict=True` falla en vez de servir lo
  degradado.
- **Una lectura en vivo** con el esquema cambiado dice «Schema drift», con la columna que cambió.
  Nunca «Not found» (M8).

**4 · Los agentes.** Cada resultado que recibe un agente lleva su bloque de salud. La política es
la misma que la de una persona:
- `Degraded` → lo dice al responder;
- una promesa `Blocking` que falla → se abstiene salvo que se le insista;
- `Unknown` → dice de cuándo es el dato.

Es lo que hoy no estandariza nadie (H0, §4): aquí sale de serie.

**5 · Quien es dueño: la bandeja de Data Health** (con la vigilancia encendida; sin ella, el
estado está en el catálogo, pero nadie avisa).
- Ve las incidencias **por causa**, ordenadas por impacto, y no una alerta por activo.
- Le llega el aviso (consola, correo o Slack) **cuando el estado cambia**.
- Con un clic puede:
  - **Acknowledge**, para que los consumidores vean que alguien lo lleva;
  - **Snooze** o **Expected**, para un mantenimiento del origen;
  - **Refresh now**;
  - **Accept the new schema**: propone el cambio al catálogo como una propuesta (0044);
  - **Edit promise**.

**6 · Cuando el origen cambia de forma.** Se detecta al pasar, no al consultar:
- «Source added column `nota`» → *[Add to dataset]* *[Ignore]*.
- «Source renamed `cliente` → `cliente_x`» → *[Map]* *[Accept]*.

Las bases foráneas que leen ese origen quedan en la misma incidencia.

**7 · Cuando lo que llega es raro: se retiene, no se publica.** La pasada que trae 0 filas donde
había 1000 no se publica sola:

```
 Refresh held · the source returned 0 rows (last good: 1 000)
 Serving the last good version.   [Publish anyway]   [Keep holding]
```

Es *write-audit-publish* con guardas de base: caída a cero, caída brusca de volumen, columna
desaparecida. Y el dueño decide (M7, avería 2).

**8 · Cuando el origen se cae o su credencial caduca.** El origen tiene salud propia: si contesta,
si la credencial vale, cuándo cambió su esquema por última vez y cuándo se catalogó. Si un origen se
cae, **una** incidencia en el origen y todos sus activos en `Degraded` con «source unreachable since
…». No hay «al-dia» con un origen muerto (M6).

**9 · Cuando no se puede saber.** Una copia que no se puede comprobar no se disfraza:
- Sin promesa y sin vigilancia, dice **«Unverified · data as of Oct 2 (11 days ago)»**, en gris.
- Con la vigilancia encendida, sabe además si el origen cambió después: **«Behind · the source
  changed 2 h after this copy»**.
- Con una promesa *Fresh within*, ORE la cumple recopiando a su ritmo, y el estado la refleja.

**10 · Los builds.** Un Transform mira la salud de sus entradas:
- Una promesa `Blocking` que falla en una entrada **detiene el build**, como un *circuit breaker*,
  y lo dice.
- `Degraded` sin `Blocking`: construye, y la salida hereda el estado y el motivo.

**11 · Quien gobierna: la vista de la organización.**
- El porcentaje de activos sanos, por base y por dueño.
- El cumplimiento de las promesas de frescura en 30 días.
- Las incidencias abiertas y el tiempo medio hasta detectar y hasta resolver.
- La cobertura.
- Los activos más ruidosos.
- Los orígenes en riesgo: credenciales que caducan, esquemas que cambian a menudo.

**12 · Escribir una promesa.**
- Desde la pestaña Health, con **plantillas y sugerencias sacadas de lo observado**, por ejemplo:
  «this table changed every hour for the last 14 days — promise *Fresh within 2h*?».
- También en el árbol, como documento.
- Queda propuesta y revisada (0044), y desde ese momento se vigila. **Las sugerencias proponen;
  una persona decide** (F2: las reglas las pone una persona).

### Los escenarios medidos, como deberían acabar

| Lo que pasó (M) | Hoy | En el producto |
|---|---|---|
| Columna renombrada; copia con testigo (P4) | Se apaga: SQL y Python fallan | `Degraded`, se sirve la última buena con su edad; Issue «source schema changed» al dueño; *[Accept]* |
| Columna renombrada; copia sin testigo | «al-dia» y datos viejos | `Degraded`: el monitor de esquema del origen lo ve en la pasada |
| Columna renombrada; lectura en vivo | «Not found» al consultar | «Schema drift» en el catálogo **antes** de consultar, en la misma Issue |
| Datos cambiados sin testigo (P2) | «al-dia» con 900 donde hay 892 | «Unverified · as of …»; con *Fresh within*, se recopia y queda al día |
| Columna nueva (P3) | Ignorada en silencio | «Source added column `nota`» → *[Add]* *[Ignore]* |
| Origen vaciado (P5) | Publica 0 filas | *Refresh held*; se sirve la última buena; el dueño decide |
| Origen muerto o credencial inválida | «al-dia» | La salud del origen en rojo; una Issue; sus activos `Degraded` |
| Una pasada colgada y reintentada | Sin rastro | En la historia de refrescos, y como *Refresh overdue* si una promesa vence |
| La hora (M1) | El commit del puntero | **Data as of** (τ_lectura) y **Checked** |

### Cómo se sabrá que funciona

- **Antes que el consumidor**: de cada Issue, la parte que el dueño conoció antes de la primera
  lectura afectada. Objetivo: casi todas.
- **Cero verdes sin comprobar**: ningún activo `Healthy` cuya última comprobación no lo respalde.
- **Nunca a oscuras**: de las lecturas de un activo degradado, las que se sirvieron con aviso frente
  a las que fallaron. Objetivo: que no falle ninguna salvo en modo estricto.
- **Silencio**: avisos por Issue y falsas alarmas por semana.
- **Cobertura**: los activos con promesas, con sólo la vigilancia de base, y los no comprobables.

### Lo que la visión deja para después

- **Monitores aprendidos.** La cadencia esperada y la estacionalidad del volumen, sobre la historia
  guardada. Llegan cuando haya semanas de historia, y **lo explícito gana** (F2).
- **Perfiles de distribución y deriva.**
- **Conciliar** el origen con la copia, fila a fila.

## Bajarla a tierra: el plan por capas (propuesto, 2026-10-13)

### Cómo se construye

- **Por capas, y dentro de cada capa por cortes verticales.** Primero la capa 0 entera: lo que la
  plataforma debe y hoy incumple. Luego la 1 y la 2, que se ofrecen. Cada paso toca a la vez lo que le
  toque de la spec, de ORE y de la consola, y entrega algo que se nota.
- **En la capa 0, primero el daño medido.** Lo que **apaga** al consumidor (M5), lo que le **miente**
  (M6), lo que **publica basura** (P5) y lo que **se rompe al consultar** (P4, M8).
- **La vara antes que el código.** Los escenarios M1–M8 pasan a ser pruebas que tienen que salir como
  dice la visión. Hoy salen en rojo, y cada paso dice cuáles pone en verde. Se miden en producción.
- **Cada paso, su plan y su go.** Lo que toca la malla lleva su go aparte (flag nuevo y malla, nunca
  juntos). Lo que toca la spec sube su versión en `C:\oos`.

### La capa 0 es las cañerías, y también el suelo

S1–S8 son lo que hace que lo demás funcione. Y son, al mismo tiempo, lo primero del producto que ve
quien consume. Las dos cosas a la vez, y por eso van primero y no se apagan.

**Las cañerías: lo que las capas 1 y 2 reutilizan, en vez de construir lo suyo.**

| Pieza de la capa 0 | Lo que deja hecho | Quién lo reutiliza encima |
|---|---|---|
| **S1** · el vocabulario en la spec | Los cuatro estados, *data as of*, *checked*, *last good version*, las tres capas, el bloque de salud y sus motivos | **Todo**: las capas 1 y 2 hablan con esas palabras y devuelven ese bloque. Una incidencia (S11) es un motivo con historia; la cobertura (S9, S15) es qué capas están encendidas |
| **S2** · la última buena y `datos_de` | La versión que se sirve mientras la nueva no llega, y la hora real de los datos | **S6**, porque la edad sale de `datos_de`. **S11**, porque una incidencia dura «desde» que dejó de haber una versión nueva. **S15**, porque el cumplimiento de *Fresh within* se mide sobre esa hora. **S10**: el vigilante compara «el origen cambió después de *as of*» con esa misma hora |
| **S3** · la señal de cambio de cada driver | Una pregunta por tipo de origen, con el ámbito del activo: «¿cambió desde *as of*?». Su coste está medido (M3) | **S10**: el vigilante hace **la misma pregunta**, sólo que entre pasadas y donde se enciende. **S8**: el refresco no copia lo que no cambió. **S14**: los monitores aprendidos estiman la cadencia del origen con la historia de esas respuestas |
| **S4** · retener antes de publicar | El camino «se escribe, se comprueba, se publica o se retiene», con *Publish anyway* | **S7** y **S13**: una promesa `Blocking` retiene **por ese mismo camino**. **S11**: una retención abre su incidencia |
| **S5** · el esquema del origen frente al declarado | Un diff de esquema (nueva, quitada, renombrada, otro tipo) en la copia y en el catálogo, y el error *Schema drift* al leer en vivo | **S10**: el vigilante lo aplica a lo que se lee en vivo **antes** de que alguien consulte. **S7**: la promesa *Schema* es ese diff con una severidad. **S11**: un cambio de esquema es una causa, con todos sus activos debajo |
| **S6** · el estado calculado | La función del estado: lo medido, lo prometido y la hora, el peor gana, baja por el linaje. Y por dónde llega a quien lee (`/health`, `/assets`, SQL, Python, la vista previa, los agentes) | **S11**: una incidencia se abre **cuando el estado cambia**, y su alcance es el linaje de S6. **S15**: la vista de la organización suma estados. Las capas 1 y 2 **sólo añaden señales** a esa función; no traen otra |
| **S7** · las métricas en la pasada | Las promesas *Schema*, *Volume* y *Valid*, calculadas sobre los lotes que ya se leen. La forma de una promesa en el `Ruleset` | **S12**: el perfilado son **las mismas métricas, más y con historia**. **S13**: las promesas SQL son otra forma de la misma gramática. **S15**: sugerir una promesa es proponer un documento de esa gramática |
| **S8** · la pasada periódica | Un CronJob por celda que recorre lo prometido y deja cada pasada registrada | **S10**: el vigilante **es esa pasada**, ampliada donde la vigilancia está encendida. No hay un segundo planificador |

**Lo que no se reutiliza, porque es propio de cada capa:** la tabla de salud del lago (S9), que
guarda lo medido entre pasadas; el interruptor por ámbito (S9); el ciclo de una incidencia y sus
avisos (S11); y la historia de perfiles (S12). Es lo que cuesta, y por eso se enciende.

**El suelo: lo que ya ve quien consume con sólo la capa 0, sin encender nada.**
- Lo que se lee es **la última versión buena**, con su edad y su motivo, aunque la pasada haya
  fallado (S2).
- **Data as of** en cada fila del catálogo, nunca la hora de un commit (S2, S6).
- **Unverified** donde hoy pone «al-dia» sin haber mirado (S3).
- **Refresh held** donde hoy se publicarían 0 filas (S4).
- **Schema drift** donde hoy dice «Not found» (S5).
- El aviso **dentro del resultado** de SQL, de Python, de la vista previa y de lo que recibe un
  agente (S6).
- Lo prometido, cumplido o no, al copiar (S7), y refrescado a su ritmo (S8).

Es **la promesa de la visión entera**: cada activo dice la verdad sobre sí mismo. Las capas 1 y 2
añaden **saberlo antes**, **saberlo entre pasadas** y **saber más**.

**Por eso la capa 0 no se puede apagar y va primero.** Lo que se construya encima de una plataforma
que miente vigila mentiras. Eso es lo que pasó en M6: un monitor de frescura habría visto «al-dia»
en una copia con el origen muerto, y lo habría dado por bueno.

### Las decisiones que hay que tomar antes de S1

| | Decisión | Recomendada |
|---|---|---|
| **D-A** | **Dónde vive cada cosa medida** | Lo que se sabe **al escribir** (capa 0: *data as of*, la última buena, el resultado de la pasada, las promesas comprobadas al copiar), **en el puntero**, en el mismo commit que publica; su historia es el `git log`. Lo que se mide **en el tiempo** (capas 1 y 2: comprobaciones entre pasadas, incidencias, perfiles), en **una tabla Iceberg del lago de la celda**, append-only y con la forma de OpenLineage. Así no se llena el árbol de commits por hora, y se consulta por SQL |
| **D-B** | **El estado se calcula, no se guarda** | Se calcula con lo medido, lo prometido y la hora. Una promesa que cambia recalifica sin reescribir nada. Y *Fresh within* se juzga **al leer**, con el reloj: no necesita a nadie vigilando para saber que una copia se pasó de plazo |
| **D-C** | **Refrescar** | **Sin promesa, no se recopia sola** (el coste lo declara quien lo paga). **Con *Fresh within*, ORE refresca para cumplirla**: eso es mantener la copia, que es lo que la spec ya pide del mantenedor, así que es **capa 0**. Necesita la pasada periódica (la malla) |
| **D-D** | **La gramática de las promesas** | **Un `Ruleset`** que aprende a nombrar activos y ámbitos (lo que la spec ya decidió, OOS1005). `Dataset.freshness` pasa a ser la *Fresh within* de ese dataset. ODCS, para importar y exportar |
| **D-E** | **Qué retiene la capa 0 sin que nadie lo prometa** | **Retiene**: cero filas donde había, y una columna declarada que desaparece. **Avisa sin retener**: una caída brusca de volumen y una ola de nulos |
| **D-F** | **Al leer algo degradado** | **Servir con aviso.** Fallar es opcional al leer (`strict=True`, `fresh_within=`). Un build se para sólo ante una promesa `Blocking` |
| **D-G** | **Los interruptores** | La capa 1 se enciende **por base o por schema** y la capa 2 **por activo**. Las dos vienen apagadas. **El interruptor es un documento del árbol**, no un ajuste escondido: se revisa, se propone (0044) y tiene versión. La consola lo enciende con un commit, como cualquier otra decisión |

### Los pasos

**Capa 0 · La verdad (siempre, sin interruptor)**

| | Paso | Lo que entrega | Dónde | Pone en verde |
|---|---|---|---|---|
| **S0** ✅ | **La vara** — **hecha** (2026-10-13, `pruebas-de-fuego/salud/vara.py`): ver «S0 · la vara» | M1–M8 como pruebas con el resultado de la visión, con orígenes de prueba propios (Neon, BigQuery, un bucket) que se montan y se desmontan solos. Más una versión de laboratorio para el CI | `pruebas-de-fuego/salud/`, CI | — (todas en rojo) |
| **S1** ✅ | **El vocabulario y lo que se debe** — **hecho** (2026-10-13): OOS v1alpha30, [`01-la-salud`](../../vendor/oos/spec/v1alpha30/01-la-salud.md) y `health.schema.json` (`oos` `887a7a0`) | En la spec: los cuatro estados, *data as of*, *checked*, *last good version* y las tres capas. Sus obligaciones: no servir lo no comprobado como fresco, servir lo último bueno con su estado, `unknown` ≠ `pass`, y que la capa 0 no se puede apagar. Sin código | `C:\oos` (v1alpha30) | — |
| **S2** ✅ | **Nunca a oscuras, y la hora verdadera** — **hecho, desplegado y medido en la copia** (2026-10-13, `1466340d`, en el clúster con `a5d6e184`; consola `9fb92ff`): ver «S2 · lo hecho» y «S2 y S3 · medidos» | Una pasada que falla deja servir **la última versión buena**, con su estado y su motivo. El puntero guarda **`datos_de`** (τ_lectura, tomada antes de leer). SQL, Python y la vista previa leen lo último bueno y llevan **el bloque de salud**. La consola enseña «as of», no el commit | `materializar.rs` (`informe_de`, `escribir_informe`), los lectores (`Puntero::hecho`; `ore-serve`: `puestos.rs`, `funciones.rs`, `copia.rs`), el SDK (`_o_el_error`, `sql`), `assets.rs`, consola | **M5, M1** |
| **S3** ✅ | **No decir «al día» sin comprobar** (cierra PG1) — **hecho, desplegado y medido** (2026-10-13, `e1005d8d`): el verbo `cambio` en Postgres (contadores y `relfilenode`) y BigQuery (`lastModifiedTime`, sin *streaming*); la copia guarda la señal y decide con ella. Los objetos, el SFTP y el JSONL, todavía sin el verbo: ver «S2 y S3 · medidos» | Un verbo nuevo de los drivers, **la señal de cambio con el ámbito del activo**, distinto del testigo (M3): `lastModifiedTime` en BigQuery, los contadores por tabla en Postgres, el hash de contenido en los objetos. **Se pregunta en la pasada**, no entre pasadas. «Ya está» sólo si esa señal lo respalda; si no, **Unverified** con su edad. `log` deja de recopiar entero cuando nada cambió | `ore-driver`, `ore-read-*`, `al_dia_con`, informe, consola | **M6**, P1, P2 |
| **S4** ✅ | **Retener lo raro** — **hecho, desplegado y medido** (2026-10-13, `2745ab7a`; C7 en verde): ver «R1, R2, S4 y S5 · lo hecho» | Las guardas de D-E antes de publicar. Lo retenido no mueve el puntero, se sigue sirviendo lo último bueno, y la consola dice **Refresh held** con *[Publish anyway]* | `materializar.rs`, `ore-store`, `ore-serve`, consola | **P5** |
| **S5** ✅ | **El esquema del origen, al copiar y al catalogar** — **hecho y desplegado** (2026-10-13, S5a `28aabd68`, S5b `a5fc920b`, S5c `21a8df6e`); S5a y S5b medidos (C4, C5, C6b en verde), S5c a mano: ver «R1, R2, S4 y S5 · lo hecho» | El esquema del origen se compara con la `Table` declarada: columna nueva, quitada, renombrada o de otro tipo. Queda en el informe y en el catálogo. Una lectura en vivo que lo encuentra dice **Schema drift**, nunca «Not found». *[Accept the new schema]* abre una propuesta (0044) | `materializar.rs`, el Job de catálogo, `ore-federation` (el código del error), `ore-serve`, consola | **P3, P4**; M8 en lo que se dice al leer |
| **S6** | **El estado, en todas partes** | El estado calculado (D-B) con lo que la capa 0 sabe: el peor gana, baja por el linaje, `Unknown` en gris. `GET /health` y `health` en `/assets`. En la consola, el punto de color y «as of» en cada fila, el filtro «Needs attention» y la sección Health. **El bloque de salud en lo que recibe un agente**, con su política | `ore-core`, `ore-serve`, el agente del puesto, consola | La visión §1–§4 sobre la capa 0 |
| **S7** | **Las promesas que se comprueban al copiar** | La gramática (D-D) en la spec. *Schema*, *Volume* y *Valid* de biblioteca, calculadas **en la misma pasada** (generaliza `sin_nulos`, sin leer un byte más). `Blocking` retiene; `Warning` publica y anota. Un build se para ante una `Blocking`. *Fresh within* se juzga al leer (D-B) | `C:\oos`, `ore-store` (`carga.rs`), `materializar.rs`, 0055, consola (escribirlas) | La visión §10 y §12 |
| **S8** | **El refresco que cumple *Fresh within*** | La pasada periódica por celda, que recopia lo que tiene *Fresh within* y va a vencer, con la señal de S3 para no copiar lo que no cambió. Cada pasada queda en su puntero (intentos, fallos, duración) | malla (**go aparte**: un CronJob sobre `48-la-copia`), `ore-cli` | M7, avería 1 en lo prometido |

**Capa 1 · Vigilancia (por base o schema, apagada por defecto)**

| | Paso | Lo que entrega | Dónde | Pone en verde |
|---|---|---|---|---|
| **S9** | **El interruptor y la tabla de salud** | *[Enable monitoring for database …]*, que escribe el documento (D-G) y explica qué hará y qué costará. La tabla de salud del lago (D-A). La cobertura: qué está vigilado y qué no | `C:\oos`, `ore-serve`, ore-store (la tabla), consola | — |
| **S10** | **El vigilante** | **Entre pasadas**, sólo en lo encendido: la señal de cambio («el origen cambió después de *as of*»), **la salud de cada origen** (si contesta y si la credencial vale) y el **esquema de lo que se lee en vivo antes de que alguien consulte**. *Refresh overdue*. Todo, a la tabla de salud | La pasada de S8, ampliada; drivers; la tabla | **M7** (averías 1 y 3), **M8**, el SFTP y el S3 muertos de `victor` |
| **S11** | **Las incidencias y los avisos** | Una incidencia por causa (origen, esquema, pasada, promesa), con su ciclo, su dueño (0052) y su alcance por el linaje. Avisos **al cambiar el estado**. La **bandeja de Data Health** con *Acknowledge*, *Snooze*, *Expected* y *Refresh now* | `ore-serve`, la tabla, `ore-iam` (a quién se avisa), consola | La visión §5 y §8 |

**Capa 2 · Perfilado y promesas caras (por activo, apagada por defecto)**

| | Paso | Lo que entrega | Dónde |
|---|---|---|---|
| **S12** | **El perfilado** | *[Enable profiling]* por activo: métricas por columna en cada versión (nulos, distintos, rangos, cuantiles) y su historia en la tabla de salud, con sus gráficas en la consola | `ore-store`, la tabla, consola |
| **S13** | **Las promesas SQL** | Se evalúan sobre el snapshot nuevo **antes de publicar**; cuestan lo que cueste su consulta, y se dice | `ore-store`, `ore-motor`, consola |
| **S14** | **Los monitores aprendidos** | La cadencia esperada y la estacionalidad del volumen, sobre la historia de S10 y S12. **Lo explícito gana** | Sobre la tabla de salud |

**El gobierno (sobre lo que haya encendido)**

| | Paso | Lo que entrega |
|---|---|---|
| **S15** | **La vista de la organización y las sugerencias** | Salud por base y por dueño, cumplimiento de *Fresh within* a 30 días, tiempos hasta detectar y resolver, **cobertura por capa** y ruido. Promesas sugeridas desde lo observado, que una persona acepta |

### S2 · lo hecho (2026-10-13)

- **`ore_core::salud`**:
  - `legible()`: hay qué leer con `copiada`, `al-dia` **y `error` con versión vigente**.
  - `bloque()`: el bloque de v1alpha30 §6, calculado con el puntero, la `freshness` y una hora
    que llega de fuera. Sin promesa y sin una comprobación **posterior** a los datos, `unknown`
    (`unverified`). Con promesa, se juzga con el reloj.
- **La copia** escribe en el puntero:
  - **`datos_de`**: τ_lectura, tomada antes del testigo y de leer. Lo derivado del lago hereda la
    de su dataset; la copia de una vista SQL, la más vieja de lo que lee.
  - **`intento`** `{cuando, resultado, motivo}`.
  - **`comprobado`**. «Ya está» sólo lo pone si el testigo igual lo **garantiza**: `log`,
    `snapshot` o `listing`, no `field` ni `none` (M2). Si no, el intento es `sin-comprobar`.
- **Leen lo último bueno**:
  - `ore ask` e `invoke` (`Puntero::hecho`);
  - los datos de un puesto, es decir SQL y Python, con el bloque de salud en la respuesta;
  - las funciones;
  - el recuento de copias hechas.
- **`/assets`** lleva `salud` en cada `Dataset`, calculada **al servir** y no en la caché, porque
  una promesa vence con el reloj.
- **El SDK**:
  - `HealthWarning` al leer algo `degraded` o `warning`, con de cuándo es y por qué;
  - `attrs["health"]`;
  - `strict=True` no lee lo degradado. Es el `strict` de quien llama, no el de un build (D-F).
- **La consola**:
  - la salud en la cabecera y en el raíl de la ficha;
  - en la pestaña Copy: *Data as of*, *Checked*, *Fresh within*, el último refresco, *Serving
    the last good version* y los motivos;
  - en la lista, la hora de un dataset es su `dataAsOf`.

**Lo que S2 deja para después:**
- **Los datasets escritos** (`write()`, un Transform) no llevan `datos_de`. Su hora es la de sus
  entradas, y eso es el linaje de S6. Hasta entonces son `unknown`.
- **Las colecciones**, que tienen su propio puntero.
- **Los punteros de antes de S2** no saben de cuándo son (`dataAsOf: null`) hasta su próxima copia.
  No se rellena con la hora del commit.

### S0 · la vara (2026-10-13)

`pruebas-de-fuego/salud/vara.py`: cada caso cambia el origen, pasa la copia de verdad (un Job con la
plantilla de la copia de `main` de la cola, sólo sobre los datasets de la vara) y compara lo que ORE
dice —el puntero y la salud de `/assets`, con el token del agente de la celda— con lo que la visión
espera. Cada caso en rojo dice qué paso lo pone en verde. Corre en `victor`, en producción, en unos
15 minutos, y deja un informe `vara-<fecha>.md`.

| Caso | M | Lo que la visión espera | Paso |
|---|---|---|---|
| C1 | M1 | la copia sabe de cuándo son sus datos, y `/assets` lo dice | S2 |
| C2 | M6 | sin cambios: «al día» comprobado, sin leer, `healthy` | S3 |
| C3 | M2 | una fila actualizada (lo que `field` no ve) se copia | S3 |
| C4 | P3 | una columna nueva se dice: `source-schema-changed` | S5 |
| C5 | M8 | en vivo, una columna renombrada es *schema drift*, no «Not found» | S5 |
| C6 | M5 | la copia que falla sirve lo último bueno, `degraded` | S2 |
| C6b | P4 | y dice que el esquema del origen cambió | S5 |
| C7 | P5 | un origen vaciado se retiene, no se publica con 0 filas | S4 |
| C8 | M7 | sin origen: `refresh-failed`, y lo último bueno se sigue leyendo | S2 |

Los orígenes son propios y pequeños, y la vara los deja en su estado base al empezar y al terminar:
`ore_vara.pedidos` en el Neon de la fuente `postgresql_20260918_1920` (por un Job en la celda con la
credencial del cofre: la URL no sale de ella) y `ventas.ore_vara_pedidos` en BigQuery.

**Lo que la vara no monta sola, y por qué.** Volver a leer el catálogo de una fuente y retirar una
base son gobierno: el agente de la celda no puede (`puerta_del_agente`), y la vara no se lo salta.
Así que **una vez**, una persona recataloga las dos fuentes y crea cuatro bases: `vara_pg` y `vara_bq`
(standard) y `vara_pg_f` y `vara_bq_f` (foreign). `python vara.py montar` lo comprueba y lo explica.
La versión de laboratorio para el CI queda para después: hoy la vara mide en producción.

**La primera vuelta (2026-10-13 20:32, [`vara-20261013-2032.md`](../../pruebas-de-fuego/salud/vara-20261013-2032.md)):
8 de 18 en verde.** Con las fuentes `pg_quality` y `bq_quality` y las bases `vara_*`, que dio de alta
el usuario. En verde, C1–C3 en los dos orígenes, C6 en BigQuery y C8 en Postgres. En rojo, lo que se
esperaba: C4, C5 y C6b (S5) y C7 (S4). Y **dos rojos que nadie esperaba, y los dos son huecos de S3**:

- **C6 en Postgres: la señal no ve el esquema.** Añadir o renombrar una columna no mueve los
  contadores de filas ni `relfilenode`. Con `cliente` renombrada, la copia dijo «ya está» y `healthy`
  sobre una tabla que ya no la tiene. La señal tiene que llevar también la forma de la tabla
  (`pg_attribute`: nombre, tipo, posición). En BigQuery, `lastModifiedTime` sí se mueve con el esquema.
- **C8 en BigQuery: un fallo de la señal se tomó por «no hay señal».** Con la tabla borrada,
  `tables.get` contesta 404, `senal_de` devuelve `None` como si el driver no tuviera el verbo, y con
  el testigo `none` la copia dijo «al día». Que el verbo falle no es «no sé»: la copia tiene que
  intentarlo, y fallar de verdad.

**S3b (`ce7d853a`, 2026-10-13) y la segunda vuelta: 10 de 18.** La huella de Postgres lleva la
forma de la tabla (`contadores-y-forma`: el `md5` de cada columna viva) y una tabla que no está es
un error del verbo; la copia que tenía señal y ve fallar el verbo lo intenta, en vez de darse por
buena (`la_senal`). En la vuelta: C6 en Postgres y C8 en los dos, **en verde**, con «la señal de
cambio no contestó (… no está en el origen): se intenta la copia» en el log. C4 en Postgres ya no
dice `healthy`: la columna nueva se copia (sigue en rojo hasta S5). La vuelta se cortó al leer el
último puntero (el DNS del clúster tardó 300 s en resolver la forja); C8 se leyó a mano.

**Y lo que esa vuelta destapó, que es lo más grave hasta ahora: el recolector borra lo que otra
pasada acaba de escribir.** A las 19:06:23 Flux volvió a crear `copiar-ee4bf960` (la cola que se
repite). Clonó el árbol con el puntero de `vara_bq` en la `00009`, y su `--recoger` retiró «3
fichero(s) que nadie nombraba»: entre ellos la `00010`, que la pasada de la vara acababa de publicar
y a la que el puntero ya apuntaba. Su push se rechazó; el borrado no tiene vuelta. `Lago::huerfanos`
borra todo lo no alcanzable **sin mirar su edad** (`ORE_RECOGER_EDAD` sólo vale para expirar
snapshots), y dos pasadas a la vez —la cola y la consola, o una repetida— bastan. Iceberg resuelve
esto en `remove_orphan_files` con `older_than` (3 días por defecto). Y el dataset queda **sin
salida**: ni una pasada ni un rehacer abren una base que no está.

### R1, R2, S4 y S5 · lo hecho (2026-10-13)

**R1 · la recogida no se lleva lo reciente** (`ae226c88`, `bfed52c3`). `Lago::huerfanos` sólo borra
un fichero que nadie nombra si es más viejo que **la gracia**: 24 h por defecto, como el
`older_than` de Iceberg pero más corta, porque aquí la pasada más larga dura minutos. Se ajusta con
`ORE_RECOGER_GRACIA` en la copia y `--gracia` en `ore datasets mantener`; las pruebas que recogen al
momento la ponen a 0. Dos pasadas a la vez ya no se borran una a otra.

**R2 · una base que no está no bloquea.** Si la versión que nombra el puntero ya no está en el
almacén, la copia lo dice («la versión que nombra el puntero ya no está en el almacén: se copia
entera, como una tabla nueva») y copia entera en vez de fallar para siempre. Medido: `vara_bq`, la
del borrado de la segunda vuelta, se curó sola en la siguiente pasada.

**S4 · retener lo raro** (`2745ab7a`). La guarda de hoy es una: **0 filas sobre una versión
publicada con filas**. La copia no publica. El puntero queda en `error` con su versión de antes y
`intento {resultado: retenida, guarda: zero-rows}`, y la salud dice `refresh-held` con su `guard`,
mientras se sigue sirviendo lo último bueno. Un **rehacer** publica de todas formas: es una persona
diciendo «hazlo», y es lo que hace *[Publish anyway]* en la consola. Las copias de una vista SQL no
llevan guarda: una vista vacía puede ser lo correcto.

**S5a · el esquema del origen, al copiar** (`28aabd68`). El verbo `cambio` devuelve además las
columnas de ahora, con su tipo en el vocabulario de OOS: `pg_attribute` en Postgres y
`schema.fields` de `tables.get` en BigQuery. No cuesta otra pregunta. La copia las compara con la
`Table` declarada y deja en el puntero `esquema_origen {added, removed, retyped, visto}`. La salud
lo dice como `source-schema-changed` con sus `columns`. Si la copia lee una columna que se fue, no
copia: falla con ese motivo y sirve lo último bueno (C6, C6b). Una columna sólo añadida se copia y
se avisa (C4).

**S5b · en vivo, *schema drift* y no «Not found»** (`a5fc920b`). Un código nuevo de los drivers,
`esquema` (HTTP 409), distinto de `objeto` (404): la tabla está y le falta una columna que se pide.
El mensaje dice qué falta y qué columnas tiene ahora. En BigQuery, el clasificador lleva
«Unrecognized name» a `esquema`. La consola lo titula «The source schema changed».

**S5c · *[Accept the new schema]*** (`21a8df6e`). `POST /datasets/{b}/{s}/{n}/esquema/aceptar`,
sólo para una persona. Abre una rama (`esquema-<dataset>-<hora>`) y en ella:
1. vuelve a leer el catálogo de la fuente (`ore source catalog` por la pasarela, con la credencial
   del custodio), que regenera la `Table`;
2. pone al día el `fields` de cada `Dataset` que se copia de esa tabla: un espejo pasa a las
   columnas de ahora; con nombres propios sólo se quita lo que se fue, y sólo en los ficheros que
   escribió la inducción;
3. compara los diagnósticos antes y después: lo nuevo es lo que se rompería.

Y abre la propuesta (0044) con todo eso dicho. Si no hay nada que aceptar, contesta 409 y retira la
rama. La vara no lo mide: abrir propuestas está vetado al agente de la celda. Se prueba a mano desde
la ficha.

**La tercera vuelta (2026-10-13 23:35, [`vara-20261013-2335.md`](../../pruebas-de-fuego/salud/vara-20261013-2335.md),
con `21a8df6e` desplegado): 18 de 18 en verde.** Lo nuevo de esta vuelta:
- C4: la columna nueva se copia y se dice, `source-schema-changed`, con 1000 filas.
- C5: la lectura en vivo de una columna renombrada contesta 409 `esquema` («`cliente` no está;
  sus columnas son …») en los dos orígenes.
- C6b: la copia que falla dice además que el esquema cambió.
- C7: el origen vaciado se retiene, `refresh-held`, y se siguen sirviendo las 1000 filas.

Todos los casos de la capa 0 que la vara sabe medir, en verde. Quedan S6–S8.

### S6 · el diseño (2026-10-14; S6a, S6c y S6e hechos)

**Lo que hay.** Tras S2–S5 la salud sale de **un sitio**: el puntero de una copia. `/assets` la
calcula al servir, pero **sólo para los `Dataset` copiados** (`anotar_salud`). La ficha la enseña, y
SQL y Python la reciben (`puestos.rs`). Todo lo demás calla:
- una vista y un dataset escrito son `unknown` o no dicen nada, aunque lean de algo degradado;
- una base o un schema no suman a sus hijos;
- las funciones, `ore ask` y las herramientas MCP devuelven datos sin su bloque, y v1alpha30 §3.1
  lo exige **con cada lectura**;
- un origen que no contesta se ve como N copias que fallan, cada una por su cuenta.

**Lo que ya está para construir encima:**
- La spec ya tiene las reglas: §2.1 el peor gana, §2.2 baja por el linaje con `upstream-degraded`
  y `asset`, y §3.3 lo desconocido se cuenta aparte.
- El índice de `/assets` ya lleva el grafo: `relaciones` con `sale_de`/`produce` para las vistas,
  los datasets (también los escritos, por `derivedFrom` o `procedencia.leidas`), las colecciones y
  los modelos entrenados.
- Los drivers ya distinguen el fallo (`Codigo`: `credencial`, `conexion`, `tiempo`, `origen`…).

No hace falta tocar el origen ni gastar nada nuevo: **S6 junta lo que la capa 0 ya sabe**.

**Las piezas**

| | Pieza | Qué | Dónde |
|---|---|---|---|
| **S6a** | **El linaje, en el cálculo** | `salud::propagar(items, ahora)`: con el bloque propio de cada activo y sus `sale_de`, en orden topológico y a salvo de ciclos. Lo derivado: (1) si una entrada está `degraded`, `upstream-degraded` con `asset` y, al menos, `warning` (§2.2); (2) una entrada `unknown` deja `unknown` lo que no tenga medida propia, con `unverified` y el nombre de la entrada; (3) una vista sin copia toma como `dataAsOf` el más viejo de sus entradas. Una ref `rota` (otra celda, o que no resuelve) no degrada: no se sabe, y se dice. Y `salud::sumar(estados)`: `{healthy, warning, degraded, unknown}`, con lo desconocido aparte (§3.3) | `ore-core/src/salud.rs` |
| **S6b** | **Lo escrito sabe de cuándo es** (el hueco que dejó S2) | Al escribir un dataset (`write()`, un build de Transform), la procedencia guarda el `datos_de` de cada entrada **tal como se leyó**, y el puntero lleva `datos_de` = el más viejo de esos. Se toma al escribir, no al servir: una entrada refrescada después no rejuvenece lo que se construyó antes | el SDK al escribir (`ore.procedencia` en el resumen del snapshot), `ore-cli/src/datasets.rs` al registrarla en el puntero |
| **S6c** | **El estado, en `/assets` y en `GET /health`** | `anotar_salud` pasa a todo lo que se sirve (`Dataset`, `View`, `MediaCollection` con puntero) y aplica S6a. Cada base y cada schema llevan su `salud` sumada. `GET /health?ref=<ref>` da el bloque de un activo, y `GET /health?scope=<base>[.<schema>]` los de un ámbito con su suma, sin bajar el catálogo entero. Sale del mismo índice cacheado y de la hora de la respuesta | `ore-serve/src/assets.rs`, `rutas.rs` |
| **S6d** | **El bloque, con cada lectura** (§3.1) | Las respuestas de una función (`invoke`, `funciones.rs`), de `ore ask` y de cada herramienta MCP (en `_meta.health`) llevan el bloque de lo que leyeron, **con la propagación**. Una lectura en vivo que va bien lleva `dataAsOf` = la hora de la lectura. La vista previa de la consola enseña la franja de la experiencia 3 | `ore-serve`, `ore-cli` (`preguntar.rs`, `invocar.rs`, `mcp.rs`), consola |
| **S6e** | **La salud del origen, con lo que ven las copias** (la parte de capa 0 de la experiencia 8) | La copia guarda en `intento` el `codigo` del fallo. Con `credencial`, `conexion` o `tiempo`, el motivo es `source-unreachable` y no un `refresh-failed` genérico. La fuente (`DataSource`) lleva su propia salud: el peor de lo que vieron sus copias en su última pasada («2 of 3 copies could not reach it since 13:44»). Una fuente sin copias no dice nada: saberlo sin copiar es capa 1 | `materializar.rs`, `salud.rs`, `assets.rs` |
| **S6f** | **La consola** | En cada fila del árbol y de las listas, el punto de color, «as of …» y el motivo al pasar el ratón. En la base y el schema, «3 of 41 degraded · 5 unverified». El filtro **Needs attention**. La sección Health de la ficha también para vistas, datasets escritos y fuentes, con `upstream-degraded` enlazado a su entrada | `CatalogTreeSidebar`, `SchemaDetail`, `DatabaseDetail`, `AssetDetail`, `lib/server/assets.ts` |
| **S6g** | **Los agentes** | El asistente de la consola recibe el bloque (por S6d) y su política, que es la de una persona: con `degraded`, lo dice al responder; con `unknown`, dice de cuándo es el dato; y no da por bueno lo que no lo está. Lo mismo, escrito en la descripción de las herramientas MCP | consola (`components/assistant`), `mcp.rs` |

**S6a y S6c, hechos** (2026-10-14). `ore_core::salud::{propagar, en_vivo, sumar}` y, en `anotar_salud`, la salud de vistas, colecciones y tablas en vivo, más `salud` y `salud_por_schema` en cada paquete y la suma del árbol. `GET /health`, `/health/{base}[/{schema}]` y `/health/{kind:qname}`, del mismo índice. Lo que las copias vieron de una tabla no les vuelve como `upstream-degraded`.

**S6e, hecho** (2026-10-14). Medido al desplegar S6c: 14 colecciones `degraded` en `victor`, todas de
dos orígenes caídos (el S3 que no deja asumir su rol y el SFTP que ya no existe), y 15 activos más en
`warning` por ellas. Eran 14 fallos sueltos; ahora son un motivo del origen. Y al hacerlo salió
**un hueco de la pasarela**: todo fallo de un verbo simple (`change`, `versions`, `witness`…) se
contestaba `conexion`, fuera lo que fuera. Una tabla borrada habría sido «el origen no contesta».
- **La pasarela** respeta el código que el conector dejó en su última línea, tipada; lo que llega
  sin tipar es `origen`, como en v1.
- **Los lectores de objetos** tipan también sus verbos simples (`versiones`, `testigo`, `bajar`),
  con el mismo clasificador que `leer`.
- **La copia** lleva el código en el texto del error (`la pasarela contestó 502 (conexion): …`) y
  lo guarda en `intento.codigo`.
- **La salud**: con `conexion` o `credencial`, `source-unreachable` en vez de `refresh-failed`.
  `tiempo` no: una consulta lenta no es un origen caído. Una tabla en vivo lo hereda de sus copias
  (D-S6-1), y la fuente lleva `salud_del_origen` («2 of 3 copies could not reach this source»),
  también en `GET /health` y en `/health/{fuente}`. Si todas sus copias la alcanzaron, no se afirma
  nada.

**Medido (2026-10-14, `989b4962`).** Una pasada sobre el SFTP muerto: `sftp_s.media.audio` es
`source-unreachable` con `intento.codigo: conexion`; el objeto en vivo `sftp.media.audio` lo hereda
(`degraded`), y `/health/sftp` dice «1 of 7 copies could not reach this source» (las otras seis lo
dirán en su próxima pasada: sus punteros son de antes). Hizo falta un arreglo más: el índice
copiaba del puntero de una colección ocho campos fijos, sin `intento` ni `datos_de` (`e7808149`).
La vara, cuarta vuelta (`vara-20261014-1117.md`): 21 de 23; C12 en verde en los dos orígenes (una
tabla que no está no es un origen caído); C9 y C11 esperan la vista `pedidos_por_cliente` en `main`.

**S6f, la consola** (2026-10-14, local, sin empujar): la sección Health en el raíl de la base y del
schema con su suma y *Needs attention* (`f23a2c4`); la pestaña **Health** de un activo, que sustituye
a «Data quality», por capas —el ahora con la acción de cada motivo, Freshness, lo que mira cada
refresco, el linaje con su estado, y promesas, vigilancia y perfilado dichos como *Off*— (`bb6bce7`);
y la barra de pestañas de una ficha, rehecha como botones con la activa por nombre (`dd81724`).

**El orden y los commits.** Primero S6a y S6c juntos (ore-core y ore-serve: es lo que el resto
consume), después S6b, S6e y S6d, y al final S6f y S6g en la consola. Cada uno se compila con fmt,
clippy y las pruebas en Docker, y se mide desplegado.

**La vara (S0) crece.** Hace falta, una vez y desde la consola, una vista SQL sobre `vara_pg` (es
gobierno: el agente no puede crearla).

| Caso | Lo que la visión espera | Paso |
|---|---|---|
| C9 | La vista sobre una copia degradada (C6) está en `warning` con `upstream-degraded` y el `asset`, y su `dataAsOf` no es posterior al de la copia | S6a, S6c |
| C10 | `GET /health?scope=vara_pg` cuenta cada estado, con lo desconocido aparte | S6c |
| C11 | Una lectura de esa vista por SQL lleva el bloque propagado | S6d |
| C12 | Con la tabla borrada (C8), el motivo es `refresh-failed` y no `source-unreachable`: el origen contestó. El caso del origen que no contesta se mide a mano, porque cambiar la conexión de una fuente es gobierno | S6e |

**Lo que se mide además:**
- lo que cuesta propagar en cada `/assets` de `victor` (milisegundos, antes y después);
- que ningún bloque de un `Dataset` copiado cambie, salvo lo que añade el linaje.

**Las decisiones** (tomadas por el usuario el 2026-10-14: las tres, como se recomendaron, y `never-refreshed` en *Needs attention*)

- **D-S6-1 · Lo que se lee en vivo, en el catálogo.** Una `Table` o una vista sobre ella no tiene
  copia: no hay *data as of* hasta que alguien lee. **Recomendación:** en el catálogo, «Live» sin
  punto de color, salvo que la capa 0 sepa algo de ella. Por ejemplo, el esquema que vio una copia
  de esa tabla (`source-schema-changed`) o el origen que no contesta (S6e): entonces lleva ese
  motivo. Al leer, `dataAsOf` es la hora de la lectura. Pintarla `unknown` llenaría de gris todo
  lo que es en vivo por diseño, y `healthy` sin medida lo prohíbe §2.
- **D-S6-2 · Needs attention.** **Decidido:** `degraded`, `warning` y, de lo `unknown`, sólo `never-refreshed` (un dataset que nunca se copió pide que alguien haga algo). `unknown` va
  aparte, como «Unverified», con su propio filtro: §3.3 lo cuenta aparte, y mezclarlo ahogaría lo
  urgente en lo que sólo no está comprobado (los objetos, el SFTP y el JSONL no tienen señal todavía).
- **D-S6-3 · La salud del origen (S6e) entra en S6.** **Recomendación: sí**, porque sale de las
  copias y no cuesta nada. Lo que exige preguntar al origen entre pasadas sigue en la capa 1 (S10).

**Lo que S6 deja fuera:**
- **Que un build se pare** ante una entrada degradada: eso es S7, con las promesas `Blocking`.
- **Lo que vive en otra celda** (0059): una ref `rota` no propaga.
- **«Construido con una versión vieja de su entrada»** (la entrada se refrescó después del build):
  S6b lo hace honesto en `dataAsOf`, pero no lo nombra como motivo. Si hace falta, lo hará un motivo
  `x-` o una versión de la spec.

### S6 · cerrado (2026-10-14)

Lo que faltaba de S6, hecho:

- **S6d · el bloque con cada lectura** (`8e58f341`, `5997ee80`). Un ayudante de ore-serve,
  `salud_del_activo`, da la salud del índice anotado, con el linaje. La llevan:
  - la vista previa (lo leído en vivo, sin nada sabido, es de ese momento: `leido_en_vivo`);
  - `datos_en`, es decir el SQL y el Python del puesto y lo que calcula `ore-motor`;
  - una función de modelo encolada, con la salud de su `over`;
  - `POST /vistas/…/ejecutar`, que es `ore ask` servido;
  - y la cabecera de `ore ask` en local.

  El servidor MCP de `ore-cli` no sirve datos (`ontology_schema`, `ontology_describe`): no hay
  bloque que llevar. **Medido:** las cuatro vistas previas lo llevan, entre 1,3 y 5,3 s. La vista
  sobre la colección S3 caída sale `warning` con su raíz, y la tabla en vivo `healthy` con la hora
  de la lectura.
- **S6b · lo escrito sabe de cuándo es** (`377dc1b0`). El SDK anota el `dataAsOf` de cada cosa que
  lee y lo deja en la procedencia; al registrar, el puntero toma el más viejo, o `null` si de
  alguna entrada no se sabe. La CI cazó que la procedencia pasaba por `Json::de_node` y un `null`
  llegaba como la cadena `"null"`: ahora va por `de_node_fiel` (`f41631f0`). **Medido** con un
  puesto nuevo: `sandbox.vara_derivado`, escrito desde `vara_pg.ore_vara.pedidos`, lleva
  `datos_de: 2026-10-14T09:17:22Z` (la hora de su entrada, no la de la escritura) y sale `unknown`
  por `unverified` con esa hora. Una escritura idéntica es `repeated` y no toca el puntero: para
  medir hubo que cambiar el contenido.
- **S6g · el asistente** (consola `8449835`, local). `ejecutar_vista` le pasa la salud, en corto y
  antes de las filas, y el prompt del sistema lleva la política: `degraded` se dice al empezar,
  `warning` con su causa, `unknown` con su hora.
- **S6f · la consola** (local): ver «S6e, hecho».

### S7 · el diseño (rehecho, 2026-10-14)

Sustituye al primer diseño del mismo día. Decidido con el usuario: **D-S7-1** las promesas van en
el documento del activo; **la biblioteca es por familia de activo**; **las colecciones entran en
S7**; **D-S7-2** duplicados exactos con tope; **D-S7-3** el build no se para (S6a y S6b ya lo dicen
en la salida; esto deja sin efecto la última frase de D-F y el freno del build de la visión).

**Los precedentes.** Escribir la promesa en la definición del activo es lo de dbt (`tests` en el
YAML del modelo), Databricks DLT (`@dlt.expect…`), Foundry (Data Expectations sobre la salida, `FAIL`
/ `WARN`) y Snowflake (Data Metric Functions sobre la tabla). Lo aparte (Soda, Great Expectations,
Dataplex, un contrato ODCS) es para quien exige desde fuera: eso es S15 (gobierno), y por eso
usamos los nombres de métrica de ODCS, para que un contrato pueda nombrar las mismas promesas.

**Lo que hay.**
- La única promesa juzgada es `Dataset.freshness` (al leer, S2). `MediaCollection` no tiene.
- La pasada de una tabla cuenta filas y no nulos por columna (`ore-store`, `ciclo.rs`, sobre los
  lotes de Arrow). La de una colección escribe su manifiesto: por ítem, `camino`, `huella` (de
  contenido), `formato`, `tamano` y `estado` (`actual`, `retirado`, `perdido`).
- S4 retiene con una sola guarda fija, `zero-rows`, en `materializar.rs`.
- El `Ruleset` (v1alpha3) solo cuenta para la cobertura de gobierno: no se evalúa ni se ve.

**Los principios del diseño.**
1. **Solo lo que sale de la pasada.** Una promesa de S7 no lee un byte más del origen ni del lago:
   se mide sobre los lotes que ya pasan, o sobre el manifiesto que ya se escribe.
2. **Solo se mide lo prometido.** La copia pide a `ore-store` las medidas que sus promesas
   necesitan; una copia sin promesas no cambia ni un byte (ni de coste ni de puntero).
3. **Un solo juez.** Las promesas se evalúan en un sitio (`ore_core::promesas`), con las medidas
   como entrada, para la copia de una tabla, la de una colección y lo escrito con `write()`.
4. **Promesa sin pasada, error de compilación.** Lo que se lee en vivo (una `View` sin copia, una
   `Table` o `ObjectTable` de una base foránea) no tiene dónde comprobarse: eso es la capa 1.
5. **Lo que no se pudo medir es `unknown`, nunca `pass`.**

**La gramática (v1alpha31).** `spec.promises` en `Dataset` y en `MediaCollection`:

```yaml
kind: Dataset
metadata: { name: pedidos }
spec:
  freshness: 1h
  promises:
    - id: volumen
      metric: rowCount
      mustBeGreaterOrEqualTo: 900
      severity: error              # Blocking: si falla, la pasada no publica
    - id: sin-caidas
      metric: rowCountChange       # contra la versión publicada anterior
      unit: percent
      mustBeGreaterThan: -50
      severity: error
    - id: cliente-presente
      column: cliente
      metric: nullValues
      unit: percent
      mustBeLessOrEqualTo: 1
      severity: warning            # Warning: publica y lo anota
    - id: id-unico
      column: id
      metric: duplicateValues
      mustBe: 0
      severity: error
    - id: estado-valido
      column: estado
      metric: invalidValues
      arguments: { validValues: [pendiente, pagado, enviado, anulado] }
      mustBe: 0
      severity: warning
```

```yaml
kind: MediaCollection
metadata: { name: imagenes }
spec:
  promises:
    - id: solo-imagenes
      metric: invalidFormats
      arguments: { families: [image] }   # las familias de v1alpha29, o formatos concretos
      mustBe: 0
      severity: warning
    - id: sin-borrado-masivo
      metric: itemCountChange
      unit: percent
      mustBeGreaterThan: -20
      severity: error
```

Cada promesa: `id` único en su activo, `metric` de la biblioteca de su familia, `column` cuando la
métrica es de columna, `unit` (`rows` por defecto, o `percent`), `arguments` si la métrica los
pide, un solo comparador (`mustBe`, `mustNotBe`, `mustBeGreaterThan`, `mustBeGreaterOrEqualTo`,
`mustBeLessThan`, `mustBeLessOrEqualTo`, `mustBeBetween`, `mustNotBeBetween`) y `severity` (`error` = **Blocking**, `warning` =
**Warning**). `description` opcional, para la consola.

**La biblioteca, por familia.**

| Familia | Métrica | Qué cuenta | De dónde sale |
|---|---|---|---|
| tabular | `rowCount` | filas de la versión | ya se cuenta |
| tabular | `rowCountChange` | % de filas respecto de la versión publicada anterior | ya se cuenta (y el puntero anterior) |
| tabular | `nullValues` | nulos de `column` | ya se cuenta |
| tabular | `missingValues` | nulos y los valores de `arguments.missingValues` (`''`, `'N/A'`…) | un recorrido de la columna |
| tabular | `invalidValues` | filas fuera de `validValues`, o fuera de `arguments.between: [min, max]` | un recorrido de la columna |
| tabular | `duplicateValues` | filas cuyo valor (o la combinación de `columns`) ya apareció | memoria, exacto con tope (D-S7-2) |
| objetos | `itemCount` | ítems actuales | el manifiesto |
| objetos | `itemCountChange` | % de ítems actuales respecto de la transacción anterior | el manifiesto |
| objetos | `emptyItems` | ítems de 0 bytes | el manifiesto |
| objetos | `invalidFormats` | ítems fuera de `arguments.formats` o `arguments.families` | el manifiesto |
| objetos | `invalidSizes` | ítems fuera de `arguments.between: [min, max]` bytes | el manifiesto |
| objetos | `duplicateItems` | ítems actuales con la huella de otro (mismo contenido, otro camino) | el manifiesto |
| objetos | `lostItems` | ítems `perdido` (una virtual no puede servirlos) | el manifiesto |

Esquema y frescura no son métricas de la biblioteca: el esquema ya lo vigila S5 (`source-schema-
changed`) y la frescura es `freshness`, que sigue siendo la *Fresh within* (D-D).

**La guarda de S4 pasa a ser una promesa de serie.** `zero-rows` se convierte en la promesa
implícita `no-vaciarse` (`rowCount > 0` si la anterior tenía filas, `Blocking`), igual para
`itemCount` en las colecciones. La consola la enseña como *Built-in*. Quien la quiera más blanda la
declara con su `id` y `severity: warning`; no se puede quitar.

**La biblioteca es nuestra, y son tres cosas.** No se adopta Great Expectations ni Soda: los dos
releen el dato para medirlo, y aquí la medida va montada en la pasada. Construirla es:
1. **el catálogo** (S7a): qué significa cada métrica, de qué familia es, qué `arguments` y qué
   `unit` admite. Está en la spec, y el compilador lo hace cumplir;
2. **los medidores** (S7c y S7d): el código que cuenta cada métrica sobre los lotes de Arrow o
   sobre el manifiesto. Uno por métrica; añadir una métrica es añadir un medidor y su entrada en
   el catálogo;
3. **el juez** (S7b): compara lo medido con lo prometido, igual para todas.

**Las piezas**

| | Pieza | Qué | Dónde |
|---|---|---|---|
| **S7a** | **La gramática** | v1alpha31: `promises` en `Dataset` y `MediaCollection`, la biblioteca por familia y su esquema. El compilador dice: promesa en un activo sin pasada; métrica de otra familia; columna que el documento no tiene; `id` repetido; comparador ausente o doble. Recatalogar y *Accept the new schema* conservan `promises`; una columna que desaparece deja su promesa `unknown` y lo dice en la propuesta | `C:\oos` (v1alpha31), `ore-core` (compilar, diagnósticos), `ore-cli` (recatálogo) |
| **S7b** | **El juez** | `ore_core::promesas`: de las promesas, **las medidas que hacen falta**; de las medidas, el resultado de cada una (`pass`, `fail`, `unknown`) con lo medido y lo esperado. Puro y probado sin red | `ore-core` |
| **S7c** | **Las tablas** | La copia pasa a `ore-store` las medidas pedidas y las calcula sobre los lotes de Arrow que ya pasan (un `HashSet` con tope para `duplicateValues`; `ORE_PROMESAS_MEMORIA`, 256 MiB por defecto). `materializar.rs` juzga antes de publicar, por el camino de S4: una `Blocking` que falla retiene (`refresh-held`, `guard` = su `id`); una `Warning` publica y anota (`guard-warning`). El puntero lleva `promesas: [{id, resultado, medido, esperado, severidad}]` | `ore-store` (`ciclo.rs`), `ore-cli` (`materializar.rs`) |
| **S7d** | **Las colecciones** | Lo mismo en la transacción de una colección, mantenida o virtual, con las medidas del manifiesto. Retener es no mover el puntero a la transacción nueva | `ore-cli` (`coleccion.rs`) |
| **S7e** | **Lo escrito** | `write()` y los builds pasan por el mismo juez al registrar el puntero. Una `Blocking` que falla no publica, y el puesto recibe el error con la promesa, lo medido y lo esperado, y la salida *Publish anyway* | `ore-store`, `ore-cli` (`datasets.rs`), SDK |
| **S7f** | **La consola** | La sección *Promises* de la pestaña Health: cada promesa (las *Built-in* también) con su resultado, lo medido y lo esperado, y su historia en las últimas pasadas. *Add promise*: plantillas de lo que el puntero ya sabe («1 000 rows each refresh», «`cliente` 10 % empty», «only images in the last 3 refreshes»), que escriben el YAML en una rama y abren su propuesta (0044) | consola |

**Las decisiones (todas decididas el 2026-10-14)**
- **D-S7-4 · `no-vaciarse` de serie.** **Decidida:** sí, la guarda de S4 se expresa como una
  promesa más: visible, con historia, y ablandable a `warning`, nunca quitada.
- **D-S7-5 · Una promesa `unknown` no mueve el estado.** **Decidida:** se enseña en *Promises*
  con su motivo, pero el activo no pasa a `unknown` por ella: el estado lo dicen la frescura y lo
  medido. Si no, una columna enorme pondría en `unknown` un activo sano.
- **D-S7-6 · `write()` retenido.** **Decidida:** igual que la copia: la versión se escribe y no
  se publica, el puesto recibe el error con *Publish anyway*, y quien lee sigue en la anterior.

**La vara crece:**
- **C13:** `rowCount ≥ 900` (`Blocking`) sobre `vara_pg` retiene una pasada de 500 filas.
- **C14:** `nullValues(cliente) ≤ 1 %` (`Warning`) publica y lo anota (el 10 % es nulo).
- **C15:** un build sobre lo retenido construye sobre su última versión buena, y la salida sale
  `warning` · `upstream-degraded` con el `datos_de` de esa versión.
- **C16:** `invalidFormats` (`Warning`) en una colección viva, con un `.txt` entre imágenes.
- **C17:** `itemCountChange > -20 %` (`Blocking`) retiene una transacción que retira la mitad.
- **C18:** `write()` desde un puesto con `duplicateValues(id) = 0` (`Blocking`) y un `id` repetido:
  no publica, y el puesto lo dice.

**Lo que se mide además:**
- lo que cuesta medir por pasada (segundos y memoria) en `vara_pg` y en la copia más grande de
  `victor`, con todas las métricas y sin ninguna;
- que una copia sin promesas no cambie ni un byte de su puntero.

**El orden:** S7a → S7b → S7c → S7d → S7e → S7f. S7c ya cierra C13–C15; S7d, C16–C17; S7e, C18.

**Lo que S7 deja fuera:**
- **Las aserciones `sql`** y lo que cruza activos (integridad referencial): cuestan su consulta y
  van en S13, en la capa 2.
- **Las vistas en vivo y las bases foráneas:** sin pasada no hay dónde comprobar; capa 1 (S10).
- **Las sugerencias aprendidas y la deriva** (S12, S14): S7f solo propone plantillas de lo medido.
- **Los avisos al dueño** (S11) y el **`Ruleset` que exige desde fuera** (S15).
- **Importar y exportar ODCS:** los nombres ya cuadran; el traductor, después.

### S7a · el plan (propuesto, 2026-10-14)

**Lo que S7a deja hecho:** un `Dataset` o una `MediaCollection` pueden declarar `promises`, y el
compilador las acepta o dice exactamente qué está mal. Todavía no se mide ni se juzga nada (S7b–S7d):
una promesa bien escrita compila y no cambia ninguna pasada.

**Lo que ya hay y se aprovecha.**
- La forma de la aserción ya está en la spec: la `assertion` del `Ruleset` de v1alpha3 (perfil de
  ODCS v3.1), con `metric`, los operadores de ODCS (`mustBe`, `mustNotBe`, `mustBeGreaterThan`,
  `mustBeGreaterOrEqualTo`, `mustBeLessThan`, `mustBeLessOrEqualTo`, `mustBeBetween`,
  `mustNotBeBetween`), `unit: rows | percent` y `severity: error | warning`. La promesa es esa
  aserción, de tipo `library`, con lo que le falta: `column`, `arguments` y las métricas nuevas.
- Las claves de `spec` van por versión del documento (`document.rs`): basta con
  `Kind::Dataset if version >= V1Alpha31` y lo mismo en `MediaCollection`. Un documento anterior
  que escriba `promises` da `OOS1005` sin código nuevo.
- `View`, `Table` y `ObjectTable` no admiten la clave, así que **«promesa sin pasada» es `OOS1005`**
  (con el remedio en el mensaje: va en el `Dataset` o la colección que lo copia).
- *Accept the new schema* (`esquema.rs`) solo reescribe el bloque `fields`: `promises` se conserva
  sola, y una columna que se va sale en los diagnósticos de la propuesta (paso 5) con `OOS2052`.
- Subir un `Dataset` o una `MediaCollection` de v1alpha12–19 a v1alpha31 es neutro: entre medias
  solo cambian `media` (v1alpha29, que se amplía) y nada del `Dataset`.

**Los pasos.** Cada uno se compila y se prueba (fmt, clippy `-D warnings`, tests) y se empuja solo.

| Paso | Qué | Dónde | Hecho cuando |
|---|---|---|---|
| **S7a·1** | **La spec v1alpha31.** `00-scope` y `01-las-promesas` (gramática, biblioteca por familia, `no-vaciarse`, la puerta del documento, lo que no se comprueba en S7). `promise.schema.json` (la aserción de v1alpha3 + `column`, `arguments`, métricas por familia), `dataset.schema.json` y `media-collection.schema.json` de v1alpha31. Dos filas en `99-errors`. Casos: 4 válidos (dataset copiado, dataset escrito, colección, `no-vaciarse` ablandada) y 7 inválidos (antes de v1alpha31, en una `View`, métrica de otra familia, sin comparador, dos comparadores, columna que no existe, `id` repetido) | `C:\oos` | los casos escritos y el commit en `oos` |
| **S7a·2** | **El catálogo y el compilador.** `ApiVersion::V1Alpha31`; `promises` en las claves de `Dataset` y `MediaCollection`; las listas nuevas en `normalize` (`promises` secuencia; `validValues`, `missingValues`, `formats`, `families` conjuntos; `mustBeBetween` y `between` secuencias). `ore_core::promesas`: la **biblioteca** (métrica → familia, si lleva `column`, qué `arguments` y qué `unit` admite) y `validar`, que emite `OOS1006` (promesa mal formada, con el porqué) y `OOS2052` (columna que el activo no declara: `fields` en una copia, `columns` en uno escrito; si no declara ninguna, no se comprueba aquí y será `unknown` al medir) | `ore-core`, submódulo `vendor/oos` | los casos de conformidad de v1alpha31 pasan en `ore-core` |
| **S7a·3** | **Que nada lo pierda.** Pruebas: subir la versión es neutro (mismos diagnósticos antes y después); el registro de `write()` conserva `promises` y el `apiVersion` al reescribir `columns`; *Accept the new schema* conserva `promises` y una columna quitada sale como `OOS2052`; el índice (`/assets`) lleva las promesas declaradas de cada activo (`promesas_declaradas`), para que la consola las enseñe antes de S7b | `ore-cli` (`datasets.rs`), `ore-serve` (`esquema.rs`, `assets.rs`) | las pruebas en verde y desplegado (`ore.dev/commit`) |

**Por defecto, salvo que se diga otra cosa:**
- Los nombres son los de ODCS tal cual (`mustBeGreaterOrEqualTo`, `unit: rows | percent`); las
  métricas de objetos y `rowCountChange` son nuestras, porque ODCS no las tiene.
- `duplicateValues` es de **una** columna en S7. La clave compuesta (`id` + `linea`) llega después:
  `columns` ya es un mapa en el `Dataset`, y el nombre de la lista hay que pensarlo.
- `invalidValues` con `pattern` (regex) se deja fuera de S7: cuesta por fila y no lo pide nadie aún.
- La **puerta es del documento**: para escribir `promises` el documento sube a v1alpha31. La consola
  (S7f) lo sube sola al añadir la primera; el inductor sigue emitiendo lo de hoy.

**Lo que se mide.** Nada en producción todavía: S7a no cambia ninguna pasada. Se comprueba que el
árbol de `victor` compila igual con el binario nuevo (sin diagnósticos nuevos) y que un `Dataset` de
la vara con promesas en una rama compila sin ruido.

**Orden y tiempo:** S7a·1 → S7a·2 → S7a·3, cada uno en su commit. S7a·2 es el grueso; S7a·1 y S7a·3,
cortos.

### S2 y S3 · medidos (2026-10-13, en `victor`, con `e1005d8d` desplegado)

Pasadas con la plantilla de la copia de `main` (`pruebas-de-fuego/salud/pasada.py`), sólo sobre
los datasets dados.

**S2: un refresco que falla deja lo último bueno.** Un rehacer de `sftp_s.tablas.us_states`, cuyo
SFTP ya no existe. La pasada falla de verdad (`35.241.131.133:22` no contesta) y el puntero queda en
`error` **con** su `metadata_location` y sus 50 filas, y con `intento {resultado: error, motivo}`.
Antes de S2 ese puntero se quedaba sin versión y el dataset dejaba de leerse (M5). Falta el lado de
quien lee: SQL con el aviso y la ficha en *Degraded*.

**S3: la señal decide.** Dos copias de BigQuery (`bq.ventas.pedidos`, `bq.ventas.clientes`) y dos de
Postgres (`standard_test.public.products`, `…prueba_de_pk`), todas con testigo `none`:

| Pasada | Qué cambió en el origen | Lo que hizo la copia | Puntero |
|---|---|---|---|
| 1 | nada | las cuatro **copiadas**: «sin señal de la copia anterior» | `senal` guardada (`modificada` en BigQuery, `contadores` en Postgres), `datos_de` |
| 2 | nada | las cuatro **«ya está» sin leer una fila** | `comprobado` posterior a `datos_de`, `intento: sin-cambios`; `datos_de` no se mueve |
| 3 | `INSERT` de una fila en `ventas.pedidos` | **sólo `pedidos` copiada** (9 filas): «la tabla cambió» | `senal` nueva; las otras tres, comprobadas otra vez |
| 4 | `DELETE` de esa fila | **sólo `pedidos` copiada** (8 filas) | lo que `field` no ve (M2) lo ve la señal |

Antes de S3, con `none`, las cuatro decían «al día» sin preguntar, y la pasada 3 no habría copiado
nada. Cada pasada, 30–38 s de Job. La tabla de prueba quedó como estaba.

**Lo que la medida sacó a la luz, fuera de S2 y S3:**
- **Un paquete que no compila hace que `ore view` salga con 65.** `azure_blob_mnist` (2026-10-12)
  declara la columna `"0"` 784 veces: `ore discover` tomó como cabecera la primera fila del CSV de
  MNIST, que son píxeles (OOS1001). La plantilla de `main` lo tolera desde 0053 (`beed5b7b`); los
  Jobs de rama y de rehacer encolados antes de eso, no.
- **La cola repite lo viejo.** Los Jobs de la cola caducan a las 24 h (`ttlSecondsAfterFinished`)
  y Flux los vuelve a crear desde su fichero en `trabajo.git`. El 13 de octubre, a las 15:31 y
  15:42, volvieron a correr siete *rehacer* del 5 y 6 de octubre, con el guion de entonces, y
  fallaron. Un rehacer repetido copia entero sin que nadie lo haya pedido otra vez. Es la avería 1
  de M7 al revés: trabajo que nadie pidió, y nadie lo ve.

### Por qué este orden

- **La capa 0 primero, porque es lo que hoy se incumple.** S2–S5 arreglan, con lo que ORE ya sabe en la
  pasada, todo lo que M5–M8 sacaron a la luz. Ninguno necesita un interruptor, ni el vigilante, ni la
  tabla de salud.
- **S6 junta la capa 0 en un estado** cuando ya hay algo verdadero que juzgar.
- **S7 y S8 cierran la capa 0**: lo prometido que se comprueba al copiar, y el refresco que cumple lo
  prometido. S8 es la primera pieza de la malla, y lleva su go.
- **La capa 1 reutiliza la pasada de S8** y añade lo que cuesta tocar el origen entre pasadas. Por eso
  va detrás, y sólo donde se enciende.
- **La capa 2 y el gobierno** valen poco hasta que lo de debajo diga la verdad y tenga historia.

### Lo que se mide en cada paso

- La vara (S0) sobre los orígenes de prueba, en producción.
- Cuánto cuesta lo nuevo, **por capa**: segundos por pasada, bytes facturados por pregunta en
  BigQuery, y conexiones y WAL en Postgres. Lo de la capa 0 no puede costar más que la copia de hoy;
  lo de las capas 1 y 2 se dice en su interruptor.
- Que nada de lo que hoy funciona cambie. Lo que ya dice la verdad (M4: el informe cuando copia) tiene
  que seguir diciéndola, byte a byte.
