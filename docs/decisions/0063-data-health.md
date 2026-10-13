# 0063 · Data Health — la salud, la frescura y la calidad de los activos del catálogo

**Estado:** propuesto (2026-10-13). **H0 hecho**: el estado del arte, investigado el 2026-10-13, y la
naturaleza del producto que se deriva de él. **H1–H9 por decidir y construir**: no hay una línea de
código escrita.

## Contexto

Al cerrar el origen PostgreSQL (PG1) salió un fallo que no es de Postgres. **Una copia sin testigo no
se refresca nunca.** `al_dia_con` (`materializar.rs:837`) compara la cabecera, que incluye el
testigo. Si no hay testigo, la cabecera no cambia, y la copia contesta «ya está» para siempre. Lo
mismo pasa con una réplica sin `wal_level=logical`, con un BigQuery sin cursor y con cualquier
`witness: none`. Y aunque lo hubiera, **nadie pasaría a mirar**: la copia es un Job que Flux crea
cuando cambia la lista de datasets, no un CronJob (`malla/48-la-copia.yaml:25`, «la pasada
siguiente»).

Tirando de ese hilo, lo que aparece es un producto que no existe: la **salud de los datos**. ORE
tiene casi todas sus piezas, pero sueltas, y lo que no tiene es justo lo que las une. Lo midió el
mapa del 2026-10-13:

| Pieza | Dónde | Qué hace hoy |
|---|---|---|
| `Dataset.freshness`, el *target lag* | spec v1alpha12 `01-dataset.md:102` | **Se declara y nada lo mide.** Ningún reloj la compara: ningún dataset sale degradado jamás |
| «Superado, el dataset está degradado y **NO DEBE** servirse como fresco» | spec v1alpha1 `03-binding.md:224` | **ORE lo incumple**: no hay estado degradado en ninguna vía de lectura |
| `degradado` en el código | `registro::frescura_comprobable` (`registro.rs:276`), `ore view` | Significa otra cosa: «la frescura **no se puede comprobar**» (`witness: none`), no «se ha pasado» |
| El testigo (`none · snapshot · log · field · listing`) | ADR 0016, 0015; spec v1alpha8, v1alpha16 | Dice hasta dónde llega una copia. Es la mejor señal de frescura que existe, y sólo se usa para decir «ya está» |
| El informe en el puntero (`datasets/**.json`) | `materializar.rs::informe_de` (:1059) | `estado` (`copiada · al-dia · error`), `motivo`, `filas`, `leidas`, `bytes`, **`columnas`** (no nulos por columna), `esquema_cambiado`, `testigo`. **Cuándo** sale del commit, no del fichero |
| Una pasada que falla no borra el puntero | ADR 0031 §10 (:480) | Conserva lo que había y pone `estado: error` encima: **ya es *write-audit-publish*** |
| «impone nunca nula» | ADR 0051; `ore-store::carga::sin_nulos` | La **única** comprobación que se ejecuta sobre los datos, y la única que niega una copia |
| `Ruleset.assertions` (perfil de calidad de ODCS) | spec v1alpha3 `02-ruleset.md` §3 | Se compilan, se comparan entre versiones y se cuentan (OOS8001). **No se ejecutan nunca.** Sus `targets` sólo eligen **propiedades**, ni un dataset ni una columna |
| «El dataset no lleva calidad: un `Ruleset` lo nombra» | spec v1alpha12 `01-dataset.md:35`; ADR 0033:127 | **Un `Ruleset` no puede nombrar un dataset.** Es un hueco de la spec |
| `schedule` y `scheduler` fuera de la spec | `02-ruleset.md:276` | «Ese motor es Soda, Great Expectations o dbt». No hay motor |
| Las estadísticas del catálogo | ADR 0062 | Filas contadas o estimadas de un origen, sin escanear |
| La actividad | `ore-serve::actividad` | Actos de escritura de los usuarios. El resultado de una copia no está |
| El mantenimiento diario | `malla/53-el-mantenimiento.yaml` | Recoge snapshots viejos. No refresca nada |

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

### Lo que comparten (y de donde sale lo nuestro)

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

## Lo que ORE tiene y los demás no

Antes de derivar nada, lo que hace distinto el problema en ORE.

- **El testigo.** Todos miden la frescura con un reloj: la última escritura, o un `MAX(ts)`. ORE
  sabe **hasta dónde llega** una copia en la versión del origen: un LSN, un snapshot, un cursor.
  Eso resuelve lo que la industria no resuelve, porque un testigo comprobado igual al de la copia
  quiere decir que **la copia está al día a esa hora**, aunque se escribiera hace una semana. Si el
  testigo ha avanzado y la copia no, **la copia se ha quedado atrás**. Si el testigo no se mueve, **es
  el origen el que no cambia**.
- **El puntero es la publicación.** Una copia se publica con un commit del puntero, y si falla no se
  publica (0031 §10). **ORE ya es *write-audit-publish*** sin ramas de Iceberg. Sólo le falta
  auditar algo más que los nulos.
- **El informe ya mide al escribir.** `filas`, `bytes`, `columnas` (no nulos por columna, que es la
  completitud) y `esquema_cambiado` salen gratis de cada copia, como el `record_count` de Iceberg. Es
  el `DataQualityMetrics` de OpenLineage antes de tenerlo.
- **La spec es la declaración.** Todo activo es un documento en un árbol con dueño (0052) y con
  versiones (0056). Lo que en Foundry es una regla de UI aquí es un documento que se revisa, se
  propone (0044) y se diferencia.
- **El catálogo es de todos** (0059) y lo leen también **los agentes**. Es la vía para hacer lo que
  nadie tiene: que un agente sepa que un activo está degradado y **se abstenga o lo diga**.
- **Las bases foráneas leen en vivo.** Un activo de una foreign database **no tiene copia**: su
  frescura es la del origen, y su calidad, la del origen. No se puede auditar al escribir, porque
  ORE no escribe.

## La naturaleza del producto

**ORE Data Health** es el producto que **declara** lo que se espera de cada activo del catálogo,
lo **mide** de la fuente más barata que sea verdad, **juzga** un estado por activo, y **hace algo con
él**: no publicar, servir avisando, avisar a su dueño, o decírselo a un agente.

Son cuatro capas, y cada una tiene una casa distinta:

```
 DECLARAR            MEDIR                       JUZGAR                     REACCIONAR
 (la spec)           (al escribir y al vencer)   (calculado, nunca guardado) (según quién lee)
 ───────────         ──────────────────────      ──────────────────────     ───────────────────
 freshness       ─▶  testigo comprobado      ─▶  healthy                ─▶  copia: no publicar (error)
 Ruleset.assert. ─▶  informe de la copia     ─▶  warning                ─▶  lectura: servir avisando
 (ODCS quality)      aserciones al copiar        degraded                   build: no construir encima
                     resultados (OpenLineage)    unknown                    dueño: aviso
                                                 ↓ baja por el linaje       agente: abstenerse o decirlo
```

### Lo que se propone decidir

**H-D1 · Medir no es juzgar, y el estado no se guarda: se calcula.** Lo medido se guarda: el
informe de cada copia y cada comprobación. El estado de un activo es una función de lo medido, de lo
declarado y de la hora. Se calcula al pedirlo, como la versión de 0056, y nunca se escribe como
verdad. Así no puede desincronizarse, y cambiar una expectativa recalifica la historia sin
reescribirla.

**H-D2 · Cuatro estados, el peor gana.** `healthy · warning · degraded · unknown`, como la salud
de Dagster:
- `unknown`, si no hay nada declarado o nada medido. **Nunca cuenta como sano** y la consola lo
  pinta gris.
- Por cada comprobación: `pass · warn · fail · error · skipped`. **`error` (no pudo correr) no es
  `fail`.**
- `degraded` también sustituye en el código al «degradado» de hoy, que significa «no comprobable» y
  pasa a ser `unknown`. La palabra queda para lo que dice la spec.

**H-D3 · La frescura se mide con el testigo, no con el reloj de escritura.** De una copia se
guardan dos tiempos: cuándo se tomó su testigo (`copiada_en`) y cuándo se comprobó por última vez
que seguía igual (`comprobada_en`). Su **edad** es `ahora − max(copiada_en, comprobada_en)`.
- **Con testigo**: comprobarlo es barato (el verbo `testigo` de 0016). Si es igual, la copia está
  al día **a esa hora**, sin copiar nada. Si no, se copia.
- **Sin testigo** (`none`, una réplica sin `wal_level=logical`, BigQuery sin cursor): **la única
  manera de rejuvenecerla es volver a copiarla entera**, y la edad es la de la última copia. Lo
  dice el estado: «copiada hace 40 min, sin testigo». Y «ya está» deja de ser posible. Esto es PG1.
- **Las dos señales, separadas.** «La copia va atrás» (el testigo ha avanzado: es nuestro, de ORE)
  y «el origen no cambia» (el testigo no se mueve: es del dueño del origen). Son distintas, con
  dueños distintos, y ORE las ve las dos.

**H-D4 · `freshness` es un objetivo que el vigilante intenta cumplir**, como `TARGET_LAG`, y la
spec ya lo dice así.
- Un dataset con `freshness: 15m` se comprueba, y se recopia si hace falta, para no pasar de 15
  minutos. Pasado el plazo, está `degraded`.
- **Sin `freshness` no hay promesa**: no hay refresco automático y el estado de frescura es
  `unknown`, con la edad a la vista. Rehacer a mano siempre funciona.
- Así **el coste lo declara quien lo paga**. Un BigQuery sin cursor con `freshness: 1h` es un
  escaneo entero por hora, y el compilador lo avisa al declararlo.

**H-D5 · Lo derivado hereda la frescura de sus raíces.** La de una `View` o un dataset escrito por
un `Transform` es la de su raíz más vieja, medida contra el origen, como Snowflake. Un activo con una
raíz `degraded` sale `degraded` con el motivo «aguas arriba: `x`». Y el linaje da también el radio:
qué se rompe por debajo.

**H-D6 · La calidad se declara en un `Ruleset`, y el `Ruleset` aprende a nombrar activos.** La
spec ya decidió que el dataset no lleva calidad y que la lleva un `Ruleset` (OOS1005). Lo que falta
es que un `Ruleset` pueda apuntar a algo más que propiedades:
- `targets` gana activos: por nombre (`datasets`, `tables`), **por ámbito** (todo un schema, todo un
  paquete, todo lo de una fuente), y columnas.
- El ámbito es lo que hace el producto escalable. Las *monitoring views* de Foundry y el «un clic por
  schema» de Databricks lo enseñan: **lo que entra en el ámbito queda cubierto solo**.
- Las aserciones siguen siendo el perfil de ODCS (`library · sql · text · custom`, `mustBe*`,
  `dimension`).
- **`severity` se cierra a `warning | error`**, porque ODCS lo deja libre y sin eso no se puede
  decidir qué bloquea.
- Una aserción puede llevar un umbral de aviso y otro de error, como el `additional:` de Soda.
- **Se importa y exporta ODCS**: es el formato de intercambio, no el nuestro.

**H-D7 · Nadie declara cuándo: se comprueba cuando cambia, y la frescura cuando vence.** La spec
deja fuera `schedule` y `scheduler`, y sigue fuera. Las aserciones corren **en cada copia o
escritura** (como `TRIGGER_ON_CHANGES`) y la frescura **al escribir y al vencer el plazo** (como el
modo automático de Foundry). La cadencia se deriva y no se configura. Lo que la spec decía, «ese
motor es Soda, GX o dbt», cambia: **el motor es ORE**, y la spec sigue sin nombrarlo.

**H-D8 · Medir de lo más barato que sea verdad** (como 0062), en tres niveles:
1. **Gratis**: lo que el informe de la copia ya mide (`filas`, `bytes`, nulos por columna, esquema),
   el testigo, y las estadísticas del catálogo. Cubre volumen, completitud, esquema y frescura de
   **todo**, sin declarar nada.
2. **Al escribir**: las aserciones de biblioteca se calculan sobre los lotes de la copia, como hoy
   `sin_nulos` (nulos, valores válidos, rangos, filas). No se lee un byte más.
3. **Después de escribir, antes de publicar**: las aserciones `sql`, sobre el snapshot nuevo, que
   todavía no tiene puntero. Esto cuesta, y lo declara quien lo quiere.

Leer un **origen en vivo** para comprobarlo (lo de una foreign database) gasta en la cuenta del
cliente. **Nunca por defecto.**

**H-D9 · Reaccionar según quién lee.**

| Quién | Ante `error` | Ante `degraded` (frescura) | Ante `warning` |
|---|---|---|---|
| **La copia** | **No se publica**: el puntero se queda con lo anterior y lleva el motivo encima (lo de 0051, generalizado) | Recopia si hay `freshness` | Publica y lo deja anotado |
| **Una lectura** (SQL, Python, Java, la vista previa) | Lee lo publicado, que es lo bueno | **Sirve avisando**, como pide la spec: un aviso en el resultado, nunca «fresco» | Sirve; el estado va en los metadatos |
| **Un build** (0055) | No construye encima de una entrada en `error` | Construye y la salida hereda `degraded` (H-D5); `--estricto` lo niega | Construye |
| **Un agente** | Lo dice | **Se abstiene o lo dice**: el estado viaja con el resultado (MCP, SQL) | Lo dice |
| **El dueño** (0052) | Aviso | Aviso al pasar a `degraded` | Sólo en la consola |

Se avisa **cuando el estado cambia**, nunca por cada comprobación.

**H-D10 · Dónde vive lo medido.**
- **Lo que se mide al escribir va en el puntero**, en el mismo commit que publica la copia:
  `copiada_en` y los resultados de sus aserciones. Su historia es el `git log` del puntero, como hoy
  (0031). No hace commits nuevos, porque sólo cambia cuando cambian los datos.
- **Lo que se mide por el tiempo**, las comprobaciones del testigo y las aserciones en vivo, **no
  va al árbol**: serían commits cada hora por dataset. Va a una **tabla del lago de la celda**,
  append-only, con la forma de `DataQualityAssertions` y `DataQualityMetrics` de OpenLineage. Es la
  tabla del sistema de Databricks o de Snowflake: se consulta por SQL, y sobre ella se calculan
  después los umbrales dinámicos.

**H-D11 · Se sirve por un sitio.** `ore-serve` calcula y sirve la salud:
- `GET /health`: por activo, schema, base y celda, cada estado con sus porqués.
- `GET /assets` lleva `salud` en cada activo.
- La consola pinta el estado (gris si `unknown`) en el explorador, en el linaje y en una pestaña
  **Health** por activo, con su historia.
- El MCP de los agentes lo devuelve con cada activo y cada resultado.

### Lo que no es (todavía)

- **Anomalías aprendidas** (umbrales por ML, estacionalidad). Necesitan semanas de historia, y la
  tabla de H-D10 es la que la guarda. Primero lo explícito; lo aprendido, sobre lo guardado.
  Databricks y Monte Carlo lo hicieron al revés porque no tenían spec.
- **Perfiles** de distribución, cuantiles y deriva.
- **Incidencias** con ciclo de vida (`New → Ack → Resolved`), asignación y causa raíz. Al principio,
  avisos al dueño y el estado.
- **Reconciliación** entre orígenes (`DatasetMatch`).
- **Puntuaciones** (% de reglas que pasan). Esconden lo suprimido y lo desconocido (el «100» de
  Collibra); si llegan, al lado de los estados y no en su lugar.

## El plan

| | Qué | Toca |
|---|---|---|
| **H0** | El estado del arte y la naturaleza del producto (este documento) | — · **hecho** |
| **H1** | **La forma en la spec** (v1alpha30): los `targets` de activos y ámbitos del `Ruleset`, `severity` cerrado, los cuatro estados y sus obligaciones (qué es `degraded`, qué es `unknown`, qué se debe al consumidor), la frescura medida por el testigo, el aviso de coste de `freshness` sin testigo. Con sus códigos OOS | `C:\oos`, submódulo |
| **H2** | **Medir la frescura** (y arreglar PG1): `copiada_en` en el informe; sin testigo, «ya está» es imposible y rehacer siempre copia; `unknown` en vez del «degradado» de hoy | `materializar.rs`, `registro.rs` |
| **H3** | **El vigilante**: la pasada periódica por celda. Comprueba testigos, recopia lo que tiene `freshness` y va a pasarse, y escribe lo medido en la tabla de salud | malla (**go aparte**), `ore-cli` |
| **H4** | **Juzgar y servir**: el estado calculado, la herencia por el linaje, `GET /health`, `salud` en `/assets` | `ore-core`, `ore-serve` |
| **H5** | **Las aserciones al copiar**: las de biblioteca sobre los lotes. `error` no publica (generaliza `sin_nulos`), `warning` publica anotado | `ore-store`, `materializar.rs` |
| **H6** | **Las aserciones `sql`** sobre el snapshot nuevo, antes de mover el puntero | `ore-store`, `ore-motor` |
| **H7** | **La consola**: el estado en el explorador y en el linaje, la pestaña Health, la vista por base y schema | `rubix-platform` |
| **H8** | **Los que leen**: el aviso en SQL, Python y Java; un build que no construye sobre un `error`; el estado en el MCP de los agentes | `ore-serve`, SDK, 0055 |
| **H9** | **Los avisos** al dueño cuando el estado cambia | `ore-serve`, `ore-iam` |

Cada etapa se mide antes de escribirse y tiene su go. H1 y H2 se pueden empezar ya, y H2 es la que
cierra PG1.

## Preguntas abiertas (para decidir antes de H1)

1. **Sin `freshness`, ¿no hay refresco automático (H-D4)?** Es lo coherente con la spec y con el
   coste. La alternativa es una frescura por defecto de la celda, por ejemplo una hora, que
   refresque también lo que nadie declaró: es más cómodo, pero cada copia sin testigo pasaría a ser
   un escaneo entero por pasada sin que nadie lo haya pedido.
2. **¿El `Ruleset` aprende a nombrar activos (H-D6), o la calidad entra en el propio activo?** La
   spec decidió lo primero (OOS1005) y ODCS lo pone en el contrato. Lo recomendado es mantenerlo: un
   `Ruleset` por ámbito escala, y la calidad la puede gobernar alguien que no es el dueño del dato.
3. **La tabla de salud de H-D10, ¿en el lago de la celda o en `ore-iam`?** Lo recomendado es el
   lago: se consulta por SQL como cualquier dataset, es de la celda y no del plano, y es el sitio
   natural de la historia.
4. **Un build sobre una entrada `degraded`, ¿construye con aviso o se niega (H-D9)?** Lo recomendado
   es construir y heredar, con `--estricto` para negar: negar por defecto paraliza un pipeline
   entero porque un origen va tarde.

## Lo que se acepta a cambio

- **Un motor más que mantener.** La spec decía que el motor de calidad era de otro (Soda, GX, dbt).
  Ahora es de ORE, con todo lo que eso trae: el vigilante, la tabla de salud y su retención.
- **Una frescura sin testigo cuesta copias enteras.** H-D4 lo hace explícito y lo carga a quien
  declara `freshness`. No lo esconde, pero tampoco lo evita.
- **Lo que se mide en el tiempo no está en el árbol.** El árbol deja de ser el único sitio de la
  verdad sobre un activo: lo que se publica sigue allí, y lo que se observa va al lago. Es el precio
  de no hacer un commit por hora y por dataset.
- **Lo aprendido llega tarde.** Sin anomalías, quien no declara nada sólo tiene la cobertura
  gratuita de H-D8 nivel 1: volumen, nulos, esquema y edad, sin umbrales.
