# 0027 · El modelo vive en el árbol

**Estado:** propuesto · **Fecha:** 2026-09-15 · **Decide:** que un modelo desplegado es **un
documento del árbol** (`kind: Model`) y no un recurso del plano de control; que `ore-serve` lo
convierte en manifiesto **rellenando una plantilla que dejó el aprovisionador**, en un segundo
repositorio de trabajo de la celda, que Flux aplica **con una cuenta que sólo puede desplegar**;
que los pesos no pasan por el árbol —**el árbol guarda el puntero y el digest, el sustrato los
bytes**—; que el estado lo trae **el informador** (0026, snapshot v2); y que el plano de control
**observa y no posee**. Cierra la E6 de [`0024`](0024-donde-corre-el-inquilino.md).

---

## El problema

La consola tiene *Models → Hub* y *Models → Deployments* y ningún conducto detrás: la lista de
despliegues está vacía y lo dice. Antes de darle una fila se midió la malla
(`pruebas-de-fuego/medida-la-malla-para-modelos.py`, 2026-09-15): un nodo `e2-standard-4` spot
(3,9 vCPU · 13 GiB · **0 GPU**), `jobs-p` 0→3, GPU en cuota **0** en toda la región, celda con
`requests.cpu: 10 · requests.memory: 36Gi` y **sin dimensión GPU**. Hoy un cliente puede servir
embeddings y LLMs ≤ 8B cuantizados, en CPU, una réplica. Se ha pedido cuota G4 (8 × RTX PRO
6000, `europe-west1`); si llega, cambia el suelo y no la decisión.

La pregunta no es «cómo se levanta un `Deployment` de vLLM» —eso lo hace cualquiera— sino
**qué es un modelo en este sistema**, y de la respuesta sale dónde se escribe, quién lo aplica,
quién lo mide y qué no puede hacer.

---

## Lo que se miró antes de decidir

- **El árbol ya tiene tres documentos** (`docs/modelo.md`): `Table` —qué hay ahí fuera—, `View`
  —qué se pregunta—, `Entity` —qué es una fila—. Y una `Function` **propone, no aplica**
  (`functions.md` §5: *«`ore` declara y verifica; ejecutar se delega»*). Un modelo encaja como
  **un cuarto documento**: *qué razona sobre eso*. Y sus salidas —una extracción, un embedding,
  una clasificación— son escrituras, y [`0018`](0018-la-ontologia-es-el-sistema-de-registro.md)
  ya dijo dónde aterriza una escritura: en la ontología. **El modelo no es un servicio al lado
  de los datos: es un operador dentro del grafo.** Eso es lo que Databricks Serving (un endpoint
  junto a una tabla) y Vertex (un catálogo de APIs ajenas) no tienen, y lo que aquí sale gratis.
- **`ore-serve` ya escribe manifiestos**: la cola (`cola.rs`). El alta de una fuente rellena
  `plantilla-catalogo.txt` —que **el aprovisionador dejó rendida para esa celda**— y la empuja a
  `t-<celda>/trabajo.git`; Flux la aplica con `cola-<celda>`, cuyo `Role` es **`batch/jobs` y
  nada más** (`13-el-inquilino-reconciliado.yaml`: *«Ni `pods`, ni `secrets`, ni `deployments`»*).
  ⇒ Un `Deployment` en `trabajo` **no se aplicaría**, y esa negativa es deliberada: es lo que
  cierra el ④ de `0022` —el gobernado escribiendo su gobierno—. No se reabre; se añade **otra
  cuenta con otro verbo**.
- **Los pods nuevos nacen sin salida.** La `NetworkPolicy` de la celda deniega todo egress salvo
  DNS, forja, cofre, control e IdP. Un pod de modelo que quiera bajar pesos de Hugging Face
  muere por timeout, igual que `ore-serve` contra el API server en `0026`. Y `ore-serve` **no
  puede escribir `NetworkPolicy`** (ni debe): la salida de un pod de modelo la tiene que dar la
  plataforma, de antemano, por clase.
- **`0024 ②` ya decidió el sitio**: el modelo va con el árbol, en el clúster del inquilino, en
  otro pool (`gpu`, con su taint, de 0). Dedicado y BYOC son *el mismo manifiesto en otro
  sitio*; lo que se decida aquí tiene que ser ese manifiesto.
- **`0026` ya decidió cómo llega el estado**: el informador, con `Role` de sólo lectura, cada
  60 s, a `ore-iam`. Un `Deployment` más en el namespace es tres campos más en su snapshot.
- **Lo que hacen los demás con los pesos**: nadie los mete en git. HF los sirve por HTTP con
  digest por fichero; vLLM y llama.cpp los leen de disco. Un 8B en Q4 son ~5 GB; un 235B en
  FP8, ~235 GB. El árbol guarda **qué** pesos (repositorio, revisión, digest) y el sustrato
  —bucket de la celda, o HF la primera vez— guarda los bytes. Es `01-table §2` aplicado a un
  artefacto: **puntero en el árbol, bytes fuera**.

---

## La decisión

> ### ① Un modelo es un documento del árbol: `kind: Model`.

Vive en `ontologia/` de la celda, junto a `Table`, `View` y `Entity`, y se escribe por
`ore-serve` como cualquier otro documento (`POST /modelos`, el mismo acto que `POST /fuentes`:
commit con quién lo pidió). La forma, a fijar en la especificación (`oos`) en la E1:

```yaml
kind: Model
name: qwen3-8b
task: chat                       # chat · embed · rerank
weights:
  repo: Qwen/Qwen3-8B            # hf://<repo>  o  bucket://<celda>/modelos/<ruta>
  revision: 1c4f…                # el commit de HF o el digest del directorio: lo que se bajó
runtime: vllm                    # vllm (GPU) · llamacpp (CPU)
resources: { gpu: 1 }            # o { cpu: "3.5", memory: 8Gi }
serve: { replicas: 1 }           # escalar a 0: cuando haya KEDA, y se dirá
expose: internal                 # internal · public (con clave; ⑤)
```

Lo que eso regala sin programarlo: el modelo es **direccionable** (`modelo/qwen3-8b`, y una
`Function` que lo llame nombra un nodo, no una URL); **versionado** (cambiar `revision` es un
commit, volver es un `revert`, quién lo pidió está dentro); y **gobernado** (Governance ve qué
vistas lo alimentan y qué escribe, porque está en el grafo).

> ### ② `ore-serve` deriva el manifiesto rellenando una plantilla, y lo empuja a `modelos.git`.

La misma figura que la cola, sin excepciones: el aprovisionador deja en la celda
`plantilla-modelo-vllm.txt` y `plantilla-modelo-llamacpp.txt`, **rendidas para esa celda**
(namespace, pool, taint, toleración, etiquetas, cuenta, límites), con los huecos del modelo
intactos: nombre, pesos, recursos, réplicas. `ore-serve` sustituye esos huecos y **nada más**.
No inventa un `Deployment`: rellena el que la plataforma escribió, y `gen-inquilino.py` lo
comprueba byte a byte como comprueba el resto.

El destino es **un tercer repositorio** en la forja de la celda, `t-<celda>/modelos.git`
(`ore-serve --modelos URL`; mismo usuario `serve-<celda>`, colaborador de tres y de ningún
otro). No la cola, porque la cola la aplica una cuenta que sólo crea `Job` y así debe seguir.

> ### ③ Flux lo aplica con una cuenta que sólo puede desplegar.

`GitRepository` + `Kustomization` `modelos-<celda>` en `flux-system`, con la etiqueta
`ore.dev/rol: agente` —entra en el `Receiver` de `17-` por clase, el webhook dispara en
segundos— y `serviceAccountName: desplegar-<celda>`, cuyo `Role` en `t-<celda>` es exactamente:

```
apps/deployments   get list watch create patch delete
services           get list watch create patch delete
namespaces         get
```

Ni `pods` (los crea el `Deployment`), ni `secrets`, ni `networkpolicies`, ni `jobs`. Lo que se
concede se lee de un vistazo: **desplegar un modelo**. `prune: true`, con lo que arrastra: un
`Model` que desaparece del árbol **retira** su `Deployment`.

> ### ④ Los pesos no pasan por el árbol, y la salida la da la plataforma por clase.

El `Deployment` lleva un init container que trae los pesos a un volumen —de HF con la clave del
cofre si el repositorio es *gated* (`t-<celda>-cofre-hf`, `0024 ⑤`), o del bucket de la celda—
y **comprueba la `revision`** antes de arrancar: lo que corre es lo que el árbol dice, o no
corre. `ore-serve` no toca un byte de pesos: sigue sin red.

La salida es una `NetworkPolicy` **del compartimento** (la escribe la plataforma, la aplica el
`Kustomization` de la celda, no el de modelos) que selecciona pods `ore.dev/rol: modelo` y les
permite egress **sólo** a Hugging Face y al bucket de la celda. La plantilla de ② pone esa
etiqueta; `ore-serve` no puede quitarla sin que `gen-inquilino.py` deje de reconocer el
manifiesto. El pod del modelo, una vez arrancado, no necesita salir: sirve dentro.

> ### ⑤ La puerta: un `Service` interno, y lo público es otra etapa.

`modelo-<nombre>.t-<celda>.svc:8000`, con la API OpenAI que vLLM y llama.cpp ya hablan. Es lo
que `ore-serve` y los Jobs de la celda alcanzan, y lo que una `Function` invoca. **Público**
—URL bajo la entrada de la celda, con clave— es la E4: la misma pieza que `entrada` en `0025`,
y no antes de que lo interno sirva y se mida.

> ### ⑥ El estado lo trae el informador; el plano de control observa y no posee.

Snapshot **v2** (`0026 ②` + un campo):

```json
"despliegues": [ { "nombre": "qwen3-8b", "modelo": "Qwen/Qwen3-8B", "revision": "1c4f…",
                   "listo": 1, "pedidas": 1, "reinicios": 0, "desde": "…" } ]
```

El `Role` del informador gana `apps/deployments get,list` —sigue siendo sólo lectura, sigue en
su namespace—. `ore-iam` acepta `v: 2` y guarda como hasta ahora: una fila por celda, sin
huella por snapshot. **No hay tabla de despliegues en `iam`**: *Deployments* en la consola es
`GET /celdas → estado_medido.despliegues`, cruzado con lo que el árbol declara (`GET /modelos`
de `ore-serve`). Declarado y no medido = *provisioning*; medido y no declarado = *retiring*;
los dos = lo que Kubernetes diga. Ninguno de los dos planos finge saber lo del otro (`0024 ④`).

> ### ⑦ El encaje se decide en el servidor, con la misma tabla que ve el Hub.

`POST /modelos` rechaza (422, con el motivo) lo que no cabe: GPU pedida y `gpu: 0` en la cuota;
recursos por encima de lo que le queda a la celda; `runtime` que la plataforma no rinde. La
`ResourceQuota` gana `requests.nvidia.com/gpu` (0 por defecto; sube por contrato) y es la
segunda cerradura: aunque el manifiesto entrara, Kubernetes no lo programaría. La consola no
inventa el encaje: lo pinta.

---

## Lo que se acepta a cambio

- **Un tercer repositorio por celda y un tercer `Kustomization`.** Es el precio de que
  `cola-<celda>` siga pudiendo sólo `Job`. Dos cuentas con un verbo cada una, no una con dos.
- **Una plantilla más que envejece con la malla.** La comprueba `gen-inquilino.py` como la de
  catálogo; si diverge, falla la comprobación, no el cliente.
- **El pod de modelo tiene salida a HF.** Acotada a esa clase de pod y a ese destino, y sólo
  hasta que el bucket de la celda tenga los pesos (E5): entonces la regla a HF se puede cerrar
  por celda.
- **Arranque en frío real.** Bajar 5 GB y cargarlos son minutos; con `spot` te quitan el nodo y
  vuelves a pagarlos. Se mide en la E0 y se enseña en *provisioning* con la hora, no se
  esconde. Escalar a 0 (KEDA) queda dicho y no hecho.
- **GPU entera por celda.** Sin MIG medido, la unidad de cuota es una GPU. Un cliente con un
  8B en una RTX PRO 6000 usa un tercio de su memoria; el resto es suyo y está parado.
- **El árbol lleva un `kind` más**, y la especificación (`oos`) tiene que decirlo antes de que
  `ore-serve` lo acepte.

---

## El abordaje — y por qué es así y no de golpe

Igual que en `0026`: cada etapa deja el sistema entero y medido. Se empieza por **un modelo
sirviendo a mano en una celda real**, porque un conducto para un modelo que nadie ha visto
arrancar en esa malla es un conducto hacia un número inventado —hoy `encaje()` dice
`vcpu: 3.5` para un 8B **sin haberlo medido**—.

### E0 · La medida y el contrato

`pruebas-de-fuego/medida-un-modelo-en-la-celda.py`. En `t-victor`, **a mano y con `kubectl`**
—sin `ore-serve`, sin Flux, sin plantilla—, un `Deployment` de `llama.cpp` sirviendo
`Qwen2.5-1.5B-Instruct-Q4_K_M` y otro con un 7–8B Q4, con los recursos que hoy pide el catálogo
de la consola. Y se mide: **(a)** que el init **no puede** bajar los pesos con la
`NetworkPolicy` de hoy —el timeout, con código—, y que sí puede con la regla de ④ aplicada a
mano; **(b)** arranque en frío: descarga + carga, en segundos; **(c)** memoria residente real
frente a la pedida; **(d)** tok/s con 1 usuario y con 4, `curl` contra
`/v1/chat/completions` desde un Job de la celda; **(e)** que la `ResourceQuota` lo cuenta y que
`--cotejar` de `0026` lo ve como un pod más. Si la cuota G4 llega antes, **(f)** lo mismo con
vLLM, `Qwen3-8B` FP8, 1 GPU, en el pool `gpu` con su taint. De aquí salen dos cosas y no
opinión: **los números de `encaje()`** (los que hoy son estimados pasan a medidos, o cambian) y
**el manifiesto de referencia** del que se recorta la plantilla de ②, más la forma del `Model`
de ① con lo que de verdad hizo falta para arrancarlo. **Acepta:** la tabla de la medida con
(a)–(e) en `victor`, y un `curl` de la celda contestado por el modelo.

### E1 · El documento y la plantilla

`kind: Model` en `oos` (bump del submódulo); `ore-serve` `POST /modelos` · `GET /modelos` ·
`DELETE /modelos/{n}` con el encaje de ⑦; `--modelos URL`; las plantillas en `malla/` y en
`gen-inquilino.py` con su comprobación; `aprovisionar-inquilino.sh` funda `modelos.git` y deja
las plantillas. **Acepta:** `los-verbos` con los casos de ⑦ (cabe → 201 y el commit en
`modelos.git` con el manifiesto rendido; GPU sin cuota → 422; retirar → el fichero desaparece).

### E2 · Flux lo aplica y el informador lo cuenta

`modelos-<celda>` con `desplegar-<celda>` (③) y la `NetworkPolicy` de clase (④) en
`13-el-inquilino-reconciliado.yaml`; `Role` del informador con `deployments`; `informar.sh` v2;
`ore-iam` acepta `v: 2`; `medida-el-estado-de-la-celda.py --snapshot` rinde v2. **Acepta:** en
`victor`, `POST /modelos` desde `curl` → el `Deployment` existe en < 60 s sin que nadie toque
`kubectl`; `--cotejar` cuadra `despliegues`; `DELETE` lo retira.

### E3 · Deployments tiene filas

La consola cruza `GET /modelos` con `estado_medido.despliegues` (⑥); *Crear* deja de estar
deshabilitado y llama a `POST /modelos`; el detalle con *Overview · Endpoint · Events*; los
motivos de 422 en pantalla tal cual. **Acepta:** desde el Hub, *Deploy in this cluster* → fila
en *provisioning* con la hora → *running* cuando el informador lo vea; y el `Model` está en el
árbol con el autor de la sesión.

### E4 · La puerta pública

`expose: public`: ruta bajo la entrada de la celda y clave en el cofre; la pestaña *Endpoint*
con la URL y las claves. **Acepta:** un `curl` desde fuera con la clave contesta; sin clave,
401; la celda de al lado no lo alcanza.

### E5 · Los pesos viven en la celda, y la GPU

`bucket://` como origen de pesos (el bucket de la celda de `0024 ③`), `ore modelos traer` que
los deja allí con su digest; cerrar la salida a HF por celda cuando ya no haga falta. Y con la
cuota G4: pool `gpu` en la malla compartida, `requests.nvidia.com/gpu` en la cuota, `encaje()`
con la dimensión GPU medida, `MALLA_COMPARTIDA.gpu` deja de ser `null`. **Acepta:** un modelo
arranca desde el bucket sin regla a HF; un 8B FP8 sirve en 1 GPU con sus tok/s medidos.

---

## Lo que este abordaje NO hace, y por qué

- **No pone una tabla de despliegues en `ore-iam`.** Sería el plano de control poseyendo lo que
  el árbol declara: dos verdades del mismo objeto. `iam` guarda lo que la celda **informa**.
- **No ensancha `cola-<celda>`.** Una cuenta que puede `Job` y `Deployment` ya no se lee de un
  vistazo. Dos cuentas, un verbo cada una.
- **No deja a `ore-serve` escribir un `Deployment` libre.** Rellena una plantilla que la
  plataforma rindió y comprueba. Lo que puede cambiar es el modelo; lo que no, el pod.
- **No mete modelos propietarios por API.** Eso no es un `Model` del árbol: es una
  **conexión**, con clave del cliente y una excepción de salida explícita —«los datos salen del
  clúster»—, y va en otro ADR cuando *Training* exista y el caso maestro/obrero lo pida.
- **No decide escalar a cero, MIG ni entrenamiento.** Se nombran como huecos con su sitio.
