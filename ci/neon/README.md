# El motor de ORE Serverless Postgres: compilarlo y mantenerlo

Es el procedimiento de [ADR 0058](../../docs/decisions/0058-ore-serverless-postgres.md) (B.8 C5 y B.11 P1), con lo que medimos en P1.

## Qué es de quién

| | |
|---|---|
| `describeloai/neon` | El fork de `neondatabase/neon`. Nuestra rama es **`ore/main`**: fa504217 más nuestros parches y el puntero a nuestro Postgres. |
| `describeloai/postgres` | El fork de `neondatabase/postgres`. Nuestra rama es **`ore/REL_17_STABLE_neon`**: la 17.5 de Neon (`1e01fcea`) más **sólo upstream**. |
| `describeloai/autoscaling` | El fork de `neondatabase/autoscaling`. De aquí sale **todo NeonVM y autoscaling** (kernel, controller, runner, vxlan, daemon, agent, scheduler) y `vm-builder`, en **una sola etiqueta** (`v0.49.1`, P3·1). |
| `ci/neon/*.yaml` | **El único sitio que dice qué motor corre** (`_COMMIT`). Los forks no llevan CI. |

Hay tres reglas:
- **Cada commit fijado lleva una etiqueta `ore/…` en su fork**, para no depender de que upstream lo conserve.
- **Los parches en el fork, cortos y con un porqué en el mensaje.** Cada uno se paga en cada rebase.
- **No se adoptan las ramas nuevas de Neon para Postgres** (`REL_17_STABLE_neon` 17.8 y siguientes). Van emparejadas con un `neon` que no es público: mueven piezas a la extensión `neon` cuyo otro lado no tenemos. Se fusiona upstream de PostgreSQL directamente (P1·4).

## Las recetas (Cloud Build, cuenta `ore-ci`, E2_HIGHCPU_32)

| receta | qué da | tiempo medido | coste aprox. |
|---|---|---|---|
| [`almacen.yaml`](almacen.yaml) | `neon:<commit>` (pageserver, safekeeper, broker, storage_controller, proxy…) | 14 min si cambia Rust · 2 min 20 s sin cambios · ~28–38 min escribiendo la caché | 0,15–2,5 USD |
| [`autoscaling.yaml`](autoscaling.yaml) | `neonvm-kernel:<versión>-<árbol>` y las 6 imágenes de NeonVM y autoscaling en `:<etiqueta>` | *(midiendo, P3·1)* | |
| [`computo.yaml`](computo.yaml) | `compute-node-v17`, `vm-compute-node-v17:<commit>` (con el `vm-builder` y el `neonvm-daemon` de `autoscaling.yaml`: va **después**) | 1 h 10 min en frío (Postgres y ~40 extensiones, en serie) | ~4,5 USD |
| [`postgres-check.yaml`](postgres-check.yaml) | `make -k check-world` del fork de Postgres, con aserciones | 3,5 min | ~0,2 USD |

```bash
P=project-8853a180-450d-47be-b83
gcloud builds submit --no-source --async --region=global --config ci/neon/<receta>.yaml \
  --substitutions=_COMMIT=<commit>[,_CACHE=escribir] \
  --service-account=projects/$P/serviceAccounts/ore-ci@$P.iam.gserviceaccount.com
```

- **`_CACHE=escribir` sólo si cambian `Cargo.lock`, Postgres, una extensión o build-tools.** Escribir la caché cuesta 11–15 min (`mode=max`). Leerla es gratis.
- **No lances dos compilaciones a la vez que escriban la misma caché.** Se pisan (P1·3, R3).

## Cuándo se compila

1. **Cada versión menor de PostgreSQL** (feb, may, ago, nov) y **cada CVE serio**, fuera de calendario. Es la sección siguiente. Unos 7–8 USD cada vez.
2. **Un parche nuestro en `neon`** (compute_ctl, proxy…): `almacen.yaml` y/o `computo.yaml` con el commit nuevo, que se pone luego en `_COMMIT`.
3. **Cada semana, `cargo audit`** del `Cargo.lock` fijado. No compila nada. *(Pendiente de montar.)*

## Una versión menor nueva de PostgreSQL, paso a paso

Ejemplo: de la 17.10 a la 17.11.

```bash
cd C:/ore-neon/pg17                         # clon COMPLETO (el de sin blobs no sirve para fusionar)
git fetch upstream refs/tags/REL_17_11:refs/tags/REL_17_11
git checkout ore/REL_17_STABLE_neon && git merge REL_17_11     # resolver; el porqué de cada conflicto, en el mensaje
git push origin ore/REL_17_STABLE_neon
git tag ore/v17.11-$(git rev-parse --short=8 HEAD) && git push origin ore/v17.11-<sha>   # sólo esa etiqueta, no --tags
```

1. **Regresión**: `postgres-check.yaml` con el commit nuevo **y** con el anterior. Hay que comparar los dos (`== errores nuevos…`); no basta con que pase. Sin la extensión `neon`, el Postgres de Neon falla siempre igual: 108 `resource manager with ID 134 not registered` en `pg_walinspect` y `test_decoding`, más sus errores en cascada. **Lo que cuenta es que no haya ningún fallo nuevo.**
2. **`neon`**: en `C:/ore-neon/neon`, rama `ore/main`.
   - `git update-index --cacheinfo 160000,<sha>,vendor/postgres-v17`;
   - en `vendor/revisions.json`, la versión y el sha;
   - commit y push.
3. **Compilar**: `almacen.yaml` y `computo.yaml` con el commit nuevo de `neon` y `_CACHE=escribir`.
4. **Aceptación**: [`pruebas-de-fuego/ore-postgres/`](../../pruebas-de-fuego/ore-postgres/) con `ORE_PG_COMMIT=<commit>`, mínimo `arranque`, `escalado`, `migrar` y `c4`.
5. **Fijar**: `_COMMIT` en las tres recetas, más una línea en la zona borrador del ADR.

## Riesgos conocidos

- **Fuentes de extensiones de internet.** `compute-node.Dockerfile` descarga ~40 al compilar. `h3-pg` ya desapareció de su sitio (parche `6941d9e`, mismo sha256). Si una descarga da 404: buscar el repositorio nuevo, **comprobar que el sha256 es el mismo** y parchear la URL. Pendiente decidir si guardamos las fuentes en un bucket nuestro.
- **Memoria al compilar el cómputo.** Con más de 1 etapa de BuildKit en paralelo, el enlazado con LTO muere por SIGKILL en 32 GB. `_PARALELO` se queda en 1.
- **`HOME` en Cloud Build** es `/builder/home`, y `nonroot` no puede escribir ahí. `postgres-check.yaml` usa `/tmp/ore`.
