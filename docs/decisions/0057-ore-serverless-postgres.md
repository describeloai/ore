# 0057 · ORE Serverless Postgres

**Estado:** **propuesto** (2026-10-06) · D0 (local), D0b·1–5 (GKE) y D0c·C1–C2 (Neon compilado por
nosotros, GCS nativo) medidos; siguiente D0c·C3. **Decide:** qué es ORE Serverless Postgres para quien
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

### B.9 · Los pasos

| paso | qué | estado |
|---|---|---|
| D0c·C3 | compatibilidad: pgbench de escritura/lectura y una rama con el cómputo publicado | **siguiente** |
| D0c·C4 | «sin fondo»: borrar el pageserver con su disco y recuperar el tenant y la rama desde GCS; tiempo | pendiente |
| D0c·C5 | cerrar: cómo mantener el fork (commit fijado, cada cuánto, caché en el CI) | pendiente |
| D0b·6 | recoger lo de la prueba (B.10) y volver a 5/12 | pendiente |
| P1… | construir: el plano de control (API, ciclo de vida, proxy en la overlay, pool precalentado, `storage_controller`), publicar al catálogo por CDC, nodos grandes | por planificar |

### B.10 · Lo que hay vivo en GKE para la prueba (se recoge en D0b·6)

Pool `neon-d0`; namespaces `d0-neon`, `neonvm-system`, `cert-manager`; en `kube-system` Multus,
whereabouts, `autoscale-scheduler` y `autoscaler-agent` (todos con `nodeSelector ore.dev/pool=neon`);
CRDs de NeonVM, cert-manager y Multus; bucket `ore-neon-d0-1006`; cuenta `neon-d0` (Workload Identity →
`d0-neon/neon`); imagen `ore/neon:fa504217…` (se queda).
