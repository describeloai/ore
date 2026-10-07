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
| P2·5 | retención y limpieza (PITR, GC, scrubber; borrar un tenant vacía GCS) | **siguiente** |
| P2·6 | aceptación (+ decidir `--timelines-onto-safekeepers`) | pendiente |

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
| las VMs de cómputo | en el namespace del inquilino, `t-<org>` | Su ResourceQuota es el límite de la organización, y su aislamiento, el de la organización. |
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
  - la IP de la overlay reutilizada (C4): ARP gratuito o reintento;
  - los límites de cada endpoint, desde la API: CU mínimas y máximas, y el tiempo hasta dormir.
- Hecho cuando:
  - el despertar está medido en p50 y p95, con un objetivo fijado tras la primera medida (hoy, sin pool, ~16 s: B.4);
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

