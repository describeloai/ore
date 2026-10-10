# 0060 · GCP resources — el mismo resultado por una fracción del coste

**Estado:** en stand by (2026-10-09). G1 y G2 hechos (`42a3da8f`, `1ce4e27a`), sin probar en una corrida de verdad; lo siguiente, G4 (en local, en Docker) y G0 (con la cuenta activa). Propuesto el 2026-10-08. El plan —G0–G7, nivel 1 (~$60–90 al mes), y G8–G11, nivel 2
(~$20–40 al mes)— está decidido en su forma; cada paso que toca
la malla, el IAM o la infraestructura de Google pide su go antes de hacerse, y se mide antes y
después. Lo que es de la cuenta —la facturación, las cuotas, un pago— es de la persona dueña, no de
este ADR.

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
   `v0.49.1-ore.1` (NeonVM y el autoescalado), y la caché de construcción de Neon
   (`cache:neon-almacen`, `cache:neon-computo-v17`). Quien cambie uno de esos fijados, lo cambia
   aquí;
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
ORE en sí (puestos, capas, servicios, binarios, deps) cabe en ~6 GB. De ~$27 al mes a ~$5, o a
~$2 sin la caché de Neon (que solo sirve para reconstruir Neon desde el fork).

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
3. **El horario va fuera del clúster** (un `CronJob` que despierta no puede estar dormido): **Cloud
   Scheduler** (tres trabajos gratis por cuenta) llama a la API de GKE para dejar cada grupo con
   mínimo y máximo 0 a la hora de dormir, y con su tamaño a la de despertar. El orden importa: al
   dormir, primero lo que escribe (puestos, builds, Postgres); al despertar, primero el sistema.
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
