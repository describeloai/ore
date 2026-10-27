# 0060 · GCP resources — el mismo resultado por una fracción del coste

**Estado:** **ITERADO (2026-10-11)**, con deudas menores. Hechos A1–A4 (Postgres a cero,
retención de Artifact Registry, secretos, discos), B1 (construir en los runners de GitHub, sin
Cloud Build), C1 (horario fijo: despierta de 08:00 a 00:00, hora de Madrid; a mano desde la consola), C2 (el túnel de Cloudflare,
sin balanceadores) y C3 (presupuesto de €20 con alertas). Fijo con la malla dormida, estimado:
**~16 $/mes**, más ~0,05 $/h despierta; se confirma con la factura. Estado final y deudas, en
«**Cierre**», al final de la historia. Propuesto el 2026-10-08. Cada paso que tocó la malla, el
IAM o la infraestructura de Google tuvo su go; la facturación y las cuentas son de la persona.

Recoge **el espectro entero de prácticas, hábitos y optimizaciones** con que seguimos desarrollando
sobre GCP con el coste de infraestructura **al mínimo**: qué se paga, por qué, y qué se deja de
pagar sin perder nada de lo que se obtiene. Construye sobre el CI de hoy (memoria *CI · tiempo*:
binarios por huella, una compilación por huella), el plan de mover la construcción a GKE (*Migrar
el CI a GKE*, M0 medido el 2026-10-01) y el ajuste de reservas O0–O5 (2026-10-06).

## Qué pasó

El 2026-10-08 la cuenta de facturación quedó **impagada** y Google suspendió el proyecto: el clúster
dejó de contestar y un build de Java terminó *lost* sin poder correr. En los primeros ocho días de
octubre:

| servicio | uso | créditos | a pagar | qué es |
|---|---|---|---|---|
| Cloud Build | €85,43 | −€30,74 | €54,70 | el CI: cada push, una construcción en `E2_HIGHCPU_8` |
| Artifact Registry | €36,35 | −€11,06 | €25,29 | las imágenes —y, sospecho, el tráfico de sus capas (G0)— |
| Compute Engine | €30,27 | −€13,87 | €16,40 | los nodos de GKE, encendidos siempre |
| Kubernetes Engine | €14,96 | −€14,96 | €0 | la tarifa del clúster: la cubre el nivel gratuito |
| Networking | €8,64 | −€5,27 | €3,37 | tráfico, IPs, el balanceador |
| Secret Manager, DNS, Storage, KMS | €2,75 | −€1,11 | €1,64 | |
| **total** | **€178** | **−€77** | **€101** | |

A ese ritmo, **unos €650 al mes**. **Tres cuartas partes no son la plataforma corriendo: es la
manera de construirla** (Cloud Build + Artifact Registry, €122 de €178).

## Lo que hay hoy (leído del código, 2026-10-08)

- **Cada push a `main` lanza el CI**, y el CI, `gcloud builds submit` de `cloudbuild.yaml`. El
  2026-10-08 hubo **59 corridas** (el 2026-10-07, una): varias sesiones empujan cada pocos
  minutos. Muchas compilaron y empujaron para que su despliegue saliera `skipped`, porque ya había
  un commit más nuevo (sólo se despliega la punta).
- **Cada construcción hace 15 imágenes** etiquetadas con el commit —`ore`, `ore-informador`,
  `capa-jvm`, `capa-node`, `capa-python`, `idp`, `ore-drivers`, `ore-serve`, `ore-iam`, `ore-cofre`,
  `ore-postgres`, `puesto-python`, `puesto-node`, `puesto-jvm`…—, cada una `--cache-from` la de la
  rama. Sólo los binarios de Rust se reutilizan por su huella (`ore-binarios:h-<huella>`): las
  imágenes se rehacen y se suben aunque sus fuentes no hayan cambiado.
- **`gcloud builds submit` va sin `--region`**: la construcción corre en el pool `global`, y el
  registro está en `europe-west1`. Bajar la caché de 15 imágenes y subirlas otra vez **puede cruzar
  regiones en cada corrida** (G0 lo confirma o lo descarta).
- **Artifact Registry no tiene política de limpieza**: cada commit deja otra tanda, y la del puesto
  de Python pesa gigas.
- **Los nodos están encendidos siempre**; los de puestos y trabajos escalan, el de sistema no baja.
- **La cuota `CPUS_ALL_REGIONS` es de 12 vCPU** y Google no la amplía (`NOT_ENOUGH_USAGE_HISTORY`).
  Por ella se paró el CI en GKE el 2026-10-01, y lo que O5 liberó lo usa Postgres (0058). Cualquier
  nodo nuevo tiene que caber en ella.

## 2026-10-09 · la cuenta cerrada seguía cobrando: las máquinas, apagadas

**Lo visto** (sólo lecturas, `gcloud`): la cuenta de facturación del proyecto (`01AC8A-568391-9ADCC2`)
figura **cerrada**, el proyecto sigue con `billingEnabled: true`, y el saldo pasó de 104 € a 111 € en
una noche. El API de GKE contesta 403 (*requires billing*), pero **Compute Engine seguía corriendo y
cobrando**: los nodos de `pg` (2 × `n2-standard-2`, normales), el de `sistema-spot`
(`e2-standard-4` spot) y `modelos-e0` (`e2-micro`), además de 15 discos, 3 IPs estáticas, Cloud NAT
(`ore-mesh-salida`) y las 3 reglas de reenvío de los dos balanceadores globales. Unos **$9/día** a
precio de lista (de memoria, sin medir), de ellos ~$6 las máquinas.

**Lo hecho** (con el go de la persona, 2026-10-09): las tres máquinas apagadas, nada borrado.

| qué | antes | ahora | para volver |
|---|---|---|---|
| `gke-ore-mesh-pg-9f6e5373-grp` | 2 | 0 | `gcloud compute instance-groups managed resize gke-ore-mesh-pg-9f6e5373-grp --size=2 --zone=europe-west1-b` |
| `gke-ore-mesh-sistema-spot-f053e8ae-grp` | 1 | 0 | `… resize gke-ore-mesh-sistema-spot-f053e8ae-grp --size=1 --zone=europe-west1-b` |
| `modelos-e0` | RUNNING | TERMINATED | `gcloud compute instances start modelos-e0 --zone=europe-west1-b` |

Los 11 discos de datos (`pvc-…`) y el de `modelos-e0` siguen, sin máquina; los de arranque de los
nodos se fueron con ellos, como siempre en GKE (se rehacen al subir el grupo). **Postgres de 0058**
se apagó de golpe, como un corte de luz: sus datos están en sus discos y en GCS.

**Lo que sigue cobrando** (~$3/día): los balanceadores (no se tocan: los gestionan los controladores
de GKE, y rehacer el dominio y los certificados a mano es peor), las IPs, el NAT, los discos, y el
almacenamiento (Artifact Registry, GCS, Secret Manager, KMS). Es lo que G3, G9 y G11 bajan.

**Al volver la cuenta**: subir los dos grupos y arrancar `modelos-e0` **antes** de nada que necesite
el clúster, comprobar que los pods vuelven (y Postgres, con quien lleva 0058), y luego G0.

## 2026-10-10 · Google vuelve: nada construye, y Postgres a cero (A1)

**Lo visto** (lecturas): Cloud Build sin nada en curso (la última construcción, 2026-10-08 18:30)
y ninguna corrida de GitHub en cola. Pero el grupo `pg` tenía autoescalado con **mínimo 1**: al
volver la facturación GKE rehízo un `n2-standard-2` normal (~$2,5/día) con el resto de la malla a
0. Lo fijo, medido: Artifact Registry **266 GB** sin limpieza (`ore` 250, `bastion` 16), **222
versiones activas** en 27 secretos, 13 discos (~200 GB), 3 IPs externas, 3 reglas de reenvío, el
NAT. Y el repositorio `describeloai/ore` es **público**: los runners de GitHub no cuestan, así que
construir puede salir de Cloud Build sin esperar a G5.

**Lo hecho** (con el go de la persona): `pg` a mínimo 0 y tamaño 0. **El autoescalador lo subió a
2 en el acto**: el pool no tiene taint, y los pods de `kube-system` que esperan sitio (con
`sistema-spot` a 0) lo empujan. Así que `pg` va **sin autoescalado y a 0**. `malla/80-postgres-gcp.sh`
baja su mínimo por defecto a 0.

| qué | antes | ahora | para volver |
|---|---|---|---|
| `pg` | autoescalado 1–3, 1 nodo | sin autoescalado, 0 | `gcloud container node-pools update pg --cluster ore-mesh --zone europe-west1-b --enable-autoscaling --min-nodes 0 --max-nodes 3 --location-policy ANY` (con `sistema-spot` de pie antes) |

**Nivel 3 · €20 al mes sostenidos** (propuesto el 2026-10-10; cada paso con su go):
A1 Postgres a cero (hecho) · A2 retención en Artifact Registry (G3) · A3 versiones viejas de
secretos (G11) · A4 discos huérfanos (G11) · B1 construir en los runners de GitHub en vez de Cloud
Build · B2 imagen por huella (G4) · C1 el clúster duerme (G8; medir si cabe en un `e2-standard-2`
spot) · C2 túnel de Cloudflare (G9) · C3 presupuesto de €20 con alertas (G7, de la persona).
Estimado: ~$23–26 al mes; por debajo de €20 si el sistema cabe en un `e2-standard-2` o la IP de
salida se suelta mientras no haya un SFTP de cliente.

### A2 · la limpieza de Artifact Registry, diseñada (2026-10-10)

**La regla** (`malla/registro-limpieza.json`, la misma para `ore` y `bastion`), en el orden en que
se lee —una Keep gana a la Delete—:

1. **lo desplegado**: las etiquetas `main` y `en-uso`. `en-uso` es nueva: el despliegue la mueve
   con `:main` y `:1` (`ci.yml`), y hoy se puso a mano en las 14 imágenes desplegadas. Hace falta
   porque la regla casa **por prefijo**, y `1` es también el principio de uno de cada 16 commits;
2. **lo fijado en los manifiestos**: `neon:8269bece…` y `vm-compute-node-v17:baad49aa…` (0058),
   `v0.49.1-ore.1` (NeonVM y el autoescalado). Quien cambie uno de esos fijados, lo cambia aquí;
   `simular-limpieza.py --manifiestos malla` lo comprueba contra **todos** los ficheros de
   `malla/`, estén o no en una kustomization (el proxy de 0058, lo comentado);
3. **lo de fuera** que no se reconstruye: `forgejo:15`, `postgres:16`, `neonvm-kernel`;
4. **las 3 versiones más nuevas** de cada paquete, para volver atrás;
5. **todo lo demás, borrado a los 7 días**. Lo que nombra un índice multi-arquitectura no se borra
   mientras el índice siga (lo garantiza Google).

**Simulado** (`malla/simular-limpieza.py`: aplica la regla a la lista real, lee los manifiestos y
cuenta cada capa una vez, como cobra el registro; la duda de la documentación —si «las 3 más
nuevas» cuenta las hijas de un índice— se resuelve tomando la que guarda menos):

| repositorio | hoy | el primer día | a los 8 días sin construir |
|---|---|---|---|
| `ore` | 250 GB, 5.486 versiones | ~125 GB (lo de esta semana aún no cumple 7 días) | **37 GB, 81 versiones** |
| `bastion` | 17 GB, 16 versiones | — | **13 GB, 10** (`env:0.29.0-sm120.1` y `.2`) |

De los 37 GB de `ore`, **26 son la caché de construcción de Neon** y 4 las imágenes de Neon; lo de
ORE en sí (puestos, capas, servicios, binarios, deps) cabe en ~6 GB. **La caché de Neon se borra**
(lo acepta quien lleva 0058, 2026-10-10: solo hace larga la próxima reconstrucción); va a mano, una
vez, al aplicar la regla, porque «las 3 más nuevas» la guardaría. Con eso, ~11 GB en `ore` y 13 en
`bastion`: de ~$27 al mes a **~$2,5**. Un `:main` o `en-uso` que lleve meses sin moverse no se
borra nunca (simulado a 30 días: la Keep gana a la antigüedad).

**Aplicado** (con el go de la persona, 2026-10-10, sin la prueba en seco de Google): la regla en
`ore` y `bastion` (`set-cleanup-policies --no-dry-run`), después de pasar otra vez el simulador
con `--manifiestos malla` (64 citas, todas se quedan); y el paquete `cache` borrado entero. Google
pasa la regla una vez al día: el tamaño del repositorio (262 GB al aplicar; lo recalcula con
retraso) se mide mañana y a los 8 días, y el número va aquí.

### A3 · Secret Manager: una versión por secreto (2026-10-10)

**Lo visto:** 27 secretos y **225 versiones cobradas** (222 activas y 3 desactivadas; Google cobra
las dos), ~$13 al mes. Todo lector pide `latest` y solo al arrancar (las ~25 lecturas de `malla/`
y del aprovisionador): las anteriores no las lee nadie. De dónde salían: `t-*-forja-admin` 62 cada
uno, y **creciendo** —el `comprobar` de `46-la-forja-del-inquilino.yaml` tomaba cualquier fallo al
leer (la identidad del pod aún no lista en un nodo recién creado) por «no hay testigo», y acuñaba
token y versión: a ráfagas, 17 el 16-sep—; `t-demo-agente-*` 37 cada uno (el bug del aprovisionador
que ya compara por huella desde el 14-sep); `base-del-cofre`, las rotaciones de 0054, desactivadas.

**Lo hecho:** el escape, cerrado en git (`39fb0284`: solo `NOT_FOUND` acuña; lo demás se
reintenta), y con el go de la persona, **198 versiones destruidas**: queda la más nueva de cada
secreto, **27 activas, ninguna desactivada**. De ~$13 a **~$1,3 al mes** (6 versiones gratis).
Queda: los tokens viejos de Forgejo que acuñó el escape, revocarlos con el clúster despierto; los
10 `*-cofre-fuente-*` de fuentes de prueba de septiembre, revisarlos contra el cofre; y un aviso
de secretos con más de una versión cobrada.

### A4 · los discos (2026-10-10)

**Lo visto:** 12 discos, 125 GB, ~$9,5 al mes, cobrando sin máquinas. Dos eran **huérfanos**: la
`forja-datos` de `t-prueba` y de `t-prueba-dos`, celdas borradas, que la clase `retiene` guardó a
propósito (sin tocar desde el 02-oct y el 14-sep). El resto está en uso: las forjas de `t-demo`,
`t-victor` y la compartida, `idp-datos` (Keycloak), los cinco de `ore-pg` (0058) y `modelos-e0`
(el gateway de Bastion, parado).

**Lo hecho** (con el go de la persona): una instantánea de cada huérfano
(`forja-t-prueba-20261010`, `forja-t-prueba-dos-20261010`, 1,7 MB y 0,3 MB dentro, en
europe-west1) y los dos discos y sus `PersistentVolume` borrados: **10 discos, 105 GB, ~$7,3**.

**Queda para C1** (con el clúster despierto): pasar a `pd-standard` las cuatro `pd-balanced` en uso
(forjas e idp; ~$2,6 menos, copiando cada disco), y los discos de arranque de los nodos a lo que
usan (`jobs-s` arranca con 50 GB `pd-balanced`).

### B1·0 · construir en el runner de GitHub, medido (2026-10-10)

El repositorio es público: los runners de GitHub no cuestan. `construir.yml` (sólo en ramas `b1/*`
y a mano, sin autenticarse en GCP ni bajar nada del registro) compila los binarios —las
dependencias por su receta, en la caché de GitHub— y construye las catorce imágenes en local, en
`ubuntu-24.04` (4 núcleos, 16 GB; 145 GB de disco, 110 libres tras quitar .NET, Android y GHC).
Corridas 38048545642 (en frío) y 38049751766 (con la caché), sobre `origin/main`:

| etapa | en frío | con la caché |
|---|---|---|
| dependencias de Rust | 617 s (se compilan) + 23 s (a la caché: 0,96 GB) | 27 s (de la caché) |
| nuestro código, entero | 311 s | 299 s |
| las 14 imágenes, 3 a la vez | 105 s | 107 s |
| **el job** | **1.074 s (~18 min)** | **461 s (~7,7 min)** |

Pico de disco usado: 46 GB. La mayor imagen, `puesto-python`, 957 MB. **Cabe, y en caliente tarda
menos que Cloud Build (~19 min).** Con B1·1 (una imagen por huella) y los binarios guardados por
su huella, un commit que no toca Rust no compila nada ni rehace las imágenes que no cambian.

### B1·1 · una imagen por huella (G4), hecho en local (2026-10-10)

`ci/huella-de-la-imagen.py` da 16 hex por imagen siguiendo su cadena en el Dockerfile (su `FROM` y
cada `COPY --from`): el texto de cada etapa sin comentarios, el contenido de lo que copia del
contexto (lo que `.dockerignore` deja pasar), el nombre de cada imagen de fuera, y en vez de la
etapa `bin` la huella de los binarios. `--explicar <etapa>` dice de qué sale; `--comprobar`, que las
14 del CI existen y se pueden seguir; `FORZAR=<x>` las mueve todas (una base de fuera se nombra por
su etiqueta: un parche suyo no mueve la huella). `ci/prueba-huella-de-la-imagen.py`, sobre una
copia, 11 casos: un fichero del puesto de Python mueve sólo `puesto-python`; la lista de Node,
`capa-node` y `puesto-node`; el informador, el tema del login, una instrucción de `serve` y la
etapa `pyright`, sólo lo suyo; un comentario, un fichero fuera del contexto o una etapa que nadie
usa, nada; la huella de los binarios, las nueve que los copian. Las dos van al job `suite` de
`ci.yml`. Lo usa B1·2.

### B1·2 · publicar por huella, medido en la rama (2026-10-10)

`construir.yml` (aún sólo en `b1/*`, con la etiqueta `prueba-b1-<commit>`): por cada imagen, si el
registro ya tiene `<imagen>:h-<huella>` (metadato: no se baja nada) se le añade la etiqueta por el
API del registro; si no, se construye en el runner y se sube con las dos. Los binarios, por su
huella, en la caché de GitHub, y sólo si alguna imagen que se construye los copia. Tres corridas:

| corrida | qué pasó | el job |
|---|---|---|
| 38051037533: la primera (ninguna huella en el registro) | las 14 construidas y subidas; deps de la caché, binarios compilados (223 s) | **492 s** |
| 38051711781: el mismo commit otra vez | las 14 reutilizadas (2–6 s cada una) | **82 s** |
| 38051989928: un commit que sólo toca `puesto/python/agente.py` | 13 reutilizadas, `puesto-python` construida y subida (64 s); binarios de la caché (0 s) | **125 s** |

Lo que costaba ~19 min de `E2_HIGHCPU_8` en Cloud Build por commit es, sin pagar nada, 1,5–2 min si
no cambia Rust y ~8 si cambia. Siguiente: B1·3, el relevo en el CI de `main`.

### B1·3 · el relevo, en hitos (2026-10-10)

1. **`construir` de `ci.yml` al runner** — hecho en local: los pasos de B1·2 con la etiqueta del
   commit (`<sha12>`, la que el job `imagen` mueve a `main`, `1` y `en-uso`); se quedan la punta y
   las salidas `clave`/`clave_motor`; fuera `gcloud builds submit`, la espera a Cloud Build y
   `REGION_CB`. `construir.yml` queda sólo a mano. Comprobado: YAML, `bash -n` de los once guiones
   de `construir` e `imagen`, `ci/solo-papel.py`.
2. **Desplegar con el clúster dormido**: si `sistema-spot` no tiene nodos `Ready`, `imagen` mueve
   las etiquetas y **no toca los Deployments**: con los nodos a 0 el plano de control contesta,
   pero escribir un Deployment pasa por el webhook de Kueue (`mdeployment.kb.io`,
   `failurePolicy: Fail`), que no tiene pods (medido en la primera corrida, 38053087597: la
   primera versión anotaba y falló así). El relevo lo hace `malla/despertar.sh` (C1) volviendo a
   lanzar el job `imagen` de la última corrida (`gh run rerun --job`), que con el clúster en pie
   releva y comprueba como siempre. ⚠️ Mientras tanto `:main` ya es lo nuevo y las anotaciones
   dicen lo viejo: un pod que arrancara antes de ese relevo (`imagePullPolicy: Always`) correría
   lo nuevo con la anotación vieja; por eso el despertar relanza el job antes de dar nada por
   bueno.
3. **El primer push a `main`** — hecho: el `87-postgres-la-entrada.yaml` a `FUERA` de
   `gen-inquilino.py` (`f8287429`; avisada la sesión de 0058), y 39 commits a `main`. Corrida
   38053087597: todas las pruebas verdes, `construir` en el runner en 1.307 s (se construyeron
   las 14: venían seis orígenes nuevos de 0061), las etiquetas movidas (`ore-serve:main` =
   `:f828742982bb`), y el despliegue dormido falló por el webhook de Kueue (hito 2, corregido).
   Corrida 38054786204 (`b968829b`): **verde entera**, `construir` 82 s (las 14 reutilizadas, sin
   binarios), `imagen` 55 s por el camino dormido. `gcloud builds list`: nada desde el
   2026-10-08 18:30. **El CI de `main` ya no paga construir.**
4. **Cerrar la puerta a Cloud Build** (IAM, go): `ore-ci` sin permiso de construir.
5. **La comprobación al despertar**, con C1.

### B2 · una huella por binario (2026-10-11)

**Por qué.** Con B1 el CI dejó de pagar, pero un commit de Rust tardaba ~11 min en `construir`:
33 s el plan, 133 s esperando a `rm` para liberar el disco, 304 s compilando los 24 binarios y
143 s rehaciendo y subiendo las imágenes que los copian. Una sola huella para todos los binarios:
tocar `ore-serve` recompilaba el lector de SFTP, rehacía nueve imágenes y relevaba todos los
Deployments, también `ore-cofre` e `ore-iam`.

| paso | qué | medida |
|---|---|---|
| H0 | compilar un subconjunto sin recompilar dependencias | en local (16 cpu, 6 GB): los 24, 409 s; `-p ore-serve`, 404 s y **78 dependencias de terceros otra vez** (otras features unificadas); todos los `-p` y `--bin ore-serve`, 304 s y ninguna. `compilar-binarios.sh` elige con `--bin` y los mismos `-p` con los que se cocinan las dependencias |
| H1 | `ci/huella-de-los-binarios.py`: la de cada binario, de su paquete y de los del workspace de los que depende (`[dependencies]` con `path`, sin `tests/`, `examples/` ni `.md`) y de lo que su código de producción incluye de fuera (`include_str!`, leído del código; los de un `#[cfg(test)] mod` no). `--comprobar` cuadra los binarios del Dockerfile con los del guion | prueba de 15 casos en 22 s; `ore-core` mueve 22 de 24, `ore-serve` sólo el suyo |
| H2 | la huella de cada imagen lleva sólo las de los binarios que copia | prueba de 14 casos: `ore-serve` rehace `serve`; `ore-store-gcs`, las cinco que lo copian |
| H3 | `construir`: una caché de binarios con su manifiesto (`.huellas.json`), restaurada por prefijo; se compilan sólo los que faltan; el disco en segundo plano; el **ensayo** (`gh workflow run ci --ref <rama> -f ensayo=true`: sólo `construir`, sube `h-<huella>`, ni etiqueta de commit ni despliegue) | abajo |
| H4 | `imagen`: la anotación `ore.dev/binarios` es la huella de la imagen de cada Deployment (`ore-serve:h-…`), no una clave global | el primer push a `main` releva todo una vez (cambia el formato) |

**Medido en la rama** (`ci/b2-huella-por-binario`):

| corrida | qué | `construir` |
|---|---|---|
| 38135140187 | en frío: sin binarios ni dependencias en la caché | 14,5 min (688 s cocinar y compilar los 24) |
| 38136105667 | **un commit que sólo toca `ore-serve`** | **288 s**: compila `ore-serve` (119 s), los 23 de la caché, rehace una imagen (17 s) |

- **El disco no hacía falta:** 85 GB libres al empezar y 102 al acabar, con todo compilado. El `rm`
  sigue, en segundo plano, sin que nadie lo espere.
- ⚠️ **Una caché de una rama no la ve `main`** (GitHub). El primer push de B2 a `main` va en frío
  (~15 min) y releva todo una vez; los siguientes, como la segunda corrida.
- **Lo que manda ahora** es la prueba más larga (`conectores`, ~8 min), no `construir`. Y tocar
  `ore-core` sigue compilando 22 binarios: la cadena `ore-code → ore-core` es el camino crítico.
- Fuera lo que B1·4 dejó muerto: `cloudbuild.yaml`, `construir.yml` (lo hace el ensayo) y
  `ci/huella-de-los-binarios.sh`.

### C2 · lo que de verdad cuestan los balanceadores (2026-10-10)

La tarifa pública (Precios de red, Cloud Load Balancing, leída hoy): las reglas de reenvío
**globales** —externos globales, clásicos y regionales de aplicación, juntos— cuestan **0,025 $/h
por las cinco primeras, sean 1 o 5** («si creas 3 reglas de reenvío, se te cobrará la misma
cantidad»), y 0,01 $/h cada una más. Y **una IP externa estática asignada a una regla de reenvío
no se cobra**. Nuestras tres reglas (la `Gateway` y las dos del `Ingress` del idp) pagan un solo
tramo: **~18 $ al mes en total**, y las IPs `ore-puerta` y `ore-idp`, nada.

⇒ **Juntar los dos balanceadores en uno no ahorra nada** (descartado). Lo único que baja esos ~18 $
es no tener ninguna regla global: el túnel (G9). Corrige la tabla del nivel 2 («balanceadores e
IPs ~$32»): eran ~$18. Y la entrada TCP de Postgres (0058, el `87`) sería una regla **regional**,
otro tramo de ~18 $.

### C2 · diseño: el túnel, sin cambiar un nombre (decidido con la persona el 2026-10-10)

**Opción C**, elegida entre cuatro:

| | cómo | zona `paladio.io` (GoDaddy) | nombres | coste |
|---|---|---|---|---|
| A | `paladio.io` entera a Cloudflare, nombres a un nivel (`demo.paladio.io`) | migrar todos los registros, correo incluido | cambian (`fundar.rs`, `gen-inquilino`, migraciones, filas de `iam`) | 0 |
| B | A sin renombrar, con el certificado de dos niveles de pago | igual que A | no | 10 $/mes |
| **C** | **Cloudflare for SaaS sobre una zona ancla** (`ontoforge.org`): los nombres de siempre como *custom hostnames* (100 gratis, a cualquier profundidad), por CNAME al túnel | **un registro** (`login`) | **no** | 0 |
| D | sin Cloudflare: un proxy en el nodo con IP pública y el DNS reescrito al despertar | un registro | no | ~0 |

D se descarta: el nodo queda expuesto a internet, su IP cambia con cada despertar y con cada
reclamación de la máquina spot, y no tiene nada delante.

**La forma:** navegador → Cloudflare (TLS, un certificado por nombre) → túnel (`cloudflared` en
`ore-system`: sale de la malla; no se abre nada hacia dentro) → **Envoy Gateway** (ClusterIP) → las
`HTTPRoute` de siempre.
- La `puerta` sigue siendo una `Gateway`, de clase Envoy. Las rutas de los inquilinos y la frontera
  de `allowedRoutes` (`cargas`, `identidad`) no cambian; el IdP pasa de `Ingress` a una
  `HTTPRoute` en `identidad`.
- Lo de GKE se traduce: `HealthCheckPolicy` → las readiness de los pods; `GCPBackendPolicy` de
  300 s → `BackendTrafficPolicy`; las NetworkPolicies de `130.211/22` y `35.191/16` → desde Envoy.
- `cloudflared`, con una regla: todo a Envoy. Su token, en Secret Manager.
- Dormida, Cloudflare contesta 530 y nada cobra. La consola ya lo lee como dormida (falla el
  descubrimiento OIDC).
- `paladio.io` sigue en GoDaddy y `ore.paladio.io`/`pg.paladio.io` en Cloud DNS. Sólo cambian los
  registros que apuntan a las IPs de los balanceadores: pasan a CNAME al ancla.

**Hitos:**
- **C2·0 medir** (piezas nuevas al lado, nada existente se toca): un nombre de prueba
  `c2-prueba.ore.paladio.io`. ① SaaS con túnel en el plan gratuito; ② **100 s sin datos** de
  Cloudflare contra el flujo de los puestos (0037: ¿latido antes de 100 s?); ③ **100 MB por
  petición** contra la subida de ficheros (0049); ④ lo que ocupa Envoy en el nodo.
- **C2·1 la puerta nueva:** Envoy Gateway y una `Gateway` de túnel junto a la actual; las rutas,
  con dos `parentRefs` (`gen-inquilino`, la forja).
- **C2·2 el túnel:** `cloudflared`, su secreto y sus NetworkPolicies; los *custom hostnames* de
  `login`, `iam`, `demo`, `victor` y `ore-mesh`, probados contra Cloudflare antes de tocar el DNS.
- **C2·3 el cambio:** TTL a 60; CNAME en Cloud DNS y `login` en GoDaddy (la persona). Se comprueban
  el login, la consola, el CLI, el flujo, despertar y dormir. Mientras siga el balanceador, volver
  atrás es un registro.
- **C2·4 el aprovisionador:** fundar un inquilino da de alta su *custom hostname*. La comprobación
  del comodín (`aprovisionar-inquilino.sh`, «IP_COMODIN») pasa a comprobar el CNAME.
- **C2·5 retirar**, tras 24 h estable: la `Gateway` de GKE, el `Ingress`, el `ManagedCertificate`, el
  `certmap`, `ore-comodin` y su autorización, y **liberar `ore-puerta` y `ore-idp`**: una IP reservada
  sin regla de reenvío cobra ~7 $/mes cada una, casi lo que se ahorra.
- **C2·6 (opcional):** un Worker gratuito que ante un 530 enseñe «la plataforma duerme» con el
  enlace para despertarla, o que la despierte él.

**Fuera de C2:** la entrada TCP de Postgres (0058, el `87`). El plan gratuito de Cloudflare no
pasa TCP público: sería una regla regional, ~18 $/mes. En desarrollo, `port-forward` o
`cloudflared access`; la entrada pública, cuando haya clientes. Lo HTTP/WS de 0058 sí va por el túnel.

**Hecho el 2026-10-10:**
- La cuenta de Cloudflare es de la persona. El token `ore-c2` (Tunnel, SSL and Certificates, DNS
  y Workers, sólo sobre `ontoforge.org`) vive en su `.env.local`, y no va a Vercel.
- La zona `ontoforge.org` está en Free. Sus registros (el Website Builder de GoDaddy y `_dmarc`)
  están en DNS only, como estaban.
- Los nameservers están cambiados en GoDaddy, `darwin`/`kelly.ns.cloudflare.com`, sin DNSSEC.
  La zona pasó a `active` a las 20:26 UTC. Cloudflare for SaaS lo habilitó la persona (0 de 100
  nombres).

### C2·0 · medido (2026-10-10)

Montaje: el túnel `ore-c2-prueba` (gestionado desde Cloudflare, una regla, todo a un servidor de
prueba en el PC), con `entrada.ontoforge.org` (proxied, CNAME al túnel) como *fallback origin*. Como
nombre de prueba, `c2-prueba.ore.paladio.io`: un CNAME en Cloud DNS a `entrada.ontoforge.org` y un
*custom hostname* con DV por HTTP.

| | resultado |
|---|---|
| ① SaaS + túnel en Free | ✔ **nombre verificado y certificado emitido en ~1,5 min** sin TXT: basta el CNAME. Al origen llegan el `Host` original, `X-Forwarded-Proto: https` y la IP real (`Cf-Connecting-IP`, `X-Forwarded-For`) |
| ② callado 90 s | ✔ 200 |
| ② callado 130 s | ✘ **524 a los ~125 s**. El límite de Free es el tiempo hasta el **primer byte** |
| ② flujo SSE, un evento cada 60 s | ✔ vivo más de 240 s |
| ② flujo SSE, un evento cada 110 s | ✔ vivo; ya con el flujo abierto, un hueco de 110 s no lo corta |
| ③ subida de 99 MB y de 101 MB | ✔ 200 (31 s y 36 s desde casa) |
| ③ subida de 110 MB | ✘ **413 de Cloudflare, en 1 s**. El techo es **100 MiB** (104,9 MB) por petición |

**Lo que significa:**
- **El flujo de los puestos no se toca**: ya late cada 10 s (`LATIDO`, `ore-serve/src/puestos.rs`). Una
  respuesta normal tampoco: `ore-entrada` corta lo callado a los 30 s (`PLAZO`).
- **Las subidas tampoco, hoy.** La puerta pública (`ore-serve`, `ore-iam`) ya acepta como mucho
  **1 MiB** por petición (`CUERPO_MAXIMO`, `ore-entrada/src/http.rs`). Los 5 GiB de `SUBIDA_MAXIMA`
  (0049 B4b·1) son de `ore-medios` en el 8098, adonde sólo llega el puesto desde dentro: no pasa por
  la puerta, y no pasará por el túnel. **Condición de diseño a partir de ahora:** una subida de
  más de 100 MiB desde fuera (la consola, un `ore upload`) va por trozos o con URL firmada. No
  bloquea C2·3.
- ④ (lo que ocupa Envoy) se mide en C2·1, en la malla.

### C2·1 · la puerta nueva, escrita (2026-10-10; se aplica al despertar)

- **Envoy Gateway v1.8.5**, rendido por `malla/gen-envoy-gateway.sh` en `malla/19-envoy-gateway.yaml`
  (como Flux: no hay `helm-controller`). No es la v1.9 porque los CRDs de Gateway API los gestiona
  GKE (v1.5.0) y la v1.8 es la que está probada contra la v1.5; la v1.9 pide la v1.6. Al chart se le
  quitan sus CRDs de Gateway API, el webhook `topologyInjector` y el TTL del Job `certgen`.
- **`malla/19-la-puerta-del-tunel.yaml`:**
  - `EnvoyProxy tunel`: Service **`ClusterIP`** con nombre fijo `puerta-del-tunel`. Por defecto
    sería `LoadBalancer`, es decir, otra regla de reenvío.
  - `GatewayClass ore-tunel` y `Gateway puerta-tunel` (`ore-system`) con dos listeners HTTP: `ore`
    (`*.ore.paladio.io`, a `cargas` e `identidad`, como la `14`) y `login` (`login.paladio.io`,
    sólo a `identidad`).
  - `BackendTrafficPolicy` de **300 s**: Envoy corta a los 15 s si no se le dice nada.
  - NetworkPolicies: `envoy-gateway-system` cerrado. Entra `cloudflared` (`ore.dev/rol: tunel`, en
    C2·2) al 10080; Envoy sale sólo al controlador y a los backends.
- **Rutas:** la de cada celda (`43`, que llega a `t-demo` y `t-victor` por el convergedor) y la de
  `iam` (`67`) cuelgan de las dos puertas. El emisor pasa a ser una `HTTPRoute` (`63`, listener
  `login`) al lado del `Ingress`. Cada destino tiene su NetworkPolicy desde Envoy.
- Comprobado en local: `kubectl kustomize` (193 objetos) y `gen-inquilino.py` (el render de `victor`
  lleva la segunda puerta y su política). `--comprobar` sólo falla por `87` en `FUERA` y en la lista,
  que ya fallaba en `main` antes de esto (0058, `5d10fb11`).
- **Al despertar se mide:** Gateway `Programmed`, rutas `Accepted` en las dos puertas, `curl` a Envoy
  por `port-forward` con cada `Host` (celda, `iam`, login), el flujo SSE más de 15 s, y ④ lo que
  ocupa en el nodo.

### C2·2 y C2·3 · el túnel en la malla y el cambio de DNS (2026-10-11)

**C2·2:**
- Túnel `ore-malla` en Cloudflare, configurado desde allí: `pg.ontoforge.org` → `tcp://proxy-postgres
  .ore-pg:5432`, y todo lo demás a `http://puerta-del-tunel.envoy-gateway-system:80`.
  `entrada.ontoforge.org` es su CNAME y el *fallback origin*. El túnel de prueba, borrado.
- Token en Secret Manager (`tunel-token`), legible sólo por `ore-tunel` (Workload Identity desde
  `ore-system/tunel`).
- `malla/19-el-tunel.yaml`: `cloudflared` 2026.9.3 con `--token-file` desde un `emptyDir` en memoria
  y red cerrada. La cuota de `ore-system` exige `limits.cpu`.
- Los cinco *custom hostnames* (`login`, `iam`, `demo`, `victor`, `ore-mesh`) validados por TXT
  **antes** de tocar el DNS: los de `ore.paladio.io` en Cloud DNS y los de `login` en GoDaddy (la
  persona). Los certificados salieron en ~2 min.
- Envoy necesitaba el DNS en su NetworkPolicy: encuentra a su controlador por nombre.
- Las celdas cuelgan de la puerta nueva cuando las reescribe el convergedor, cada hora o con
  `kubectl create job --from=cronjob/aprovisionador`.

**C2·3, el cambio** (08:33–08:44 UTC):
- Cloud DNS: `*.ore.paladio.io` y `ore-mesh.ore.paladio.io`, de `A 136.69.102.80` a CNAME
  `entrada.ontoforge.org` (TTL 60), en una transacción.
- GoDaddy (la persona): `login`, de `A 136.68.214.241` a CNAME `entrada.ontoforge.org`.
- Comprobado: `iam`, `demo` y `victor` dan 200 por Cloudflare; la consola lista las celdas; en
  `login` funcionan el descubrimiento, JWKS y la página de entrada, con formulario a
  `https://login.paladio.io`.

**Tres tropiezos, los tres corregidos:**
1. **`iam` sin resolver de ~07:47 a 08:34.** Los TXT de validación bajo `iam.ore.paladio.io` lo
   convirtieron en nodo vacío, y un comodín no cubre un nombre que existe. ⇒ CNAME explícito
   `iam.ore.paladio.io`. **C2·4 tiene que crear el registro propio del nombre antes que sus TXT.**
2. **`http://login.paladio.io` daba 200 en claro**; el balanceador redirigía con 301. ⇒ *Always
   Use HTTPS* activado en la zona `ontoforge.org` (cubre los *custom hostnames*). Es una
   configuración de Cloudflare, fuera del repositorio.
3. **Cookies de Keycloak sin `Secure`, con `SameSite=Lax`** (por el balanceador: `Secure;
   SameSite=None`). Envoy reescribe `X-Forwarded-Proto` a `http`, porque le llega HTTP desde
   `cloudflared`, y `numTrustedHops` no lo evita. ⇒ La ruta `login` fija `X-Forwarded-Proto:
   https`, que no miente tras el punto 2. Se queda también `ClientTrafficPolicy` con un salto de
   confianza, para que la IP del cliente sea la de la persona y no la del túnel.

### C2·4 · el aprovisionador da de alta los nombres (2026-10-11)

- `malla/cloudflare-saas.py asegurar|quitar <host>`: el *custom hostname* en `ontoforge.org`.
  Valida por HTTP con el CNAME que ya escribe el paso ⑧, sin TXT bajo el nombre, así que no hay
  trampa del comodín. Contesta como `preguntar`: 0 estaba, 3 hecho, 2 no se sabe (pasada en
  rojo), 4 sin token (se dice y sigue).
- Lo llaman el aprovisionador (⑧b al dar de alta, y en la retirada ⓪) y `desaprovisionar-inquilino.sh`
  (⑤).
- El token es `cloudflare-saas` en Secret Manager, creado por la persona: sólo «SSL and
  Certificates: Edit» sobre `ontoforge.org`. Contra el DNS da 403, comprobado. Lo lee
  `ore-aprovisionador` por el init container, como opcional.
- Pasada entera en el clúster: `demo` y `victor` «active, ya estaba»; las tres retiradas
  (`cobalt-polar-camel`, `prueba`, `prueba-dos`) «no estaba».
- Fuera de esto: la entrada propia de una organización (su dominio) también se da de alta así,
  pero su CNAME lo pone su dueño, y hasta entonces Cloudflare no lo valida.

### C2·5 · fuera los balanceadores (2026-10-11, 09:30–09:50 UTC) — C2 CERRADO

- **Antes de borrar:** las métricas (`loadbalancing…/https/request_count`) daban 0 peticiones en
  la `Gateway` desde el cambio de las 08:33 y en el login https desde las 08:53. Al puerto 80 del
  login llegaron 8 en 10 min, bots barriendo IPs.
- **Los certificados de los cinco nombres, a validación por HTTP.** Se emitieron por TXT y la
  renovación (~90 días) habría pedido TXT nuevos: una avería con fecha. Con el CNAME puesto, HTTP
  se renueva solo.
- **La malla** (`34518e26`): fuera `14-la-puerta.yaml`; la `43`, la `67` y la `63` sin `parentRef` a
  `puerta`, sin `HealthCheckPolicy`, `GCPBackendPolicy`, `Ingress`, `ManagedCertificate`,
  `FrontendConfig`, `BackendConfig`, NEG ni rangos de Google. La comprobación ⑮ de
  `gen-inquilino.py` mira el plazo en la `BackendTrafficPolicy` de la puerta (300 s > 240).
- **En vivo:** las celdas se limpiaron solas al converger (su Kustomization poda). Lo de
  plataforma (`prune: false`) se borró a mano: la `Gateway` `puerta`, el `Ingress` `idp` y sus
  piezas. Reglas de reenvío: 0 a las 09:46:47. Liberadas `ore-puerta` y `ore-idp`. Borrados el mapa
  `ore-puerta`, el certificado `ore-comodin`, su autorización DNS y
  `_acme-challenge.ore.paladio.io`.
- **Comprobado después:** `login`, `iam`, `demo` y `victor` dan 200 por Cloudflare;
  `http://login` → 301; cookies `Secure; SameSite=None`.

**El fijo con la malla dormida, ahora:** sin balanceadores (eran ~18 $, más ~18 $ de Postgres
desde el día 10). Quedan los discos (~7 $), NAT e IP de salida (~4–5 $), Artifact Registry
(~2,5 $), secretos (~1,3 $) y las zonas DNS (~0,6 $): **~16 $/mes**, y lo despierto, aparte. Se
medirá en la factura de la semana que viene.

**Fuera de C2:** quitar de GoDaddy los TXT `_cf-custom-hostname.login` y `_acme-challenge.login`
(ya no hacen falta; la persona) y los de `ore.paladio.io` en Cloud DNS (inofensivos ahora que
cada nombre tiene su registro propio).

### C2 · la entrada de Postgres, también al túnel (2026-10-10, decidido por la persona)

0058 desplegó hoy (13:12) su entrada, `ore-pg/proxy-postgres`, como `LoadBalancer` L4 en la IP
`ore-pg-entrada`. Era **una regla de reenvío regional: ~18 $/mes aunque la malla duerma**, y con
ella el fijo dormida se quedaba en ~34 $ aunque C2 acabara. La persona decidió llevarla también al
túnel en desarrollo:
- `malla/postgres-entrada/proxy.yaml`: el Service, a **`ClusterIP`**. Lo que era queda escrito en
  su comentario, para volver a ponerlo cuando haya clientes que esperen `psql -h` directo.
- Aplicado en vivo, con la malla dormida (el Service lo resuelve el plano de control): la regla
  regional se borró a las 21:22 UTC. Liberada la IP `ore-pg-entrada`: sin regla detrás cobraba
  ~7 $/mes. Borrado `*.europe-west1.pg.paladio.io A`, para no dejar un nombre apuntando a una IP
  que ya no es nuestra.
- **C2·2 la recoge** con reglas propias de `cloudflared`, sin pasar por Envoy (el proxy termina su
  propio TLS):
  - el 443 (HTTP/WS) como un *custom hostname* más, con origen `https://proxy-postgres.ore-pg:443`;
  - el 5432 como `tcp://` detrás de un nombre de `ontoforge.org`, con `cloudflared access tcp` en el
    cliente. El endpoint va en `options=endpoint=`, porque por el túnel no llega SNI; hay que
    comprobar que nuestro proxy lo lee.
- `87-postgres-la-entrada-gcp.sh` volvería a crear la IP y el `A` si se relanza: no hace falta
  mientras sea `ClusterIP`.
- `gen-inquilino.py`: `87-postgres-la-entrada.yaml` sale de `FUERA` (estaba ahí mientras iba
  comentada; su frase decía que saliera al desplegarse). Era lo que tenía rojo el CI de `main`
  desde `5d10fb11` (paso «la plantilla del inquilino se puede renderizar»).

**Lo que queda:** el *custom hostname* y el CNAME de prueba están borrados, y el servidor y el
`cloudflared` locales parados. Siguen en Cloudflare el túnel `ore-c2-prueba` (sin conexión) y
`entrada.ontoforge.org` como *fallback origin*; C2·2 los reapunta al túnel de la malla.

### C1 · la malla vive de 08:00 a 00:00, hora de Madrid (2026-10-10 → 2026-10-11)

**Decidido con la persona el 2026-10-11** (`cfdb5117`; lo que quedaba de la inactividad, quitado el mismo día): un horario
fijo, y fuera de él dormida. Cualquiera puede despertarla a mano desde la consola, sin límite
(configuración de desarrollo; en producción real se revisa).

| | quién | cuándo |
|---|---|---|
| **despertar** | Cloud Scheduler `ore-despertar` (`europe-west1`) → `POST https://app.paladio.io/auth/despertar` → `horario.yml` → `malla/despertar.sh` | `0 8 * * *` en `Europe/Madrid` (el cambio de hora lo lleva Scheduler), hasta 3 reintentos. Dormida no hay CronJob que corra, por eso va fuera. El cron de GitHub (`0 6,7` UTC) **no corrió el 2026-10-12** ni a las 06 ni a las 07: fuera |
| **dormir** | la CronJob `dormir-de-noche` (`ore-system`, `timeZone: Europe/Madrid`) → `malla/dormir-de-noche.py` | `0,30 0-2 * * *`: la primera pasada duerme; las demás, sólo si un trabajo de Kueue la tuvo despierta. Dentro del clúster porque el cron de GitHub **no corrió ni una vez en dos horas** el 2026-10-10 (GitHub avisa: sus horarios se retrasan o no corren) |
| **a mano** | la consola (`/auth/despertando` → `/auth/despertar`), `horario.yml` (`despertar`, `dormir`, `dormir-ya`), `curl -X POST https://app.paladio.io/auth/despertar` | cuando sea. Despertada de madrugada, duerme en la siguiente pasada de 00:00–02:30, o sigue hasta la medianoche siguiente |

**Por qué no por inactividad** (lo que hubo del 2026-10-10 al 11: dormir tras 15 y luego 30 min
sin peticiones de una persona). La señal eran las líneas de registro de los pods **vivos** de
`ore-serve` y `ore-iam`: reiniciar uno se llevaba la actividad. El 2026-10-11, al reiniciar
`ore-iam` para arreglar un 502, la malla se durmió **dos veces con la persona dentro** (18:35 a
medias, 19:35 del todo). Un horario no depende de nada que se pueda perder. Quitados de raíz: la
CronJob `dormir-si-inactiva`, `dormir.sh --si-inactiva`, el permiso `pods/log` de `ore-horario`,
la línea de actividad de `ore-iam`, la clase `fondo` y la cabecera `x-ore-fondo` de `ore-serve`, y
el latido de la consola como señal (sigue, sólo para avisar de que se durmió).

**Dormir** (los mismos pasos dentro y a mano): con trabajo de Kueue en curso no duerme (salvo
`dormir-ya`); las **copias al dormir** —la forja, el idp y, si su grupo está en pie, la base del
controlador de Postgres—, porque los datos sólo cambian despierta y las copias diarias no corren
dormidas; y los grupos a 0 por la API de GKE: `pg` y `jobs-s` esperando (GKE hace una operación a
la vez), comprobado hasta tres veces que quedan a 0, y `sistema-spot`, donde corre el guion, el
último y `--async`. Antes del resize se terminan los pods de `pg` y `jobs-s`: los PDB de ore-pg
no dejan desalojar con el grupo entero apagándose y GKE esperaría una hora (2026-10-10, 21:03–21:43:
un `n2-standard-2` 40 min de más). **~3–4 min.**

**Despertar**, medido (2026-10-10 y 11): el nodo listo a ~55 s, Kueue a ~90 s, y entonces **el
empujón** —`rollout restart` de cada Deployment y StatefulSet—: con los nodos a 0 los ReplicaSets
llevan horas de `FailedCreate` contra el webhook de Kueue (`failurePolicy: Fail`) y reintentan con
una espera de hasta ~16 min. Cada 30 s reinicia lo que esté en `CrashLoopBackOff` esperando a otro
(el informador necesita a Keycloak). Luego relanza el job `imagen` de la última corrida de `ci` y
prueba las entradas. **Hasta poder entrar: 6–11 min.**

**La consola** (rubix-platform): `/auth/login` pregunta primero si Keycloak contesta (su
descubrimiento OIDC; dormida, ni la página de entrada carga) y, si no, manda a
`/auth/despertando`, que lanza el despertar —sin un segundo si ya hay uno en marcha— y cuenta desde
que empezó; en cuanto Keycloak contesta, sigue al login. `/auth/despertar` lanza `horario.yml` con
`RUBIX_DESPERTADOR_TOKEN` (GitHub, grano fino, sólo `actions: write` sobre `describeloai/ore`, en
Vercel). En el workspace, si se durmió con la pestaña abierta, un aviso con «Despertar».

**Quién: `ore-horario`.** Desde GitHub, la cuenta de Google federada desde su OIDC **sólo para
`main`** (`principalSet` por `attribute.ref/refs/heads/main`: el enlace por `subject` daba
`getAccessToken` denegado); dentro, la cuenta `horario` con Workload Identity. En Google, el papel
`oreHorario` (cambiar el tamaño de los grupos de nodos); en el clúster (`malla/18-el-horario.yaml`)
ver nodos, Kueue y HTTPRoute, reiniciar Deployments/StatefulSets, vaciar nodos y crear Jobs **sólo**
en `forja`, `identidad` y `ore-pg`. Su NetworkPolicy sólo deja salir a la API del clúster, a los
metadatos y a las APIs de Google.

**Lo que destaparon los primeros ciclos** (2026-10-10):
- El login roto por A2: la limpieza de Artifact Registry borró la imagen de Keycloak fijada por
  digest (la protegía `main`, que se movió). Ahora `simular-limpieza.py --comprobar-reglas`, en el
  CI, falla si la malla cita una imagen que la regla no protege por sí misma.
- Las forjas no arrancaban: `ore-forja-<n>` nunca pudo leer su `t-<n>-forja-admin`, y el código
  viejo lo tomaba por «no hay» y acuñaba una versión en cada arranque (las 62 versiones). Ahora
  cada una lee la suya.

**Deudas** (2026-10-11):
- **`ore-iam` no reconecta a su base.** Si arranca antes que `idp-db`, se queda con una conexión
  muerta y da 502 en todo (`no se pudieron leer tus organizaciones`) hasta reiniciarlo, y sigue
  `Ready`. Pasó en los dos despertares del 11.
- **`despertar.sh` da por listo un nodo `SchedulingDisabled`** (el dormir de las 18:35 lo dejó
  acordonado y sin borrar, con el grupo en objetivo 1): cada despertar esperaba 300 s a Kueue y
  fallaba. Y una corrida que esperó en vano a todas las entradas terminó «✓ despierta».
- Despertar depende de que la consola de Vercel (`app.paladio.io`) tenga `RUBIX_DESPERTADOR_TOKEN`
  (el 2026-10-12 no lo tenía: la consola de producción no podía despertar; la local sí). Si un día
  no despierta a las 8, se despierta desde la consola.
- **Para tenerla despierta de noche** (una ventana de pruebas): suspender la CronJob
  (`kubectl -n ore-system patch cronjob dormir-de-noche -p '{"spec":{"suspend":true}}'`) y
  reanudarla al acabar. La anotación `ore.dev/despierta-hasta` ya no cuenta —y Flux la borraba al
  reconciliar el namespace (0058)—.
- C1·5: `jobs-s` a spot y discos de balanced a standard (pocos $); el nodo `e2-standard-2` es
  dudoso (el sistema ocupa ~45 % de un `e2-standard-4`).

## Cierre (2026-10-11)

**Los últimos pasos:**
- **B1·4:** `ore-ci` pierde `roles/cloudbuild.builds.builder` y `roles/storage.objectAdmin`. Le
  quedan `artifactregistry.writer`, `container.developer` y `logging.logWriter`, que son lo que
  usan `construir` e `imagen`; las pruebas de GCS van contra `fake-gcs-server`. Su última llamada
  a Cloud Build, el 2026-10-08 18:41. Borrada la rama `b1/medir`; su contenido está en `main`.
- **Los TXT de validación de C2** fuera de `ore.paladio.io`. Los certificados se renuevan por HTTP
  y cada nombre tiene su registro propio, así que el comodín no vuelve a morder. Los tres de
  `login` en GoDaddy los quita la persona.
- **C3:** presupuesto `ORE 0060 C3: 20 EUR al mes` (`372820c1`) sobre este proyecto, con avisos
  por correo a los administradores de facturación al 50, 90 y 100 % gastado y al 100 % previsto.
  Activada la API `billingbudgets` (gratuita).
  - ⚠️ **Un presupuesto avisa, no corta.**
  - ⚠️ **Octubre saltará entero:** los €178 del 1 al 8 de octubre ya están dentro. La señal útil
    empieza en noviembre, o antes si «previsto» sale disparado.

- **B2 (después del cierre):** una huella por binario. Un commit que sólo toca `ore-serve` pasa
  `construir` de ~11 min a 288 s (sección B2, tras B1·3).

**Lo que queda, sin urgencia:**
- Medir la factura real con la malla dormida, una semana. Esperado: ~16 $ de fijo.
- La consola de Vercel y el CLI `ore` por el túnel.
- Las de C1: que `ore-iam` reconecte a su base, que `despertar.sh` no dé por listo un nodo
  acordonado, y C1·5 (`jobs-s` a spot, discos standard).
- Las versiones 63–66 de `t-demo-forja-admin`.
- Envoy Gateway v1.8 deja de tener soporte el 2026-11-08: subir cuando GKE suba sus CRDs de
  Gateway API a v1.6.
- La entrada TCP pública de Postgres (0058) vuelve cuando haya clientes: otra regla regional,
  ~18 $/mes.

## Primeros principios

1. **Se paga el resultado, no la actividad.** Una imagen desplegada es el resultado; diez
   construcciones que nadie despliega son actividad.
2. **Se construye lo que cambió, y una vez.** Una imagen se identifica por la huella de lo que la
   hace, como ya los binarios; con la misma huella no se construye ni se sube nada: se le pone otra
   etiqueta.
3. **Todo en la misma región.** Lo que se construye, donde se guarda y donde se ejecuta, en
   `europe-west1`: el tráfico dentro de la región no se cobra.
4. **Nada se guarda para siempre sin una regla.** Cada repositorio de imágenes lleva su política de
   retención, como cada dato su copia.
5. **Lo que no trabaja, a cero.** Un nodo sin carga es dinero; spot donde se pueda reintentar.
6. **Medir antes de recortar y después de recortar.** Cada paso dice cuánto esperaba ahorrar y
   cuánto ahorró, por SKU.
7. **Un tope que avisa.** Que la factura no vuelva a sorprender a nadie.

## Decisiones

| # | decisión | fecha |
|---|---|---|
| D1 | **El CI construye sólo la punta**: una corrida que queda vieja se cancela antes de llegar a Cloud Build (`concurrency` con `cancel-in-progress` en el job que construye), y lo que sólo toca documentación (`docs/`, `*.md`) no construye | 2026-10-08 |
| D2 | **Una imagen por huella de sus fuentes**, como los binarios: si ya existe una con esa huella, se etiqueta con el commit (`gcloud artifacts docker tags add`, sin capas que mover) en vez de construirla. Un cambio en Rust no rehace el puesto de Python | 2026-10-08 |
| D3 | **Construir en `europe-west1`**: de inmediato, `--region=europe-west1` en Cloud Build; después, BuildKit en un nodo **spot** de nuestro GKE (la M de *Migrar el CI a GKE*), a cero cuando no construye, con la caché de cargo y de BuildKit persistente. Cabe en la cuota o no va | 2026-10-08 |
| D4 | **Retención en Artifact Registry**: se quedan las últimas pocas versiones de cada imagen y lo desplegado; lo que no tiene etiqueta se borra a los pocos días. Primero en seco (`dry-run`), luego de verdad, y el atraso se limpia una vez | 2026-10-08 |
| D5 | **El cómputo, a su carga**: grupos de nodos con mínimo 0 donde no haya servicio de pie; el de sistema, al tamaño que dice lo medido (O0); puestos con caducidad corta; spot donde un reintento no rompa nada | 2026-10-08 |
| D6 | **Un presupuesto con alertas** (50 %, 90 %, 100 %) y una revisión por SKU cada semana mientras se recorta | 2026-10-08 |
| D7 | **Hábito: un push por bloque**, no por cada commit. Se valida en local (Rust en Docker, el agente en su JDK, la consola en el banco) y se empuja al cerrar el paso | 2026-10-08 |

## El plan

| paso | qué | espera | toca |
|---|---|---|---|
| **G0** | **medir** con la cuenta activa: el desglose por SKU de estos días (¿Artifact Registry es almacenamiento o tráfico?), el tamaño de cada repositorio de imágenes, los minutos de Cloud Build por corrida y la cuota usada | decide el orden | nada |
| **G1** | D1: `concurrency` y `paths-ignore` en `ci.yml`. **Hecho en local (2026-10-08)**: lo que es solo papel (`docs/`, `*.md`, las pruebas a mano contra prod: `ore-postgres/`, `b<N>/`, `medida-*`) no corre; medido, 17 de 64 commits el 8 de octubre y 87 de 427 desde el 1. El paso «la punta» mira lo que hay detrás: si es solo papel, el commit de código sigue siendo la punta y se despliega (probado contra la historia real con `gh` simulado, 7 casos). La lista va en tres sitios y `ci/solo-papel.py` comprueba que dicen lo mismo. El `concurrency` ya estaba: en `main` no cancela a propósito (2026-09-30), y la construcción ya empieza sólo en la punta; lo que se gasta en una que queda vieja a mitad va a G4 | 17 de 64 corridas | git |
| **G2** | D3 de inmediato: `--region=europe-west1` (el pool regional por defecto, sin pool privado). **Hecho en local (2026-10-08)**: `REGION_CB` en el job `construir` y en las cinco llamadas (`submit`, `list`, `describe` ×2, `log`), porque una construcción regional sólo se ve con su región. Queda por ver en la primera corrida: que `E2_HIGHCPU_8` está en el pool regional por defecto de europe-west1 (si no, el `submit` falla en el acto y se sabe). Lo que no se toca: el código fuente (~12 MB) se sigue subiendo a `gs://<proyecto>_cloudbuild`, multirregión de EE. UU.; son céntimos, y moverlo (`--default-buckets-behavior=REGIONAL_USER_OWNED_BUCKET`) crea un bucket y pide IAM: G0 dice si vale. Las recetas de 0058 (`ci/neon/*.yaml`) dicen `--region=global` en sus comentarios: lo cambia su sesión | el tráfico entre regiones, si G0 lo confirma | git |
| **G3** | D4: la política de retención en seco, luego de verdad, y el atraso | casi todo Artifact Registry | GCP: go |
| **G4** | D2: la huella de cada imagen y la etiqueta en vez de la construcción | la mayor parte de lo que queda de Cloud Build | git |
| **G5** | D3 entero: BuildKit en GKE spot (M0b medir la caché, M1 el grupo de nodos, M2 el job que lo usa, M3 el relevo) y Cloud Build fuera | Cloud Build | malla, IAM: go |
| **G6** | D5: mínimos a 0, el sistema a lo medido, caducidades | parte de Compute Engine | malla: go |
| **G7** | D6: el presupuesto y sus alertas; y el balance contra la primera semana | — | cuenta: la persona |

**Lo que se espera** (estimación, no medida: G0 y la factura de la primera semana la corrigen):
con G1–G4, del orden de **€60–90 al mes** en vez de €650; con G5 y G6, menos. El objetivo es que
el coste fijo —el clúster de pie— sea casi todo el coste, y que construir apenas cueste.

## Riesgos

- **Un nodo spot se va a mitad de una construcción**: el job se reintenta; la caché persistente
  hace que la segunda vez cueste poco (M0 midió un `failed to authorize` en un push: reintento).
- **La cuota**: un constructor de 8 vCPU no cabe con Postgres (0058) de pie; G5 mide qué cabe
  (`e2-highcpu-4` spot, o turnos con lo que no corre a la vez) antes de crear nada.
- **Retener de menos**: la política nunca borra lo que está desplegado ni lo que el despliegue
  comprueba (`comprobar que corre ESTE commit`); va en seco primero.
- **Flux deshace un parche**: lo de la malla se declara en su manifiesto, no a mano (lo aprendido en
  O1).
- **El determinismo de `release.yml`** (LTO, cgu=1) no se toca: esto es el CI de cada día.

## Fuera

La facturación, las cuotas y los pagos (de la persona); programas de créditos (Google for
Startups y otros), que alargan lo que dura cada euro pero no cambian lo que se gasta; y mover la
plataforma de cuenta o de nube.

## Nivel 2 · de 30 a 50 $ al mes

G1–G7 quitan lo que se paga **por construir**. Para bajar de ~$60–90 a **$30–50 al mes** hay que
cambiar lo que se paga **por estar de pie**: el cómputo que nadie usa de noche, los balanceadores
que cobran aunque no pase nada, y la parte de Postgres (0058) que está encendida sin ningún
cliente. El reparto que se busca (precio de lista, sin créditos; estimación hasta G0):

| partida | hoy, al mes | nivel 1 (G1–G7) | **nivel 2 (G8–G11)** | con qué |
|---|---|---|---|---|
| construir (Cloud Build → BuildKit spot) | ~$320 | $10–20 | **$1–3** | G4 + G5: unas 30 construcciones de ~30 min al mes en un `e2-highcpu-8` spot que sólo existe mientras compila |
| Artifact Registry | ~$135 | $5–10 | **$2–4** | G2 + G3: sin tráfico entre regiones; 3 versiones por imagen, lo desplegado y las cachés por huella |
| cómputo de GKE (nodos) | ~$115 | $50–70 | **$10–25** | G8: spot y dormido fuera de horas; G10: Postgres dormido |
| balanceadores e IPs | ~$32 | ~$30 | **~$0** | G9: un túnel de Cloudflare en vez de los dos balanceadores globales |
| discos | (en cómputo) | $6–10 | **$4–6** | G11: tamaños a lo usado, huérfanos fuera |
| Secret Manager | ~$9 | ~$9 | **$1–2** | G11: versiones viejas destruidas, lecturas al arrancar |
| tarifa de GKE | $0 | $0 | **$0** | un clúster zonal: lo cubre el nivel gratuito |
| DNS, KMS, Storage | <$1 | <$1 | **<$1** | |
| **total** | **~$650** | **~$60–90** | **~$20–40** | |

### Decisiones

| # | decisión | fecha |
|---|---|---|
| D8 | **El clúster duerme cuando no se trabaja.** Fuera de horas y en fin de semana, los grupos de nodos a cero; los discos se quedan, así que no se pierde nada, y al despertar todo vuelve como estaba. Hay un horario y, además, `dormir`/`despertar` a mano. Fuera de horas la consola ve el backend caído: en desarrollo, se acepta | 2026-10-08 |
| D9 | **Sin balanceadores de Google**: un **túnel de Cloudflare** (`cloudflared` en el clúster, que sale hacia fuera) sirve la puerta y el login, con el TLS y el dominio en Cloudflare. Ni reglas de reenvío, ni IP estática, ni certificado gestionado | 2026-10-08 |
| D10 | **Postgres serverless (0058) duerme sin clientes**: su almacenamiento (pageserver, safekeepers, controlador) y su cómputo a cero cuando no hay una base en uso, y de pie a demanda o en horario. Sus datos ya viven en GCS (D0c: recuperación desde GCS en ~21 s, RPO 0). Lo decide quien lleva 0058 | 2026-10-08 |
| D11 | **Nada huérfano ni de más**: cada disco a lo que usa; los que la clase `retiene` dejó sin dueño, copiados y borrados; en Secret Manager, sólo las versiones en uso, y los pods leen sus secretos al arrancar, no en cada petición | 2026-10-08 |

### G8 · el clúster duerme (D8)

**Qué hay.** El grupo `sistema-spot` (ore-serve por celda, ore-medios, la forja, Keycloak y su
base, Flux, Kueue, cert-manager…), `jobs-s` (`e2-standard-2`, 0→1: los puestos y los builds) y el
grupo de Postgres de `80-postgres-gcp.sh` (`n2-standard-2` **normal, no spot**, 50 GB
`pd-standard` por nodo).

**Qué se hace.**
1. **Medir lo que se usa de verdad** (Cloud Monitoring por contenedor, como O0), en un día de
   trabajo y en una noche: qué cabe en un solo nodo `e2-standard-4` spot, o en dos `e2-standard-2`.
2. **Todo spot** donde un reinicio no rompa nada. La forja y las bases tienen su disco persistente
   y aguantan que el nodo se vaya; lo que no aguante, a uno normal y pequeño.
3. **El horario**: despertar va fuera del clúster (un `CronJob` que despierta no puede estar
   dormido) y dormir, dentro. Hecho así en C1: Cloud Scheduler despierta a las 08:00 de Madrid y la
   CronJob `dormir-de-noche` duerme a las 00:00. El orden importa: al dormir, primero lo que
   escribe (puestos, builds, Postgres); al despertar, primero el sistema.
4. **A mano**: `malla/dormir.sh` y `malla/despertar.sh`, los mismos pasos, para un día de guardia
   o un fin de semana de trabajo.
5. **Al despertar, comprobar**: la entrada contesta, un puesto abre, un build pasa (lo de
   `54-la-comprobacion.yaml`).

**Lo que hay que mirar.** Flux reconcilia al despertar sin pisar nada (el tamaño de un grupo no es
suyo); Kueue retoma su cola; los puestos que quedaron abiertos se cierran solos (el barrido de la
cola, `eafdfd0`); un build que se estaba haciendo a la hora de dormir no se corta: el horario
espera a que la cola esté vacía, con un tope.

**Espera:** con ~50 h de trabajo a la semana el clúster está de pie un 30 % del tiempo, y en spot:
de ~$115 a **$10–25**.

### G9 · un túnel en vez de los balanceadores (D9)

**Qué hay.** Dos balanceadores HTTP(S) globales: la `Gateway` `puerta`
(`gke-l7-global-external-managed`, `14-la-puerta.yaml`, con el mapa de certificados `ore-puerta`
de Certificate Manager para `*.ore.paladio.io`) y el `Ingress` `idp` (`63-entrada-del-idp.yaml`,
IP estática `ore-idp`, certificado gestionado, `login.paladio.io`). Cada uno cobra sus reglas de
reenvío por hora, pase tráfico o no.

**Qué se hace.**
1. **El dominio a Cloudflare** (plan gratuito): los servidores DNS de `paladio.io` pasan a ser los
   suyos. Cloud DNS deja de hacer falta.
2. **Un `Deployment` de `cloudflared`** (dos réplicas pequeñas) con el token del túnel en un
   secreto. El túnel sale hacia Cloudflare: el clúster no necesita ninguna IP pública de entrada.
3. **Las rutas del túnel**: `login.paladio.io` → el servicio de Keycloak; los anfitriones de la
   puerta → lo que hoy está detrás de la `Gateway`. Las `HTTPRoute` siguen decidiendo a qué celda va
   cada petición, con un controlador de Gateway dentro del clúster, o llevando al túnel lo que hoy
   hacen ellas.
4. **El relevo**: túnel y balanceadores a la vez; se cambia el DNS; se comprueba; y se quitan la
   `Gateway`, el `Ingress`, la IP estática y el mapa de certificados.

**Lo que hay que mirar.**
- ⚠️ **El certificado gratuito de Cloudflare cubre un nivel de comodín** (`*.paladio.io`), no dos
  (`*.ore.paladio.io`). Dos salidas: anfitriones de un solo nivel por celda (`acme-ore.paladio.io`)
  o el certificado avanzado de Cloudflare (de pago, del orden de $10 al mes). Se decide en G9,
  antes de mover nada.
- El túnel pasa por el borde de Cloudflare, como hoy por el de Google. El plan gratuito limita el
  cuerpo de una petición (100 MB): las subidas grandes de media (0049) ya van firmadas directas al
  almacén, no por la puerta.
- Los WebSockets y los flujos largos (el LSP, `GET /puestos/{id}/flujo`) se prueban en el relevo.

**Espera:** de ~$32 a **~$0**.

### G10 · Postgres duerme (D10)

**Qué hay.** Lo de 0058: el controlador de almacenamiento y su base (`81-…`), el pageserver y tres
safekeepers (`83-…`), el plano de control y el cómputo en NeonVM (`84-…`–`86-…`), en un grupo de
nodos normales `n2-standard-2`. Está de pie aunque no haya ninguna base en uso.

**Qué se hace** (propuesta para quien lleva 0058, que decide):
1. **El cómputo de cada base ya duerme** por diseño (NeonVM): el escalado a cero, encendido y con
   un plazo corto en desarrollo.
2. **El almacenamiento, a cero sin bases activas**: safekeepers y pageserver a `replicas: 0`
   cuando ninguna base está despierta, y de pie al despertar la primera. Los datos están en
   `gs://ore-pg-almacen-euw1`, y se reconstruye desde ahí (D0c lo midió).
3. **Su grupo de nodos en spot** y con mínimo 0, dentro del horario de G8.

**Lo que hay que mirar.** La primera conexión tras dormir tarda lo que tarda despertar el
almacenamiento (decenas de segundos, por D0c): en desarrollo se acepta; con un cliente de verdad,
decide 0058. Tres safekeepers son el quórum: o los tres, o ninguno.

**Espera:** es la mayor partida fija que queda en cómputo; sin ella, el nivel 2 no baja de ~$50.

### G11 · nada huérfano ni de más (D11)

1. **Discos**: el inventario (`gcloud compute disks list`, y a qué está atado cada uno); los que no
   tienen dueño, una instantánea y fuera; los que sobran de tamaño, al que usan (un disco de 50 GB
   con 2 GB dentro); `pd-standard` donde no haga falta más.
2. **Secret Manager**: se cobra cada versión activa y cada lectura. Las versiones que ya no se
   usan, destruidas; los pods leen sus secretos al arrancar, montados, en vez de pedirlos en cada
   petición. El cofre sigue igual de cerrado: cambia cuántas copias guarda Google, no quién las
   abre.
3. **Instantáneas e imágenes de disco viejas**: las que no tengan una razón escrita, fuera.
4. **Registros**: `CLOUD_LOGGING_ONLY` ya está; se comprueba que nada manda a Logging más de lo
   que entra gratis, y si no, una exclusión.

**Espera:** discos de ~$8 a **$4–6**; Secret Manager de ~$9 a **$1–2**.

### El orden

Primero lo que no se nota y más ahorra: **G1–G4** (el CI y Artifact Registry), **G11** (la
limpieza) y **G8** (dormir). Después **G5** (construir en spot), que necesita la cuota que G8 deja
libre fuera de horas; **G10**, de la mano de 0058; y **G9** el último, porque cambia el dominio y es
lo único que un cliente vería. Cada paso: medir, hacer, medir, y el número a este ADR.
