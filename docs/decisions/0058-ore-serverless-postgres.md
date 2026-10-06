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

- **C1** (`cloudbuild-neon.yaml`, cuenta `ore-ci`, E2_HIGHCPU_32, sin caché), commit `fa504217`
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
| P1–P9, Q1–Q5 | construir (B.11) | **plan escrito**; P1 esperando el go |

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
