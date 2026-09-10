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
  pool default      e2-standard-2   1 fijo    ore.dev/pool=system   IP pública
  pool jobs-p       e2-standard-4   0 → 3     ore.dev/pool=jobs     PRIVADO
                                              taint ore.dev/jobs=true:NoSchedule
```

**El pool de jobs es privado, y eso quitó un techo que nadie había contado.** Cada nodo con
IP pública consume una dirección de `IN_USE_ADDRESSES`, que está en 4: un nodo de sistema
más tres de jobs eran exactamente cuatro, y el clúster no podía crecer más **aunque hubiera
32 vCPU libres**. Sin IP pública, el mismo par de nodos usa **1 de 4**.

**✓ Y el 2026-09-10 hizo falta, exactamente como estaba escrito abajo.** Un cliente dio de
alta un Postgres de verdad —Neon, en AWS— y el Job de catálogo murió con «no se pudo
conectar» mientras el MISMO driver leía esa base sin problema desde un pod del pool público.
La diferencia no era el driver, ni la credencial, ni la `NetworkPolicy`: era **dónde corre**.

Se crearon tres cosas, con `gcloud` y fuera de GitOps —son recursos de red del proyecto,
como el papel de IAM del aprovisionador—:

```
dirección  salida-a-origenes   34.156.87.237   RESERVADA, europe-west1
router     ore-mesh-salida     sostiene el NAT y no enruta nada más
NAT        salida-a-origenes   MANUAL_ONLY · LIST_OF_SUBNETWORKS
                               acotado a 10.10.0.0/20 · PRIMARY_IP_RANGE
```

**Y no hizo falta acotarlo al pool.** Cloud NAT sólo lo usan las instancias **sin IP
externa**, y `default-pool` tiene la suya: sale por NAT `jobs-p` y nadie más, sin una regla
que lo diga. ⚠️ Lo que eso arrastra: un nodo privado **futuro** en esta subred heredaría la
salida sin que nadie lo decida. Hoy hay dos pools y se ve; con seis no.

**La IP fija es el producto, no un detalle de red.** Es lo que un cliente pone en su lista
blanca antes de dar acceso a su base: convierte «nuestro clúster puede conectarse» en «sólo
nuestro clúster puede». Vale para Neon y para cualquier origen en la nube. ⛔ Y **no es
autenticación**: dice de dónde viene el tráfico, no qué Job lo mandó ni por cuenta de qué
inquilino — eso lo dice la huella del custodio, y son dos preguntas distintas.

**Lo que se pierde, sin adornarlo.** Hasta hoy un driver comprometido no podía sacar datos a
internet porque NO HABÍA RUTA. Ahora podría, dentro de 443 y 5432 y sin tocar redes privadas.
Deja de ser imposible y pasa a ser filtrado — por una regla que alguien puede editar. Y no
hay forma de tenerlo y no tenerlo: **los datos del cliente viven en la nube**, así que un
lector de orígenes que no puede salir no es seguro, es inútil.

**La cuenta de la cuota sale, y es la clave de por qué esto no deshace nada.** Una dirección
reservada gasta de `IN_USE_ADDRESSES` —la MISMA cuota que hizo privado este pool— pero **una
sirve a los tres nodos**: antes tres nodos públicos gastaban tres. Se recupera la salida por
un tercio de lo que costaba tenerla, y el motivo por el que el pool es privado sigue en pie.

⚠️ **El dinero NO está medido** contra la Billing API: Cloud NAT cobra por pasarela y hora
más por GB procesado. Queda pendiente y se dice, en vez de escribir una cifra de memoria.

── Lo que decía este párrafo cuando se escribió, y sigue siendo verdad ──

No hizo falta Cloud NAT ni esperar a la cuota. **Private Google Access —ya encendido en la
subred— deja que una VM sin IP pública alcance las APIs de Google**, y eso cubre lo que la
malla necesita sacar: Artifact Registry, BigQuery, las credenciales y los logs. NAT hará
falta el día que haya que salir a algo que *no* sea de Google — un Postgres de un cliente.

Y la migración no tocó nada de encima: el `ResourceFlavor` de Kueue selecciona por la
**etiqueta** `ore.dev/pool=jobs`, no por el nombre del pool. Se creó `jobs-p`, se borró
`jobs`, y ni la cola ni los Jobs se enteraron. Es lo que la etiqueta compró.

**Zonal y no regional, y es una decisión de dinero medida.** El fijo del plano de control
es `$0.10/h` para cualquier clúster, y el crédito de free tier —`$74.40/mes`— cubre **uno
zonal o Autopilot, no regional**. Regional se lleva 73 $/mes antes de encender un nodo:
con 300 $ de colchón son 1,2 meses contra 5,3. La arquitectura es idéntica en las dos, y
pasar a regional es recrear el clúster, no rediseñarlo.

**El pool de jobs es on-demand porque `PREEMPTIBLE_CPUS` está a 0.** La petición de cuota
está presentada. El día que entre, se sustituye el pool por uno Spot y **nada más cambia**:
la misma etiqueta, el mismo taint, el mismo sabor de Kueue encima — que es exactamente lo
que acaba de comprobarse al hacerlo privado.

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

## 5. Las dos imágenes, y por qué dos

```
europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/
  ore          :main · :d77cb7b     7,4 MB     scratch
  ore-drivers  :main · :d77cb7b     721 MB en disco, ~110 MB de descarga
```

**La razón no es el tamaño.** Los drivers son binarios separados por decisión —ADR 0008:
`ore` los busca en el `PATH` y habla con ellos por stdin/stdout, así que el motor no
enlaza un cliente de nube y un driver lo puede escribir cualquiera en cualquier lenguaje—,
pero **eso no obliga a imágenes separadas**: los diez caben en una, y la diferencia de
arranque en frío son segundos.

Lo que sí justifica separarlas:

> **`ore` no puede salir a la red aunque quiera.** No lleva certificados ni cliente TLS. Un
> `validate` que corre desde ahí no habla con nadie, y eso es una garantía **estructural** —
> más fuerte que una política que se lo prohíba, porque no hay nada que aplicar.

`ore-drivers` lleva **todo lo que `ore` puede ejecutar**: `ore-read-jsonl`,
`ore-read-postgres`, `ore-read-bigquery`, `ore-fetch`, `ore-log`, `ore-sign` y
`ore-store-r2`. La lista salió de leer el árbol, no de la memoria.

Va sobre el SDK de Google Cloud por una sola razón: **`ore-read-bigquery` no habla con
BigQuery** — delega en `bq`. Fue una decisión deliberada, y `ore-sql` la explica en su
cabecera: no meter un cliente de nube dentro del árbol. El precio es que esa imagen
necesita el SDK, y el precio se paga **sólo cuando se consulta BigQuery**.

### Comprobado en el clúster

```
ore           13 s     nodo caliente
ore-drivers   95 s     nodo desde CERO (≈50 s levantarlo, ≈30 s descargar)
```

### Lo que hizo falta además del `push`

Los nodos usan la cuenta de servicio por defecto de Compute, que **no** puede leer Artifact
Registry: el primer intento dio `403 Forbidden` en el `pull`. Se concedió

```
339497864493-compute@developer.gserviceaccount.com  →  roles/artifactregistry.reader
```

Todo lo demás había funcionado ya en ese primer intento —Kueue admitió, el autoscaler
levantó el nodo, el pod se programó con su tolerancia— así que el fallo aisló exactamente
una cosa. Es lo que se quiere de una malla.

## 6. Los ficheros

| | |
|---|---|
| [`00-base.yaml`](00-base.yaml) | `ore-system`, deny-all de entrada y salida, cuota |
| [`10-kueue.yaml`](10-kueue.yaml) | el sabor, la `ClusterQueue` con su cohorte, el tenant `t-demo` y su cola |
| [`90-prueba.yaml`](90-prueba.yaml) | el Job de `busybox` que ejercitó la cadena la primera vez |
| [`91-ore-version.yaml`](91-ore-version.yaml) | el Job que corre **nuestra** imagen desde el registro |

Kueue se instala aparte, desde su release:

```bash
kubectl apply --server-side -f https://github.com/kubernetes-sigs/kueue/releases/download/v0.19.3/manifests.yaml
```

## 7. Un falso positivo, dicho para que nadie lo repita

Durante un rato pareció que **`kubectl logs` estaba roto** en el pool de jobs: `dial tcp
10.10.0.x:10250: i/o timeout`, repetido. Se persiguió como un agujero de la VPC custom y se
llegó a añadir una regla de cortafuegos del plano de control a los nodos.

**No había tal cosa.** `optimize-utilization` retira los nodos ociosos deprisa, y lo que se
estaba leyendo era la salida de pods cuyo nodo ya estaba drenado. Leyendo *inmediatamente*
después de que el Job termina, sale a la primera. La regla se retiró, y se comprobó que sin
ella los logs siguen — dejar un `allow` «por si acaso» es lo que se pudre.

## 8. Lo que falta

- **El pool Spot**, cuando entre la cuota.
- **La política de salida del driver** con los 145 prefijos públicos de Google: fea, y
  cierra la salida hoy sin montar DNS.
- **Zona privada de Cloud DNS + `restricted.googleapis.com` (199.36.153.4/30)**, que
  sustituye la lista por cuatro direcciones y trae la frontera de VPC-SC.
- **Kueue con `AdmissionCheck`** para exigir cuota de origen antes de admitir un `discover`.
