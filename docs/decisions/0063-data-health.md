# 0063 · Data Health — la salud, la frescura y la calidad de los activos del catálogo

**Estado:** propuesto (2026-10-13). Se construye en este orden:

1. **Los fundamentos** (escritos): qué son la frescura y la calidad, de la literatura y la evidencia
   empírica, sin mirar ORE.
2. **Las buenas prácticas** (escritas): cómo lo resuelve la industria.
3. **ORE medido contra los fundamentos** (M1–M8, **medidos** el 2026-10-13): qué hay de verdad en nuestro stack,
   medido en producción. **Nada de lo que creíamos saber se da por bueno**; lo que salga de aquí
   es lo único que cuenta.
4. **El producto**: qué construir, cómo y dónde. Sale de cruzar 1 y 2 con lo que 3 demuestre que
   falta, está incompleto o está mal. **La visión está escrita** (2026-10-13); falta bajarla a piezas.

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
4. **Vigilancia de base gratis; promesas encima.** Todo activo tiene desde el primer día su
   frescura, su volumen, su esquema y sus nulos vigilados, sin configurar nada (F2: las averías 1–4
   salen de lo que ya se mide al copiar). Sobre eso, el dueño **promete** lo que importa.
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

### El vocabulario

| Concepto | Qué es para quien lo ve |
|---|---|
| **Health** | El estado del activo: **Healthy** (verde), **Warning** (ámbar), **Degraded** (rojo), **Unknown** (gris). Gana el peor de sus señales, y baja por el linaje: un activo con una raíz degradada lo dice |
| **Data as of** | La hora del mundo que refleja: cuándo se leyó el origen (τ_lectura, F1). Es la hora principal, nunca la de un commit (M1) |
| **Checked** | Cuándo se comprobó por última vez que seguía igual. «Data as of 09:00 · checked 2 min ago · source unchanged since» es edad 0 (F1: retraso ≠ edad) |
| **Last good version** | La versión que se sirve mientras la nueva no llega o no pasa |
| **Promise** | Lo que el dueño promete del activo. Tipos: *Fresh within* (la tolerancia de F1), *Schema* (las columnas y sus tipos), *Volume* (filas esperadas), *Valid* (no nulo, único, rango, valores admitidos, una consulta SQL). Cada una, **Blocking** o **Warning** |
| **Monitor** | Lo que se vigila sin que nadie lo pida: la pasada (si corrió y si acabó), el esquema del origen, el volumen, los nulos, el origen (si contesta, si la credencial vale) |
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

**5 · Quien es dueño: la bandeja de Data Health.**
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

**9 · Cuando no se puede saber.** Un origen sin señal de cambio (`none`) no se disfraza:
- Sin promesa, la copia dice **«Unverified · data as of Oct 2 (11 days ago)»**, en gris.
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

## Lo que sigue

La visión está escrita. Lo siguiente es **bajarla a piezas**:
- qué vive en la spec (el vocabulario, las promesas y lo que se le debe al consumidor);
- qué vive en ORE (medir, retener, juzgar, servir y avisar);
- qué vive en la consola;
- en qué orden, empezando por lo que hoy hace más daño: **apagarse** (M5) y **decir «al-dia» sin
  comprobar** (M6).

Cada etapa con su plan, su go y su medida.
