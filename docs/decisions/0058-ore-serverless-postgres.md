# 0058 · ORE Serverless Postgres

**Estado:** **propuesto** (2026-10-06) · D0 (local), D0b·1–5 (GKE) y D0c (Neon compilado por nosotros,
GCS nativo, compatible con el cómputo publicado, recuperable desde GCS) medidos y cerrados; la prueba
recogida (D0b·6). **Plan de construcción escrito (B.11)**: fase I, un Postgres serverless sano (P1–P9);
fase II, el catálogo (Q1–Q5). Siguiente: P1, esperando el go. **Decide:** qué es ORE Serverless Postgres para quien
lo usa, sobre qué se construye, qué es nuestro y qué no, y cómo pasan sus datos al catálogo. Toca
[`0044`](0044-ramas-globales.md) (las ramas globales: no se mezclan con éstas),
[`0047`](0047-ore-access-control.md) (quién puede), [`0048`](0048-ore-idp.md) (quién es) y
[`0053`](0053-ore-federation-engine.md) (leer un origen).

## La imagen de producto

**ORE Serverless Postgres es un Postgres serverless donde el cliente despliega su aplicación —como en
Neon o Supabase— y cuyos datos, sin que el cliente monte nada, son activos del catálogo de ORE:** se
analizan, se transforman, entrenan modelos y alimentan funciones y pipelines **fuera de Postgres**, en
el lago, en Iceberg.

El cliente crea un **proyecto** (puede tener varios) y no elige máquinas ni discos. Recibe:

| | qué es |
|---|---|
| **Proyecto** | el espacio de sus datos: su historia, su retención, su región |
| **Rama** | `main` y las que cree: un punto en la historia, sin copiar nada, al instante |
| **Endpoint** | el cómputo que sirve una rama; existe mientras hay tráfico y escala solo |
| **Roles y bases** | Postgres de siempre |
| **Cadena de conexión** | `postgres://…@<endpoint>.<región>.ore…`, siempre por el proxy |

Lo único que elige son **límites**: cuánto puede crecer el cómputo, cuánto tarda en dormirse, cuánta
historia guarda. Paga cómputo mientras vive y almacenamiento por lo que ocupa; dormido, sólo sus bytes.

### Por qué existe

- **Un Postgres transaccional no escala para lo que vendemos.** Agregaciones, informes,
  transformaciones, modelos y millones de filas guardadas en pasivo no son su trabajo.
- **Neon y Supabase demostraron que el desarrollador quiere más que almacenamiento relacional**
  —ramas, escalado a cero, funciones—, pero ninguno ofrece inteligencia sobre esos datos: hay que
  copiarlos a Databricks o Snowflake y pagar el almacenamiento dos veces.
- **Databricks (Lakebase, sobre Neon) y Snowflake (Postgres, sobre Crunchy Data y `pg_lake`) ya lo
  hicieron**, con su lock-in. Nosotros ponemos ORE —catálogo, linaje, gobierno, ramas, modelos,
  funciones— **debajo del Postgres que sirve la aplicación**, en el lago del cliente y en formato
  abierto.

### Lo decidido

1. **El motor es Neon open source, y es un fork nuestro.** Almacenamiento separado del cómputo, ramas
   como puntos de la historia, WAL replicado por quórum. El repositorio público se paró tras la compra
   por Databricks (julio de 2025) y ninguna imagen publicada trae GCS: **compilamos Neon nosotros**,
   desde un commit fijado, en nuestro Cloud Build, y lo publicamos en nuestro registro.
2. **El almacenamiento es compartido y siempre está encendido; el cómputo es de cada cliente y escala a
   cero.** Una capa de almacenamiento por región —pageserver, safekeepers, broker y `storage_controller`—
   sirve a todos los proyectos, aislados por tenant. Los bytes viven en **GCS, con la identidad del pod
   (Workload Identity) y sin una sola clave**.
3. **El cómputo es una VM**, no un contenedor: NeonVM sobre KVM, con CPU y memoria que crecen y
   decrecen en caliente sin cortar conexiones, y que se mueve de nodo en vivo. Las conexiones entran por
   **el proxy, que vive en la red overlay** de las VMs: es lo que deja sobrevivir a una migración.
4. **El plano de control es de ORE**, y es lo que Neon no publica:
   - la API de proyectos, ramas y endpoints;
   - el ciclo de vida del cómputo: **suspender** por inactividad (la señal `last_active` de
     `compute_ctl` sumada a las conexiones que ve el proxy) y **despertar** a petición, desde un pool de
     VMs ya arrancadas;
   - entregar a cada cómputo su especificación y las claves con que se le habla;
   - **un solo cómputo de escritura por rama**, siempre;
   - roles, cuotas y medición.
5. **Publicar al catálogo es elegir tablas.** Cada tabla publicada llega al lago por **CDC lógico desde
   el WAL** —la ranura lógica sobrevive a dormir y a despertar—, como una base más del catálogo, y cada
   snapshot de Iceberg anota la **marca de agua** (el LSN que refleja). Desde ahí sirve todo ORE.
6. **Quién es y qué puede** lo dicen ORE IdP (0048) y ORE Access Control (0047): la concesión niega, no
   concede.
7. **Sus ramas son de Postgres**, no las ramas globales del árbol (0044). Unir las dos —una rama que sea
   a la vez un LSN de Postgres y un snapshot de Iceberg— queda para después; la marca de agua del punto
   5 es lo que lo hará posible.
8. **Es un producto aparte dentro de la organización: integrado en la identidad y la propiedad, aparte en
   el runtime.** Un proyecto de Postgres existe o no existe. Si existe, **pertenece a la organización**, y
   la identidad y la propiedad salen del plano de control común, como en cualquier otro servicio: entra
   por la celda (la celda es el workspace, como en Lakebase), `ore-iam` decide y el dueño es una persona.
   Pero **lo que corre no está enlazado a nada de la organización**: ni a la cuota de su celda ni a sus
   análisis, y si la celda o `ore-iam` caen, la base sigue sirviendo (P4). Es serverless de verdad:
   - se paga lo que se usa (cómputo por segundo despierto y bytes guardados);
   - escala sin que el cliente dimensione nada, porque la capacidad la pone la plataforma;
   - sus límites son los del plan, no los de un namespace.

   Lo único que comparte con ORE es quién es y qué puede (decidido 6) y, si el cliente quiere, la
   publicación al catálogo (decidido 5).

### Por qué así, frente al cliente

- **Una base que sirve a la aplicación y un lago que la entiende, sin pegamento.** Lo que escribe la
  aplicación es un activo del catálogo con su linaje y su gobierno; nadie monta un ETL.
- **Serverless de verdad**: el cliente no dimensiona nada, duerme sin coste de cómputo y crece en
  segundos; una rama es instantánea y no duplica datos.
- **Formato abierto y en su lago**: Iceberg y GCS, no un almacén propietario.

### Lo que se descarta

- **Usar las imágenes publicadas de Neon**: se pararon antes de GCS y nadie las parchea.
- **GCS por su API compatible con S3** (HMAC): la organización prohíbe crear claves de cuentas de
  servicio, y una clave es justo lo que no queremos.
- **El cómputo en pods** como base: medido, la VM escala y migra en caliente; los pods quedan como
  salida sólo si KVM faltara en una región.
- **Una capa de almacenamiento por celda**: multiplica el coste fijo; el aislamiento es por tenant.
- **Dual-Branching ahora** (rama = LSN de Postgres + snapshot de Iceberg): se aparca, no se cierra.

### La hoja de ruta

Primero **un Postgres serverless sano** y sólo entonces el catálogo. Cada hito se cierra con un «hecho cuando» medible; el detalle y los sub-pasos están en B.11.

**Fase I · Un Postgres serverless sano**

| hito | qué |
|---|---|
| P1 | el motor es nuestro: forks, compilar almacenamiento y cómputo, Postgres al día |
| P2 | almacenamiento de producción con `storage_controller` |
| P3 | cómputo de producción y **aislamiento entre organizaciones** |
| P4 | plano de control: `ore-postgres` y `/v1/postgres/…` en `ore-serve` |
| P5 | el proxy: conexión desde internet con TLS y SNI |
| P6 | dormir y despertar desde un pool precalentado |
| P7 | una aplicación real encima durante días |
| P8 | operarlo: PITR, alertas, actualizaciones en rodaje, game day |
| P9 | el producto alrededor: medición, cuotas y consola |
| ⛔ | **puerta de producción**: cuota, nodos grandes, multizona, revisión de seguridad |

**Fase II · Sus datos, activos del catálogo**

| hito | qué |
|---|---|
| Q1 | la base como foreign database, sin copiar |
| Q2 | publicar tablas por CDC a Iceberg con marca de agua |
| Q3 | gobierno y linaje |
| Q4 | del lago a Postgres |
| Q5 | ramas unidas |

### Dónde vive el código

- **Los forks: un repositorio público por upstream.**
  - Son `describeloai/neon`, `describeloai/postgres` y `describeloai/autoscaling`.
  - Cada uno tiene su remoto `upstream`, para poder rebasar.
  - Sólo llevan código de Neon y nuestros parches. Ni el submódulo de `neon` se toca: su URL es relativa (`../postgres.git`) y resuelve sola a nuestro `postgres`.
  - El CI no va en los forks: cada fichero nuestro allí es un conflicto en el siguiente rebase.
- **El producto vive dentro de ORE, con su forma modular de siempre:**
  - `crates/ore-postgres`: el plano de control;
  - `ore-serve`: `/v1/postgres/…`;
  - `malla/8x-postgres-*`: el despliegue;
  - `ci/neon/`: la receta y el **commit fijado**;
  - `pruebas-de-fuego/ore-postgres/`: las pruebas de aceptación.
- **La frontera entre los dos es una imagen con su commit.** ORE consume el motor como consume Keycloak.
- **La copia local para rebasar** está en `C:/ore-neon/`. Las compilaciones van siempre en Cloud Build.

---

## Zona borrador · lo medido, los arreglos y los pasos

> Cuaderno de trabajo. Al cerrar cada etapa, lo que valga sube limpio a la parte de arriba y esto se
> poda (como en 0046). Los guiones y manifiestos de la prueba están en
> [`pruebas-de-fuego/ore-postgres/`](../../pruebas-de-fuego/ore-postgres/).

### B.0 · Neon: qué publica y qué no

- **Publica (Apache 2.0)**: `neondatabase/neon` —pageserver, safekeeper, storage_broker,
  storage_controller, storage_scrubber, endpoint_storage, proxy, `compute_ctl`, la extensión `neon`,
  forks de Postgres 14–17— y `neondatabase/autoscaling` —NeonVM (QEMU como recurso de Kubernetes),
  autoscaler-agent, planificador modificado, vm-monitor, vxlan—.
- **No publica**: el plano de control de producción. `control_plane/` es `neon_local`, «no apto para
  producción». El proxy llama a una API del plano de control para despertar el cómputo y obtener el
  secreto del rol: esa API la escribimos nosotros.
- **Actividad**: 88 commits en julio de 2025 y luego ~1 al mes; `autoscaling` sin push desde el
  2025-12-16, última release v0.49.1 (2025-07-22). Releases de Neon hasta 2025-07-29. GCS entró en
  `main` el 2025-09-16 (arreglos en febrero y marzo de 2026).
- **Comunidad**: `neon-operator` (Molnett, ahora Lovable; experimental), NeonD, Vela.

### B.1 · D0 · en local (Docker en Windows; los tiempos absolutos no son de producción)

- Compose de Neon con SeaweedFS (MinIO ya no publica imágenes). Trampas: el bind mount de Windows rompe
  el `rename` del pageserver (volumen Docker), CRLF en `compute.sh`, la caché local (LFC) apagada.
- Rama en el LSN actual 171 ms; en un LSN pasado 40 ms; 0 bytes al nacer; aislamiento total.
- Cómputo listo 0,6 s tras arrancar el contenedor; despertar 0,6 s.
- Con LFC: recorrido de 1M filas 1,3 s en frío / 50 ms en caliente; PK 0,2–2 ms.
- `wal_level=logical` por defecto; **la ranura lógica sobrevive a escalar a cero** y a recrear el
  cómputo (test_decoding da INSERT y UPDATE).
- El compose enganchó dos cómputos a la misma rama al reiniciar: el motor no lo impide (⇒ decidido 4).

### B.2 · La cuota (O0–O5, 2026-10-06)

`CPUS_ALL_REGIONS` = 12, sin aumento posible. Medido 7 días por contenedor: lo nuestro reservaba 3–50×
su pico. Reservas ajustadas (6f27599, 74045ef), aprovisionador en pasada ligera (7befb7f), Jobs al nodo
del sistema con sabor `system` de Kueue y pool de desborde `jobs-s` e2-standard-2 0→1 (b38cb19);
`jobs-p` retirado. **5/12 en reposo**: 7 vCPU para esto. Cloud Build no gasta esta cuota. La cuota de
SSD regional (250 GB) está llena: los discos de prueba son `pd-standard`.

### B.3 · D0b·1 · la infraestructura en GKE (ore-mesh, 1.35, Dataplane V2)

- Pool `neon-d0`: 3 × n2-standard-2, `--enable-nested-virtualization`, Ubuntu containerd, privado,
  taint `ore.dev/neon`. **KVM funciona**: `/dev/kvm`, `vmx`; el plugin `bridge` y `vxlan` en el kernel.
- Todo DaemonSet/Deployment de Neon, fijado al pool (`preparar.py`); en GKE los binarios CNI están en
  `/home/kubernetes/bin` (sólo se cambia el `hostPath`, nunca la ruta dentro de la imagen).
- **Multus, tres capas de arreglo**:
  1. el de la release (bitnami 3.9.3, además retirado de Docker Hub) no entiende CNI 1.1.0;
  2. Multus 4.3.1 en automático deja los nodos NotReady (`STATUS` → «missing containerID»);
  3. funciona: configuración **fija** `cniVersion 1.0.0` con `clusterNetwork` apuntando a una copia,
     por nodo, del conflist de GKE reescrito a 1.0.0 (un sidecar la mantiene; `cilium-cni` sólo admite
     ≤ 1.0.0). En `multus-v4-gke.yaml`.
- GKE sólo deja usar prioridades `system-node-critical` en un namespace con ResourceQuota para ellas
  (`pods-criticos` en `neonvm-system`).
- **Neon está dimensionado para nodos grandes**: controlador 2 CPU × 3 réplicas, planificador 1 CPU,
  autoscaler-agent 1 CPU por nodo (bajados a 200m para la prueba); ~1,4 vCPU fijos por nodo más ~0,5 de
  GKE. En producción, nodos de 16–32 vCPU.

### B.4 · D0b·2 · Postgres en una VM

- `vm-compute-node-v17` (2025-08-26) no trae entrypoint: `guest.command: /usr/local/bin/compute_ctl`,
  **sin `su-exec`** (ya corre como `postgres`); `-C` con `127.0.0.1` (en la VM no resuelve
  `localhost`); la especificación entra como disco desde un ConfigMap.
- ×3: `apply` → pod con IP 3,2–3,6 s; QEMU → `compute_ctl` ~1,8 s; `compute_ctl` → `running`
  1,75–2,2 s; **`apply` → primera consulta 15,8–17 s**. El hueco (~8 s) es preparar el runner ⇒ pool de
  VMs precalentadas.
- 100 000 filas sobreviven a destruir la VM y crear otra.

### B.5 · D0b·3 · autoescalado y la señal del escalado a cero

- Requisitos: `AUTOSCALING=true` en el entorno (si no, `compute_ctl` no lanza vm-monitor),
  `--filecache-connstr` con el puerto real, puerto 9100 declarado.
- Reposo 0,25 vCPU / 1 GiB; con carga, **9 s** a 0,5 vCPU / 3 GiB (pidió 0,75: el nodo de 2 vCPU no
  daba más); vuelta al mínimo en ~3 min; la cadena métricas → agente → planificador → hotplug → vm-monitor
  ~1 s; **286 escrituras sin corte** durante subida y bajada.
- **Escalado a cero = suspender** (plano de control + proxy), no autoscaling. `GET :3080/status` da
  `last_active`; pide JWT EdDSA con `compute_id` y el JWKS en `compute_ctl_config` (la clave de ejemplo
  de Neon está mal codificada). `last_active` **no cuenta a `cloud_admin`** y en esta imagen muestrea el
  estado `active` cada 500 ms ⇒ consultas muy cortas pueden no verse.

### B.6 · D0b·4 · migración en vivo

- Necesita `spec.extraNetwork.enable: true` (IP overlay estable por whereabouts) y el cliente en la
  overlay (`k8s.v1.cni.cncf.io/networks: neonvm-system/neonvm-overlay-for-pods`).
- ×2: 19,0 / 19,3 s en total, **VM pausada 66 / 51 ms**, 134 MB comprimidos de 3,2 GiB.
- Por la overlay la sesión sobrevive (mayor hueco 0,75 s); por la IP del pod **se cuelga sin error**.

### B.7 · D0b·5 · reposo y latencia

- Reposo real: almacenamiento entero ~17 milinúcleos / ~210 MiB; piezas de NeonVM 1–3 milinúcleos; VM
  dormida 25 milinúcleos / 445 MiB.
- Safekeepers en `pd-standard`: commit de una fila **2,98 ms** (336 tps); sin esperar al quórum 0,74 ms
  ⇒ el quórum cuesta ~2,2 ms; TPC-B 1 cliente 13,8 ms; 8 clientes 186 tps de media subiendo de 135 a
  243 mientras la VM escalaba; lectura por PK ~3 ms.

### B.8 · D0c · Neon compilado por nosotros

- **C1** (`cloudbuild-neon.yaml`, hoy [`ci/neon/almacen.yaml`](../../ci/neon/almacen.yaml), cuenta `ore-ci`, E2_HIGHCPU_32, sin caché), commit `fa504217`
  (2026-08-31): **19 min 24 s** (fuente 53 s, compilar 17 min 35 s), **~1,2 USD**, imagen 1,9 GB en
  `…/ore/neon:<commit>`. Trampa: el BuildKit de `cloud-builders/docker` no entiende
  `${VAR/patrón/…}` ⇒ se antepone `# syntax=docker/dockerfile:1`. El 90 % es Postgres y dependencias:
  con caché bajará.
- **C2** (`almacen-gcs.yaml`): pageserver y safekeepers en **GCS nativo con Workload Identity, cero
  claves** (0 de usuario, 0 HMAC). Tras pgbench escala 20: `pageserver/` 32 objetos / 422 MB,
  `safekeeper/` 15 segmentos de WAL / 240 MB. Las **borradas** esperan al `storage_controller` (no
  desplegado) ⇒ en producción va desplegado. El cómputo publicado (agosto de 2025) arranca y escribe
  contra el pageserver de `main`.
- **C3** · el cómputo publicado (`vm-compute-node-v17`, agosto de 2025) contra pageserver y safekeepers
  de `main` (fa504217), por la overlay:
  - pgbench sin un fallo: TPC-B 1 cliente 8,3 ms / 120 tps; 4 clientes 286 tps; lectura 4 clientes
    3 580 tps.
  - **Rama** en el LSN de `main` en **367 ms**; una VM sobre ella ve todo lo anterior al LSN y nada de
    lo posterior; lo que escribe la rama (una fila, 1 000 borradas) no llega a `main` (2 000 000 intacto).
    La rama vive en GCS como su propio timeline.
  - ⚠️ Trampa de la prueba, no de Neon: `psql -c "a; b; select pg_current_wal_flush_lsn()"` es UNA
    transacción ⇒ el LSN sale de antes del commit y la rama no ve lo recién escrito. El LSN se pide en
    un comando aparte.
  - ⚠️ **Coste de leer en una rama** (confirmado en C5): el primer `count(*)` de 2 M filas en la rama
    escribió **138 MB de WAL** (288 MB de deltas en GCS); el segundo, 56 bytes.
  - ⇒ **Para ser compatible no hace falta compilar el cómputo**: el de agosto de 2025 vale contra
    `main`. Para estar parcheado, sí (C5).
- **C4** · «sin fondo» (`c4.sh`): se borra el pageserver **con su disco**; uno vacío recupera desde GCS.
  - **RPO 0**: están la marca subida a GCS **y la que sólo vivía en los safekeepers** (escrita justo
    antes, sin checkpoint); `main`, las dos ramas y lo que escribió la rama, intactos.
  - Tiempos: pod nuevo con disco nuevo (`pd-standard`) listo en **14,7 s**; el pageserver vacío no
    conoce ningún tenant ⇒ alguien lo **reengancha** con la generación siguiente (`location_config`,
    generación 2: lo que hará el `storage_controller`); tenant `Active` en **6,6 s** más (índices desde
    GCS, sin bajar datos). Total del desastre a tenant servido: **~21 s**.
  - **Lo cacheado no se entera**: una sonda contra `main` cada segundo respondió durante todo el
    desastre (la tabla estaba en la caché del cómputo). Sólo espera quien pide páginas que no tiene.
  - Lectura en frío: una VM nueva sobre la rama (caché vacía) recorre 2 M filas en ~1,3 s contando
    ~0,8 s de `kubectl exec` + conexión; el pageserver bajó ~200 MB de capas a demanda. Una repetición
    tardó 16 s (anómala, sin explicar) y la siguiente 1,2 s: medir dentro de la base, no desde fuera.
  - ⚠️ **La overlay y la IP reutilizada**: al recrear la VM, whereabouts le dio la **misma IP overlay
    con otra MAC**; durante ~1 min un cliente con la MAC antigua en su caché ARP no llegó (por la IP del
    pod sí). Se arregló solo. Importa para despertar: el proxy reintenta, o la VM anuncia su MAC
    (ARP gratuito) al nacer, o no se reutiliza la IP al momento.
  - ⇒ La recuperación es de **decenas de segundos**, no de minutos: un pageserver por grupo de tenants y
    reengancharlo (storage_controller) basta para empezar; los secundarios en caliente, cuando el RTO
    prometido baje de eso. El disco local es caché: perderlo no pierde nada.
- **C5** · cierre de D0c.
  - **El coste de leer en una rama, confirmado** (`c5-hints.sh`): una tabla de 2 062 páginas
    actualizada entera en `main` (filas nuevas sin hint bits) y ramificada. Primer recorrido **en la
    rama: 16 MB de WAL = una página entera por página** (2 062 × 8 KB); el segundo, 0. **En `main`, el
    mismo primer recorrido: 250 kB.** Causa: la rama nace con su punto de recuperación en el LSN de la
    rama (`redo_lsn` = el de la rama), y con `wal_log_hints=on` la primera modificación de cada página
    tras un checkpoint —también marcar hint bits— escribe la página entera. `main` ya lo había pagado;
    la rama lo paga de nuevo. ⇒ Una rama cuesta, al leer, ~8 KB por página con hint bits sin marcar
    que toque; un `VACUUM` en `main` antes de ramificar lo evita. Entra en el precio de las ramas.
  - **Postgres va por detrás**: `vendor/revisions.json` en fa504217 trae **17.5 y 16.9** (mayo de 2025);
    la última es **17.10 / 18.4 (2026-05-14), con 11 CVE** solo en esa. Neon no tiene Postgres 18. El
    cómputo publicado también es 17.5. ⇒ **Antes de producción hay que traer 17.5 → 17.10+** al fork de
    Postgres de Neon (sus parches del gestor de almacenamiento) y compilar también el cómputo.
  - **Cómo mantener el fork** (propuesta):
    1. un fork propio de `neondatabase/neon` y de su `postgres`, con el commit fijado y nuestros
       parches en ramas cortas;
    2. **cada versión menor de Postgres** (trimestral: feb, may, ago, nov) y ante un CVE: rebasar el
       fork de Postgres sobre la etiqueta nueva, compilar almacenamiento **y** cómputo (imagen de VM con
       `vm-builder`) y pasar `pruebas-de-fuego/ore-postgres/` como aceptación;
    3. dependencias de Rust: `cargo audit`/`cargo deny` semanal en el CI;
    4. **sólo Postgres 17** al principio (fuera 14–16 de la compilación: menos tiempo); 18 cuando
       exista en Neon o lo portemos;
    5. el CI en Cloud Build por etiqueta; con caché de capas en el registro (BuildKit) y compilando sólo
       lo que cambia —sin medir aún: se mide al montarlo—. Hoy, en frío: **19 min 24 s / ~1,2 USD**.

### B.9 · Los pasos

| paso | qué | estado |
|---|---|---|
| D0c·C3 | compatibilidad: pgbench de escritura/lectura y una rama con el cómputo publicado | **hecho** (2026-10-06) |
| D0c·C4 | «sin fondo»: borrar el pageserver con su disco y recuperar el tenant y la rama desde GCS; tiempo | **hecho** (2026-10-06): RPO 0, ~21 s |
| D0c·C5 | cerrar: cómo mantener el fork; el coste de leer en una rama | **hecho** (2026-10-06): hint bits confirmados; Postgres 17.5 → hay que traer 17.10+ |
| D0b·6 | recoger lo de la prueba (B.10) y volver a 5/12 | **hecho** (2026-10-06): 5/12 |
| P1–P9, Q1–Q5 | construir (B.11) | **plan escrito** |
| P1·1 | los forks y el CI apuntando a ellos | **hecho** (2026-10-07): compilado desde el fork en 14 min 7 s |
| P1·2 | la imagen de cómputo | **hecho** (2026-10-07): 1 h 10 min en frío; `vm-compute-node-v17` en el registro |
| P1·3 | la caché | **hecho**: almacenamiento 14 min con un cambio de Rust (2 min 20 s sin cambios); el cómputo es donde ahorra |
| P1·4 | Postgres 17.10 | **hecho** (2026-10-07): regresión igual que la base; almacenamiento (28 min, con caché) y cómputo (1 h 9 min) compilados en `8269bece` |
| P1·5 | procedimiento de mantenimiento y pruebas de fuego parametrizadas | **hecho** (2026-10-07): [`ci/neon/README.md`](../../ci/neon/README.md), [`pruebas-de-fuego/ore-postgres/`](../../pruebas-de-fuego/ore-postgres/) |
| P2·1 | el contrato del `storage_controller` | **hecho** (2026-10-07): leído y probado en local |
| P2·2 | infraestructura GCP (bucket, cuenta, pool no-spot) | **hecho** (2026-10-07): [`malla/80-postgres-gcp.sh`](../../malla/80-postgres-gcp.sh); cuota 7/12 |
| P2·3 | la base del controller (Postgres en el clúster, copias, restauración +1000) | **hecho** (2026-10-07): en vivo por Flux; restaurada una copia de verdad → generación 7 + 1000 |
| P2·4 | la malla: controller, broker, safekeepers, pageserver | **hecho** (2026-10-07): en vivo por Flux, a la primera, con autenticación |
| P2·5 | retención y limpieza (PITR, GC, scrubber; borrar un tenant vacía GCS) | **hecho** (2026-10-07): historia 1 día; borrar un tenant vacía su prefijo (4 → 0 objetos); scrubber diario, 0 errores |
| P2·6 | aceptación (+ decidir `--timelines-onto-safekeepers`) | **hecho** (2026-10-07): RPO 0 en todo; C4 18 s sin intervención; 2 safekeepers caídos paran sin perder; `--timelines-onto-safekeepers` off (exige 3 zonas) |
| **P2** | **el almacenamiento de producción** | **cerrado** (2026-10-07) |
| P3·1 | NeonVM y autoscaling desde el fork | **hecho** (2026-10-07): las 6 imágenes y el kernel, de `describeloai/autoscaling` v0.49.1, en 4 min 19 s |
| P3·2 | autoescalado de nodos del pool `pg` | **hecho** (2026-10-07): 1–3 nodos; sube uno en ~3,5 min, lo quita a los ~12 min de sobrar |
| P3·3 | la base del cómputo en la malla | **hecho** (2026-10-07): tres Kustomizations de Flux; una VM arranca y escribe desde git |
| P3·4 | `ore-pg-computo` y la barrera 2 | **hecho** (2026-10-07): 15/15 en `p34.sh` |
| P3·5 | la overlay cerrada (barrera 3) | **hecho** (2026-10-07): 6/6 en `p35.sh`; antes, de VM a VM sí se entraba (medido) |
| P3·6 | la IP reutilizada y la anti-suplantación | **hecho** (2026-10-07): la overlay responde 1,5–1,6 s después de la IP del pod (antes ~10 s); la MAC ya no cambia |
| P3·7 | aceptación con VMs | **hecho** (2026-10-07): todas las pruebas de fuego con VMs y las tres barreras, desde git |
| P3 | el cómputo de producción y el aislamiento | **hecho** (2026-10-07) |
| P4·1 | el contrato y el esqueleto | **hecho** (2026-10-08): `p41.sh` con dos celdas de verdad (demo y victor): una no ve lo de la otra |
| P4·2 | proyectos y ramas | **hecho** (2026-10-08): `p422`–`p424`; borrar deja bucket, safekeepers y pageserver como estaban (81 → 81) |
| P4·3 | endpoints y el cerco | **hecho** (2026-10-08): VM en 35–46 s; un segundo de escritura a la vez es 409; capa 3 medida tres veces (`p434.sh`) |
| P4·4 | roles y bases | **hecho** (2026-10-08): `p44.sh`, la vieja contraseña deja de entrar al regenerar |
| P4·5 | los avisos del almacenamiento | **hecho** (2026-10-08): `p45.sh`; el controller avisa a `ore-postgres`, stub fuera |
| P4·6 | la API de ORE | **hecho** (2026-10-08): `/v1/postgres` en `ore-serve` + 051 en `iam`; banco en CI; en vivo una persona crea su instancia desde la consola |
| P4·7 | aceptación | **hecho** (2026-10-08): `p47.sh`, todo por la API y después nada: 0 filas, VMs, ConfigMaps, objetos en GCS, safekeepers y pageserver |
| **P4** | **el plano de control** | **cerrado** (2026-10-08) |

### B.10 · Lo que hubo vivo en GKE para la prueba (recogido en D0b·6, 2026-10-06)

Pool `neon-d0`; namespaces `d0-neon`, `neonvm-system`, `cert-manager`; en `kube-system` Multus,
whereabouts, `autoscale-scheduler` y `autoscaler-agent` (todos con `nodeSelector ore.dev/pool=neon`);
CRDs de NeonVM, cert-manager y Multus; bucket `ore-neon-d0-1006`; cuenta `neon-d0` (Workload Identity →
`d0-neon/neon`); imagen `ore/neon:fa504217…` (se queda).

**Recogido** en este orden: las VMs y `d0-neon` (sus 4 discos `pd-standard` se borraron con los PV);
los webhooks de NeonVM y cert-manager; en `kube-system` los DaemonSets, el planificador, sus cuentas,
ConfigMaps y roles; `neonvm-system` y `cert-manager`; las 12 CRDs; los `ClusterRole(Binding)` por nombre
exacto (ojo: `cluster-autoscaler`, `kube-dns-autoscaler` y `horizontal-pod-autoscaler` son de GKE y se
quedan); el pool; el bucket (2,3 GB); la cuenta. Cuota: **5/12**. Queda la imagen en el registro.

### B.11 · El plan de construcción (2026-10-06)

**El orden lo manda una pregunta: ¿puede una aplicación de verdad vivir encima?**

- **Primero**, un Postgres serverless sano: que reciba y emita transacciones, que se conecte desde fuera, que escale, que duerma y despierte, todo por nuestro plano de control.
- **Sólo entonces**, el catálogo.

Cada hito se cierra con un **hecho cuando** medible. Sus sub-pasos (Pn·1, Pn·2…) se escriben al empezarlo, no antes.

**Dónde vive cada cosa** (propuesta; se cierra en P4):

| pieza | alcance | por qué |
|---|---|---|
| almacenamiento (pageserver, safekeepers, broker, `storage_controller`) | uno por región, compartido | En reposo cuesta ~17 milinúcleos (B.7). El aislamiento es por tenant. |
| `ore-postgres`, el plano de control de Postgres | uno por región | Es el dueño del estado (proyectos, ramas, endpoints). Habla con el almacenamiento, con NeonVM y con el proxy. |
| la API que ve el cliente | el `ore-serve` de cada organización (`/v1/postgres/…`) | Ya es «el plano de control de ORE: atiende a un cliente y delega lo que toca el mundo». 0047 y 0048 entran ahí. |
| las VMs de cómputo | **`ore-pg-computo`**, un namespace del producto para todos los endpoints (revisado en P3: decidido 8) | Postgres va aparte de la organización. Un cómputo no habla con ningún otro, así que basta una regla para todos. Los límites son los del plan (P4/P9), no una cuota de namespace. |
| el proxy | uno por región, en la overlay | Es la única puerta pública y sobrevive a las migraciones (B.6). |

**El desarrollo de la fase I cabe en los ~7 vCPU libres** (D0b usó 3 × n2-standard-2). Los nodos grandes y la cuota son la **puerta de producción**: van al final de la fase I, no antes.

#### Fase I · Un Postgres serverless sano

Cada hito, en orden, con lo que entra y cuándo está hecho.

**P1 · El motor es nuestro**
- Qué:
  - fork de `neon` y de su `postgres`;
  - el CI compila **el almacenamiento y el cómputo** (la imagen de VM, con `vm-builder`) desde nuestro commit, sólo con Postgres 17 y con caché de capas;
  - **rebase de Postgres 17.5 → 17.10+**, incluidos los parches de Neon al gestor de almacenamiento;
  - la aceptación es el suite de regresión de Neon.
- Hecho cuando:
  - las dos imágenes salen de nuestro CI por etiqueta y pasan la aceptación;
  - Postgres está en la última versión menor;
  - el tiempo y el coste están medidos, en frío y con caché.

**P2 · El almacenamiento, de producción** (declarado en la malla, con Flux)
- Qué:
  - el `storage_controller` con su base de estado: un Postgres normal, no él mismo;
  - el pageserver;
  - 3 safekeepers con antiafinidad;
  - el broker;
  - un bucket regional con Workload Identity;
  - la retención, el GC y el `storage_scrubber`.
- Hecho cuando:
  - los tenants y timelines se crean por la API del `storage_controller`;
  - **C4 se repite sin mano**: el controller reengancha solo;
  - borrar un tenant borra sus bytes en GCS.

**P3 · El cómputo, de producción** (declarado en la malla)
- Qué:
  - NeonVM, el autoscaling, Multus para GKE (B.3) y whereabouts;
  - un pool con virtualización anidada y las piezas dimensionadas;
  - **nuestra** imagen de cómputo;
  - las VMs en `t-<org>`, con su ResourceQuota.
- ⚠️ **El aislamiento en la overlay**: la overlay es una sola red L2 para todos, y Cilium no filtra la interfaz secundaria. Se mide y se cierra en este hito.
- Hecho cuando:
  - D0b·2–4 se reproduce desde git con nuestra imagen;
  - una VM de la organización A **no alcanza** a una de la B (probado).

**P4 · El plano de control: el núcleo**
- Qué:
  - `ore-postgres`, con su estado y su reconciliador:
    - proyecto → tenant;
    - rama → timeline, en un LSN **o en un instante**;
    - endpoint → VM, especificación y JWKS;
    - los roles y las bases van dentro de la especificación;
  - **un solo cómputo de escritura por rama**, con cerco;
  - operaciones asíncronas e idempotentes, cada una con su id;
  - `ore-serve` expone `/v1/postgres/…` con 0047 y 0048;
  - las contraseñas de los roles, en el cofre.
- Hecho cuando:
  - por la API de ORE se crean proyecto, rama, endpoint y rol, y se conecta desde dentro de la malla;
  - un segundo cómputo de escritura en la misma rama es **imposible** (probado);
  - borrarlo todo no deja huella.

**P5 · La entrada: el proxy**
- Qué:
  - el proxy de Neon contra **nuestra** API del plano de control, con el contrato que espera en el commit fijado: el secreto del rol (SCRAM) y despertar;
  - el enrutado por SNI, `<endpoint>.<región>.ore…`, con TLS comodín;
  - un balanceador L4 público;
  - el proxy, en la overlay;
  - un endpoint con pool (`-pooler`, el pgbouncer del cómputo) para las aplicaciones serverless;
  - más adelante, SQL por HTTP y WebSocket.
- Hecho cuando:
  - desde internet, `psql "postgres://…?sslmode=verify-full"` y pgbench funcionan sin fallos;
  - una migración en vivo **no corta** una sesión que entra por el proxy.

**P6 · Serverless de verdad**
- Qué:
  - **dormir** por inactividad: `last_active` más las conexiones que ve el proxy;
  - **despertar** al conectar, desde un pool de VMs ya arrancadas (`compute_ctl` sin especificación, esperando `/configure`);
  - los límites de cada endpoint, desde la API: CU mínimas y máximas, y el tiempo hasta dormir.
- Hecho cuando:
  - el despertar está medido en p50 y p95, con un objetivo fijado tras la primera medida;
  - el cliente no ve más error que la espera;
  - dormido, el cómputo cuesta 0.

**P7 · Una aplicación real encima**
- Qué:
  - un backend de verdad, uno nuestro: drivers (node-postgres, psycopg, JDBC, Prisma), migraciones y pool;
  - un **soak de días**: transacciones 24/7, dormir de noche, despertar, escalar y una actualización de nodos con migración en vivo.
- Hecho cuando:
  - pasan N días sin un error atribuible;
  - los SLOs están medidos: commit, disponibilidad y despertar.

**P8 · Operarlo sin miedo**
- Qué:
  - PITR (una rama en un instante) y la retención;
  - métricas y alertas: el retraso de safekeeper a pageserver, las subidas a GCS, la salud del cómputo;
  - actualizaciones en rodaje: el almacenamiento por generaciones, el cómputo por migración;
  - las pruebas de fuego (C4 y las demás), periódicas en el CI;
  - runbooks.
- Hecho cuando:
  - un **game day** sobre el soak de P7 (matar el pageserver, un safekeeper y un nodo) da RPO 0 y un RTO medido;
  - una actualización del almacenamiento pasa sin que el cliente lo note.

**P9 · El producto alrededor**
- Qué:
  - la medición: cómputo·s por CU, bytes, historia y salida;
  - las cuotas por organización y por proyecto;
  - la consola: proyectos, ramas, endpoints, la cadena de conexión, el editor SQL y las métricas;
  - la documentación.
- Hecho cuando: una organización se da de alta, crea su base y conecta su aplicación **sin nosotros**.

**⛔ La puerta de producción**
- Qué hace falta:
  - cuota de CPU (≫ 12) y nodos de 16–32 vCPU (B.3);
  - cuota de SSD, que hoy está llena;
  - **los safekeepers en zonas distintas**: el clúster es zonal, pero los nodos pueden ser multizona;
  - una revisión de seguridad: aislamiento, TLS y secretos;
  - Postgres al día.
- Hecho cuando: llega el primer cliente externo.

#### Fase II · Sus datos, activos del catálogo

**Q1 · La base en el catálogo, sin copiar**
- Un proyecto o una rama aparece como **foreign database** (0057) y se lee federado (0053).
- Se lee desde un **cómputo de sólo lectura**: el análisis nunca toca el cómputo de escritura.
- Da valor inmediato, sin pipeline.

**Q2 · Publicar tablas**
- CDC lógico → Iceberg, con **marca de agua** (el LSN).
- Entran el snapshot inicial, los updates y deletes, los cambios de esquema y la medida del retraso.
- ⚠️ Dos cosas abiertas:
  - el CDC necesita un cómputo despierto que decodifique: ¿el de la aplicación al despertar, o uno propio?
  - mientras duerme, la ranura retiene WAL.

**Q3 · Gobierno y linaje**
- Las tablas publicadas tienen linaje y acceso como cualquier activo (0047).

**Q4 · De vuelta**
- Tablas del lago servidas en Postgres, como las *synced tables* de Lakebase.

**Q5 · Ramas unidas**
- Dual-Branching: una rama es un LSN más un snapshot.
- Está aparcado; la marca de agua de Q2 es lo que lo hará posible.

#### P1, por dentro (lo siguiente)

1. **P1·1 · Los forks**
   - `neondatabase/neon` y `neondatabase/postgres` (rama `REL_17_STABLE_neon`), en la organización de GitHub que se decida;
   - la base es `fa504217`;
   - el CI apunta a ellos.
2. **P1·2 · La imagen de cómputo**
   - `compute/compute-node.Dockerfile`, sólo v17;
   - después `vm-builder` (del repositorio `autoscaling`), que da `…/ore/vm-compute-node-v17:<commit>`;
   - se mide.
3. **P1·3 · Sólo 17 y con caché**
   - el almacenamiento sin 14–16, y caché de capas en el registro;
   - se mide en frío y en caliente, con un cambio de una línea en Rust.
4. **P1·4 · El rebase a 17.10+**
   - se trae la etiqueta de upstream a `REL_17_STABLE_neon` y se resuelven los conflictos con los parches de Neon;
   - se compila;
   - pasa `make check` y el suite de regresión de Neon (`test_runner`, en Cloud Build).
5. **P1·5 · Cerrar**
   - la cadencia (B.8, C5) queda escrita como procedimiento;
   - las pruebas de `pruebas-de-fuego/ore-postgres/` dejan de depender de `C:\tmp`: parametrizadas, son la aceptación de P2 y P3.

P1 no gasta cuota: todo va en Cloud Build. Pero P1·1 crea repositorios en GitHub, así que **necesita el go y el dónde**.

#### P1·1 · Los forks (2026-10-07)

- **Los forks**: `describeloai/neon`, `describeloai/postgres` y `describeloai/autoscaling`.
  - Son públicos, con todas sus ramas.
  - La copia local está en `C:/ore-neon/{neon,postgres}`, clonada sin blobs (57 y 133 MB), con su remoto `upstream`.
- **Ni un parche hace falta**: los submódulos de `neon` usan la URL relativa `../postgres.git` y desde el fork resuelven solos a `describeloai/postgres`. El paso `fuente` de la receta lo comprueba y falla si algún Postgres viene de otro sitio.
- **Los commits fijados, protegidos con etiquetas** en los forks, para que no dependan de upstream: `ore/base-fa504217` en `neon` y `ore/v17-base-1e01fcea` en `postgres`.
- **La receta** se mueve a [`ci/neon/almacen.yaml`](../../ci/neon/almacen.yaml): `_REPO` es el fork y `_COMMIT` el commit fijado.
- **⭐ Hallazgo: el fork de Postgres de Neon sí se mantiene**, aunque `neon` no lo recoja:

  | rama | versión | fecha |
  |---|---|---|
  | `REL_17_STABLE_neon` | **17.8** | 2026-04-09 |
  | `REL_18_STABLE_neon` | **18.2** | 2026-04-08 |
  | `REL_17_STABLE_neon_17_6` | 17.6 | 2025-08-20 |

  `neon/main` (fa504217, 2026-08-31) sigue apuntando a 17.5. ⇒ **P1·4 se acorta**: en vez de rebasar 17.5 → 17.10 desde cero, se adopta `REL_17_STABLE_neon` (17.8) y sólo se traen 17.9 y 17.10. Queda por medir que la 17.8 encaje con el almacenamiento de fa504217: la extensión `neon` y el protocolo con el pageserver.
- **Compilado desde el fork**: build `4ee49216`, SUCCESS.
  - Tiempos: **14 min 7 s** en total; fuente 39 s, compilar 12 min 33 s.
  - Los cuatro `vendor/postgres-v*` llegan de `describeloai/postgres`.
  - Imagen `ore/neon:fa504217…`, digest `c3eff1c6…`. Sustituye a la de C1, porque cambia el `BUILD_TAG`.
  - Sin caché, tardó 5 min menos que en C1: es la variación de Cloud Build, no una mejora. La caché se mide en P1·3.
  - Los `fatal: not a git repository` del log son inofensivos: dentro de Docker no se copia `.git`, y el Dockerfile de Neon lo tolera.

#### P1·2 · La imagen de cómputo (2026-10-07)

- **La receta es [`ci/neon/computo.yaml`](../../ci/neon/computo.yaml).** Lo compila todo desde nuestros forks:
  - `compute-node-v17` (`compute/compute-node.Dockerfile`, sólo v17);
  - `neonvm-daemon` y `vm-builder` desde `describeloai/autoscaling`, en la etiqueta que usa el CI de Neon en ese commit (`v0.46.0`, comprobado por la receta), en vez de bajar el binario publicado;
  - `vm-compute-node-v17`, el disco de la VM, que es lo que va en `rootDisk.image`.
- **Primer parche al fork**, `ore/main` = fa504217 + 1 commit: `h3-pg` perdió sus etiquetas en `zachasme/` (404) y vive en `postgis/h3-pg`. El tarball tiene **el mismo sha256**, así que sólo cambia la URL.
  - ⚠️ **Cadena de suministro**: el Dockerfile de Neon baja las fuentes de ~40 extensiones de internet al compilar, y una ya había desaparecido.
  - Pendiente decidir entre dos salidas: guardar las fuentes en nuestro bucket, o reducir el catálogo de extensiones que ofrecemos.
- **Memoria**:
  - con 4 etapas de BuildKit en paralelo, el enlazado con LTO de `compute_ctl`, `fast_import` y `local_proxy` murió por **SIGKILL** (32 GB);
  - Neon lo pone a 1 en su CI, y aquí igual.
- **Medido** (build `153a2e5d`, sin caché, en serie):

  | paso | tiempo |
  |---|---|
  | total | **1 h 10 min** |
  | `compute-node` | 1 h 5 min |
  | `vm-builder` (disco de 2 GB) | 2 min 50 s |
  | `neonvm-daemon` | 1 min 25 s |

  | imagen | tamaño comprimido |
  |---|---|
  | `compute-node-v17` | 0,45 GB |
  | `vm-compute-node-v17` | 0,52 GB |

#### P1·3 · La caché (2026-10-07)

- **Del almacenamiento no se pueden quitar Postgres 14–16.** `libs/postgres_ffi/build.rs` genera los tipos de las cuatro versiones, porque el pageserver entiende el WAL de todas. Quitarlas sería parchear Rust en muchos sitios y pagarlo en cada rebase. Como la capa `pg-build` sólo depende de `vendor/`, la caché la hace casi gratis.
- **La caché es de BuildKit**, guardada en nuestro registro (`…/ore/cache:neon-almacen`, `…:neon-computo-v17`). Se escribe con `buildx` (driver docker-container) y el login sale del token del metadata (`ore-ci`, sin claves).
- **Medido en el almacenamiento** (E2_HIGHCPU_32):

  | caso | total | nota |
  |---|---|---|
  | en frío, sin caché (P1·1) | 14 min 7 s | |
  | R1 · en frío **escribiendo** la caché | 37 min 53 s | 15 min son exportarla (`mode=max` sube la capa de cargo) |
  | R2 · sin cambios | **2 min 20 s** | todo `CACHED` |
  | R4 · una línea de Rust, sólo leyendo | **14 min 14 s** | cargo 6 min, plan/chef 3,5 min, imagen y tar ~2 min; Postgres sí sale de la caché |

  R3 (31 min) no vale: corrió a la vez que R2 y además escribía la caché.
- ⇒ **En el almacenamiento la caché ahorra poco** (Postgres, ~2 min): lo que cuesta es recompilar el workspace de Rust. Bajar de ~14 min pide compilación incremental de cargo, con un `target` persistente. En ORE, sccache dio SIGSEGV. Queda anotado; no se persigue ahora.
- ⇒ **Regla**: la caché se lee siempre y **sólo se escribe con `_CACHE=escribir`**, cuando cambian `Cargo.lock`, Postgres, una extensión o build-tools.
- ⇒ **En el cómputo la caché vale mucho**: Postgres y las extensiones son ~65 min que casi nunca cambian. Medida pendiente: se pobló con la compilación de la 17.10.

#### P1·4 · Postgres 17.5 → 17.10 (2026-10-07)

- **La 17.8 de Neon no sirve tal cual.** El `neon` público está congelado: desde 2025-09 sólo recibe GCS, arreglos del proxy y documentación, y sigue en 17.5. Su fork de Postgres sigue avanzando, pero emparejado con un `neon` que no es público. Hay commits que mueven piezas entre Postgres y la extensión `neon`:
  - `LastWrittenLsnLock`;
  - los hooks `set_lwlsn_block_*`;
  - `neon_storage_token`;
  - `get_pin_limit_hook`;
  - la interfaz de SLRU, ya en su rama 17.6.

  El lado de la extensión de esos cambios no está en fa504217.
- ⇒ **La fusión se hace sólo con upstream**: `REL_17_10` (563 commits: 17.6–17.10, 11 CVE en la 17.10) sobre nuestra 17.5 (`1e01fcea`, 102 commits de Neon). El resultado es `describeloai/postgres` `ore/REL_17_STABLE_neon` = **`b51bab53`**, con la etiqueta `ore/v17.10-b51bab53`.
- **Sólo 2 conflictos:**
  - `walsender.c`: la 17.10 cambia `fullyAppliedLastTime` por `prevWrite/Flush/ApplyPtr`, y en Neon el cuerpo vive en `ProcessStandbyReply` (lo comparte con el safekeeper), así que las variables estáticas pasan allí;
  - `postgres.c`: el `SlotSyncShutdownPending` de upstream va antes del `ProcessInterruptsCallback` de Neon, que hace `goto retry`.
- `neon` `ore/main` = **`8269bece`** apunta el submódulo a `b51bab53`, con `revisions.json` en 17.10.
- **Regresión** ([`ci/neon/postgres-check.yaml`](../../ci/neon/postgres-check.yaml)): `make -k check-world` con `--enable-cassert`, sobre build-tools de Neon, como `nonroot`.
  - Resultado: **la 17.10 y la 17.5 de Neon fallan exactamente igual**:
    - los mismos 108 `resource manager with ID 134 not registered` en `pg_walinspect` y `test_decoding`; el 134 es `RM_NEON_ID`, que registra la extensión `neon`, y aquí corren sin ella;
    - y sus errores en cascada.
  - **Ningún fallo nuevo** por la fusión.
  - Sin TAP: no se configuró `--enable-tap-tests`. La aceptación de verdad del Postgres de Neon es su `test_runner` con `neon_local`, o nuestras pruebas de fuego sobre la imagen.

#### P1·5 · Cerrar P1 (2026-10-07)

- **El procedimiento de mantenimiento está en [`ci/neon/README.md`](../../ci/neon/README.md)**: qué es de quién, las tres recetas con su tiempo y su coste, cuándo se compila, el paso a paso de una versión menor nueva y los riesgos conocidos.
  - Regla nueva de la regresión: se compara con el commit anterior; no basta con que pase. El Postgres de Neon sin su extensión falla siempre igual (los 108 del gestor 134).
- **Las pruebas de fuego ya no dependen de `C:	mp`.**
  - Todo lo variable es `ORE_PG_*` en `entorno.sh`, y el estado vive en `$ORE_PG_TRABAJO`.
  - Los manifiestos son plantillas (`plantilla` se niega si falta un valor).
  - Lo que se hizo a mano en D0b/D0c ahora es código: los clientes (`cliente.yaml`), el tenant, `main` y las ramas (`tenant.sh`), y el endpoint (`vm.sh` con `especificacion.py`: la especificación, la clave Ed25519, el JWKS y el JWT). El token se verifica con el JWKS de la propia especificación (probado en local).
  - Es el embrión de lo que harán el `storage_controller` (P2) y `ore-postgres` (P4).
  - ⚠️ **Sin correr aún contra un clúster**: no hay infraestructura viva. La primera pasada es la aceptación de P2/P3.

**P1 cerrado** (2026-10-07). Las imágenes de `8269bece` (= fa504217 + h3-pg + **Postgres 17.10**) ya están en el registro:
- `neon` (build `5dc9d1a0`, 28 min 15 s escribiendo la caché);
- `compute-node-v17`, con `postgres (PostgreSQL) 17.10` y 97 extensiones;
- `vm-compute-node-v17` (build `ffeedb74`, 1 h 9 min).

⚠️ Esa compilación del cómputo arrancó antes de que la receta tuviera caché, así que **la caché del cómputo sigue vacía**. La próxima compilación del cómputo va con `_CACHE=escribir`.

Siguiente: **P2**. Go del 2026-10-07: crear infraestructura en GCP, 1 nodo n2-standard-2 no-spot con virtualización anidada, y la base del controller en el clúster (como la del IdP).

#### P2·1 · El contrato del `storage_controller` (2026-10-07)

Leído en `8269bece` y probado en local con Docker ([`controlador-local/`](../../pruebas-de-fuego/ore-postgres/controlador-local/)).

- **Su base**: Postgres, en `--database-url` o `DATABASE_URL`. **Aplica solo sus 25 migraciones al arrancar** (diesel). Lo que guarda importa: tenants, shards, **generaciones**, nodos, safekeepers.
- **Modo estricto** (el de producción; `--dev` sólo para pruebas):
  - exige `--control-plane-url`;
  - exige las claves de autenticación, tomadas del entorno:
    - `PUBLIC_KEY` valida a quien le llama;
    - `PAGESERVER_JWT_TOKEN` y `SAFEKEEPER_JWT_TOKEN` son los tokens con que les habla;
    - `CONTROL_PLANE_JWT_TOKEN` es el token para las notificaciones.

  ⇒ **En P2 la capa de almacenamiento va con autenticación**: un par Ed25519 propio del almacenamiento y tokens firmados con él (D0b iba sin autenticación).
- **El plano de control recibe notificaciones**: `PUT {control_plane_url}/notify-attach` (dónde vive cada tenant: `tenant_id`, `shards[{node_id, shard_number}]`, `preferred_az`) y `/notify-safekeepers`.
  - Visto en local: al crear un tenant llega su `notify-attach`.
  - **Es el gancho con que P4 reconfigura los cómputos** cuando un tenant cambia de pageserver.
  - Hasta P4, un receptor mínimo que conteste 200.
- **Los pageservers se registran solos**:
  - con `control_plane_api = 'http://<controller>/upcall/v1/'` en su configuración;
  - y un `metadata.json` en su directorio (`host`, `port`, `http_host`, `http_port`, `availability_zone_id`). Sin zona de disponibilidad no arranca.
  - Al arrancar llaman a `re-attach`. Visto: `GET /control/v1/node` los lista sin que nadie los dé de alta.
- **Los safekeepers no se registran solos**: `POST /control/v1/safekeeper/{id}` (id, region, host, puertos, zona). Sólo hace falta con `--timelines-onto-safekeepers` (por defecto off): con esa opción el controller elige los 3 safekeepers de cada timeline por zona y avisa al cómputo por `notify-safekeepers`. Se decide en P2·4.
- **La API para crear**: `POST /v1/tenant`, `POST /v1/tenant/{t}/timeline` (`new_timeline_id`, `pg_version`; una rama añade `ancestor_timeline_id` y `ancestor_start_lsn`). `DELETE` para borrar.
  - ⚠️ El controller responde al crear el tenant antes de que esté `Active`: en local, el primer `POST …/timeline` dio 409 «Timed out waiting 5s for tenant active state». **Quien lo llame (P4) reintenta.**

**Probado en local:**

| caso | resultado |
|---|---|
| reinicio del pageserver con su disco | reengancha solo, generación +1, ~4 s |
| **pageserver con el disco VACÍO** | ⚠️ **no recupera el tenant**: lo deja así a propósito («Local data loss suspected»). El controller sólo lo arregla con **`--handle-ps-local-disk-loss`** (por defecto off, función de Hadron/Databricks); sin ella el tenant se queda colgado indefinidamente. **Con ella: `Active` en 13,5 s sin intervención, timeline intacto** ⇒ va activada |
| **restaurar una copia vieja de la base del controller** | ⚠️ **reparte de nuevo generaciones ya usadas** (antes de restaurar el pageserver iba por la 8; la copia decía 6; tras restaurar volvió a dar la 8). Es la condición de que dos pageservers crean tener derecho a escribir |
| remedio: antes de arrancar el controller, `update tenant_shards set generation = generation + 1000` | reparte 1010, timeline sano ⇒ **va en el procedimiento de restauración de P2·3**; con safekeepers gestionados, también sus generaciones (`timelines`) |

⇒ **Para P2·4:**
- controller en modo estricto con autenticación y `--handle-ps-local-disk-loss`;
- un receptor de `notify-*` hasta P4;
- `metadata.json` del pageserver generado por pod, con la zona de su nodo;
- reintentos al crear;
- decidir `--timelines-onto-safekeepers`.

#### P2·2 · La infraestructura en GCP (2026-10-07)

Está en [`malla/80-postgres-gcp.sh`](../../malla/80-postgres-gcp.sh), un guion idempotente: lo que ya existe se deja, y para crecer el pool se cambia `NODOS` y se vuelve a pasar.

| recurso | configuración |
|---|---|
| bucket `gs://ore-pg-almacen-euw1` | regional (europe-west1), acceso uniforme, sin acceso público, **borrado suave de 7 días** |
| cuenta `ore-pg-almacen` | `roles/storage.objectUser` **sólo en ese bucket**; `workloadIdentityUser` para `ore-pg/neon`; **0 claves** |
| pool `pg` | 1 × n2-standard-2, **no spot**, virtualización anidada, Ubuntu containerd, `pd-standard` 50 GB (la cuota SSD está llena), etiqueta `ore.dev/pool=neon`, taint `ore.dev/neon`, Secure Boot |

Comprobado:
- el nodo está `Ready`, con `/dev/kvm` y `vmx`;
- cuota **7/12**;
- un pod con la cuenta de Kubernetes `ore-pg/neon` escribe, lee y borra en el bucket, y **otro bucket le niega el acceso** (privilegio mínimo).

El namespace `ore-pg` y su cuenta de Kubernetes se crearon a mano para la prueba; los declarará la malla en P2·4.

#### P2·3 · La base del controller (2026-10-07)

- **[`malla/81-postgres-la-base-del-controlador.yaml`](../../malla/81-postgres-la-base-del-controlador.yaml)**, reconciliado por Flux (en la lista de plataforma de `kustomization.yaml`), declara:
  - el namespace `ore-pg` y la cuenta `neon` (Workload Identity → `ore-pg-almacen`);
  - Postgres `ore/postgres:16` en el pool `pg` (no spot), con 5 Gi en la clase nueva **`retiene-estandar`**: pd-standard y `Retain`, porque `retiene` es pd-balanced y la cuota SSD está llena;
  - un CronJob de copia diaria a las 12:00 a `gs://…-copias/ore-pg/`, con la cuenta `ore-copias` de las copias del IdP.
- **El `Secret` `storcon-db`** (usuario, clave, url) lo crea [`80-postgres-gcp.sh`](../../malla/80-postgres-gcp.sh): `/dev/urandom` directo al Secret, sin imprimirse.
- **Restaurar es [`82-restaurar-la-base-del-controlador.sh`](../../malla/82-restaurar-la-base-del-controlador.sh)**, nunca a mano: para el controller, restaura, **suma 1000** a `tenant_shards.generation` (y a `timelines.generation` si existe) y lo arranca.
- **Probado en vivo:**
  1. un controller temporal (`--dev`) aplicó las **25 migraciones**: 11 tablas;
  2. una fila de prueba con la generación 7;
  3. la copia del CronJob, lanzada a mano: volcado de 15,6 kB, verificado con `pg_restore --list` y subido;
  4. se simulan dos reenganches (generación 9);
  5. se restaura con el guion: **generación 1007**;
  6. el controller arranca sano sobre la base restaurada y ve el tenant en la 1007.

  Después se recogió la prueba: el pod temporal, la fila y el volcado de prueba (para que no fuera «el último» en una restauración real).

#### P2·4 · La capa de almacenamiento en la malla (2026-10-07)

- **[`malla/83-postgres-el-almacenamiento.yaml`](../../malla/83-postgres-el-almacenamiento.yaml)**, reconciliado por Flux, todo en `ore-pg`, en el pool `pg` y con las imágenes de `8269bece` (17.10):

  | pieza | configuración |
  |---|---|
  | `storage-controller` | **modo estricto**, `--handle-ps-local-disk-loss`, 1 réplica en `Recreate` (nunca dos repartiendo generaciones); las sondas son tcp, porque la API pide token |
  | `avisos` | el receptor de `notify-*` hasta P4: contesta 200 y lo apunta |
  | `storage-broker` | — |
  | `safekeeper` ×3 | autenticación pg y http; `--auth-token-path` para hablar entre ellos; WAL a `gs://ore-pg-almacen-euw1/safekeeper/`; disco `retiene-estandar`; PDB `maxUnavailable: 1` |
  | `pageserver` ×1 | la configuración se escribe al arrancar: token de upcall, `metadata.json` con el nombre estable del pod y la zona; NeonJWT en http y pg; `NEON_AUTH_TOKEN` para leer de los safekeepers; capas a `…/pageserver/`; disco `standard`, porque es caché |

- **La autenticación**:
  - un par Ed25519 propio del almacenamiento y 5 tokens, uno por scope: `pageserverapi`, `safekeeperdata`, `generations_api`, `infra` y `admin`;
  - los crea [`80-postgres-gcp.sh`](../../malla/80-postgres-gcp.sh) directos a Secrets, sin imprimirse;
  - `almacen-jwt` (la pública y 4 tokens) es lo que montan las piezas; `almacen-jwt-privada` (la privada y `admin`) no lo monta ninguna: es para P4 y las pruebas.
- **Probado en vivo:**
  - todo `Running` a la primera;
  - la API sin token responde **401**;
  - el pageserver se registró solo (`pageserver-0.pageserver.ore-pg.svc.cluster.local`, `europe-west1-b`, Active);
  - un tenant y su timeline creados por el controller con el token `admin`, al primer intento;
  - el `notify-attach` llegó a `avisos`;
  - 10 objetos en el bucket;
  - **0 errores** en los logs del controller y del pageserver.
- ⚠️ Para P2·6: las pruebas de fuego (`tenant.sh`, `entorno.sh`) hablaban con el pageserver sin token. Ahora van **por el controller con el token `admin`**, y el cómputo necesita un token de scope `tenant`, que acuña quien tenga la privada.

#### P2·5 · Retención y limpieza (2026-10-07)

- **La historia por defecto es de 1 día**, no los 7 que trae Neon: `[tenant_config] pitr_interval = '1 day'` en el pageserver. Es lo que el cliente puede recuperar en el tiempo y lo que se paga en GCS. Por proyecto se cambia en el controller (P9).
  - Efectivo en vivo: `pitr_interval 1day`, `gc_period 1h`, `gc_horizon 64 MB`, `lsn_lease_length 10m`.
  - El cambio reinició el pageserver: es el **primer reenganche en vivo**, y fue limpio (`re-attach`, 0 errores).
- **Borrar un tenant por el controller vacía su prefijo en GCS: 4 → 0 objetos**, con el 404 a los 3,2 s.
  - En D0c esto no pasaba, porque no había controller.
  - ⚠️ Pendiente para P2·6, cuando haya cómputo: **el WAL de los safekeepers** de un tenant borrado (`safekeeper/…`). Con el controller sin gestionar los safekeepers, nadie les dice que lo borren.
  - Lo borrado se queda **7 días en el borrado suave** del bucket: se paga, y es lo que permite deshacer un error.
- **El scrubber, un CronJob diario a las 12:30**, después de la copia:
  - `scan-metadata --post` le cuenta al controller la salud de cada tenant;
  - `pageserver-physical-gc --mode full --min-age 24h` borra índices de generaciones viejas y capas que nadie referencia;
  - va con su propio token de scope `scrubber`, acuñado con la privada (no con `admin`), y con la identidad `neon` para GCS;
  - un bucket sin timelines no cuenta como fallo.
  - Lanzado a mano: **11 s, 1 tenant / 1 timeline, 0 errores, 0 capas huérfanas**, nada que borrar.
  - `find-garbage` (tenants que el plano de control ya no conoce) necesita la API de administración de Neon. No la usamos: lo cubre que borrar por el controller vacía el prefijo.

#### P2·6 · La aceptación (2026-10-07)

- **El cómputo de la prueba va en un pod**, no en una VM ([`computo.yaml`](../../pruebas-de-fuego/ore-postgres/computo.yaml), `ORE_PG_COMPUTO=pod`): es la misma imagen `compute-node-v17` y deja probar el almacenamiento sin NeonVM (P3).
  - Lleva en la especificación un `storage_auth_token` de scope `tenant`, firmado con la privada.
  - Las pruebas de fuego van ahora por el controller con el token `admin` ([`p26.sh`](../../pruebas-de-fuego/ore-postgres/p26.sh)).

| prueba | resultado |
|---|---|
| ① escrituras con autenticación | TPC-B **258–265 tps** con 1 cliente y **511–531** con 4, **0 fallidas**. En pod, sin la VM, rinde el doble que en C3 (120 / 286) |
| ② safekeepers (corte de red real) | 3 vivos: 4,0 commits/s. **1 caído: 3,5/s, sigue**. **2 caídos: se para sin error** (hueco de 34 s). De vuelta: 4,0/s. **657 confirmados = 657 en la base** |
| ③ C4: pageserver **y su disco** | tenant `Active` **sin intervención en 18,1 s** (pod nuevo en 15,3 s), con generación +1 y `--handle-ps-local-disk-loss`. **RPO 0** (264 = 264). Con la caché del cómputo fría, las escrituras esperan al pageserver (hueco de 17,6 s); con caché caliente, 0,9 s. Lectura fría de 1 M filas, correcta |
| ④ reiniciar el controller con escrituras | de vuelta en 3,6 s; **las escrituras ni lo notan** (está fuera del camino de los datos); conserva tenants y generaciones |
| ⑤ borrar el tenant | el controller vacía `pageserver/`, pero **el WAL de los safekeepers queda huérfano**: 317 MB en GCS y 5,6 MB por disco. `DELETE /v1/tenant/{t}` en cada safekeeper lo borra todo (local y GCS) ⇒ **así borra P4** (`tenant.sh borrar`). Tras la prueba, el bucket queda vacío |

**Hallazgos** (cada uno, arreglado o decidido):
1. **El DNS de pod deja conexiones colgadas.** El nombre `pageserver-0.pageserver…` cambia de IP al recrear el pod, y tarda en enterarse: quien conecta a la IP vieja se queda en SYN **~127 s**. El primer C4 parecía de 155 s cuando el pageserver lo resolvió en 24 s.
   - ⇒ **Un Service ClusterIP por pageserver y por safekeeper** (IP que no cambia): con eso se registra el pageserver, eso anuncian los safekeepers (`--advertise-pg`) y eso usa el cómputo.
   - Cambiar la dirección de un nodo exige darlo de baja **y quitar su lápida** (`DELETE /debug/v1/tombstone/{id}`); si no, el re-attach da 409 para siempre.
2. **Cilium no usa `statefulset.kubernetes.io/pod-name` para las identidades.** Una NetworkPolicy que selecciona por ella **no hace nada, en silencio**, y aplicar una política tarda 5–15 s.
   - Revisadas las 76 de ORE: ninguna depende de esa etiqueta.
   - ⇒ **Importa en P3**: el aislamiento entre organizaciones no puede apoyarse en etiquetas de alta cardinalidad.
3. **`--timelines-onto-safekeepers` se queda en off.** En modo estricto exige 3 safekeepers **en 3 zonas distintas**, y el clúster es zonal.
   - El borrado lo cubre P4 llamando a los safekeepers.
   - Se reconsidera en la puerta de producción (safekeepers en 3 zonas): da migración de safekeepers y membresía por generaciones.
4. **Arnés en Windows**: Git Bash reescribe las rutas de los argumentos de `kubectl exec` (`MSYS_NO_PATHCONV=1`), y bash y Python no ven el mismo `/tmp` (rutas `C:/…`).

**P2 cerrado.** El almacenamiento de producción está vivo en `ore-pg`, declarado en git y reconciliado por Flux:
- autenticación en todas las piezas;
- recuperación sin intervención;
- copias y restauración con salto de generaciones;
- retención de 1 día y limpieza diaria.

Queda vivo para P3: el pool `pg` (1 nodo) y la capa de almacenamiento, sin ningún tenant.

#### P3 · El cómputo de producción: el diseño (2026-10-07, revisado)

> **Revisado con el principio del decidido 8**: Postgres es un producto aparte, que existe o no por proyecto y no está enlazado a ningún otro servicio de la organización.
>
> La primera versión metía las VMs en `t-<org>` y en su `ResourceQuota`. Eso acoplaba la base de datos de una aplicación a los análisis de la misma organización: un análisis grande podía impedir que despertara la base, y al revés. **Se descarta.**

**Qué es un endpoint en producción.** Una `VirtualMachine` de NeonVM que arranca nuestra imagen (`vm-compute-node-v17`, 17.10) y vive en **`ore-pg-computo`**.
- Es **un solo namespace para todos los endpoints de todos los proyectos**, propio del producto, igual que `ore-pg` lo es del almacenamiento.
- No toca `t-<org>`, ni `ore-serve`, ni el cofre, ni la cuota de la organización.
- Cada VM lleva etiquetas de proyecto y endpoint, que son para operar y medir, **no para aislar**.

**El aislamiento no necesita fronteras por organización**, porque la regla es universal: **un cómputo no habla con ningún otro cómputo, nunca**. Con un único conjunto de reglas para todo el namespace:

| barrera | regla | dónde |
|---|---|---|
| 1 · almacenamiento | el token de cada cómputo es de scope `tenant`: sólo abre su tenant | en vivo desde P2 |
| 2 · red de pods | `ore-pg-computo` niega todo por defecto. Las VMs (`ore.dev/rol: postgres`) sólo **salen** a `ore-pg` (5454 y 6400) y al DNS, y sólo **entran** desde el proxy y el plano de control. `ore-pg` acepta en los safekeepers y el pageserver sólo lo que llegue de ese namespace. Nada llega a `t-*` ni sale de allí | NetworkPolicy por etiquetas de baja cardinalidad (rol, namespace), nunca por `pod-name` (lección de P2·6) |
| 3 · overlay | un marco pasa sólo si su origen o su destino está en el **rango reservado para el proxy**: **de VM a VM, nada** | un filtro nftables `bridge` en cada nodo (DaemonSet), fuera de la VM |

**Serverless de verdad: sin cuota fija, se paga lo que se usa.**
- **No hay `ResourceQuota` por organización.** Lo que limita a un proyecto lo pone el plano de control (P4): los límites de escalado de cada endpoint (CU mínimas y máximas, que el autoscaler respeta) y los del plan contratado (P9).
- **La medida es el uso real**: CU·segundo de cada VM despierta (el autoscaler y el vm-monitor ya saben cuánto usa) y los bytes del almacenamiento. Dormido, el cómputo cuesta 0 (P6).
- **La capacidad la pone la plataforma, no el cliente.** El pool `pg` lleva el **autoescalado de nodos de GKE**: añade nodos cuando los runners no caben y los quita cuando sobran. Hoy el máximo lo marca la cuota del proyecto (12 vCPU); en la puerta de producción, nodos grandes y cuota alta. Así es como el producto «escala hasta el infinito» sin que el cliente dimensione nada.
- Una `ResourceQuota` **de plataforma** en `ore-pg-computo` queda sólo como cinturón de seguridad contra un error del plano de control. No limita a ningún cliente.

**Todo desde nuestro fork.** Las imágenes de NeonVM y autoscaling (controller, runner, vxlan-controller, daemon, autoscaler-agent, scheduler) y `vm-builder` se compilan de `describeloai/autoscaling` en una sola etiqueta, **v0.49.1** (la de D0b). El cómputo se reempaqueta con ese `vm-builder`, y de paso se llena su caché.

**Los sub-pasos:**

| paso | qué | hecho cuando |
|---|---|---|
| **P3·1 · Imágenes** | `ci/neon/autoscaling.yaml`: las 6 imágenes y `vm-builder` v0.49.1 desde el fork; recompilar `vm-compute-node-v17` con caché | están en el registro; tiempo medido con caché caliente |
| **P3·2 · Nodos** | el pool `pg` con **autoescalado de nodos**: mínimo 1 y máximo 3 (lo que cabe en la cuota: 11/12 en el pico) | GKE añade un nodo cuando una VM no cabe y lo quita al sobrar (medido) |
| **P3·3 · La base en la malla** | cert-manager, Multus para GKE (B.3), whereabouts, NeonVM y autoscaling, vendorizados y preparados (`preparar.py`), en `malla/84-…`–`87-…`, con Flux | D0b·1 se reproduce desde git: la overlay arriba y una VM de prueba arranca |
| **P3·4 · El namespace del producto** | `ore-pg-computo`: deny-all, las políticas de la barrera 2, la cuota de plataforma y la forma de la VM (plantilla con etiquetas de proyecto y endpoint) que usará P4 | una VM llega a `ore-pg`; **no** llega a `t-demo`, al cofre, a `ore-serve` ni a otra VM; desde `t-*` nadie llega a ella |
| **P3·5 · La overlay cerrada** | medir qué viaja de verdad por cada red; el rango del proxy en whereabouts; el filtro de puente | **dos VMs de proyectos distintos no se alcanzan por ninguna red**, en las dos direcciones; un pod del rango del proxy sí |
| **P3·6 · La IP reutilizada** | el ARP viejo de C4 (~1 min): ARP gratuito desde el runner (parche en nuestro fork) o no reutilizar la IP en caliente | recrear una VM con la misma IP y llegar al momento |
| **P3·7 · Aceptación** | las pruebas de fuego con `ORE_PG_COMPUTO=vm`: arranque, escalado, inactividad, migración con sesión por la overlay, C4 con VM, las tres barreras y el autoescalado de nodos | los números de D0b, o mejores, desde git |

**Fuera de P3:**
- quién crea las VMs y fija sus límites: el plano de control, en P4;
- el proxy: P5, con su rango reservado en P3·5;
- dormir, despertar y el pool precalentado: P6;
- la medición y la facturación: P9;
- nodos grandes: la puerta de producción.

#### P3·1–P3·5 · hecho (2026-10-07)

**P3·1 · Imágenes.** [`ci/neon/autoscaling.yaml`](../../ci/neon/autoscaling.yaml) compila de `describeloai/autoscaling` en `v0.49.1` (fijada por commit `aea4f327`):
- el kernel de las VMs (6.12.26 de kernel.org más los parches del fork), etiquetado por el árbol de `neonvm-kernel/`, de modo que si ya está, no se recompila;
- controller, runner (con el kernel dentro), vxlan-controller, daemon, autoscaler-agent y autoscale-scheduler.

**4 min 19 s en total**: el kernel 3 min 40 s y las seis imágenes 36 s. `computo.yaml` usa ahora `vm-builder` v0.49.1 (el CI de Neon usaba v0.46.0), con alpine y busybox fijados por sha, y el daemon de esa receta.

**P3·2 · Nodos.** El pool `pg` autoescala de 1 a 3 nodos ([`80-…sh`](../../malla/80-postgres-gcp.sh), [`nodos.sh`](../../pruebas-de-fuego/ore-postgres/nodos.sh)).
- GKE pide un nodo a los **4,3 s**, y el pod corre en él a los **202,6 s**.
- Lo quita **~12 min** después de quedarse sin pods (perfil BALANCED).
- **Sólo sube un nodo si el pod cabría en uno nuevo**: con 1500m no lo hizo (`no.scale.up.mig.failing.predicate`). Un n2-standard-2 da 1930m, y los DaemonSets de GKE ya reservan 483m.

**P3·3 · La base en la malla.** cert-manager v1.21.2, Multus para GKE, whereabouts, NeonVM y el autoscaling se generan con [`vendorizar.py`](../../malla/postgres-computo/vendorizar.py) (sucesor de `preparar.py`) a partir de las releases, con las imágenes del fork y todo en el pool `pg`.
- Los aplican **tres Kustomizations de Flux encadenados** ([`84-…`](../../malla/84-postgres-la-base-del-computo.yaml)): cert-manager y la red, luego NeonVM.
- Van **fuera de la lista de la malla**: un webhook de cert-manager aún arrancando haría fallar la reconciliación de toda la plataforma.
- **Reservas a lo medido**: lo que va en cada nodo reservaba 471m y usa ~10m; ahora reserva ~100m. El controller y el scheduler, 200m cada uno.
- Una VM arranca desde git (74 s en un nodo nuevo, imagen incluida), escribe 100 000 filas, y el agent la baja a 0,25 CPU y 1 GiB, quitando memoria en caliente.

**P3·4 · `ore-pg-computo`** ([`85-…`](../../malla/85-postgres-el-computo.yaml), [`p34.sh`](../../pruebas-de-fuego/ore-postgres/p34.sh): **15/15**).
- Una VM llega a los safekeepers (:5454) y al pageserver (:6400), y a nada más: ni a la API del pageserver, ni al controller, ni a otra VM, ni a `ore-serve` o el cofre de `t-demo`, ni a la API de Kubernetes, ni a los metadatos de Google, ni a internet.
- A ella llega sólo `ore-pg` con `ore.dev/pg-acceso`. Un pod de otro namespace **con esa etiqueta** no llega; tampoco llega al almacenamiento.
- **La reserva de una VM es su mínimo** (`spec.podResources`). Sin reservas, los runners de NeonVM «caben» en cualquier nodo y **GKE nunca subiría uno por una VM**. Neon usa su propio cluster-autoscaler, que en GKE no se puede poner. Lo destapó la cuota de plataforma, que rechazaba los pods. Por encima del mínimo, dentro del nodo, decide el autoscale-scheduler.
- **Cilium tarda 15–30 s en aplicar un cambio de etiqueta** si el destino está en otro nodo (5–15 s en el mismo, P2·6).

**P3·5 · La overlay cerrada** ([`cerrada.yaml`](../../malla/postgres-computo/neonvm/cerrada.yaml), [`p35.sh`](../../pruebas-de-fuego/ore-postgres/p35.sh): **6/6**).
- **Medido antes:** desde dentro de una VM, su Postgres abría sesión en el de otra por la overlay (`dblink` → OK).
- Quién entra en la overlay: Multus con `namespaceIsolation`. Las NADs salen de `neonvm-system`: la de las VMs a `ore-pg-computo`, la del proxy a `ore-pg` (`overlay-del-proxy`). Un pod de fuera que pide cualquiera de las dos no arranca («namespace isolation enabled, annotation violates permission»).
- Quién habla con quién: `ebtables` en el puente de cada nodo. Una trama pasa sólo si su origen o su destino está en el lado del proxy (`10.100.0.0/17`); las VMs van en `10.100.128.0/17`. Probado con las dos VMs en nodos distintos (VXLAN).

**Deudas de P3** (cada una, con su por qué):
1. ~~**Una VM puede falsificar un origen del lado del proxy.**~~ **Cerrada en P3·6**, y era peor de lo que decía aquí. Con un ARP que diga «la IP del proxy soy yo», que el puente dejaba pasar, una VM con root podía desviar hacia ella las **respuestas** que otra VM manda al proxy. Ahora el runner de cada VM sólo deja salir **su** IP y **su** MAC (ver P3·6).
2. **La migración en caliente va de runner a runner por la red de pods** (:20187), así que la barrera 2 abre ese puerto entre VMs. Un huésped podría alcanzarlo por el NAT de su runner, y sólo escucha mientras hay una migración entrante.
3. **Imágenes de terceros sin espejo:** cert-manager, Multus, whereabouts y el device plugin (éste, fijado por digest). Se espejan en la puerta de producción.

#### P3·6 · La IP reutilizada y la anti-suplantación (2026-10-07)

**El problema, medido** ([`p36.sh`](../../pruebas-de-fuego/ore-postgres/p36.sh)):
- Al recrear una VM, el IPAM de NeonVM le da la IP **más baja libre**, que es la suya de antes, con **otra MAC**.
- Responde por la IP del pod a los 32–35 s (su arranque real). Por la overlay, **~10 s después**: el lado del proxy aún tiene la MAC vieja en su caché ARP.

**El arreglo: la VM se anuncia al nacer.**
- Un comando `sysinit` en la imagen (nuestro fork de `neon`, `baad49aa`) envía un ARP gratuito con la IP que NeonVM le pone en la línea del kernel (`ip=…:eth1:off`).
- **Medido**, haciendo eso mismo a mano en cuanto el huésped arranca: la overlay responde **1,7 s** después de la IP del pod, frente a ~10 s. Ese 1,7 s es el coste de la propia sonda (`kubectl exec` + `psql`).

**Y lo que destapó: la anti-suplantación.**
- Dejar pasar el ARP gratuito obligó a mirar quién puede decir qué en la overlay. La regla de P3·5 dejaba pasar cualquier ARP cuyo origen dijera ser del lado del proxy, y eso lo puede escribir una VM con root.
- Se probó desde dentro de una VM, con un socket crudo en Perl ([`garp.pl`](../../pruebas-de-fuego/ore-postgres/garp.pl)). La trama falsificada **salía del runner hacia el puente**; no se vio envenenar la caché de la otra VM, pero el porqué no se entendió, y eso no es una garantía.
- ⇒ **Filtro dentro del runner de cada VM** ([`cerrada.yaml`](../../malla/postgres-computo/neonvm/cerrada.yaml), ①): la `tap` de la VM sólo emite IPv4 y ARP con **su IP y su MAC** (las de la línea de comandos de su QEMU); lo demás se tira, IPv6 incluido. El DaemonSet entra en el netns del runner por el pid de QEMU, cada 5 s.
- **Medido** con contadores:
  - 3 ARP con la IP del proxy → 3 a `DROP`;
  - 3 con la IP de la otra VM → 3 a `DROP`;
  - 3 gratuitos con la suya → 3 aceptados;
  - el tráfico normal sigue.
- ⚠️ **ebtables-nft ignora `-P DROP` al crear una cadena**: quedaba en `RETURN` y lo falsificado pasaba (los contadores lo enseñaron). Va un `-j DROP` explícito.

**Con la imagen nueva (`baad49aa`) y el runner `v0.49.1-ore.1`, medido con `p36.sh 2`:**
- la overlay responde **1,6 s y 1,5 s** después de la IP del pod (antes, ~10 s);
- la MAC **ya no cambia** al recrear (`02:4f:52:45:80:00` antes y después): el runner la saca de la IP (P3·7, hallazgo 1), así que la caché ARP del proxy sigue valiendo. El ARP gratuito queda como segunda capa;
- arranques de 70 s (la primera vez en el nodo, bajando la imagen nueva) y 33 s.

#### P3·7 · Aceptación con VMs (2026-10-07, hecho)

Con la imagen de cómputo `8269bece` (todavía sin el anuncio ARP) y NeonVM `v0.49.1-ore.1`, con **todas las barreras puestas**. Los guiones viejos ya llevan las VMs en `ore-pg-computo` (`kc`).

| prueba | resultado | D0b (sin barreras) |
|---|---|---|
| arranque ×3 (`arranque.sh`) | **32–35 s** del `apply` a la primera consulta; pod con IP en 4,1–4,4 s | 15,8–17 s |
| escalado en caliente (`escalado.sh`) | sube a 1 CPU y 3 GiB en **8 s**; 4 311 tps de lectura (8 clientes); baja a 0,25 CPU y 1 GiB en ~2 min 50 s; **la sesión que escribe cada 0,2 s no se corta** (2 294 filas; mayor hueco 4,4 s, con la VM saturada) | 9 s / ~3 min |
| inactividad (`inactividad.sh`) | `last_active` salta con 5 s de un rol de aplicación y se queda quieto 90 s sin consultas: la señal de P6 sirve a través de la barrera 2 | igual |
| migración en caliente (`migrar.sh`) | `Succeeded` entre nodos; **la sesión por la overlay sobrevive** (503 filas, mayor hueco 1,49 s); la de la IP del pod se cuelga (esperado, B.6) | hueco 0,75 s |
| C4 con VM (`p26.sh c4`) | **RPO 0** (81 = 81); tenant `Active` sin intervención a los **39,2 s** (el pageserver nuevo, con disco nuevo, a los 38 s); lectura en frío de 2 M filas correcta | ~21 s (a mano) |
| escrituras (`p26.sh escrituras`) | TPC-B **130 tps** con 1 cliente y **231** con 4, 0 fallidas. En pod: 258/511 | C3: 120/286 |

**Hallazgos de P3·7:**
1. **La migración rompía la overlay con el filtro de P3·6.**
   - NeonVM da una MAC **aleatoria a cada runner**; tras migrar, el huésped conserva la suya y el runner de destino recibe otra. El filtro, que leía la MAC de la línea de QEMU, lo tiraba todo («No route to host»).
   - Además QEMU anuncia la VM migrada con **RARP**, que el filtro no dejaba pasar.
   - ⇒ **Parche en nuestro fork** ([`describeloai/autoscaling` `v0.49.1-ore.1`](https://github.com/describeloai/autoscaling/tree/v0.49.1-ore.1)): la MAC de la overlay **sale de la IP** (`02:4f:52:45` + los dos últimos octetos). Es estable para toda la vida de la VM e igual para cualquier VM que tenga esa IP: **misma IP, misma MAC**, y la caché ARP del proxy nunca queda vieja.
   - El filtro calcula la MAC de la IP (no la lee de fuera) y deja pasar el RARP sólo con esa MAC.
2. **El arranque es el doble de lento que en D0b, y es disco.** El runner copia un `rootdisk.qcow2` de **1,6 GB** en cada arranque. El disco del nodo (pd-standard 50 GB) da ~125 MB/s ⇒ ~13 s sólo en la copia. El huésped arranca con 9,7 s parado tras `udevd`, y `compute_ctl` → `running` tarda 8,3 s (`sync_safekeepers` 5,8 s). D0b usó el disco por defecto de GKE (pd-balanced, SSD). Es un dato, no una deuda: el arranque en frío deja de estar en el camino del cliente cuando hay un pool de VMs ya arrancadas, que es P6.
3. **C4 con VM: las escrituras paran 68 s**, ~29 s más que el tiempo hasta `Active`. En pod el hueco coincidía con la caída. Parece el backoff de reconexión del cómputo al pageserver tras una caída larga. Es un dato.
4. **Arnés:**
   - `escalado.sh` pasaba los `insert` a `kubectl exec … psql` **sin `-i`**: psql no recibía nada y salía bien, así que el «0 cortes» de B.5 no medía nada. Ahora la sesión corre dentro del clúster (`nohup` en `cliente`), porque un corte de la red de quien lanza la prueba también la mataba.
   - `kubectl cp` no entiende rutas `C:/…`.
5. **La compilación del cómputo no puede tener una sesión de buildx que escriba en el registro durante más de una hora**: la credencial del metadata caduca, y buildkit la pide al abrir la sesión y la guarda (dos compilaciones de ~1 h 30 perdidas). `computo.yaml` construye lo largo en sesiones sin exportar y deja la subida de la caché para una sesión corta.

6. **Con la imagen `baad49aa`**, otra vez: `p34.sh` (la barrera 2 aguanta) y `p35.sh` (6/6, la overlay cerrada), y `p36.sh` (arriba, en P3·6).
7. **La compilación en frío con las sesiones partidas** (`f59c0832`): 1 h 32 en total, y esta vez **sin el 401**. Extensiones 53 min 48 s, `compute-tools` 5 min 8 s, imagen final y subida de la caché 27 min 25 s, y `vm-builder` 2 min 53 s. La caché quedó escrita.

**Hallazgo para P3·6:** al borrar una VM, su runner sigue vivo unos segundos con **la misma IP de la overlay**, y contestaba el `select 1` de la VM nueva. Los «arranques de 3,5 s» eran eso. `vm.sh` ahora espera a que se vaya; el plano de control (P4) tendrá que hacer lo mismo.


#### P4 · El plano de control: el diseño (2026-10-07)

> Con lo aprendido de **Lakebase** (Databricks, Neon por dentro): el proyecto pertenece al workspace y su API cuelga del host del workspace, con su OAuth y sus permisos; lo que corre es un servicio serverless aparte. En ORE **la celda es el workspace**: lo que el cliente aprovisiona.

**El decidido 8, precisado.** Postgres está **integrado en la identidad y la propiedad** y **aparte en el runtime**.
- La identidad y la propiedad salen del plano de control común, como en cualquier otro servicio de ORE: el mismo login (0048), `ore-iam` decide (0047), el dueño es una persona (0052).
- El cómputo, el almacenamiento, la capacidad, la medida y la facturación son del producto. Ni la cuota ni los recursos de la celda los tocan.
- **Si la celda o `ore-iam` caen, se para la gestión, no los datos**: conectar, consultar, escalar y dormir no pasan por ellos.

```
persona ──token del IdP──► ore-serve (su celda) ──WI de la celda──► ore-postgres ──► storage_controller, safekeepers
                           /v1/postgres/…                           org = la de la celda   NeonVM (ore-pg-computo)
                           puede / hizo / quien (ore-iam, 0047)     estado + reconciliador  compute_ctl /configure
aplicación ──────────────────────────────── (P5: proxy) ─────────────────────────────────► VM ──► almacenamiento
```

**Lo que se reutiliza, y nada se inventa:**

| necesidad | pieza de ORE que ya existe |
|---|---|
| quién eres, sin segundo login | ORE IdP (0048): el token que ya trae la persona a su `ore-serve` |
| ¿puede?, ¿qué hizo? | `ore-acceso` (el PEP de 0047): `puede` → `ore-iam`, `hizo` → `iam.huella` |
| de quién es | `quien` → `owner: user:<handle>` (0052) |
| la celda ante `ore-postgres` | el token de Workload Identity de su `ore-serve` (el primero de los dos tokens de 0047), verificado con las llaves de Google que ya trae `68-las-llaves-de-las-celdas`. **La organización no viaja nunca**: sale de la celda |
| la base del estado | el Postgres de `ore-pg` (`storcon-db`), con su copia diaria: una base más, `ore_postgres` |
| el login con identidad de ORE (después de P4) | `pg_session_jwt`, que ya va en nuestra imagen de cómputo, con el JWKS del realm de `50-jwks` |

**El modelo** (como Lakebase: ids que pone el usuario, `[a-z0-9-]{1,63}`, inmutables):

```
/v1/postgres/proyectos/{p}                      → un tenant
  /ramas/{r}                                     → un timeline (de otra rama: en su punta, en un LSN o en un INSTANTE)
    /endpoints/{e}   (lectura-escritura | lectura) → una VM en ore-pg-computo, con su especificación
    /roles/{rol}                                 → dentro de la especificación (el estado de los roles es de cada rama)
    /bases/{b}
  /operaciones/{id}                              → toda creación, cambio o borrado
```

- **Al crear un proyecto** nacen la rama `main`, su endpoint de lectura-escritura y **un rol para quien lo crea** (su handle), dueño de la base por defecto.
- **Operaciones largas con id.** Toda escritura devuelve `{operacion, hecha: false}`; se sondea hasta `hecha: true`. **Una operación en curso por proyecto**: otra devuelve **409** («no se aceptó»). Repetir la misma petición, con el mismo id de recurso, no crea dos cosas.
- **Contraseñas sin cofre**: se generan, se enseñan **una vez** y sólo se guarda el verificador SCRAM. Es justo lo que el proxy de P5 pedirá al plano de control. Se pueden regenerar, nunca leer.
- **Potestades nuevas** en el catálogo de `ore-iam`: `postgres:ver`, `postgres:usar` (conectarse, roles) y `postgres:gestionar` (proyectos, ramas, endpoints), como `CAN_USE` / `CAN_MANAGE` de Lakebase.

**`ore-postgres`** (`crates/ore-postgres`, en `ore-pg`; uno por región):
- **El estado**: proyectos (con su organización), ramas, endpoints, roles (sólo el verificador), bases y operaciones. Cada fila lleva lo **deseado** y lo **observado**.
- **El reconciliador** lleva lo observado hacia lo deseado, paso a paso y de forma idempotente. Lo que hoy hacen a mano los guiones de la prueba, pasa a hacerlo él:

  | hoy, a mano | en `ore-postgres` |
  |---|---|
  | `tenant.sh` | tenant y timelines por el `storage_controller`; rama en un instante con el `get_lsn_by_timestamp` del pageserver |
  | `especificacion.py` | la especificación y el token de tenant (con la privada del almacenamiento) |
  | `vm.sh` | la `VirtualMachine` y su ConfigMap en `ore-pg-computo`, con una cuenta cuyo RBAC sólo alcanza ese namespace; **espera a que el runner viejo se vaya** (P3·5) |
  | `tenant.sh borrar` | el tenant, más `DELETE` en cada safekeeper (P2·6) |
  | `avisos` (el stub) | los avisos del `storage_controller`: si un tenant cambia de pageserver, se reconfigura su cómputo (`compute_ctl /configure`) |

- **Un solo cómputo de escritura por rama, con cerco**, en tres capas:
  1. en la base: una restricción única (rama, lectura-escritura);
  2. en el reconciliador: el endpoint lleva una **generación**; la VM nueva no se crea hasta que la vieja y su runner no existen;
  3. en el almacenamiento: los términos de los safekeepers, de modo que un proponente viejo pierde la votación.
- **Las llaves**: un par Ed25519 propio para hablar con cada `compute_ctl` (su JWKS va en la especificación) y la privada del almacenamiento para acuñar los tokens de tenant. Las dos en Secrets de `ore-pg`; nunca en el repositorio.

**Cómo pregunta `ore-postgres` a `ore-iam` de quién es una celda** (decidido en P4·1). `ore-postgres` reenvía tal cual el token que la celda le presentó a `POST /access/v1/celda` de `ore-iam`. Ese token lleva la audiencia **`ore-postgres`**, no la de `ore-iam`. `ore-iam` lo verifica entero, con las mismas llaves y el mismo emisor que las celdas y una audiencia por producto (`--audiencias-productos`), y contesta la celda y su organización.
- No hace falta una cuenta de Google para `ore-postgres` ni un segundo registro de celdas.
- Con la audiencia del producto, el token sólo sirve para eso. Con la de `ore-iam`, `ore-postgres` podría preguntar `puede` y contar `hizo` como si fuera la celda.
- La respuesta se guarda lo que diga `vale` (≤ 30 s) y nunca más allá del `exp` del token. Sin `ore-iam`, 503.

**Fuera de P4**: la entrada pública (el proxy, P5); dormir, despertar y el pool (P6); la medida y la facturación (P9). P4 deja anotados los cambios de estado de cada endpoint, que es lo que P9 medirá.

**Los sub-pasos:**

| paso | qué | hecho cuando |
|---|---|---|
| **P4·1 · El contrato y el esqueleto** | la API (rutas, cuerpos, errores, operaciones) escrita aquí; `crates/ore-postgres` con su base `ore_postgres` y la verificación de la celda | una celda de prueba crea y lee un proyecto **vacío**; otra celda no lo ve |
| | ↳ **escrito y probado** (2026-10-07, `09cdfc06`): `crates/ore-postgres` (proyectos y operaciones; migración `001` dentro del binario; base `ore_postgres` creada en `storcon-db` por `malla/86-…sh`, con su copia diaria), `POST /access/v1/celda`, la imagen y el CI. El contrato contra un Postgres de verdad (`tests/contrato.rs`, también en CI): otra organización no ve, no lee y no borra; el mismo id en dos organizaciones; repetir es un 409; la segunda operación en curso la para la base. **Falta en vivo**: desplegar (CI en rojo por una prueba ajena a esto, así que `:main` no se ha movido) y `p41.sh` con dos celdas de verdad (demo y victor). | |
| **P4·2 · Proyectos y ramas** | tenant y timelines por el `storage_controller`; rama en la punta, en un LSN o en un instante; borrar | crear y borrar dejan el bucket y los safekeepers como estaban (medido) |
| **P4·3 · Endpoints** | la especificación en Rust (sustituye a `especificacion.py`); la VM en `ore-pg-computo`; el cerco | un endpoint responde; **un segundo de escritura en la misma rama es imposible** (probado a la vez, no en serie) |
| **P4·4 · Roles y bases** | SCRAM, la contraseña una vez, `compute_ctl /configure` | un rol creado por la API se conecta; regenerar su contraseña invalida la vieja |
| **P4·5 · Los avisos del almacenamiento** | `ore-postgres` sustituye a `avisos` (`--control-plane-url`) | C4 con VM: el cómputo se reconfigura solo |
| **P4·6 · La API de ORE** | `/v1/postgres/…` en `ore-serve` con `ore-acceso`; las potestades en las migraciones de `iam`; el dueño por `quien` | una persona crea proyecto, rama, endpoint y rol **con su token de ORE** y se conecta desde la malla; sin la potestad, 403; con `ore-iam` caído, 503 en la gestión y la base sigue sirviendo |
| **P4·7 · Aceptación** | las pruebas de fuego por la API, ya sin guiones de tenant ni de VM | borrarlo todo no deja huella: ni filas, ni VMs, ni ConfigMaps, ni prefijos en GCS, ni WAL |

**P4 cerrado** (2026-10-08). `ore-postgres` vive en `ore-pg` (malla/86), el `storage_controller` le avisa (malla/83) y `ore-serve` lo expone en `/v1/postgres` con las potestades `postgres:ver|crear|usar` (todo miembro, el estándar de Databricks y Neon) y `postgres:gestionar` (ORGADMIN y ACCOUNTADMIN; el dueño gestiona lo suyo). Desde un puesto, nada: un producto no sabe del otro. La consola (`rubix-platform`) ya crea instancias, ramas y conexiones de verdad.

Lo medido y lo aprendido:

- **El cerco, capa 3** (`p434.sh`, tres veces con un escritor sin parar y un segundo cómputo de escritura por fuera del API): nunca sirven los dos al final y **no se pierde ninguna fila confirmada**; el que queda tiene las de los dos, una sola historia. Unas veces gana el intruso, otras el principal, que se reinicia («crash of another server process») y sigue.
- **Reiniciar el pageserver** con un escritor en marcha: hueco de 38 s, nada perdido; con un solo pageserver no hay aviso (mismo nodo).
- **`/configure` pide `{spec, compute_ctl_config}`**, no sólo `spec`: 422 en vivo, que el doble del contrato no veía (29cc5576). Desde entonces el contrato con `compute_ctl` se prueba contra el de verdad.
- **NeonVM exige `guest.cpus` numérico**; el `DELETE` de un timeline que ya no está da 200 `null` (se confirma con un `GET`).
- **El aviso del controller llega firmado** (`scope: infra`) y se verifica con la pública del almacenamiento.
- **Con el plano parado, la base sigue sirviendo** (medido en vivo); la gestión da 503.
- **La Kustomization `malla` tiene `prune: false`**: lo que se quita del repositorio no se borra solo (el stub `avisos` se borró a mano).
- En los guiones: `gcloud` en Git Bash se rompe con `MSYS_NO_PATHCONV=1`; `psql` sin `-q` imprime la etiqueta; un escritor de N filas fijas puede acabar antes de que nazca lo que se quiere medir.

#### P5 · La entrada: el diseño (2026-10-08)

**El nombre**: `ep-<id>.europe-west1.pg.paladio.io` (comodín `*.europe-west1.pg.paladio.io`). La región va en el nombre, como en Neon, para abrir otra sin cambiar las cadenas de nadie. `paladio.io` vive en GoDaddy; `pg.paladio.io` está delegada a Cloud DNS (zona `pg-paladio-io`, 2026-10-08, comprobada con un TXT desde 8.8.8.8 y 1.1.1.1).

**El proxy es el de Neon** (Rust, el que sirve Neon en producción), ya compilado en nuestra imagen del almacenamiento (`proxy`, `pg_sni_router`), con `--auth-backend control-plane` contra **nuestra** API. Su contrato, leído en el fork (`proxy/src/control_plane/client/cplane_proxy_v1.rs`, `baad49aa`), son dos `GET` con `Authorization: Bearer <jwt>`:

| llamada | entra | sale |
|---|---|---|
| `…/get_endpoint_access_control` | `endpointish` (la primera etiqueta del SNI), `role` | `role_secret` (el verificador SCRAM tal cual, el de P4·4), `allowed_ips`, `block_public_connections`, límites; 404 = no hay tal rol |
| `…/wake_compute` | `endpointish` | `address` (`ip:puerto` en la overlay), `aux` (ids para métricas) |

Nuestras VMs ya se llaman `ep-<20 hex>`: el convenio de Neon.

**Lo que se construye**:

- Postgres nativo en el 5432 y **SQL por HTTP y WebSocket** en el 443 (el driver serverless de Neon);
- TLS 1.3 con el comodín recargado en caliente; SCRAM-SHA-256 con *channel binding*;
- el endpoint `-pooler` (el pgbouncer del cómputo) para miles de conexiones cortas;
- IPs permitidas y bloqueo público por endpoint; límite de intentos por IP y por endpoint;
- al menos 2 réplicas, PodDisruptionBudget, parada que vacía, autoescalado por conexiones;
- un balanceador L4 de paso directo (la IP del cliente llega tal cual, sin protocolo PROXY).

⚠️ El clúster es **zonal** (`europe-west1-b`): dos réplicas cubren un pod o un nodo, no la zona. Regional es P8.

**Los sub-pasos:**

| paso | qué | hecho cuando |
|---|---|---|
| **P5·1 · El contrato del proxy** | `ore-postgres` sirve las dos llamadas; el token del proxy en un Secret | contra el proxy de verdad en el clúster (sin entrada pública todavía): `psql` por el proxy entra con la contraseña del rol y no con otra; un endpoint que no existe, un rol que no existe y otra organización, fuera |
| **P5·2 · El nombre y el certificado** | IP estática regional; cuenta de servicio con `dns.admin` **sólo** sobre `pg-paladio-io`, por Workload Identity y sin claves; ClusterIssuer de Let's Encrypt por DNS-01; el comodín | `Certificate` `Ready`; `*.europe-west1.pg.paladio.io` resuelve a la IP; renovar no pide a nadie |
| **P5·3 · El proxy en la malla** | Deployment en el pool `pg` con pata en la overlay (`overlay-del-proxy`, `10.100.0.0/17`); Service `LoadBalancer` en el 5432; NetworkPolicies; PDB | desde internet, `psql "postgres://…@ep-….europe-west1.pg.paladio.io/…?sslmode=verify-full"` entra; pgbench sin fallos; reiniciar una réplica no tira a la otra |
| **P5·4 · SQL por HTTP y WebSocket** | el 443 del proxy | `@neondatabase/serverless` consulta desde internet |
| **P5·5 · El pooler** | `ep-…-pooler` → pgbouncer del cómputo | pgbench con una conexión por transacción, sin fallos |
| **P5·6 · Quién puede entrar** | IPs permitidas, bloqueo público y límites, desde la API (`postgres:gestionar` o el dueño) | una IP fuera de la lista no entra; la fuerza bruta se corta |
| **P5·7 · Connect, con el nombre de verdad** | la consola enseña `ep-….europe-west1.pg.paladio.io` y `sslmode=verify-full` | se copia el snippet y conecta desde fuera |
| **P5·8 · Aceptación** | todo lo anterior, junto | una migración en vivo **no corta** una sesión que entra por el proxy; pgbench desde internet sin fallos |

#### P5 · En el laboratorio local (2026-10-08)

El 2026-10-08 a las 18:38 UTC la cuenta de facturación del proyecto quedó **cerrada**: el clúster, el registro de imágenes y el CI se pararon («This API method requires billing to be enabled»). P5 es **la primera P de la saga que se construye en un laboratorio local**, en Docker y sin depender de Google, para llegar a producción con todo hecho y medido y que el día que Google vuelva sólo quede desplegar y probar lo que sólo se puede probar allí.

**El laboratorio** (Docker Compose en `pruebas-de-fuego/ore-postgres/lab/`):

- `ore-postgres` de verdad, contra un Postgres local: proyectos, endpoints y roles nacen por su propia API;
- un «cómputo» `postgres:17` con el mismo rol y el mismo verificador SCRAM: el proxy hace el SCRAM con el cliente y entra en el cómputo con las mismas claves, así que un Postgres sin tocar sirve (es lo primero que se mide);
- el proxy de Neon, de **nuestra** imagen (`neon:8269bece`, ya en local), con un certificado autofirmado de `*.europe-west1.pg.paladio.io`; el cliente entra por SNI;
- un pgbouncer delante del cómputo, para el pool.

**Qué se hace en el laboratorio y qué no:**

| paso | en el laboratorio | sólo en producción |
|---|---|---|
| P5·1 · el contrato | entra con su contraseña y no con otra; rol, endpoint u organización ajenos, fuera; arrancando, el proxy reintenta | lo mismo contra VMs en la overlay |
| P5·2 · el nombre y el certificado | el guion de GCP escrito y revisado | la IP, la cuenta de DNS, el comodín de Let's Encrypt |
| P5·3 · el proxy en la malla | la malla escrita y validada contra el esquema de Kubernetes | desplegarla; el balanceador; `psql` y pgbench desde internet |
| P5·4 · HTTP y WebSocket | `@neondatabase/serverless` consulta por los dos | lo mismo desde internet |
| P5·5 · el pooler | pgbench con una conexión por transacción | contra el pgbouncer de la VM |
| P5·6 · quién entra | IPs permitidas, bloqueo público, límites | con la IP real del cliente tras el balanceador |
| P5·7 · Connect | en modo banco | el snippet copiado conecta desde fuera |
| P5·8 · aceptación | — | la migración en vivo y pgbench desde internet |

#### P5·1 · Medido en el laboratorio (2026-10-08)

`lab/p51.sh` contra el proxy de Neon de verdad (`8269bece`), con `ore-postgres` de `main`, `verify-full` sobre el certificado autofirmado y el SNI de `ep-….europe-west1.pg.paladio.io`: **todo en verde, dos veces seguidas y desde cero**.

- **El SCRAM pasa de punta a punta con un Postgres sin tocar**: el proxy hace el SCRAM con el cliente usando el verificador que le da `ore-postgres` y entra en el cómputo con las mismas claves. La contraseña no la ve nadie más que el cliente.
- **Fuera lo que debe quedar fuera**: otra contraseña, un rol que no existe, un endpoint que no existe y la contraseña de una organización en el endpoint de otra (mismo proyecto y rol, `demo` y `victor`) dan todos el mismo `password authentication failed`, que no dice qué existe. Sin SNI, el proxy no sabe a qué endpoint va y lo dice.
- **Arrancando**: con `RUNNING_OPERATIONS` el proxy reintenta `wake_compute` 8 veces (~7 s) y entra si el cómputo está listo a tiempo. Una VM tarda ~35 s, así que esperar lo que haga falta es P6.
- **Hallazgo: la caché del proxy.** El proxy guarda el verificador cuatro minutos (`--project-info-cache ttl=4m`). Tras un *Reset password*, la contraseña nueva tardó **231 s y 302 s** en entrar. Se resuelve como en Neon, **avisándole por Redis**: `ore-postgres --redis` publica en `neondb-proxy-ws-updates` un `/project_settings_update` del proyecto cuando el reconciliador da por `hecha` una operación suya, es decir, cuando el cambio ya está en el cómputo y no antes (`olvidar.rs`). Con eso la nueva entra **a la primera** y la vieja ya no; las métricas del proxy cuentan cada olvido (`invalidate_project`). Sin Redis no se rompe nada: la caché caduca sola.
- **`project_id` es el tenant**: el proxy agrupa por él lo que guarda y lo que olvida, y el id del proyecto se repite entre organizaciones. `account_id` es la organización.

El laboratorio hace de reconciliador (`lab.sh reconcilia`: el rol con su verificador en el cómputo, la fila `listo` con su dirección, la operación `hecha`) porque no hay NeonVM. Ese tramo ya está medido en el clúster (P4·3–P4·7).

#### P5·4 · HTTP y WebSocket, medido en el laboratorio (2026-10-09)

`lab/p54.sh`: `@neondatabase/serverless` 1.1.0 desde Node 22, contra el mismo proxy, que sirve las dos puertas en 443 (`--wss`) con el mismo certificado. **Todo en verde, dos veces desde cero, sin tocar `ore-postgres`**: el proxy autentica igual que en P5·1, con el verificador que le da `ore-postgres`.

- **HTTP** (`neon()`, un `POST /sql` por consulta): consulta con parámetros, una transacción de tres sentencias en una sola petición; ~11 ms por consulta en el laboratorio.
- **WebSocket** (`Pool`, el protocolo de Postgres dentro de `wss://…/v2`): sesión de verdad (el mismo `pg_backend_pid`), `begin`/`rollback` que deshace; ~2 ms por consulta con la conexión abierta.
- **Fuera**: otra contraseña, por las dos puertas, y la contraseña de una organización en el endpoint de otra.
- **Hallazgo: `api.`** Desde la 1.0 el driver no manda el HTTP al nombre del endpoint, sino a **`api.europe-west1.pg.paladio.io/sql`**, y el endpoint viaja en la cabecera `Neon-Connection-String`. El comodín del certificado y del DNS (P5·2) lo cubren; lo que no hay que hacer es un registro por endpoint ni un certificado sin comodín.
- **El token del proxy, con `=`**: `--control-plane-token="$(…)"`. Un token que empieza por `-` (pasa con `token_urlsafe`) se leía como otro flag y el proxy no arrancaba.

#### P5·5 · El pool, medido en el laboratorio (2026-10-09)

**Como en Neon, el pool vive en la VM.** La imagen de cómputo que ya construimos (`vm-image-spec-bookworm.yaml`) arranca un pgbouncer 1.24.1 en el **6432**, en modo `transaction`, con `default_pool_size=64` y `max_client_conn=10000`. Autentica por SCRAM con `auth_user=cloud_admin`: lee el verificador del rol en Postgres, y el proxy entra con las mismas claves que en P5·1. No hay un pgbouncer aparte que desplegar, escalar ni vigilar.

- **`ep-…-pooler`** es la misma VM por otra puerta: `wake_compute` contesta la dirección del cómputo en el **6432** en lugar del 5432 (`proxy.rs`), y el secreto es el mismo.
- **Postgres pasa del 55433 al 5432.** El pgbouncer de la imagen apunta a `localhost:5432` y `pgbouncer_settings` no puede cambiar su sección `[databases]`. El 55433 es el puerto de `neon_local` (desarrollo); el de las VMs de Neon es el 5432. Cambian la especificación, los argumentos y puertos de la VM, el contrato del proxy y la `NetworkPolicy` de la malla 85, que abre 5432 y 6432 **sin quitar todavía** el 55433.

`lab/p55.sh`: un cómputo con Postgres en el 5432 y **el `pgbouncer.ini` de la imagen de Neon sin tocar**, en su misma versión (`lab/computo/`). Por el proxy, con `verify-full`, todo en verde y repetible:

- **150 clientes a la vez por `-pooler`**, 20 s de pgbench: 4689 transacciones, ~235 tps, **0 fallidas**, y **como mucho 64 conexiones** del rol en Postgres;
- **los mismos 150 directos no caben**: `max_connections` es 100 y Postgres los rechaza (*remaining connection slots are reserved*). Es justo lo que resuelve el pool;
- **el protocolo extendido** (`-M extended`), sin fallos; **otra contraseña** por el pool, fuera.

⚠️ **Modo `transaction`**, como Neon: lo que vive en la sesión (`SET`, `LISTEN`, *advisory locks* de sesión, tablas temporales entre transacciones) no sobrevive entre transacciones por `-pooler`. Para eso está el endpoint directo. Connect (P5·7) ofrecerá los dos y dirá cuál usar.

#### P5·5b · Las conexiones, a la medida del cómputo (2026-10-09)

Hasta aquí `max_connections` era 100 fijo para cualquier tamaño, y el pool de la imagen daba 64 por pareja rol + base sin mirar cuántas cabían. Ahora la especificación calcula las dos cosas:

- **`max_connections` sale de las CU máximas**: unas 450 por CU, la escala de Neon (0,25 → 112, 1 → 450, 8 → 3600), con un suelo de 100 y un techo de 4000. Salen de las **máximas** porque `max_connections` solo cambia al reiniciar y el escalado no reinicia.
- **En una réplica**, sale de las CU del escritor de su rama si son más (`CU_DE_LAS_CONEXIONES`). ⛔ Postgres no deja a una réplica seguir a un primario con más `max_connections` que ella: pausa la recuperación hasta que se reinicie.
- **El pool**: el 90 % de `max_connections` repartido entre las bases de la rama, como `default_pool_size` y `max_db_connections` en `pgbouncer_settings`. compute_ctl lo escribe en el `pgbouncer.ini` y hace `RELOAD`, también con `/configure`, así que se reajusta sin reiniciar cuando se crea o se borra una base. pgbouncer no tiene un techo global, solo por base: repartido así, **la suma nunca pasa de lo que Postgres admite** y queda un **10 % para las conexiones directas**. `postgres` no cuenta en el reparto: casi nadie la usa por el pool, y contarla le quitaba la mitad del pool a una rama de una sola base.
- **La API lo dice**: cada endpoint trae `conexiones: {maximas, pool_por_base}`. Connect (P5·7) lo enseñará.

**Medido** (`lab/p55b.sh`, un cómputo de 0,25 CU con dos bases y 150 clientes en cada una a la vez por `-pooler`, con transacciones que retienen la conexión 50 ms):

| | pool en Postgres | clientes del pool | una conexión directa a mitad de carga |
|---|---|---|---|
| **antes** (el `pgbouncer.ini` de la imagen: 64 por pareja) | 105–107 de las 109 ranuras para usuarios | casi siempre bien; 2 de 5 pasadas, un pgbench muere al arrancar | según el momento |
| **ahora** (50 por base) | **≤ 100**, siempre | **0 fallidas** | **entra, siempre** |

Lo que medí **corrige lo que esperaba**: con el pool de la imagen, pgbouncer no hace fallar a los clientes cuando Postgres se llena, los pone en cola. El daño es otro, y es intermitente. Al rechazar Postgres una conexión, pgbouncer marca ese pool como *server login has been failing* y durante 15 s devuelve error a todo cliente nuevo; además, las conexiones directas se quedan sin sitio. Con el reparto, nada de eso ocurre, y el rendimiento apenas cambia (26 700 frente a 29 500 transacciones en 15 s).

`p55.sh` lee ahora los límites de la API. En un cómputo de 1 CU (450 conexiones, pool de 405): **490 clientes por el pool, 0 fallidas**, como mucho 405 en Postgres; los mismos 490 directos no caben.

#### P5·6 · Quién entra, medido en el laboratorio (2026-10-09)

**Lo comprueba el proxy de Neon; lo decide `ore-postgres`.** Por proyecto, como en Neon (migración 005), y se le contesta en `get_endpoint_access_control`:

- **`ips_permitidas`** (`allowed_ips`): vacía significa todas. Admite una IP, una subred (`203.0.113.0/24`) o un rango (`203.0.113.1-203.0.113.9`), IPv4 o IPv6, hasta 100 entradas. ⛔ El proxy convierte una entrada que no entiende en «ninguna IP», lo que deja fuera a todos; por eso la API valida cada entrada con las mismas reglas del proxy (`patron_ip_valido`) y una mala es un 400.
- **`bloquear_publico`** (`block_public_connections`): nadie entra por la entrada pública. Hoy es el interruptor que cierra el endpoint a internet; cuando haya una entrada privada, será la única puerta.
- **`limites`** (`rate_limits.connection_attempts`): intentos de conexión por endpoint y protocolo, en cubeta (por segundo y ráfaga). **Siempre hay uno**: tcp y ws a 100/s con ráfaga de 1000; http a 1000/s con ráfaga de 10000, porque por HTTP cada consulta es una conexión. Es lo que corta la fuerza bruta, junto al SCRAM de 4096 iteraciones y contraseñas de 32 caracteres al azar.

`GET`/`POST /v1/postgres/proyectos/{p}/acceso` (en `ore-serve`: `postgres:ver` para leer; `postgres:gestionar` o ser el dueño para cambiar). En un `POST`, lo que no viene en el cuerpo se queda como estaba. Cambiar es una operación, `configurar-acceso`: el reconciliador no tiene nada que hacer en el cómputo, y al quedar hecha el proxy olvida su caché por Redis (P5·1), así que **el cambio vale ya**, sin esperar los 4 minutos.

`lab/p56.sh`, con TCP desde una IP y HTTP desde otra, todo en verde y repetible:

- **IPs**: con solo la de HTTP en la lista, TCP no entra y HTTP sí; con un rango que cubre solo la de TCP, al revés; una subred que cubre las dos, ambas; otra subred, ninguna; la lista vacía, todas otra vez. Cada cambio se nota en la conexión siguiente.
- **Bloqueo público**: nadie entra, por TCP ni por HTTP; al quitarlo, se vuelve a entrar.
- **Fuerza bruta**: con una cubeta de 1/s y ráfaga de 3, de 12 intentos a la vez **9 se cortan** por el límite (*too many connections*); con el límite de siempre, se vuelve a entrar.
- Una entrada mala, **400**; otra organización no lo ve ni lo cambia (**404**).

Lo que solo se puede medir en producción: que el proxy vea **la IP real del cliente** tras el balanceador (P5·3).

#### P5·7 · Connect (2026-10-09)

El modal **Connect to your database** de la consola, iterado sobre un boceto con el usuario:

- **El nombre público lo dice la API**, no la consola: con `ore-postgres --dominio europe-west1.pg.paladio.io`, cada endpoint trae `host` y `host_pool` (con `-pooler`), además de `conexiones` (P5·5b). La consola nunca escribe a mano región ni dominio.
- **El interruptor «Connection pooling», encendido por defecto**, como Neon. Cambia `ep-…` por `ep-…-pooler` en el host y dice cuándo apagarlo (migraciones, `pg_dump`, `LISTEN/NOTIFY`). A la derecha, lo que cabe según la API: «Up to 10,000 clients · 405 to Postgres» o «Up to 450 connections».
- **TLS con el SCRAM atado a él**: `sslmode=require&channel_binding=require`. Medido antes con nuestro proxy (SCRAM-SHA-256-PLUS, directo y por el pool) y comprobado ya en `lab/p51.sh`.
- **Doce formatos en fichas**: Connection string, psql, Node.js, Serverless driver (`neon()` por HTTP y `Pool` por WebSocket, P5·4), Python, SQLAlchemy, **Prisma** (siempre con dos URLs: `DATABASE_URL` con pool para la app y `DIRECT_URL` directa para las migraciones, el error más común), Django, Java, .NET, Go y parámetros sueltos.
- Se mantienen *Reset password* (la nueva solo vive en el snippet mientras el modal está abierto) y el aviso de que las contraseñas se enseñan una vez. Desaparecen la dirección interna, el 55433 y el aviso de «no se alcanza desde internet».

Probado en la consola en modo banco. Que el snippet copiado conecte desde fuera es el paso 6 de las pruebas que solo se pueden hacer en producción. Los ajustes de acceso (P5·6) irán en la configuración del proyecto, como en Neon, no en Connect.

#### P5·2 · El nombre y el certificado, escritos (2026-10-09)

Escrito, validado y **sin aplicar** (Google sin facturación):

- **`malla/87-postgres-la-entrada-gcp.sh`** (idempotente):
  - la IP regional `ore-pg-entrada`;
  - la cuenta `ore-pg-dns`, con `dns.admin` **solo sobre la zona `pg-paladio-io`**; el guion comprueba que tiene 0 roles en el proyecto y 0 claves;
  - Workload Identity para `cert-manager/cert-manager`;
  - el registro `*.europe-west1.pg.paladio.io` A → la IP, que se corrige si apunta a otra;
  - la retirada del TXT `_delegacion`.
- **Parche del cert-manager vendorizado** (`postgres-computo/cert-manager/kustomization.yaml`): la anotación de Workload Identity en su cuenta de servicio. Es un parche porque `vendorizar.py` reescribe el fichero vendorizado.
- **`malla/postgres-entrada/`**, con su propio Kustomization de Flux (`ore-pg-entrada`, `dependsOn: ore-pg-cert-manager`, como la base del cómputo):
  - el `ClusterIssuer` `letsencrypt-pg`, por DNS-01 en Cloud DNS con las credenciales del pod, sin claves;
  - el `Certificate` `entrada-pg`: comodín ECDSA P-256 en el Secret `entrada-pg-tls` de `ore-pg`, renovado 30 días antes.
- **Validado**: `kustomize build` de los dos directorios, `kubeconform -strict` con los esquemas de Kubernetes, cert-manager y Flux (3 de 3 válidos), y `shellcheck` del guion.

⛔ **`87-postgres-la-entrada.yaml` no está en la lista de la malla**, solo comentado. Flux reconcilia desde `main`: en cuanto vuelva Google lo aplicaría, antes de que exista la cuenta del DNS-01, y Let's Encrypt limita los retos fallidos. Se descomenta en el despliegue, **después** del guion.

#### P5·3 · El proxy en la malla, escrito (2026-10-09)

Escrito, validado y **sin aplicar**. Con P5·2, P5 queda **cerrado en el laboratorio**: lo que falta es desplegar y las pruebas que solo se pueden hacer en producción.

- **`malla/postgres-entrada/proxy.yaml`**:
  - el proxy de Neon, con **los mismos argumentos que en el laboratorio**, 2 réplicas repartidas por nodo en el pool `pg`, con pata en la overlay (`overlay-del-proxy`) y `pg-acceso`;
  - puertos altos dentro (4432, 4444), porque la imagen corre como `neon` (uid 1000) sin capacidades; el Service los saca al 5432 y al 443;
  - sondas en `/v1/status`; para vaciar al parar, `preStop` de 15 s y hasta 300 s para que terminen sus clientes;
  - el Service `LoadBalancer` es un L4 de paso directo con backend services y `externalTrafficPolicy: Local`, para que la IP del cliente llegue tal cual (P5·6), en la IP fija de P5·2;
  - un PDB con `minAvailable: 1`;
  - NetworkPolicies: entrada desde internet solo al 4432 y al 4444; salida solo al DNS, a `ore-postgres`, a Redis y a las VMs.
- **`malla/postgres-entrada/redis.yaml`**: Redis 7.4 fijado por *digest*, solo pub/sub, sin disco ni salida; solo llegan a él el proxy y `ore-postgres`.
- **La IP, por sustitución de Flux**: el guion de P5·2 escribe el ConfigMap `flux-system/ore-pg-entrada` (`IP_ENTRADA`) y el Kustomization `ore-pg-entrada` la pone en el Service (`postBuild.substituteFrom`). Sin él no se aplica: nunca un balanceador con una IP efímera que el DNS no conoce.
- **Malla 86**: el token del proxy (Secret `ore-postgres-proxy`, ya creado), la entrada del proxy a `ore-postgres`, la salida de `ore-postgres` a Redis y los flags `--redis` y `--dominio`. Es segura con el binario viejo, que ignora los flags que no conoce; con el nuevo y sin Redis, `olvidar` lo dice una vez y sigue.
- **Validado**: `kubeconform -strict`, 18 de 18. El proxy del laboratorio corre ya con el mismo `securityContext` (uid 1000, sin capacidades, `no-new-privileges`), y P5·1, P5·4, P5·5 y P5·6 siguen en verde. kubeconform no veía una trampa que salió al revisar: la imagen dice `USER neon` por nombre, y con `runAsNonRoot` sin `runAsUser` numérico el kubelet no arranca el contenedor.

**El orden del despliegue** el día que vuelva Google:
1. El CI empuja `ore-postgres:main` con P5 dentro, y la malla 86 lo recoge.
2. `malla/87-postgres-la-entrada-gcp.sh`.
3. Descomentar `87-postgres-la-entrada.yaml` en la lista de la malla.
4. Las pruebas que solo se pueden hacer en producción, abajo.

#### P5 · Del laboratorio a producción: por qué llegará sano y rápido

Lo construido en el laboratorio llega a producción en una tarde y sin reescribirse, por cómo está hecho:

1. **Los mismos binarios.** El laboratorio corre la imagen de Neon que está desplegada (`8269bece`) y el `ore-postgres` de `main`; producción no estrena código, sólo red.
2. **El contrato, probado contra el de verdad.** Lo que el proxy pregunta y lo que se le contesta se mide contra el proxy real, no contra un doble: la lección del 422 de `/configure` (P4·4).
3. **Todo declarado en git.** El guion de GCP (P5·2) y la malla del proxy (P5·3) quedan escritos, revisados y validados; desplegar es aplicarlos en el orden de siempre: **binario antes que malla**.
4. **Las pruebas, escritas antes de que exista la entrada.** Cada «hecho cuando» de producción tiene ya su guion (`p51`…`p58`), así que el primer día se mide, no se improvisa.
5. **Sin estado que migrar.** El proxy no guarda nada: réplicas, reinicios y despliegues no pierden sesiones más allá de las que vacía al parar.

**Lo que sólo se puede probar en producción**, anotado desde ya (se corre en este orden el día que Google vuelva):

1. **Volver**: el clúster, `ore-pg` (almacenamiento, `ore-postgres`, las VMs) y las celdas, sanos; relanzar el CI de `5321c351` (sólo falló al subir imágenes).
2. **P5·1 en el clúster** (`p51.sh`): el proxy en la overlay contra VMs de verdad, con Redis; la malla 86 añade a `ore-postgres` `--redis` y `--dominio europe-west1.pg.paladio.io`; un *Reset password* entra a la primera.
3. **P5·2**: `malla/87-postgres-la-entrada-gcp.sh` (la IP, la cuenta del DNS-01 sólo sobre `pg-paladio-io`, el registro A, fuera el TXT `_delegacion`); **después** descomentar `87-postgres-la-entrada.yaml` en la lista de la malla; hecho cuando el `Certificate` `entrada-pg` está `Ready` y `*.europe-west1.pg.paladio.io` resuelve a la IP (también `api.`, el que usa el driver por HTTP).
4. **P5·3**: desde internet, `psql "…?sslmode=verify-full"` entra y pgbench corre sin fallos; reiniciar una réplica no tira a la otra; la IP del cliente llega tal cual.
5. **P5·4** desde internet con `@neondatabase/serverless`; **P5·5** contra el pgbouncer de la VM (`p55.sh` con 150 clientes, como mucho 64 conexiones en Postgres); **P5·6** con la IP real del cliente.
   - **El puerto, en este orden**: primero la malla 85, que solo **añade** 5432 y 6432 (aditiva: no rompe nada vivo); después el binario de `ore-postgres`; después se recrean los cómputos que nacieron en el 55433 (hoy solo los de prueba y `postgre`, el de la consola); y por último se quita el 55433 de la malla y de Connect.
6. **P5·7**: el snippet de Connect copiado en la consola conecta desde fuera.
7. **P5·8**: una migración en vivo de la VM **no corta** una sesión abierta por el proxy; pgbench desde internet durante la migración, sin fallos.
8. **Salud al terminar**: ningún pod reiniciándose, el certificado con su renovación programada, las métricas del proxy (`:7001`) sin errores de `wake_compute` ni de autenticación, y `p47.sh` (no queda huella) otra vez en verde.

#### P6 · Serverless de verdad, en el laboratorio: el plan (2026-10-09)

El cómputo duerme por inactividad y despierta al conectar, desde un pool de cómputos ya arrancados; el cliente fija sus límites. Como P5, se construye en el laboratorio local mientras Google no tiene facturación.

**Lo que es del producto y lo que es desechable.** Son del producto, en `ore-postgres`: los estados del endpoint, la API de límites, dormir, despertar, la contabilidad del pool y la invalidación al proxy. Son del laboratorio y desechables:
- **el cómputo de mentira**: Postgres 17 y pgbouncer, como en P5·5, más un `compute_ctl` falso que cumple el contrato leído en P6·0;
- **el backend de cómputos para Docker**: el reconciliador crea, para y borra contenedores en lugar de VMs.

El backend va detrás de una *feature* de Cargo (`laboratorio`): el binario de producción no lo lleva, y borrarlo no toca la lógica.

| paso | qué | hecho cuando (en el laboratorio) |
|---|---|---|
| **P6·0 · El contrato real** | leer en el fork `/status`, el arranque vacío, `/configure`, `/terminate` y lo que el proxy hace al despertar | anotado aquí; el `compute_ctl` falso se escribe contra esto |
| **P6·1 · El cómputo de mentira y el backend Docker** | el reconciliador real crea contenedores, en lugar de `lab.sh reconcilia` | P5·1–P5·6 en verde con el reconciliador real |
| **P6·2 · Los límites del cliente** | migración 006: `dormir_tras` por endpoint (0 = nunca) y cambiar `cu_min`/`cu_max` de uno existente, por la API | validación, 404 entre organizaciones, se aplica al despertar siguiente |
| **P6·3 · Dormir** | pasado `dormir_tras` sin actividad (`last_active`), `/terminate` y fuera el cómputo; el endpoint, `dormido` | duerme a su hora; con una consulta en curso no; dormido no queda contenedor |
| **P6·4 · Despertar al conectar** | `wake_compute` sobre uno dormido lo enciende y espera a que esté listo, con tope; varias conexiones a la vez, un solo despertar | `psql`, pool, HTTP y WebSocket entran a uno dormido; el cliente no ve más error que la espera |
| **P6·5 · El pool precalentado** | N cómputos vacíos; despertar toma uno, le manda `/configure` y repone el pool; un cómputo que sirvió a un tenant se destruye, nunca vuelve al pool | despertar en p50 y p95, en frío frente a desde el pool, con el arranque de una VM emulado |
| **P6·6 · La consola** | *Active* o *Idle*; los límites, editables (junto a la pestaña Settings de P5·6) | en modo banco |
| **P6·7 · ADR** | lo medido y lo que solo se puede probar en producción | — |

#### P6·0 · El contrato real, leído en el fork (2026-10-09)

Leído en el commit fijado (`8269bece`), sin Google:

1. **`GET /status`** (`ComputeStatusResponse`) devuelve `status` y `last_active`. Los estados que importan son `empty` (arrancó sin especificación y espera una), `configuration_pending`, `init`, `running`, `configuration` y `failed`. En JSON van en *snake_case*.
2. **Qué es actividad** (`compute_tools/src/monitor.rs`, cada 500 ms):
   - **cuenta**: un backend de cliente que **no** está `idle`, sin contar `cloud_admin` ni el propio monitor; un walsender lógico; una suscripción lógica activa; un autovacuum;
   - **no cuenta**: una sesión abierta y ociosa, cuyo `last_active` es la hora en que quedó ociosa.

   ⇒ **Neon duerme el cómputo aunque haya sesiones ociosas abiertas, y las corta.** Se sigue igual, y corrige el plan, que decía «nunca con conexiones abiertas». La regla es «nunca con una consulta en curso». Quien no quiera que se corten pone `dormir_tras = 0`.
3. **Dormir es `POST /terminate`**, no borrar la VM a pelo. Para Postgres limpiamente y devuelve el LSN final (`TerminateResponse`); después se borra la VM y el almacenamiento conserva todo.
4. **El arranque vacío, que es el pool**: un `config.json` con `"spec": null` y solo `compute_ctl_config` (el JWKS) deja a `compute_ctl` en `empty`, con su HTTP arriba, esperando. Es la misma forma que el cuerpo de `/configure`.
5. **`POST /configure`** solo se acepta en `empty` o en `running`, y **no contesta hasta que el cómputo está `running`** (o `failed`). Despertar desde el pool es, por tanto, una sola llamada que vuelve con Postgres listo.
   - El `compute_id` de un cómputo del pool es su propio nombre (`pool-…`), no el del endpoint, y el token de `compute_ctl` se firma para ese nombre.
   - El nombre público del endpoint (`ep-…`) no cambia: la base guarda qué cómputo sirve hoy a cada endpoint.
6. **El proxy, al despertar**:
   - el cliente HTTP con que llama a `wake_compute` **no tiene tope** (`http::new_client`, sin `timeout`), así que `wake_compute` puede esperar a que el cómputo esté listo en vez de contestar «reintenta», que solo daba ~7 s;
   - conectar al cómputo tiene 2 s por intento y 5 reintentos.
7. **La caché de direcciones del proxy** (`--wake-compute-cache idle_ttl=4m`) se invalida, y el proxy vuelve a despertar, cuando conectar al cómputo falla y también cuando el cómputo contesta un error de Postgres, incluido un fallo de contraseña (`should_retry_wake_compute`), salvo una lista corta (`too_many_connections`, sintaxis…). Un endpoint que cambia de cómputo en cada despertar no queda inservible 4 minutos por una dirección vieja, ni aunque esa IP la tenga ya otro cómputo: el SCRAM falla, el proxy olvida y despierta. **Se mide en P6·5**, forzando que se reutilice la IP.
8. **El tiempo del cliente**: un despertar en frío de una VM (~35 s, medido en P3) supera el tiempo de conexión por defecto de algunos drivers (Prisma, 5 s). El pool es lo que lo resuelve; el número de verdad se mide en producción.

**Lo que queda fuera de P6 hasta que vuelva Google**:
1. el arranque real de una VM de NeonVM y el **p50/p95 real** del despertar, en frío y desde el pool, con el objetivo fijado tras esa medida;
2. el **`compute_ctl` real** vacío aceptando `/configure` con la especificación de un tenant (el contrato de verdad, no el doble), y su `/terminate`;
3. el **`last_active` real**, con pgbouncer, `cloud_admin`, la replicación y el walproposer;
4. **dormido cuesta 0** de verdad: la VM desaparece y el nodo del pool `pg` se libera con el autoescalado;
5. el pool en el clúster: cuántas VMs caben en la cuota, el aislamiento de una VM del pool antes de tener tenant (barreras 2 y 3) y la migración en vivo de una VM despierta;
6. el despertar a través del balanceador, desde internet.

#### P6·1 · El cómputo de mentira y el backend Docker (2026-10-09)

**El laboratorio ya no tiene un reconciliador de mentira: corre el de verdad.** `lab.sh reconcilia`, `hechas` y `barre` desaparecen; las pruebas esperan a la API (`hecha`: una operación acabada; `listo`: un endpoint listo).

- **El backend Docker** (`src/laboratorio.rs`, *feature* `laboratorio`, `--computos docker:HOST:PUERTO`) implementa `Computos` sobre la API de Docker. Cada cómputo es un contenedor `ep-…` en la red del laboratorio. La API de Docker llega por un `socat` que solo existe dentro de esa red.
- **El mismo contrato que NeonVM**: `listo` y `configurar` son ahora funciones compartidas (`listo_por_http`, `configurar_por_http`) que usan NeonVM y el laboratorio, con el mismo token y la misma especificación. Lo que se prueba en el laboratorio es el código que corre en producción.
- **El almacenamiento de mentira** (`AlmacenDeMentira`): los datos de cada timeline viven en un volumen compartido (`/almacen/<tenant>/<timeline>`) que hace de pageserver, así que borrar el contenedor y crear otro conserva los datos. Al borrar el proyecto se borra su directorio.
- **El `compute_ctl` falso** (`lab/computo/compute_ctl_falso.py`, el proceso principal de la imagen `p5lab-computo:2`), escrito contra lo leído en P6·0:
  - arranca vacío (`spec: null` → `empty`) o con su especificación;
  - `/status` con `last_active` calculado con las consultas de `monitor.rs`;
  - `/configure` solo en `empty`/`running`, y contesta al quedar `running`; aplica roles, bases, `delta_operations`, `max_connections` y `pgbouncer_settings`, este último con el `.ini` y `SIGHUP`, como `tune_pgbouncer`;
  - `/terminate` devuelve el LSN;
  - el token con el `compute_id` propio;
  - un retardo de arranque configurable (`ARRANQUE_S`) para P6·5.
- **El binario de producción no lo lleva**: sin la *feature*, `--computos` no existe y `clippy` está limpio con y sin ella. Borrar `laboratorio.rs` no toca la lógica.

**Medido**, todo en verde desde cero:
- las 40 pruebas del crate;
- P5·1, P5·4, P5·5, P5·5b y P5·6, con el reconciliador real creando los cómputos.

En P5·5b, el pool de 50 por base llega ahora a la VM **por el camino real**: la base nueva lanza `configurar-rama`, `/configure` lleva `pgbouncer_settings` y el `compute_ctl` rehace el pool sin reiniciar.

Al terminar, **ni un cómputo vivo ni un tenant en el almacén**: borrar no deja rastro.

#### P6·2 · Los límites del cliente (2026-10-09)

- **Migración 006**: `dormir_tras` por endpoint, en segundos. **300 por defecto**, como Neon; **0 = nunca**; si no, de 60 s a 7 días. «Inactividad» es la de `compute_ctl` (P6·0): sin consultas en curso, no sin sesiones.
- **`POST …/endpoints/{e}/ajustes`** (`cu_min`, `cu_max`, `dormir_tras`; en `ore-serve`, `postgres:gestionar` o el dueño). Lo que no viene se queda. Las reglas son las de crear: CU de 0,25 a 2, mínimo ≤ máximo, y 400 si no hay nada que cambiar. Otra organización, 404. Crear un endpoint acepta también su `dormir_tras`.
- **Es una operación** (`configurar-endpoint`) que el reconciliador da por hecha sin tocar el cómputo:
  - **un cómputo vivo no se reinicia** por cambiar sus límites;
  - las CU y `max_connections` valen en el siguiente arranque, que en adelante es el despertar (P6·4);
  - `dormir_tras` lo lee el reconciliador al decidir si duerme (P6·3).
- **La API lo dice**: cada endpoint trae `dormir_tras`, y `conexiones.maximas` ya es la del siguiente arranque. Con 0,25 CU son 112, mientras el cómputo vivo sigue con sus 450.

**Medido** (`lab/p62.sh`, con el reconciliador real) y en el contrato: lo malo, fuera; los límites nuevos en la API; la operación, hecha; el cómputo vivo, sin reiniciar. P5 sigue en verde.
