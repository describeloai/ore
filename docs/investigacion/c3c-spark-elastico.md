# ORE 0068 C3·c · Spark elástico en GKE: lo que hay que saber antes de diseñar (2026-10-19)

El primer diseño de C3·c (un Job de N pods fijo, con N decidido por el historial del Transform) era
reactivo: un Build tenía que fallar o tardar para que el siguiente llevara más nodos. Esto reúne lo que
hace la industria, lo que dan Spark, Kueue y GKE, y lo que tiene victor. Las fuentes van enlazadas. Lo
que es deducción va como **[inferencia]**, y lo que nadie ha comprobado, como **[sin verificar]**.

## 1 · Lo que tiene victor

| qué | valor |
|---|---|
| Kubernetes | 1.35.8-gke (Dataplane V2: las NetworkPolicy siempre se aplican) |
| Kueue | v0.19.3, sin *feature gates* propios; integraciones `batch/job`, `pod`, `jobset`, `statefulset`… (`sparkapplication` comentada) |
| pool `pesado` | `n2-highmem-8` *spot*, 0–2 nodos, `locationPolicy ANY`, *image streaming* |
| autoescalador | perfil `BALANCED` |
| arranque en frío medido (6c) | nodo listo a los **46 s**, la imagen en **1,8 s** con *streaming*, la sesión de Spark 16 s |
| cuota | CPUS_ALL_REGIONS 32; el pesado lleno son 16 |
| cuota del inquilino | `ResourceQuota` 12 CPU / 64 GiB |

## 2 · Cómo escala la industria: por carga, no por historial

Todas usan el **dynamic allocation** de Spark, o una señal equivalente de recursos pendientes:

| plataforma | señal | detalle |
|---|---|---|
| Databricks (*optimized autoscaling*) | carga y estado del *shuffle* | de mínimo a máximo en ≤ 2 pasos; baja tras 40 s infrautilizado en jobs ([doc](https://docs.databricks.com/aws/en/compute/configure#autoscaling)) |
| Dataproc (clúster) | memoria y cores **pendientes** de YARN | enfriamiento de 2 min ([doc](https://docs.cloud.google.com/dataproc/docs/concepts/configuring-clusters/autoscaling)) |
| Dataproc Serverless | dynamic allocation de Spark | min 2, max 1000, `executorAllocationRatio 0.3`; v2 elige qué retirar por tamaño del *shuffle* ([doc](https://docs.cloud.google.com/dataproc-serverless/docs/concepts/autoscaling)) |
| EMR (*managed scaling*) | métricas de YARN, consciente del *shuffle* | no retira nodos con *shuffle* activo ([doc](https://docs.aws.amazon.com/emr/latest/ManagementGuide/emr-managed-scaling.html)) |
| EMR Serverless | dynamic allocation | 20 contenedores por lote cada 1 s; ejecutores de 4 cores / 14 G ([doc](https://docs.aws.amazon.com/emr/latest/EMR-Serverless-UserGuide/jobs-spark.html)) |

**Ninguna presenta el autoescalado como protección contra OOM**: escalan por cola de tareas pendientes,
no por memoria.

## 3 · Spark 4 en Kubernetes con dynamic allocation

Fuentes: [running-on-kubernetes](https://spark.apache.org/docs/latest/running-on-kubernetes.html) y
[configuration](https://spark.apache.org/docs/latest/configuration.html). La doc servida es la 4.2;
nosotros usamos la 4.1, y lo que es «since 4.2» no lo tenemos.

- **Cómo funciona:**
  - el driver crea los pods de ejecutor por la API de Kubernetes, en lotes (`allocation.batch.size 20`,
    `batch.delay 1s`);
  - pide más cuando hay tareas pendientes 1 s (`schedulerBacklogTimeout`);
  - devuelve un ejecutor ocioso a los 60 s (`executorIdleTimeout`).
- **Sin servicio de *shuffle* externo en Kubernetes**, hay que activar
  `dynamicAllocation.shuffleTracking.enabled`. Un ejecutor que guarda *shuffle* **no se retira**
  (`shuffleTracking.timeout = infinity`), así que el nodo puede quedarse encendido más de lo necesario.
- **Decommission:**
  - `spark.decommission.enabled` y `spark.storage.decommission.*` migran bloques al retirarse;
  - `fallbackStorage.path` puede ser `gs://…`, y Spark no lo limpia.
- **El driver en un pod que ya existe** (modo cliente dentro del clúster) está soportado. Hace falta:
  - un Service sin IP que apunte solo a ese pod;
  - `spark.driver.host` y `spark.driver.port` fijos;
  - `spark.kubernetes.driver.pod.name`, para que los ejecutores lleven al driver como dueño y mueran
    con él;
  - un `executor.podNamePrefix` único.
- **RBAC:** un `Role` en el namespace con `pods`, `services` y `configmaps` (`create`, `get`, `list`,
  `watch`, `delete`, `deletecollection`).
- **Plantilla de ejecutor** (`spark.kubernetes.executor.podTemplateFile`): Spark sobrescribe imagen,
  recursos y `restartPolicy`; la memoria va con *request* igual a *limit*.

## 4 · Kueue con ejecutores que crea el driver

Fuentes: los [docs de Kueue](https://kueue.sigs.k8s.io/docs/) y la
[issue #4106](https://github.com/kubernetes-sigs/kueue/issues/4106).

| vía | estado en 0.19 | ¿sirve? |
|---|---|---|
| `SparkApplication` (Spark Operator) | alfa, apagada | **no**: el webhook **rechaza** `dynamicAllocation.enabled=true` |
| ejecutores como pods sueltos (`queue-name` + `managed=true` + `pod-suspending-parent`) | **apaño de la comunidad**, no API estable; la issue se cerró «not planned» | funciona, cada ejecutor es su propio Workload, pero es frágil |
| grupos de pods | tamaño fijo | no |
| *Elastic Workloads* (`ElasticJobsViaWorkloadSlices`) | beta, encendida desde 0.18 | solo `batch/v1 Job` y Ray, no pods sueltos |
| admisión parcial | beta | solo Jobs |
| **ejecutores fuera de Kueue** | — | los limitan la `ResourceQuota`, `maxExecutors` y el máximo del pool; **Kueue no los ve** y puede creer que hay hueco que no hay |

## 5 · Seguridad: el driver puede crear pods, y el código del cliente corre en el driver

[inferencia] Con el token del driver, el código del cliente puede crear o borrar pods en el namespace
de su celda. La defensa estándar es una **ValidatingAdmissionPolicy** (GA desde 1.30,
[doc](https://kubernetes.io/docs/reference/access-authn-authz/validating-admission-policy/)) sobre
los pods que crea esa cuenta de servicio:
- solo nuestra imagen por digest, cuenta del ejecutor sin token y volúmenes `configMap` y `emptyDir`;
- sin `hostNetwork` ni privilegios, con límites de memoria y dueño un Pod;
- y además Pod Security `restricted` en el namespace.

La política **no puede** comprobar que el dueño sea un driver de verdad: no consulta otros objetos.
MutatingAdmissionPolicy está en beta y apagada en 1.35.

## 6 · El OOM: los nodos no lo arreglan

Qué lo provoca y qué se hace:

| causa | defensa |
|---|---|
| un lado difundido demasiado grande | acotarlo: ya lo hace C3·a (≤ 150 k filas, ≤ 48 MiB) |
| `collect()` grande | `spark.driver.maxResultSize` (1 g) |
| una partición sesgada en un join **por igualdad** | AQE `skewJoin` (activado por defecto) |
| **una partición sesgada en un join espacial** | **AQE no la ve**: `RangeJoin` y `DistanceJoin` de Sedona hacen su propio particionado, que no es un `Exchange` de Catalyst ([fuente](https://raw.githubusercontent.com/apache/sedona/master/spark/common/src/main/scala/org/apache/spark/sql/sedona_sql/strategy/join/TraitJoinQueryExec.scala)) [inferencia]. Depende de `sedona.join.gridtype` (KDB-tree) y `sedona.join.numpartition` |
| la memoria de Python (UDF) | `spark.executor.pyspark.memory` no tiene valor por defecto, así que Python **no tiene límite** dentro del pod y el kernel mata el pod (`OOMKilled`). En Kubernetes, `memoryOverheadFactor` sube a 0,40 en trabajos no JVM |
| ejecutores demasiado gordos | 4–5 cores por ejecutor ([AWS](https://aws.amazon.com/blogs/big-data/submitting-user-applications-with-spark-submit)) |

## 7 · *Spot*

- El aviso es de 30 s: **15 s para los pods normales**. Desde 1.35.0-gke.1171000 sube a 120 s con la
  configuración del pool ([doc](https://docs.cloud.google.com/kubernetes-engine/docs/concepts/spot-vms)).
- Los PDB no protegen frente a la reclamación.
- [inferencia] En 15–120 s no da tiempo a migrar un *shuffle* grande: lo realista es `fallbackStorage`
  en GCS, o recomputar.
- La recomendación común es **el driver fuera de *spot*** y los ejecutores en *spot*.

## 8 · Red

- En modo cliente, los puertos del driver hay que fijarlos: 7078 el driver y 7079 su *block manager*.
- Los ejecutores hablan con el driver en 7078 y 7079, y entre ellos en 7079 para el *shuffle*.
- El driver necesita salida al API server; los ejecutores no.

## 9 · Lo que sigue sin saberse, y solo se sabe midiendo en victor

1. El **tiempo de escalar** de verdad: un Build que empieza en un nodo y pide más (backlog → pod
   *pending* → nodo listo → ejecutor registrado).
2. Que la **ValidatingAdmissionPolicy** niega un pod arbitrario creado desde una celda y deja pasar el
   ejecutor.
3. Cuánto tiempo **retiene** el *shuffle tracking* un nodo que ya no hace falta.
4. Qué pasa si **se expulsa un nodo de ejecutores** a mitad: recomputar, o `fallbackStorage`.
5. Un **join espacial sesgado** (casi todo en una zona) en un ejecutor: si hay OOM y qué ajuste de Sedona
   lo evita.
6. Las **UDF de Python** con `pyspark.memory` y el *overhead*: que lo que se pasa falle con un error
   claro y no con `OOMKilled`.
7. Los ejecutores **fuera de Kueue** frente a un Build de otra celda: quién se queda sin sitio.
8. **La imagen en un nodo nuevo** para un ejecutor: 1,8 s medidos para el pod entero en 6c; confirmar
   con el pod del ejecutor.
