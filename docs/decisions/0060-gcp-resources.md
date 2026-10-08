# 0060 · GCP resources — el mismo resultado por una fracción del coste

**Estado:** propuesto (2026-10-08). El plan G0–G7 está decidido en su forma; cada paso que toca
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
| **G1** | D1: `concurrency` y `paths-ignore` en `ci.yml` | la mayoría de las 59 | git |
| **G2** | D3 de inmediato: `--region=europe-west1` (el pool regional por defecto, sin pool privado) | el tráfico entre regiones, si G0 lo confirma | git |
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
