# La malla

> **Estado:** en construcción · **Fecha:** 2026-09-07 · **Clúster:** `ore-mesh`,
> `europe-west1-b`, proyecto `project-8853a180-450d-47be-b83`
>
> Lo que corre `ore` cuando no lo corre un portátil. El razonamiento —qué se toma de
> Rubix, de Apollo, de GKE y de Kueue, y qué **no**— está medido en
> [`medida-la-malla-referencias.py`](../pruebas-de-fuego/medida-la-malla-referencias.py).

---

## 1. La forma, y por qué es ésta

```
VPC ore-mesh (custom)
  subred        10.10.0.0/20     nodos
  pods          10.20.0.0/16     secundario
  services      10.30.0.0/20     secundario
  Private Google Access · on

clúster ore-mesh    ZONAL · canal REGULAR · Dataplane V2 · Workload Identity
  pool default      e2-standard-2   1 fijo    ore.dev/pool=system
  pool jobs         e2-standard-4   0 → 3     ore.dev/pool=jobs
                                              taint ore.dev/jobs=true:NoSchedule
```

**Zonal y no regional, y es una decisión de dinero medida.** El fijo del plano de control
es `$0.10/h` para cualquier clúster, y el crédito de free tier —`$74.40/mes`— cubre **uno
zonal o Autopilot, no regional**. Regional se lleva 73 $/mes antes de encender un nodo:
con 300 $ de colchón son 1,2 meses contra 5,3. La arquitectura es idéntica en las dos, y
pasar a regional es recrear el clúster, no rediseñarlo.

**El pool de jobs es on-demand porque `PREEMPTIBLE_CPUS` está a 0.** La petición de cuota
está presentada. El día que entre, se sustituye el pool por uno Spot y **nada más cambia**:
la misma etiqueta, el mismo taint, el mismo sabor de Kueue encima.

## 2. Las tres reglas que no hay que deshacer

**El taint del pool de jobs es lo que hace real el «escala a cero».** Sin él, cualquier pod
de sistema aterriza en ese pool y lo mantiene encendido pagando. Con él, el pool está a
cero nodos y cero dólares hasta que llega un Job que lo tolere — y la tolerancia **no la
escribe el Job**: la inyecta el `ResourceFlavor`.

**Se niega por defecto, y también la salida.** La política se escribe contra la
**etiqueta**, no contra la IP —es la práctica de Rubix— y aquí compra algo concreto: lo
que planifica no necesita internet, porque 12 de 14 crates de `ore` son herméticos. Sólo
el driver que lee un origen lo necesita, y eso será una política propia contra *su*
etiqueta.

**`ore.dev/tenant` va en todo desde el primer manifiesto**, aunque hoy haya uno solo. Es lo
que permite migrar a un plano de control virtual —vCluster— sin reescribir: si nada asume
«un solo árbol», cambiar el mecanismo de aislamiento es una decisión de operación y no de
producto.

## 3. Kueue, y por qué el cohorte con un solo tenant

Kueue decide **cuándo** arranca un Job; la `ResourceQuota` del namespace decide **cuánto**
puede pedir en total. Son dos preguntas y hacen falta las dos: sin la cuota, un Job mal
escrito pide 40 CPU y Kueue lo deja esperando para siempre en vez de rechazarlo.

El **cohorte** hoy no hace nada visible. Con dos tenants, las colas del mismo cohorte se
prestan la cuota que no usan, y el segundo se declara escribiendo un manifiesto en vez de
rediseñando la cola. Es la misma figura que la etiqueta de tenant, aplicada al cómputo.

La cuota nominal —10 CPU y 36Gi— es **conservadora a propósito**. El pool da 12 vCPU y
48 Gi en bruto y GKE reserva por nodo; una cuota que promete más de lo que hay convierte
una espera en un pod que no arranca nunca.

## 4. Comprobado de punta a punta

```
19:14:56  nodos-jobs=0   Pending      ← Kueue admite, el autoscaler despierta
19:15:43  nodos-jobs=1   Pending
19:16:05  nodos-jobs=1   Running      ← 4 CPUs, sobre gke-ore-mesh-jobs-…
19:16:26  nodos-jobs=1   Completed    ← cuota devuelta a 0
```

De cero a nodo corriendo en **47 segundos**. El pod llevaba la tolerancia inyectada y
aterrizó en el pool correcto.

## 5. Los ficheros

| | |
|---|---|
| [`00-base.yaml`](00-base.yaml) | `ore-system`, deny-all de entrada y salida, cuota |
| [`10-kueue.yaml`](10-kueue.yaml) | el sabor, la `ClusterQueue` con su cohorte, el tenant `t-demo` y su cola |
| [`90-prueba.yaml`](90-prueba.yaml) | el Job que ejercita la cadena entera. Se borra después de correr |

Kueue se instala aparte, desde su release:

```bash
kubectl apply --server-side -f https://github.com/kubernetes-sigs/kueue/releases/download/v0.19.3/manifests.yaml
```

## 6. Lo que falta

- **Artifact Registry y la imagen de `ore`** — hoy no hay nada nuestro que ejecutar.
- **El pool Spot**, cuando entre la cuota.
- **Cloud NAT**, cuando un driver necesite salir a un origen de verdad.
- **La política de salida del driver** — la excepción con nombre a la regla de §2.
- **Kueue con `AdmissionCheck`** para exigir cuota de origen antes de admitir un `discover`.
