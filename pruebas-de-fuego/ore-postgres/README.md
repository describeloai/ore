# Pruebas de fuego · ORE Serverless Postgres

Estas pruebas son la aceptación del motor ([ADR 0058](../../docs/decisions/0058-ore-serverless-postgres.md)).
- Cada una reproduce una medida de D0b o D0c.
- Una **imagen nueva** (un rebase, un parche: ver [`ci/neon/README.md`](../../ci/neon/README.md)) tiene que volver a pasarlas antes de usarse.

**Todo va parametrizado.** Lo que cambia de un montaje a otro es una variable `ORE_PG_*` de [`entorno.sh`](entorno.sh). El estado que pasa de una prueba a otra (ids, IPs, la clave del JWT, la especificación) vive en `$ORE_PG_TRABAJO`, nunca en el repositorio.

## Montaje

Hace falta un pool con KVM (`--enable-nested-virtualization`) etiquetado `ore.dev/pool=$ORE_PG_POOL`. Encima van NeonVM, autoscaling y Multus ([`preparar.py`](preparar.py) y [`multus-v4-gke.yaml`](multus-v4-gke.yaml), B.3). A partir de P2/P3 todo esto lo declara la malla.

```bash
source entorno.sh                          # por defecto: ns ore-pg-prueba; imágenes las de ci/neon/computo.yaml
export ORE_PG_BUCKET=… ORE_PG_GSA=…        # el bucket del almacenamiento y su cuenta (Workload Identity)
plantilla almacen-gcs.yaml | kubectl apply -f -     # namespace, broker, 3 safekeepers, pageserver
plantilla cliente.yaml | kubectl apply -f -         # cliente (red de pods) y cliente-overlay
./tenant.sh                                # tenant + main             → $ORE_PG_TRABAJO/{tenant,main}
./vm.sh                                    # la VM sobre main          → ov-/pod-<vm>
```

## Las pruebas

| prueba | qué mide | medido (ADR 0058) |
|---|---|---|
| [`arranque.sh`](arranque.sh) `[n]` | `apply` → primera consulta, con el desglose dentro de la VM | B.4: 15,8–17 s |
| [`escalado.sh`](escalado.sh) | sube con carga y baja sin ella, con una sesión abierta todo el rato | B.5: 9 s subir, ~3 min bajar, 0 cortes |
| [`inactividad.sh`](inactividad.sh) | la señal `last_active` de `GET :3080/status` | B.5: no cuenta `cloud_admin` |
| [`migrar.sh`](migrar.sh) | migración en vivo con dos sesiones, una por la overlay y otra por la IP del pod | B.6: pausa 51–66 ms; la overlay sobrevive |
| [`c4.sh`](c4.sh) | se pierde el pageserver **con su disco**: RPO, RTO y lectura en frío | B.8 C4: RPO 0, ~21 s |
| [`c5-hints.sh`](c5-hints.sh) | WAL del primer recorrido en una rama (hint bits) | B.8 C5: 16 MB frente a 250 kB |

`c4.sh` necesita además una rama con su propia VM y pgbench inicializado en `main`:

```bash
./tenant.sh rama r1 && ORE_PG_VM=pg-rama ./vm.sh $(cat $ORE_PG_TRABAJO/rama-r1)
```

## Piezas

- [`entorno.sh`](entorno.sh): las variables, y estas funciones:
  - `q` y `qo`: psql desde la red de pods o desde la overlay;
  - `pageserver`: su API HTTP desde dentro;
  - `plantilla`;
  - `guardar` y `leer`.
- [`especificacion.py`](especificacion.py): la especificación de `compute_ctl` y el JWT, firmado con una clave Ed25519 que se crea en `$ORE_PG_TRABAJO`. Es lo que hará el plano de control en P4.
- [`tenant.sh`](tenant.sh) y [`vm.sh`](vm.sh): el tenant, `main` y las ramas en el pageserver, y el endpoint (una VM sobre un timeline). Los relevarán el `storage_controller` (P2) y `ore-postgres` (P4).
- [`vm.yaml`](vm.yaml), [`almacen-gcs.yaml`](almacen-gcs.yaml) y [`cliente.yaml`](cliente.yaml): plantillas con `${ORE_PG_*}`. `plantilla` se niega a sacarlas si falta un valor.
- [`latido.sh`](latido.sh): una sesión que escribe cada 0,2 s y corre dentro de `cliente-overlay`.
