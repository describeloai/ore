# 0027 · El modelo vive en el árbol

**Estado:** propuesto (escrito el 2026-09-15; **revisado el 2026-09-16 sobre [`0028`](0028-bastion-es-el-producto-sobre-el-stack.md)**, que fija el sustrato y aún no es definitivo) · **Fecha:** 2026-09-15 · **Decide:** que un modelo que una celda usa es **un documento del árbol** (`kind: Model`) y no un recurso del plano de control; que ese documento nombra **un perfil certificado** (máquina × modelo × motor con números medidos, 0028 B2) y **un tier** —compartido o dedicado—, no un runtime ni unos recursos inventados; que en el tier compartido el documento deriva a **una suscripción** que el gateway (0028 B3) respeta —un pod por modelo, la multi-tenencia es lógica—, y en el dedicado a **un despliegue** que Flux aplica con una cuenta que sólo puede desplegar; que los pesos no pasan por el árbol —**el árbol guarda el puntero y el digest, el sustrato los bytes**—; y que el plano de control **observa y no posee**. Cierra la E6 de [`0024`](0024-donde-corre-el-inquilino.md).

---

## El problema

La consola tiene *Models → Hub* y *Models → Deployments* y ningún conducto detrás: la lista de
despliegues está vacía y lo dice. Antes de darle una fila se midió la malla
(`pruebas-de-fuego/medida-la-malla-para-modelos.py`, 2026-09-15): un nodo `e2-standard-4` spot
(3,9 vCPU · 13 GiB · **0 GPU**), `jobs-p` 0→3, GPU en cuota **0** en toda la región, celda con
`requests.cpu: 10 · requests.memory: 36Gi` y sin dimensión GPU. Ese metro vale para los Jobs;
**para modelos mide lo equivocado**, y la primera versión de este ADR lo usó: proponía servir
en CPU con llama.cpp «lo que cabe en la cuota», y un Hub con dieciséis modelos y una píldora
*Fits · CPU* calculada en vCPU.

`0028` midió el sustrato de verdad (Bastion, 2026-09-14): el coste por token de nivel API sale
de **vLLM sobre RTX PRO 6000 GDDR7** (Qwen3-235B FP8 en 4 GPUs: 587 tok/s a 32 usuarios =
2,8 $/M), el motor propio queda congelado, y lo que se ofrece es **un perfil certificado por
par máquina × modelo, con sus números** —«un perfil sin número no existe»—. Un modelo en CPU
en la cuota de la celda no es ese producto y no se ofrece como si lo fuera.

La pregunta sigue siendo la misma: **qué es un modelo en este sistema**. La respuesta no cambia
con 0028; cambia **a qué deriva** y **cómo se despliega el primero**.

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
- **`0028` fija el sustrato y sus formas**: una imagen sellada
  (`bastion/env:0.29.0-sm120`, Artifact Registry `europe-west1`), tres máquinas (`g1`/`g4`/`g8`
  = 1/4/8 × RTX PRO 6000), **perfiles** `máquina/modelo` con los argumentos exactos de
  `vllm serve` y los `EXPECT_*` medidos (`env/profiles/`), un lanzador (B1) que lleva una
  máquina de «nada» a un endpoint OpenAI sano y de vuelta con apagado verificado, y un gateway
  (B3) con **un pod compartido por modelo**, claves, cuotas y contabilidad por inquilino, que
  respeta **la etiqueta de soberanía** de la máquina (`gcp: eu-dc, dpa, kms` para clientes;
  `vast: community` sólo para validar). Hoy hay **tres perfiles**: `g4/qwen3-235b-fp8`,
  `g4/deepseek-r1-0528-awq`, `g1/deepseek-v2-lite`. Y dos formas de cobrar: **compartido por
  token** y **máquina dedicada a precio fijo**.
- **El primer modelo real corre en una VM, no en GKE.** B1 lanza máquinas G4 (COS, contenedor,
  sin IP pública, `--max-run-duration`); docs/10 §6 de Bastion aplaza el manifiesto de
  Kubernetes a B3 «hasta que la imagen pase con GPU y haya cuota». `0024 ②` dijo *mismo clúster,
  otro pool*; el primer despliegue será **misma VPC y zona, otra máquina**. La proximidad
  árbol↔modelo↔datos que motivó el ② se conserva; el pool de GKE es el destino cuando la cuota
  bajo demanda exista (nota en `0024 ②`).
- **`ore-serve` ya escribe manifiestos**: la cola (`cola.rs`). El alta de una fuente rellena
  `plantilla-catalogo.txt` —que **el aprovisionador dejó rendida para esa celda**— y la empuja a
  `t-<celda>/trabajo.git`; Flux la aplica con `cola-<celda>`, cuyo `Role` es **`batch/jobs` y
  nada más** (`13-el-inquilino-reconciliado.yaml`: *«Ni `pods`, ni `secrets`, ni `deployments`»*).
  ⇒ Un `Deployment` en `trabajo` **no se aplicaría**, y esa negativa es deliberada: cierra el ④
  de `0022`. Cuando el tier dedicado viva en GKE se añade **otra cuenta con otro verbo**, no se
  ensancha ésta.
- **Los pods de la celda nacen sin salida.** La `NetworkPolicy` deniega todo egress salvo DNS,
  forja, cofre, control e IdP. Un Job de la celda **no alcanza** un modelo en `:8000` fuera de
  ella; la salida hacia el gateway la tiene que dar la plataforma, por clase, como `MAESTRO` en
  `0026`.
- **`0026` ya decidió cómo llega el estado de la celda**: el informador. Pero el pod del modelo
  compartido **no está en el namespace de la celda**: su estado no lo puede traer el informador.
  Lo trae quien lo sirve.

---

## La decisión

> ### ① Un modelo es un documento del árbol: `kind: Model`. Nombra un perfil y un tier.

Vive en `ontologia/` de la celda, junto a `Table`, `View` y `Entity`, y se escribe por
`ore-serve` como cualquier otro documento (`POST /modelos`, el mismo acto que `POST /fuentes`:
commit con quién lo pidió). La forma, a fijar en la especificación (`oos`) en la E1 con lo
que la E0 enseñe:

```yaml
kind: Model
name: qwen3-235b
profile: g4/qwen3-235b-fp8        # un perfil certificado (0028 B2): modelo, revisión, motor, máquina, números
digest: sha256:…                  # los pesos que ese perfil sirve (0028 B4); lo que corre es esto, o no corre
tier: shared                      # shared: un pod por modelo, por token · dedicated: una máquina, precio fijo
task: chat                        # chat · embed · rerank — lo que una Function puede pedirle
```

Ni `runtime`, ni `resources`, ni `weights.repo`: **todo eso es del perfil**, y el perfil lo
certifica quien lo mide. El árbol no puede pedir una configuración que nadie ha medido. Lo que
eso regala sin programarlo: el modelo es **direccionable** (`modelo/qwen3-235b`; una `Function`
que lo llame nombra un nodo, no una URL); **versionado** (cambiar `profile` o `digest` es un
commit, volver es un `revert`, quién lo pidió está dentro); y **gobernado** (Governance ve qué
vistas lo alimentan y qué escribe, porque está en el grafo).

> ### ② En el tier compartido, el documento deriva a una suscripción. El gateway la respeta.

No hay `Deployment` en `t-<celda>`: el pod del modelo es **uno por modelo, de la plataforma**,
servido por el perfil en una máquina con etiqueta `eu-dc`, y lo comparten las celdas que lo
nombran. Lo que el `Model` de una celda produce es **que esa celda pueda llamarlo**: identidad
de la celda (`ore-agente-<celda>`, que ya existe), el perfil, una cuota y la contabilidad por
inquilino (B3). El gateway **no enruta** a un tenant soberano a una máquina `community`: la
etiqueta viaja con la máquina y se comprueba en cada llamada.

~~⚠️ **Hueco nombrado, por decidir en la E2:** de dónde lee el gateway qué puede llamar una
celda.~~ **Decidido el 2026-09-16, con la medida hecha (Bastion `03bbc22`): la opción (a).**
El gateway acepta el **token de agente de la celda** tal cual lo verifica `ore-entrada`
—RS256 contra un JWKS de fichero, `iss` exacto, `aud` nuestra (`modelos`), `exp`,
`rubix_tipo = agente`— y lee la celda del claim `rubix_celda`; la celda es el tenant. **El token
dice quién; qué puede llamar es la suscripción que `ore-serve` provisiona** en el plano de
control del gateway (`POST /admin/tenants/{celda}/models {model}` al crear el `Model`, `DELETE`
al retirarlo). Una celda sin `Model` recibe 401 «not subscribed»; una suscrita ve y llama sólo
sus modelos; la de al lado, 401 — es exactamente la aceptación de la E2, y está en las pruebas
del crate. Medido en proceso, 256 peticiones con 32 concurrentes: clave `bk_` 0,13 ms/petición,
token de agente 0,13 (se verifica una vez por vida del token y se cachea por hash), token nuevo
en cada petición 0,30. Contra los 20 ms de un token de salida, nada.

Por qué (a) y no «el gateway lee el árbol»: leerlo obligaría al gateway a conocer git y el
formato del árbol, y pondría dos lectores del mismo documento; con (a) el árbol sigue siendo
el sistema de registro, `ore-serve` sigue siendo quien lo deriva, e `iam` no gana un verbo:
la concesión sobre el conducto no hace falta porque **la suscripción ya es la concesión** y
vive donde se ejecuta. Lo que la plataforma tiene que añadir: la audiencia `modelos` en los
tokens de agente y un mapeador por cliente que emita `rubix_celda` (un cliente por celda, como
ya tiene 0026); y `ore-serve` llamando al plano de control del gateway desde `/modelos`.

> ### ③ En el tier dedicado, el documento deriva a un despliegue con una cuenta que sólo puede desplegar.

Una máquina para esa celda —hoy una VM G4 lanzada por B1 con el perfil; cuando haya pool en
GKE, un `Deployment` en `t-<celda>` con toleración al pool `gpu`—. La figura de la cola, sin
excepciones: el aprovisionador deja en la celda **la plantilla rendida** (namespace, pool,
taint, etiquetas, cuenta, límites), `ore-serve` sustituye **sólo** nombre, perfil y digest, y
la empuja a un tercer repositorio `t-<celda>/modelos.git` que Flux aplica con
`desplegar-<celda>`, cuyo `Role` en `t-<celda>` es exactamente `apps/deployments` y
`services` (get list watch create patch delete) y `namespaces` get. Ni `pods`, ni `secrets`,
ni `networkpolicies`, ni `jobs`: **desplegar un modelo**, y se lee de un vistazo. `prune:
true`: un `Model` que desaparece del árbol retira su despliegue. Es la E5; no se construye
antes de que lo compartido sirva.

> ### ④ Los pesos no pasan por el árbol, y la salida la da la plataforma por clase.

Los bytes los trae y los cachea B4 en la máquina que sirve (`/models/hf`, y encima el `.bst`
con hash y firma); el árbol lleva el `digest`, y **lo que corre es lo que el árbol dice, o no
corre**. `ore-serve` no toca un byte de pesos: sigue sin red. Lo que sí necesita salida es
**la celda hacia el modelo**: una `NetworkPolicy` del compartimento —la escribe la plataforma,
la aplica el `Kustomization` de la celda— que permite a los pods `ore.dev/rol: serve` y a los
Jobs egress **sólo** al gateway (`IP/32:puerto`, constante nombrada como `MAESTRO`). Ninguna
celda alcanza una máquina de modelo directamente.

> ### ⑤ La puerta es el gateway. No se construye dos veces.

La celda llama a `https://modelos.<entrada de la plataforma>/v1` con su token de agente; lo
que una `Function` invoca es `modelo/<nombre>` y `ore-serve` lo resuelve a esa URL y ese
perfil. **Lo público** —una URL y claves para que el cliente llame desde fuera— es
exactamente B3 (claves por inquilino, cuotas, contabilidad): la pestaña *Endpoint* de la
consola enseña lo que el gateway emite, no una puerta propia.

> ### ⑥ El estado lo trae quien sirve; el plano de control observa y no posee.

- **Compartido:** el gateway contabiliza por celda —tokens, latencia, cuota consumida, si el
  perfil está sirviendo— y lo expone; la consola lo pinta cruzado con lo que el árbol declara
  (`GET /modelos` de `ore-serve`). Declarado y no servido = *provisioning*; servido y no
  declarado = *retiring*; los dos = lo que el gateway diga.
- **Dedicado:** `bastion status` (registro reconciliado con el proveedor) mientras sea una VM;
  el informador de la celda (`0026`, snapshot v2 con `despliegues`) cuando sea un pod en su
  namespace.

**No hay tabla de despliegues en `iam`.** Lo que `iam` guarda de esto es, como siempre, quién
puede: la concesión de ②, si se elige ese camino.

> ### ⑦ El encaje es «hay perfil y hay máquina», y se decide en el servidor.

`POST /modelos` rechaza (422, con el motivo) un `profile` que no existe en la matriz de
certificación, un `digest` que no es el del perfil, y `tier: dedicated` sin cuota de máquina
para esa organización. **El Hub enseña la matriz de certificación** —hoy tres perfiles, con
sus tok/s y $/M medidos— y lo que está *por certificar* como tal, no un catálogo de dieciséis
modelos con una píldora calculada en vCPU. La consola no inventa el encaje: lo pinta.

---

## Lo que se acepta a cambio

- **El Hub encoge a lo medido.** Tres perfiles el día uno. Es menos y es cierto; la lista
  «por certificar» dice lo que viene y no lo vende.
- **Ningún modelo en CPU dentro de la cuota de la celda.** Embeddings y modelos pequeños que
  cabrían en 10 vCPU no se ofrecen hasta que tengan perfil en una máquina certificada (un
  `g1` sirve un 8B o un embedder a muchos inquilinos por menos que N celdas en CPU).
- **Dos tiers, dos derivaciones.** Una suscripción y un despliegue son objetos distintos con
  el mismo documento delante. Lo que se gana es que compartido y dedicado se cobran como 0028
  dice, y que el dedicado es *el mismo manifiesto en otro sitio* (`0024`).
- **El primer modelo corre fuera de GKE.** Una VM en la misma VPC, con etiqueta y apagado
  verificado. El pool de GKE llega con la cuota bajo demanda; hasta entonces `0024 ②` lleva una
  nota, no una excepción silenciosa.
- **El gateway es una pieza más entre la celda y el modelo.** Un salto de red y un proceso
  que puede caer; a cambio, claves, cuotas, contabilidad y la etiqueta de soberanía en un
  sitio, no en N celdas.
- **Dependencia de la matriz de certificación.** Un modelo entra en el Hub cuando alguien lo
  mide en una máquina; es trabajo que no termina (0028 lo acepta por su lado).
- **El árbol lleva un `kind` más**, y la especificación (`oos`) tiene que decirlo antes de que
  `ore-serve` lo acepte.

---

## El abordaje — y por qué es así y no de golpe

Igual que en `0026`: cada etapa deja el sistema entero y medido. **El sustrato lo mide
Bastion** (sus hitos 1 y 2: `bench.sh` PASS con la imagen en un `g1`/`g4`, y el mismo comando
en G4 cuando haya cuota). Lo que ORE tiene que medir primero es **la otra mitad: que una celda
alcance un modelo servido por un perfil, y que el árbol lo nombre**.

### E0 · La celda alcanza el modelo — ✓ 2026-09-16 (a)–(e) en `victor`; (b) con un g1 real de Vast

`pruebas-de-fuego/medida-la-celda-alcanza-el-modelo.py`, contra el gateway de Bastion (B3,
`bastion/gateway:0.1.0-2`) en **una `e2-micro` de la misma VPC** (`modelos-e0`, IP interna
reservada `modelos` = `10.10.0.100`, sin IP pública, ~0,01 $/h) con **un vLLM de mentira**
detrás (`bastion/env/e0`: contesta `pyme` a un prompt de segmentación, una palabra cada 20 ms)
— porque cuatro de las cinco filas miden **red, identidad y árbol**, no inferencia, y el g1 no
era pagable. Dos Jobs reales de la celda (`driver`, por la cola; 100–112 s hasta arrancar: el
pool `jobs` desde cero):

| | medido | resultado |
|---|---|---|
| (a) | un Job de `victor` contra `10.10.0.100:8000` con la `NetworkPolicy` de hoy | **no llega**: `curl` rc 28 a los 8 s — `deny-all-egress` tira el paquete |
| (a) | el mismo Job con `salida-al-modelo` (`driver` → `MODELOS/32:8000`, y nada más) aplicada a mano | **llega**: 401 en 2,5 ms — la puerta pide identidad |
| (c) | el mismo `curl` con el token real de `ore-agente-victor` (acuñado dentro, como los Jobs de 44) | **401 «cell victor is not subscribed to any model»** sin `Model`; `POST /admin/tenants/victor/models` (lo que `ore-serve` hará en `/modelos`) → **200 visto desde la celda ≤ 4 s** después, y ve sólo su modelo |
| (b) | 1 y 4 llamadas concurrentes desde la celda, **g1 real** (Vast 51218218, Suiza, 1,54 $/h, `community`, registrado en este gateway por un túnel ssh que vive en la VM) | 1 llamada: **TTFT 202 ms, 16 tokens en 304 ms**; 4 concurrentes: TTFT 196–534 ms, total 276–592 ms. Lleva dentro celda → VPC → ssh Bélgica↔Suiza; el vLLM solo daba TTFT 111 ms. (Con el stub: 3–5 ms / 197 ms, que sólo medían el camino) |
| (d) | `functions/segmentar.yaml` con `entrypoint: modelo/v2-lite`; el Job la ejecuta a mano (F4 no existe), llama por el gateway con el token, hace la `Propuesta`, `ore verify` la coteja | **`ventas.Cliente.segmento [clienteId=C-0001] ← pyme`**, «la propuesta cae dentro de lo que el paquete autoriza»; commit `83ac7ac` (stub) y **`c35033e` con el g1 real** (`1. pyme`, 68+8 tokens, 259 ms) por `ventas.segmentar <modelo-v2-lite@victor.invalido>` en `propuestas/`; `GET /paquetes` de `ore-serve` lista `ventas 0.1.0` |
| (e) | `medida-el-estado-de-la-celda.py victor --cotejar` | snapshot fresco, sin diferencias; los pods vivos son `cofre, control, forja, informador` — ninguno sirve un modelo |

**Lo que E0 enseñó, y es la forma de ① que E1 tiene que escribir:**

- **La gramática de hoy admite `runtime: model` y `entrypoint: modelo/v2-lite`** sin tocar
  `oos` (el valor de `runtime` no se comprueba); el prompt sólo cabe en una extensión
  (`x-ore-prompt`) porque `Function.spec` es cerrado. E1 decide si eso es la forma o si
  `runtime: model` merece claves propias.
- **La salida de un modelo sin endoso es `untrusted`**, y `OOS7002` lo hace cumplir: la
  propiedad que escribe y el conducto de materialización tuvieron que declararse `untrusted`
  para que el paquete compilara. Es correcto —es lo que una clasificación sin revisar es en
  el retículo— y el `Model` no puede prometer más sin un endoso.
- **Resolver `modelo/v2-lite` es lo que el documento `Model` da**: hoy la plataforma da la
  puerta (`MODELOS`) y el perfil da el id servido (`deepseek-ai/DeepSeek-V2-Lite`); el Job lo
  toma de variables y lo dice. E1 lo lee del `Model` (`profile` → id).
- **La suscripción es la concesión y basta**: tenant = celda, `allowed_models` = los `Model`
  del árbol. Y un hueco del gateway que la medida destapó y se cerró (Bastion): una celda a
  la que se le retira su último modelo **es una celda no suscrita** (401), aunque su fila
  siga — la segunda pasada devolvía 200 con lista vacía.
- **Lo que aterriza es la `Propuesta`, no la copia**: aplicarla por la vista es F5
  (`functions.md`), que no existe. El eco que E0 pedía —una propiedad del árbol escrita por
  el modelo, con su commit— está; la edición sobre la copia llega con F5.
- **La máquina de modelos vive fuera de GKE y dentro de la VPC**, exactamente 0024 ②: la regla
  de red es `ipBlock` + puerto, y la firewall de la VPC (`ore-modelos-desde-la-malla`: pods y
  nodos → tag `modelos`, 8000 y 9000) es su otra mitad. COS tira lo que entra por defecto: el
  arranque de la máquina abre los dos puertos a `10.0.0.0/8`.
- **Lo que la pasada real enseñó de paso**: el nodo `sistema-spot` fue reclamado **dos veces** a
  mitad de medida (13:04 y ~14:10 UTC) y la plataforma entera tardó ~6 min en volver; un IdP que da 502 durante
  eso no puede tumbar el gateway — su JWKS es un fichero y el arranque conserva la última copia.

Para (b) el tenant `victor` se puso en `any` durante la pasada (una máquina `community` no se
enruta a un tenant `eu-dc`: la etiqueta se comprobó) y volvió a `eu-dc` después; el g1 se
destruyó con verificación (0,46 $, 0,3 h) y el vLLM de mentira volvió a ser el backend.

Lo que E0 deja en `victor`: `packages/ventas`, `functions/segmentar.yaml`, `lattices/assurance.yaml`,
`conduits.yaml` (commit `437cd35`) y `propuestas/segmentar-C-0001.json` (`83ac7ac`, `c35033e`). La regla
`salida-al-modelo` se retira al acabar; E2 la lleva a la plantilla.

*La aceptación tal como se escribió:* Con un `g1` servido por
`bastion launch` con `g1/deepseek-v2-lite` —en Vast hoy, etiqueta *community*: **sólo un
prompt de prueba, nunca datos**; en G4 cuando llegue—, desde `t-victor`: **(a)** que un Job de
la celda **no** alcanza `:8000` con la `NetworkPolicy` de hoy (el timeout, con código) y sí con
la regla de clase de ④ aplicada a mano; **(b)** latencia y TTFT **vistos desde la celda**, no
desde el bench, con 1 y con 4 llamadas concurrentes; **(c)** el mismo `curl` con el token de
agente de la celda en la cabecera, aunque hoy nadie lo compruebe: es el que B3 comprobará;
**(d)** una `Function` del árbol que nombra `modelo/v2-lite` y cuya salida **aterriza en la
ontología** —el eco que de verdad hay que demostrar—; **(e)** que `--cotejar` de `0026` no ve
nada nuevo en la celda: el modelo no está en ella. **Acepta:** la tabla con (a)–(e) en
`victor`, y una propiedad del árbol escrita por el modelo, con el commit que la trajo. De aquí
sale la forma de ① con lo que hizo falta, y la regla de red.

### E1 · El documento — es ORE, y ya tiene la forma que E0 enseñó

`kind: Model` en `oos` (bump del submódulo) con la forma de ①: `profile`, `digest`, `tier`,
`task`; y **`runtime: model` con claves propias** en `Function.spec` (el `entrypoint` nombra
`modelo/<n>` y el prompt deja de vivir en `x-ore-prompt`), con la salida `untrusted` por
defecto que `OOS7002` ya hace cumplir. En `ore-serve`: `POST /modelos` · `GET /modelos` ·
`DELETE /modelos/{n}`, que **escriben el documento en el árbol y, en el mismo acto, provisionan
la suscripción en el gateway** (`POST`/`DELETE /admin/tenants/{celda}/models`, ②; `--modelos
URL` como `--cola`); el encaje de ⑦ contra **la lista de perfiles** (un fichero que el
aprovisionador deja en la celda, como `plantilla-catalogo.txt`, hasta que Bastion la publique
como B2); y `modelo/<n>` resuelto por `ore-serve` a `(puerta, id servido)` desde el `Model`,
que es lo que el Job de E0 tomaba de variables. **Acepta:** `los-verbos` con los casos de ⑦
(perfil certificado → 201, el commit, y la suscripción provisionada —un gateway de banco en
la prueba—; perfil inexistente → 422; digest que no es el del perfil → 422; retirar → el
fichero desaparece y la suscripción también); `ore verify` acepta un paquete con `runtime:
model` sin extensiones.

**I1 · el `kind` — ✓ 2026-09-16** (`oos` v1alpha9: `spec/v1alpha9/{00-scope,01-model}.md`,
`schemas/v1alpha9/{model,function}.schema.json`, conformidad 9/9 —2 aceptan, 7 rechazan—; en
el núcleo `Kind::Model`, `V1Alpha9`, las reglas de forma y `OOS2005` cuando `model` no resuelve;
`ore init` crea `modelos/`). La forma que quedó, y en qué difiere de la frase de arriba: la
función lleva **`model: modelo/<n>`** como clave propia y `entrypoint` sigue siendo de `wasm`
—decir «lo que se ejecuta» con la misma clave para un módulo y para un nodo del árbol era dos
significados en un nombre, y `OOS1004` lo rechaza si vienen juntos—; `prompt` sólo con
`runtime: model`; `Model` sin `namespace` (vocabulario compartido, como un retículo:
`modelo/<n>` se direcciona desde cualquier paquete) y sin `labels`. Ningún código nuevo.
Falsificado a mano antes de la suite: perfil mal formado, tier fuera del vocabulario, sin
`task`, digest corto, `runtime` en un `Model`, `Model` en v1alpha8, modelo que no está, `model`
con `entrypoint`, `model` con `runtime: wasm`, `model` en una `Function` v1alpha8 — diez
rechazos, cada uno con su código y su ayuda. *Pendiente de empujar el submódulo a
`describeloai/oos` antes de que CI construya con el puntero nuevo.*

**I2 · la lista de perfiles — ✓ 2026-09-16.** Bastion la publica (`bastion profiles --json` es
el documento; `--publish gs://bastion-perfiles` lo sube, lectura pública sin credencial:
`https://storage.googleapis.com/bastion-perfiles/perfiles.json`, 3 perfiles, `digest: null`
hasta B4) y **el aprovisionador la baja a la cola** en cada pasada, al lado de
`plantilla-catalogo.txt` (`PERFILES_URL` en `aprovisionar-inquilino.sh`; sin red la celda se
aprovisiona igual y lo dice). Se baja, no se rinde: es un hecho del sustrato, y quien lo
publica es quien lo mide. Medido en `victor`: la pasada real desde fuera empujó
`perfiles.json` a `t-victor/trabajo` (`fd65e60`) y Flux aplicó la revisión sin inmutarse
—un `.json` en la cola no es un manifiesto—. *Lo que costó: el `curl` de mingw no escribe con
`-o` en la ruta de `mktemp` desde dentro del guion («(23) client returned ERROR on write»);
por redirección sí.* Lo que I3 lee: `perfiles.json` del clon de la cola, con la forma
`{v, image, generated, profiles: [{profile, model, machine, gpus, status, tok_s{1,8,16,32},
ttft_ms, usd_h, usd_per_mtok, digest}]}`.

**I3 · los verbos — ✓ 2026-09-16** (`crates/ore-serve/src/modelos.rs`; `--modelos host:puerto`,
`--modelos-url`, `--perfiles`). `POST /modelos {name, profile, tier?, task?, digest?,
description?}` · `GET /modelos` · `GET /modelos/{n}` · `DELETE /modelos/{n}`, con la figura
de `POST /fuentes` y **una diferencia deliberada: el documento y la suscripción en el mismo
acto, o nada** — si el gateway no contesta, 502 y el árbol intacto (una fuente cuya
credencial falla queda declarada; un `Model` sin suscripción sería una promesa). El encaje
de ⑦ se decide en el servidor contra `perfiles.json` de la cola: perfil que no está → 422
con los que hay; `tier: dedicated` → 422 (E5); `digest` cuando el perfil no publica el suyo
→ 422; la forma la decide la gramática (`ore validate` sobre el clon). `DELETE` con una
`Function` que lo nombra → 409 (`OOS2005`) y el fichero se queda; sin nadie → 200, fichero y
suscripción fuera. `GET /modelos/{n}` resuelve `modelo/<n>` a `{url, model}` (la puerta de
`--modelos`, el id del perfil), que es lo que el Job de E0 tomaba de variables. **Acepta:**
`pruebas-de-fuego/los-modelos.sh`, 0–8, contra el gateway de banco
(`gateway-de-banco.py`, el contrato del plano de control de B3) **y contra `bastion-gateway`
de verdad** (`BASTION_GATEWAY=…`): los dos verdes, y en CI. En la plantilla: `--modelos
MODELOS:9000` en `40-ore-serve.yaml`, con `MODELOS` como constante nombrada en
`gen-inquilino.py` (⑬, como MAESTRO). *Lo que la pasada real enseñó: sobre un directorio no
hay clon que tirar —`escribiendo` escribe en sitio— así que el verbo deshace lo suyo cuando
falla (retira el fichero; lo vuelve a escribir en un `DELETE` negado).* Lo que queda para E2:
la regla `control → MODELOS/32:9000` en la plantilla (hoy el alta en `victor` daría 502 a
los 15 s, y está dicho en 40), la imagen nueva por CI, y la primera llamada en < 60 s.

### E2 · La celda llega al gateway sin que nadie toque nada — ✓ 2026-09-16 en `victor`, de punta a punta

El hueco de ② está decidido y medido; lo que queda es que la plataforma lo lleve a la
plantilla y al realm. `salida-al-modelo` en `13-el-inquilino-reconciliado.yaml` (pods `driver`
y `ore-serve` → `MODELOS/32:8000`, constante nombrada como `MAESTRO`, cotejada por
`gen-inquilino.py` ⑫ y por la medida contra la dirección reservada `modelos`); la firewall
`ore-modelos-desde-la-malla` como su otra mitad, escrita en `malla/`; en el realm, la
audiencia `modelos` y el mapeador por cliente que emite `rubix_celda` (`gen-realm.py`, un
cliente por celda como 0026); el gateway en G4 con etiqueta `eu-dc` cuando llegue la cuota
(hito 3 de 0028) y hasta entonces en la `e2-micro` de la VPC con el vLLM de mentira o un g1
`community` sólo para medir. **Acepta:** en `victor`, `POST /modelos` desde `curl` → la
primera llamada de un Job contesta en < 60 s sin que nadie toque `kubectl`; `DELETE` → 401;
la celda de al lado, sin `Model`, → 401; `--cotejar` de E0 limpio con la regla ya en la
plantilla.

**I1 · la regla en la plantilla — ✓ 2026-09-16.** `salida-al-modelo` (`driver` →
`MODELOS/32:8000`) y `salida-al-modelo-del-control` (`control` → `MODELOS/32:9000`) en
`11-el-inquilino.yaml`; la segunda no la nombraba la ADR y es la que I3 de E1 necesita —sin
ella `POST /modelos` tira el paquete y contesta 502 a los 15 s—. `gen-inquilino.py` ⑭ exige
exactamente esas dos hacia MODELOS y que ninguna otra plantilla abra esa IP; `--cotejar`
coteja MODELOS con la reserva `modelos` de la VPC, como MAESTRO con el API server.

**I2 · la red en `malla/` — ✓ 2026-09-16.** `71-la-red-de-los-modelos.sh` (idempotente,
`--seco`): la reserva `modelos` = MODELOS —cotejada con la constante de `gen-inquilino.py`—,
`ore-modelos-desde-la-malla` (pods y nodos → tag `modelos`, 8000 y 9000) e
`ore-modelos-iap-ssh`; la máquina **no se crea aquí** (es de Bastion), sólo se comprueba que la
que use la IP lleve el tag. Primera pasada real: todo «ya estaba» (E0 lo dejó a mano) y la
máquina `modelos-e0` con su tag, parada.

**I3 · el realm — ✓ 2026-09-16.** El paso ⑦ del aprovisionador pone en cada cliente de agente
dos mapeadores y **los converge** —un cliente creado antes los gana en la pasada siguiente sin
que nadie toque el realm—: `audiencia-modelos` (`included.custom.audience`, como cadena y no
como cliente: el gateway no es cliente del realm, verifica con el JWKS de fichero y sólo mira
que `modelos` esté en `aud`; un cliente-audiencia más sería un secreto más que nadie usa) y
`rubix-celda` (`rubix_celda` = la celda; `azp` deja de ser un contrato). La receta genérica de
`gen-realm.py` lleva la audiencia. Medido: dos pasadas del CronJob —la primera cogió el guion
nuevo a mitad de corrida y sólo `victor` lo ganó: el ConfigMap montado cambia bajo el proceso—
y los tokens de `demo`, `prueba` y `victor` llevan `aud [modelos, ore-serve, account]` y
`rubix_celda` = su nombre. El gateway (Bastion `9679cb9`): `--oidc-audience modelos`,
`rubix_celda` manda sobre `azp`.

**I4 · la aceptación — ✓ 2026-09-16** (`pruebas-de-fuego/la-celda-llega-al-gateway.py`, en `victor`
con `prueba` de vecina; `kubectl` sólo para crear los Jobs de medida y leer sus logs):

| | | |
|---|---|---|
| ⓪ | la Function de E0 (forma de v1alpha8) se retira del árbol: la gramática nueva la rechaza (`runtime: model` sin `model`) | `a4b6303` |
| ① | `POST /modelos {v2-lite, g1/deepseek-v2-lite}` por la puerta pública con el token de agente | **201 en 2,0 s**, commit `5424e11`, «provisionada en 10.10.0.100:9000»; `GET /modelos/v2-lite` resuelve `{model, url, certificado: true}` |
| ② | un Job de la celda (`driver`, por la cola, su token: `aud [modelos, ore-serve]`, `rubix_celda: victor`) | ve su modelo **a la primera llamada**; TTFT 2,8 ms · 197 ms (el vLLM de mentira) |
| ③ | `DELETE /modelos/v2-lite` mientras el Job mira | 200 en 2,0 s → el Job recibe **401 «cell victor is not subscribed»** |
| ④ | un Job de `prueba` con SU token | **401** |
| ⑤ | `--cotejar` de 0026 · `GET /modelos` | limpio · `[]` |
| ⑥ | el estado final: `POST` otra vez (201), la Function de E1 en el árbol (`runtime: model`, `model: modelo/v2-lite`, `prompt`; `af029c4`), `DELETE` con la Function nombrándolo | **409** «no se retira: alguien del árbol lo nombra (OOS2005)» |

Lo que la primera pasada destapó: el árbol de `victor` llevaba la Function de E0 en la forma de
v1alpha8 y `POST /modelos` la rechazó con 422 al validar el clon — correcto: la forma provisional
se retira y la de E1 llega con el `Model` (⓪ y ⑥). **Nadie tocó `kubectl` para la plataforma**:
las reglas, `--modelos` y los claims llegaron por Flux y el CronJob; la máquina `modelos-e0` se
encendió para la pasada y se apaga después.

### E3 · Deployments tiene filas

La consola cruza `GET /modelos` con lo que el gateway contabiliza (⑥); *Crear* deja de estar
deshabilitado y llama a `POST /modelos`; el Hub pinta la matriz de certificación (⑦); el
detalle con *Overview · Endpoint · Usage*; los motivos de 422 en pantalla tal cual.
**Acepta:** desde el Hub, *Use in this cluster* → fila en *provisioning* con la hora →
*running* con tokens contados; y el `Model` está en el árbol con el autor de la sesión.

Medido antes (`pruebas-de-fuego/medida-deployments-tiene-filas.py`, 2026-09-16): el Hub pintaba
un catálogo estático de 16 con una píldora en vCPU; *Deployments* estaba vacía y dicha; la consola
no conocía `/modelos`. `GET /modelos` ya cubría nombre, modelo, tarea y puerta, pero no lo que
hace de eso una fila: estado, réplicas, autor, uso. El gateway cuenta backends `up` y uso por
día × celda × modelo, y **sólo la VPC lo alcanza**: el cruce de ⑥ se hace en `ore-serve`, no en
la consola. Cinco pasos: I1 `ore-serve` · I2 la consola conoce los verbos y el Hub pinta la
matriz · I3 filas y *Crear* · I4 el detalle y *Retirar* · I5 la aceptación en `victor`.

**I1 hecha** (2026-09-16): `GET /perfiles` (la lista tal como Bastion la publica); `GET /modelos`
y `GET /modelos/{n}` ganan `estado {fase, backends, motivo?}`, `uso {hoy, mes}`, `autor` y
`desde`. `ore-serve` pregunta al gateway **una vez por petición** —`/admin/health`,
`/admin/tenants`, `/admin/usage?tenant=&from=<mes>`— y cruza: declarado y un backend `up` sirve
su id → `running`; ninguno arriba → `provisioning` con motivo; la celda no suscrita → `error`
(deriva); el gateway no contesta → `error` con el motivo, **y la ficha sale igual** (`gateway
{contesta, motivo}` en la lista: la consola nunca espera al gateway); suscrito sin documento →
fila `retiring` con `declarado: false`. `autor` y `desde` salen de `git log -1 -- modelos/<n>.yaml`
en el clon: el verbo firma con el sujeto (RFC 8693 en el commit), así que la fila lleva quién la
pidió sin ninguna tabla. Aceptado por `los-modelos.sh` (4e–4i, 7b, 8b) contra el banco **y contra
`bastion-gateway` de verdad** (`--health-every 1`, un `--como-backend` que contesta `/v1/models`):
sin backend → *provisioning*; el backend registrado y sondeado → *running* con
`estado.backends: [banco-g1]`; retirado → *provisioning*; el uso suma hoy/mes y no cuenta otro
modelo; el gateway caído → *error* en 2 s. En CI.

**I2 hecha** (2026-09-16, `rubix-platform` `fec7f2c`): la consola conoce `perfiles`, `modelos`,
`modelo` y los mandatos `crear-modelo` / `retirar-modelo` (`lib/server/query.ts`, por la puerta
pública de la celda con el token de quien mira); `lib/models/perfiles.ts` deshace lo que el JSON
de `ore-serve` no puede decir (números como texto, `null` como `false`). El Hub retiró
`CATALOGO` (16 modelos a mano, píldora «Fits · CPU» en vCPU) y pinta la lista de certificación
tal cual: secciones por máquina, etiqueta de certificación y $/Mtok, la ficha con tok/s por
concurrencia, TTFT, $/h, digest (B4) y la imagen medida; «Use in this cluster» lleva a
Deployments con `?perfil=`. El banco de la consola contesta `celdas`, `perfiles`, `modelos` (las
cuatro fases) y `modelo`. Visto en el banco: 3 perfiles, ninguna píldora inventada.

**I3 hecha** (2026-09-16, `rubix-platform` `5c25665`): *Deployments* pinta `GET /modelos` como
tabla —nombre · estado con su motivo · modelo y perfil · máquina · réplicas = backends arriba/1 ·
uso hoy y mes · endpoint interno · autor del commit («you» si es el `sub` de la sesión) · desde—;
la deriva sale como fila `retiring` sin nombre; si el gateway no contestó, un aviso rojo arriba:
ninguna fase de abajo es real. *Use in this cluster* llama a `POST /modelos` por una Server
Action (`models/acciones.ts`): el `Model` en el árbol con la firma de la sesión y la suscripción
en el mismo acto, o nada; los 422 (perfil que no está, dedicated, digest), el 409 (ya hay) y el
502 (el gateway no contestó: nada escrito) salen en el formulario tal cual. Visto en el banco:
las cuatro fases, y la escritura rechazada por el banco con su frase (las escrituras no se
simulan). La aceptación de la ADR —*provisioning* con la hora → *running* con la máquina, el
`Model` con el autor de la sesión— es I5, en `victor` con `modelos-e0` encendida y una sesión
de persona.

**I4 hecha** (2026-09-16, `rubix-platform` `f0d0378`): el detalle `/models/deployments/{n}` pinta
`GET /modelos/{n}` en tres pestañas —*Overview* (el documento y lo que lo sirve), *Endpoint*
(la URL interna, el id, quién puede llamar y la `Function` que lo nombra como `modelo/<n>`; la
pública es E4), *Usage* (hoy y mes, tal como el gateway cuenta; la latencia por celda no se
cuenta y se dice)—. *Retire* → `DELETE /modelos/{n}`: el 409 se enseña como aviso («alguien del
árbol lo nombra: nada retirado», con el OOS2005 del servidor) y no toca nada; el 502 como «el
modelo se queda»; retirado, vuelve a la lista sin la fila. El nombre en la tabla enlaza al
detalle. Visto en el banco.

**I5 hecha — E3 aceptada** (2026-09-16 22:21–23:02, `victor`, `modelos-e0` encendida 40 min,
`pruebas-de-fuego/deployments-tiene-filas.py` a cuatro manos: la persona en su consola, el guion
alrededor):

| | quién | qué se vio |
|---|---|---|
| ⓪ | guion | `start` de la máquina → el gateway contesta con un backend arriba a los **23 s**; `GET /modelos` (con el `ore-serve` de I1, `af0ec27`): `v2-lite` *running*, `['e0-de-mentira']`, autor = el agente de E2, `uso.hoy` 19 req · 465 tok |
| ① | persona | *Deployments*: la fila *Running* 1/1 con la máquina y el uso; el detalle con *Overview · Endpoint · Usage* |
| ② | persona | *Retire* → **409 en pantalla**: «no se retira `v2-lite`: alguien del árbol lo nombra… OOS2005… Retira primero la Function que lo invoca». Nada tocado |
| ② | guion | Job en la celda: `functions/segmentar.yaml` fuera (`38e5c71`) |
| ③ | persona | *Retire* → «v2-lite retired», la lista vacía |
| ③ | guion | `GET /modelos` → `[]` (sin deriva: el verbo quitó las dos) · el Job de la celda recibe **401** «cell victor is not subscribed to any model» |
| ④ | guion | `docker stop de-mentira` en la máquina → `backends_arriba=0` en 8 s |
| ④ | persona | *Hub → DeepSeek-V2-Lite → Use in this cluster* (nombre por defecto `deepseek-v2-lite`) → fila **Provisioning 22:47:29** «ningún backend sirve … todavía», *Created by* = la persona |
| ④ | guion | `docker start de-mentira` 22:50:48 → **Running** `['e0-de-mentira']` 7 s después · `git log -1 -- modelos/deepseek-v2-lite.yaml`: **autor = el `sub` de la persona** (`21e8ffd9-…`), committer `ore-serve`, `70b17a3`, «alta de un modelo» |
| ⑤ | guion | la Function de E1 vuelve nombrando `modelo/deepseek-v2-lite` (`43828e6`, `ore validate` ok) · el Job de la celda ve el modelo (200) · máquina apagada, `TERMINATED` |

Lo que la pasada destapó: (a) un Job sin `ore.dev/rol: driver` no alcanza el metadata server —las
NetworkPolicies seleccionan por rol— y `gcloud` dice «no active account», que despista; (b)
`docker` en COS pide `sudo`; (c) un `503 unconditional drop overload` aislado de la entrada
pública en una lectura — reintentada, nada; (d) la persona se quedó con el nombre por defecto
(`deepseek-v2-lite`), y el guion se adaptó: el nombre es suyo. Y una petición de la persona al
verlo: en vez de «you», un redondel con el icono de perfil (relleno si es quien mira), hecho
(`components/models/Autor.tsx`).

### Después de E3 · de un modelo de verdad a inferencia consistente sobre los datos, medido

La pregunta (2026-09-16, con E3 aceptada): ¿qué falta, en orden, para que un modelo REAL corra
inferencia de forma consistente sobre los conjuntos de datos de la ontología, como Function o
como sea? Medido eslabón por eslabón (`pruebas-de-fuego/medida-la-inferencia-sobre-los-datos.py`,
sobre `demo`):

| eslabón | hoy |
|---|---|
| el modelo | detrás del gateway hay un vLLM **de mentira**; el g1 real corrió 20 min en Vast (E0 b) por túnel ssh; la cuota de GPU en europe-west1 es **0** para G4 (sólo K80/P100/V100 legacy a 1); **nadie enciende una máquina** cuando hace falta (`bastion launch` es a mano) |
| los datos | `demo` tiene datos REALES (Olist en Postgres, paquete `olist` elegido, 8 entidades); pero **no hay copia en ningún almacén**: la malla nunca llama a `ore materialize` y ninguna celda tiene bucket; sólo corren Jobs de catálogo |
| el vocabulario | `Function runtime: model` + `effects` + la `Propuesta` + `ore verify` existen; **una Function no dice sobre qué vista corre ni con qué clave** (el `Plan` de functions.md §3 no tiene forma en la gramática) |
| quién invoca | **F4 no existe** (`ore-invoke`); `segmentar` se ejecutó tres veces con un Job escrito a mano en la prueba, una fila fija |
| cuándo | **nadie** lanza una Function: ni el convergedor, ni la consola, ni un CronJob |
| dónde aterriza | `propuestas/<f>-<fila>.json` en el árbol (F1); aplicar sobre la copia es **F5, no existe**, y sin copia no habría dónde |
| la consola | Hub/Deployments hechas; Ontology Forge · Functions en marcha sobre datos de mentira; **ninguna pantalla enseña una Propuesta** |

**El orden, y por qué éste.** La cadena está rota en tres sitios —la copia, el invocador, el
aplicador— y el modelo NO es el primero: el de mentira contesta `pyme` y basta para construir
los tres; el real ya demostró en E0 b que sirve. Lo que desbloquea a los demás es la copia, porque
sin filas no hay sobre qué correr ni dónde aplicar.

| | qué | acepta |
|---|---|---|
| **P1** | **la copia en la celda**: un almacén por inquilino (bucket GCS, `ore-store-*`) y un Job `materializar` que el convergedor lanza como el de catálogo (`44-el-catalogo.yaml` → `45-la-copia.yaml`), sobre `olist` de `demo` | `GET /paquetes/olist` dice **N filas** por entidad y el digest de la copia; releer no lee el origen (refresco.sh, en la malla) |
| **P2** | **F4 para `runtime: model`, el invocador**: la Function gana de qué vista salen las filas (`over:`/`input:`, a medir), un delegado lee la copia, forma el `Plan`, llama por el gateway con el token de la celda, escribe Propuestas y las verifica | `segmentar` sobre 100 clientes de `olist` → 100 Propuestas verificadas en el árbol; el gateway cuenta los tokens en la fila de Deployments |
| **P3** | **quién y cuándo**: `POST /funciones/{n}/correr` en `ore-serve` → un Job `funcion-<n>` (Kueue), y la corrida como fila (filas hechas/total, tokens, $) en la consola; después, la corrida automática al refrescar la copia | desde la consola, *Run* → la fila avanza → las Propuestas |
| **P4** | **F5, aplicar por la vista**: las Propuestas caen en la copia (sucesora, idempotente por digest) y la vista devuelve el valor nuevo | `ventas.Cliente.segmento` se lee en la vista; aplicar dos veces = misma copia |
| **P5** | **el modelo de verdad, consistente**: pedir la cuota G4 en GCP **ya** (tarda días; B1 I3: `modelos` pasa a ser el G4 con gateway + vLLM en la misma máquina); mientras, la aceptación de P2 se corre UNA vez con un g1 de Vast (~1 $) como E0 b; y quien enciende/apaga: un reconciliador de Bastion sobre las suscripciones vivas (sin suscripción, apagado) | P2 aceptada con `deepseek-v2-lite` de verdad; la máquina apagada sola cuando nadie la nombra |
| **P6** | **la consola enseña la inferencia**: Propuestas y corridas, y Forge · Functions sobre datos reales | una Propuesta se lee donde se lee la entidad |

#### P1 · la copia en la celda: la materia, medida

`pruebas-de-fuego/medida-la-copia-en-la-celda.py` (2026-09-16, sobre `demo`):

| | hoy |
|---|---|
| el ciclo | **existe entero, en local**: `ore materialize` (679 líneas: plan → testigo → recibo → leer→sellar→subir → registrar → recoger), los lectores `postgres`/`bigquery`/`jsonl`, el almacén delegado `ore-store-r2` (1466 líneas, S3 SigV4 con clave estática), y `refresco.sh` que cuenta filas leídas del origen por acto. `ore` no abre sockets, por construcción |
| quién declara la copia | `View.materialized {datasource, table, key}` existe; **`ore discover` no lo escribe a propósito** («decisión de operación, no se propone»). En `demo` ninguna vista lo declara: alguien tiene que escribirlo |
| la celda | la imagen `ore-drivers` ya trae `ore-store-r2` y los lectores; el Job de catálogo (44) es la figura entera (forja + agente + cofre → verbo → commit); **no hay bucket por inquilino** y **nadie lanza `materialize`** (el convergedor sólo rinde catálogos) |
| el almacén | **GCS por la API S3 no se puede**: `gcloud storage hmac create` → 412, `constraints/iam.disableServiceAccountKeyCreation` (política de la organización, enforced). R2 de Cloudflare funciona pero **la copia saldría de la VPC**. GCS por su API JSON con Workload Identity es lo que los Jobs ya usan para Secret Manager: un token del metadata server, sin clave |
| el origen | `olist` en Postgres (8 entidades, Customers/Orders/…); **0 de 8 con clave primaria** en el catálogo: la clave que `materialized.key` y F5 necesitan es una decisión (hay 17 abiertas) |

**La materia, en cuatro piezas y en este orden:**

| | qué | acepta |
|---|---|---|
| **I1** | **`ore-store-gcs`**: el mismo protocolo que `ore-store-r2` (`sobre.rs`/`carga.rs` se reutilizan; cambia sólo el transporte: JSON API de GCS con el token de WI, y en local el de ADC) · `ore materialize` elige el delegado (`ORE_STORE=gcs`) | `refresco.sh` verde contra un bucket de GCS, con los mismos números de filas leídas |
| **I2** | **el bucket por inquilino** en el aprovisionador (`gs://<proyecto>-<ns>-copia`, `objectAdmin` para la cuenta `driver`, `--cotejar`) · **la decisión**: `POST /paquetes/{n}/vistas/{v}/copia {key}` en `ore-serve` escribe `materialized` en la vista con la firma de quien decide | la vista lleva `materialized`; el árbol compila; el bucket existe y sólo `driver` escribe |
| **I3** | **el Job `copiar-<paquete>`** (`45-la-copia.yaml`, la figura de 44) que el convergedor rinde por paquete con vistas materializadas, y repite al refrescar · `GET /paquetes/{n}` gana `copia {filas, digest, testigo, cuándo}` | la copia de `olist.Customers` está; la segunda pasada lee **0 filas** del origen |
| **I4** | la aceptación en `demo` con números, y la consola lo dice («N rows · copied at») | la tabla de la ADR |

**P1 I1 hecha** (2026-09-17, `16e3526`): `ore-store-r2` pasa a ser la crate `ore-store` — el
ciclo, el sobre y la carga compartidos tras un trait `Almacen` (seis verbos sobre claves y
bytes), y dos binarios que sólo cambian el transporte: `ore-store-r2` (S3 SigV4, intacto) y
**`ore-store-gcs`** (API JSON de GCS con el token del metadata server —Workload Identity— o
`ORE_GCS_TOKEN` en local; `ifGenerationMatch=0` es el `If-None-Match: *`, y el `crc32c` que GCS
devuelve se coteja con el nuestro: si no coincide, el objeto se borra). `ore materialize` elige
con `ORE_STORE` (`r2` por defecto). `refresco.sh` corre contra los dos; estaba **rojo desde
`OOS2030`** (7 sep: el fixture decía `namespace: bus` en el paquete `ventas`) y nadie lo vio
porque no está en CI (necesita un almacén). R6 verde contra un bucket de GCS de prueba, **los
mismos números** que contra R2: 1000 → 0 → 10/1010 → 3/1010 → 2 tras recoger, y las cuatro
negativas. Bucket de prueba borrado; la imagen `ore-drivers` lleva los dos binarios.

Y sobre la pregunta del 17 de septiembre —*«lo que falta es un espacio de decisión: cuándo es
copia que supera al origen y cuándo espejo»*—: sí, y con tres preguntas, no una. **Qué** vistas
tienen copia (hoy ninguna: `discover` no lo propone, con razón); **con qué clave** (la identidad
de la fila: en `olist`, 8 decisiones `clave` abiertas, porque el origen no la declara);
**y qué gana** cuando el origen y la copia se contradicen —functions.md §7.4, abierto—. Desde
el ADR 0018 la copia no es un espejo: es el sistema de registro, y el origen no se toca. Lo que
I2 pone es el sitio donde las dos primeras se deciden y firman; la tercera es de F5.

**P1 I2 hecha** (2026-09-17, `6588dd8`). **El almacén:** el aprovisionador pone un bucket por
inquilino —`gs://<proyecto>-<ns>-copia`, en la región de la celda, acceso uniforme, sin acceso
público, **cifrado con la KEK del inquilino** (la misma que cifra su cofre; el agente de Cloud
Storage gana el permiso sobre la llave como lo tiene el de Secret Manager)— y lo borra al retirar
la celda diciendo cuántos objetos había. Dos papeles y sólo dos: `ore-driver-<n>` escribe
(`objectAdmin`), `ore-serve-<n>` lee (`objectViewer`) — la separación que el ADR 0015 dejó
pedida sale gratis porque son dos cuentas. El papel `ore_aprovisionador` gana los permisos de
bucket y **ninguno sobre objetos salvo listar y borrar**: pone el almacén y no mira dentro.
`--cotejar` comprueba bucket, CMEK, prevención de acceso público y que ninguna otra cuenta
tenga nada. **La decisión:** `POST /paquetes/{n}/vistas/{v}/copia {key?}`. Medido antes: un
`materialized` a secas **no compila** (`OOS4011`, el conducto sin autorización), así que la
decisión son tres escrituras coherentes o nada — `materialized {datasource: <la de su tabla
raíz>, table: "copia.<v>"}` en la vista; `changes: mode: upsert, key: [...]` en la tabla raíz si
se pide la clave (la de la fila, la que F5 necesita; la de la entidad sigue siendo la decisión
`clave` de `review`); `materialization.payload` autorizado en `conduits.yaml`, que nace con el
dueño del paquete si no estaba— y `ore validate`: si no compila, nada queda escrito. Una vista
sobre otra vista → 422 (la copia es de la de abajo). `GET /paquetes/{n}/copias` las lista con su
clave. `la-copia-se-decide.sh` 0–6, en CI.

**P1 I3 hecha** (2026-09-17, `4a228a9`). El Job **`copiar-<resumen>`** (`48-la-copia.yaml`, la
figura de 44 con el verbo cambiado): clona el árbol, busca las tablas raíz de las vistas que
declaran copia, pide al cofre la credencial de cada fuente a una variable, y corre `ore
materialize . --recoger --informe copias` con `ORE_STORE=gcs` y el bucket del inquilino — `ore`
canaliza `ore-read-<tipo>` a `ore-store-gcs`, que sube con el token de la cuenta `driver`—; el
**informe** por vista (`copias/<paquete>_<vista>.json`: estado `copiada`/`al-dia`/`pendiente`/
`error`, clave, digest, plan, filas, leídas, bytes, testigo) se empuja al árbol, y el commit dice
quién y cuándo. No es el registro (0015: el recibo vive en el almacén, sin puntero mutable): es lo
que la última pasada dijo, como el snapshot del informador. Quién lo lanza: **la decisión** —
`POST …/copia` encola el Job en la cola de trabajo en el mismo acto (`cola::rendir_copia`, como el
catálogo), con TODAS las vistas con copia del árbol y el resumen de la lista en el nombre: una
decisión nueva es otro Job— y **el convergedor**, que detecta `materialized` en el árbol y rinde
48 con `--copias`. `GET /paquetes/{n}/copias` trae `copia {…, copiado_por, cuando}` o `pendiente`.
Lo que queda dicho y no hecho: el refresco periódico (un CronJob sobre la misma plantilla) es de
la pasada siguiente; hoy la copia se rehace cuando la lista cambia. `la-copia-se-decide.sh` 0–7 y
`refresco.sh` (el informe) verdes; la aceptación en `demo` con `olist` es I4.

**Antes de la aceptación, la pregunta del 17 de septiembre, medida** — *«en el espacio de nombres
del catálogo (database › schema › table) van a existir dos tipos de bases: las normales (copia
entera) y las foráneas (espejo, las de hoy). Resuelve el problema de raíz y le da el control al
cliente»*. Lo que hay:

| | hoy |
|---|---|
| qué es una «database» | **un paquete con alcance**: `POST /paquetes {name, source, only}` → `ore discover --from <catálogo> --only-file` → por cada objeto elegido una `Table` (el puntero al origen, con `reads` y `changes` sondeados) **y su View trivial** (`from: {table}`, los campos con nombre de identificador) y una `Entity`; `discover.scope.json` guarda `{only, source}`. El modal lo dice ya: *«maps content from the source … without moving the data»* — **todas las bases de hoy son foráneas** |
| dónde cabe la clase | en la gramática, **por vista**: `View.materialized` es la copia; no hay campo en `Package` (`spec` es `{owner, team}` y cerrado: `additionalProperties: false`) y no hace falta abrirlo: la clase de una base **es lo que sus vistas declaran** — todas con `materialized` = base; ninguna = foránea. Se deriva del árbol, no se apunta dos veces |
| qué cuesta copiar `olist` | Postgres con `wal_level = logical` y **sin claves primarias** → el driver sondeó `changes: {mode: append, witness: log}` en las 8 tablas. Compila como copia (`OOS2023` sólo rechaza `append` fechado **por columna**); el testigo `log` (LSN) hace que la segunda pasada diga «al día» sin leer; ningún lector sirve aún el rango, así que un refresco real relee entero; **y con `append` los borrados del origen no viajan** hasta que alguien decida la clave (`{key}` en el verbo de I2 → `upsert`) |
| quién lee la copia | **nadie todavía**: `ore-view::filter_tree` sabe elegir una materialización superconjunto del plan, pero ningún ejecutor la usa — es P4 (F5 aplica por la vista). Una «base» de hoy copia a la celda; las consultas siguen yendo al origen hasta P4 |
| la consola | `CreateDatabaseModal` (292 líneas): conexión fija, nombre, `only`; `comoDatabase` pinta `Mapped from <source>`; el árbol no distingue clases |

**La iteración, P1 I4 · la base y la base foránea** (y la aceptación pasa a ser I5):

| dónde | qué |
|---|---|
| `ore-serve` | `POST /paquetes {…, storage: "copy" \| "foreign"}` (`foreign` si falta: es lo que hay). Con `copy`, tras `discover`, **la decisión de I2 sobre todas las vistas del paquete en un acto** (la misma función, factorizada: `materialized` en cada vista, `key` de la clave primaria del catálogo si la hay, un solo `conduits.yaml`, un `validate`, un Job encolado con las N vistas) — o nada. `POST /paquetes/{n}/copia` = **ascender** una foránea a base (el mismo acto sobre lo que ya existe). `GET /paquetes` gana `storage: copy \| foreign \| mixed` y `copias {declaradas, copiadas}` derivados del árbol y los informes. Lo que no cambia: el verbo por vista (I2) sigue siendo el control fino; `discover` sigue sin escribir `materialized` (la decisión es del cliente, ahora en la unidad que él ve) |
| consola | el modal elige la clase (dos fichas: **Database** — *a full copy, kept in this cluster*; **Foreign database** — *a mirror: reads go to the source*); el árbol marca las foráneas; `DatabaseDetail` dice «Foreign database · mapped from X» o «Database · 8/8 tables copied · last copy <cuándo>» y la foránea gana **Copy into this cluster** (→ `/copia`). Banco con las dos |
| prueba | `la-copia-se-decide.sh` 8–9: `storage: copy` deja las N vistas con copia y un Job con las N; ascender una foránea da lo mismo; `GET /paquetes` clasifica |
| acepta | en `demo`, `olist` (foránea hoy) se asciende; el Job copia 8 tablas al bucket; la consola dice «Database · 8/8 · copied at»; la segunda pasada lee 0 filas |

Lo que se acepta a cambio y se dice: `olist` sin claves copia en `append` — el borrado no viaja;
la clave sigue siendo una decisión (`clave`, 8 abiertas) y al cerrarla la tabla pasa a `upsert`
sin tocar la base. Y una base copiada **no se consulta todavía desde la copia**: eso es P4.

**La consola primero, sólo UI** (2026-09-17, rubix-platform `1964756`): el `+` de «Databases»
abre `NewDatabaseModal` (nombre + *Type*: **Standard** / **Foreign**) en vez del alta inline; el
modal de crear database desde la conexión gana el mismo selector; `DatabaseTypeSelect` dice la
frase de cada clase. Lo elegido no viaja aún.

**Y la lógica de negocio, medida** (2026-09-17) — *«primero mover la database actual al estado
de foreign, luego el tipo nuevo de punta a punta: una copia entera de todo lo que se traiga a
esa base»*:

| | medido |
|---|---|
| **«mover» las de hoy a foráneas** | no hay nada que mover en el árbol: ninguna vista declara `materialized`, así que **son foráneas ya**. Lo que falta es que **se diga**: `GET /paquetes` no clasifica (`name, version, decisionesPendientes, scoped, source`) y la consola pinta `Mapped from <source>` sin clase. Ningún commit por inquilino: ausente = foránea |
| **dónde vive la clase** | no basta derivarla de las vistas: «estándar» es una **regla sobre lo que entre después** (todo lo que se traiga a esta base se copia), y las vistas de hoy no pueden decir nada de las de mañana. Va en **`discover.scope.json`** —el documento de `ore-serve` que ya guarda `{only, source}`— como `"type": "standard"`; ausente = `foreign`. Las vistas con `materialized` son su **consecuencia**, y `GET /paquetes` devuelve las dos cosas: `type` y `copias {declaradas, copiadas}`. Una estándar con vistas sin copia es una deriva que se enseña, no una tercera clase |
| **la clave, de dónde sale** | del catálogo: `discover.catalog.json` trae `primaryKey` por tabla (en la fuente de `demo`: las 8 de `olist.*` sin ella → `append`; las ~40 de `public.*` con `id` → ya `upsert`). Con clave, la copia nace en `upsert` sin decisión; sin ella, en `append` y la decisión `clave` sigue abierta |
| **el verbo de la copia, N vistas** | `decidir_copia` (copia.rs, 94–318) escribe 1–3 ficheros, `validate`, deshace o encola. Para N vistas del mismo paquete: las mismas escrituras por vista, **un** `conduits.yaml`, **un** `validate`, **un** Job con las N — factorizar, no repetir N veces (N `validate` y N commits) |
| **el Job con N vistas** | `48-la-copia.yaml` ya toma `VISTAS=a,b,c`: una tabla raíz y una credencial por vista, `ore materialize` recorre las N. El egreso del driver permite 443 al mundo salvo lo privado (20-driver.yaml): `storage.googleapis.com` llega como llega Secret Manager. El catálogo **no trae filas** (`rows: null`): el tamaño de `olist` se mide en la aceptación |
| **quién lee la copia** | nadie (P4). Una base estándar de esta iteración copia y lo dice; consultar desde la copia sigue siendo F5 |

**I4, en tres pasadas y una aceptación:**

| | qué | acepta |
|---|---|---|
| **I4a** | `type` en `discover.scope.json` (ausente = `foreign`) · `GET /paquetes` gana `type` y `copias {declaradas, copiadas}` · la consola lo dice: «Foreign database» en el árbol y en la ficha, en vez de `Mapped from` a secas | todas las bases de `demo` salen `foreign`, y la consola lo pinta |
| **I4b** | `POST /paquetes {…, type: "standard"}`: `discover` + la copia de TODAS sus vistas en un acto (clave del catálogo si la hay) + un Job · `POST /paquetes/{n}/copia`: ascender una foránea · `la-copia-se-decide.sh` 8–9 | una base estándar nace con las N vistas con copia y un Job en la cola; ascender da lo mismo; compila o nada |
| **I4c** | la consola manda `type`; la ficha de una estándar dice «Standard · N/N tables copied · last copy <cuándo>» y la foránea gana **Copy into this cluster** | banco con las dos clases |
| **I5** | la aceptación en `demo`: una base estándar sobre `olist` (o ascender la que hay) → el Job copia las 8 tablas al bucket → la ficha lo dice → la segunda pasada lee 0 filas | los números en esta ADR |

**P1 I4a hecha** (2026-09-17, ORE `cb7d2ee`; consola `f1d1785`, local). `GET /paquetes` dice
`type` —`standard` | `foreign`, leído de `discover.scope.json`; ausente = `foreign`, que es lo
que toda base era, así que **ninguna migración**: las de `demo` salen foráneas sin tocar un
árbol— y `copias {declaradas, copiadas}` (las vistas con `materialized`, y las que tienen
informe `copiada` | `al-dia`). `la-copia-se-decide.sh` lo afirma: 0/0 de partida, 2/0 tras
decidir dos, 2/1 con un informe, y decidir copias sueltas **no cambia la clase** (sigue
`foreign`: la clase es la regla, la copia es la consecuencia). La consola: la ficha de la base
dice «Foreign database» o «Database» en la cabecera y en *Type*; las del banco y las creadas en
local, sin clase, se pintan como database.

**I4b, medida al construirla** (2026-09-17) — dos cosas que la gramática y `review` dijeron en
cuanto una base estándar intentó nacer en la prueba de fuego:

| | medido |
|---|---|
| **OOS2021** | `tienda.Customers` es `nature: entity` y **una copia de una tabla que sólo anexa no puede respaldarla**: sin clave, los borrados nunca viajan y la copia sería el histórico con las filas viejas dentro. Es `olist` en `demo` (8 tablas sin PK → `append`). La regla de la base —«todo lo que entre se copia»— y la de la gramática —«una copia que respalda una entidad necesita identidad»— se encuentran en un sitio: **la base estándar copia cada tabla en cuanto tiene clave** (del catálogo, o contestada en la decisión `clave`), y hasta entonces la vista espera. No es una tercera clase: es la regla aplicada a lo que se sabe |
| **`review` vuelve a inducir** | `ore review` no edita: **re-induce el paquete entero desde el catálogo y las respuestas** (`revision.rs`: «una edición a mano entre `discover` y `review` se pierde»; `GOBERNADOS = entities, bindings, concepts, tables, views`). Así que el `materialized` que el verbo de I2 escribe en la vista y el `changes: upsert, key` en la tabla **desaparecen en la siguiente decisión contestada** — y `demo/olist` tiene 17 abiertas. El verbo de I2, tal como está, es frágil por construcción. `discover.scope.json` no se reescribe (se aplica): `type` sobrevive |
| **el dueño** | un paquete recién inducido lleva `owner: cambiame` a propósito hasta que se conteste `dueno` (OOS2009): «compila o nada» rechazaría toda copia sobre `olist` hoy. La regla honesta ya existe en `documentos.rs` (la escribió Forge): **la decisión no añade un diagnóstico, o nada** — `validate` antes y después, lo que ya estaba roto se dice |

**⇒ La copia se induce, no se edita.** Es la misma economía que `review`: lo que sale del
paquete es siempre `inducir(catálogo, alcance, respuestas)`, y la copia entra por ahí:

| | qué |
|---|---|
| el alcance | `discover.scope.json` lleva `"type": "standard"` (ya): la **regla** |
| el inductor | con `type: standard`, cada vista trivial cuya tabla **tiene clave** —`primaryKey` del catálogo, o la decisión `clave` contestada (`clave_de(t, dec)`, que ya funde las dos)— sale con `materialized {datasource, table: "copia.<v>"}` y su tabla con `changes: mode: upsert, key: [...]` (lo que el verbo de I2 escribía a mano). Sin clave, la vista espera y la decisión `clave` de la cola dice que **la copia espera esta clave**. No es proponer una copia (lo que el inductor se niega a inventar): es aplicar una regla que alguien declaró |
| `ore-serve` | `POST /paquetes {type: standard}` = escribir el alcance con la clase + `discover --from --type standard` + autorizar `materialization.payload` en `conduits.yaml` (fuera del paquete: no se re-induce) + no empeora. `POST /paquetes/{n}/copia` = la clase al alcance + `ore review` sin respuestas nuevas (re-induce con la regla). Contestar `clave` en una base estándar (`POST …/decisiones`) → `review` → la copia aparece sola. El verbo por vista de I2 se retira: la unidad de decisión es la base |
| el Job | igual (I3): copia lo que el árbol declara. El convergedor y la cola, igual |
| la consola | la ficha: «Standard · 3/8 copied · 5 waiting for a key» y la decisión `clave` en la cola de revisión es el camino |

Lo que se tira: `decidir_copia` por vista (I2) y `hacer_estandar` escribiendo vistas a mano
(el primer intento de I4b, sin commit). Lo que se queda de I2/I4a: el bucket, el conducto
autorizado desde `ore-serve`, `clase_de`/`copias_de`, `GET /paquetes {type, copias}`.

**P1 I4b hecha** (2026-09-17, `f7580aa`). El alcance lleva la regla (`"type": "standard"`;
ausente = `foreign`); el inductor la aplica (`inducir_con_regla`): la vista de cada tabla **con
clave** sale con `materialized` y su tabla en `upsert` por esa clave; sin clave, espera y la
decisión `clave` lo dice. `ore discover --type`, `ore review --reinducir`. `ore-serve`: el alta
pasa `--type`; `POST /paquetes/{n}/copia` asciende; y tras cada inducción (`tras_inducir`) el
conducto y el Job. Dos cosas más que la gramática dijo al construirlo y se resolvieron por diseño,
no por excepción: **`materialization.payload: {}` es ⊥ —sólo `STABLE`— y una vista inducida es
`DRAFT` (OOS4002)**, así que el conducto admite `oos.maturity: DRAFT` (la copia es el registro
del inquilino, no una superficie de consumo; los retículos propios siguen siendo decisión de
alguien); y **`conduits.yaml` no nace hasta que el paquete tenga dueño** —con `cambiame` en la
raíz del árbol no lo re-induciría nadie—: la pasada de decisiones lo trae. La prueba de fuego
(0–6) fija lo que importa: contestar `clave` y `dueno` en una base estándar **trae la copia de
customers y conserva la de orders** —lo que el verbo a mano perdía— y el árbol compila. El verbo
por vista se retira. `GET /copias` sigue: es lo que el Job va a copiar, con el informe.

**El dueño es la organización (18 de septiembre).** En `victor`, una base recién creada desde la
consola no corría (`Run` → OOS2009: `owner: cambiame`) y su copia no se encolaba. La consola lo
enseñaba y no lo dejaba resolver, y las dos salidas que se probaron —un campo *Owner* en el modal
y un «Set owner & run» en el panel— eran la misma cosa: pedirle a la persona **que se invente una
cadena**. La raíz es una desconexión entre dos mundos: `owner` en OOS es «quién responde» como
**handle de forja** (`team:x`, se resuelve contra CODEOWNERS, y de él heredan las políticas), y la
CLI no lo deriva porque no sabe quién la ejecuta; en la plataforma no hay CODEOWNERS, ni equipos,
ni handles: identidad en Keycloak, pertenencia en `ore-iam`, y **el inquilino es el repositorio
de la organización** (0022) que nadie edita a mano. La respuesta, en la plataforma, es un hecho y
no una decisión: **el árbol es de la organización**, y `ore-serve` ya corre con `--organizacion`.
Así que el alta contesta `dueno` con `team:<organización>` —`discover --owner`, que entra por
`Decisiones` y se guarda en `discover.answers.json` para que `review` no lo devuelva a
`cambiame`— y la base nace compilando, con el conducto y la copia encolada. Quién **pulsó** ya va
en el commit (`sub` + `act`); quién **responde** es la organización; nada se inventa. La doctrina
de la CLI no cambia (`owner` se pregunta, no se deriva: lo contesta quien llama, que aquí sí
sabe). Transferir la propiedad a un equipo, cuando IAM los tenga, es contestar `dueno` otra vez
(`la-copia-se-decide` 3), y el conducto **no** sigue al paquete: es la política del inquilino, no
del paquete. Si el nombre de la organización no puede ser un handle, se vuelve a lo de antes
(`cambiame` y la decisión en la cola), y la respuesta del alta lo dice en `owner`.

**El catálogo de la conexión (18 de septiembre, medido).** *«¿Al crear el source se crean tablas,
vistas o entidades, o sólo la conexión?»*. Las dos cosas, y la segunda era la inducción del 30 de
agosto: el Job de catálogo (`44-el-catalogo.yaml`) hacía `ore discover` entero sobre el catálogo —
por fuente de 48 tablas, **48 Table + 48 View + 48 Entity, 35 decisiones de modelado que nadie
pidió (`clave` 8, `concepto` 15, `relacion` 11, `dueno` 1), `owner: cambiame`, y un paquete que no
compila** (OOS2009, OOS2010 ×8)—. C1 decidió que el catálogo no modela y se aplicó a las databases,
no a la fuente. `medida-el-paquete-de-la-fuente.py` sobre los árboles reales: en demo los paquetes
de fuente eran el **93 % de los ficheros y el 84 % de los bytes** del árbol, y `ore validate` —que
corre en cada Save, cada Run y cada alta— **pasa de 3,06 s a 0,07 s sin ellos** (56 diagnósticos →
9, todos de `olist`). Y nadie los usaba: la database se induce **del catálogo** (`POST /paquetes`
→ `discover --from`), y de `tables/` sólo leía `GET /paquetes/{n}/esquema` (la ficha de la
conexión, el modal de nueva database), que trae lo mismo que el catálogo (objeto, columnas,
`sourceType`, claves). Decisión: **tres actos, tres cosas** — *conectar* (`ore source add`: la
conexión en `ontology.config.yaml`, la credencial en el cofre), *catalogar* (el Job deja
`packages/<fuente>/package.yaml` con `owner: team:<organización>` y `discover.catalog.json`, y
**nada gobernado**: compila desde que nace), *modelar* (al crear una database nacen Tables y
Views; la Entity, tabla a tabla, al promoverla). `GET /esquema` lee el catálogo cuando no hay
`tables/`, con la misma forma (`la-copia-se-decide` 0b). Los paquetes de fuente que ya existían
en demo y victor se dejaron en manifiesto + catálogo con un commit por forja: eran prescindibles,
y los catálogos se conservan para crear databases sin recatalogar. Y el inverso del alta, servido:
**`DELETE /fuentes/{n}`** —la conexión fuera del manifiesto (`ore source remove`, hermético), su
catálogo fuera del árbol, su Job fuera de la cola; 409 con la lista mientras alguna `Table` la
nombre como `datasource`; y la credencial **dicha**: sigue en el custodio, porque el cofre no
tiene baja y darla es un acto suyo con su huella (`la-copia-se-decide` 10). En la consola, el ⋮ de
la ficha de la conexión.

**Los dos huecos de la baja, medidos y cerrados (18 de septiembre,
`medida-los-huecos-de-la-baja.py`).** *Copias de nadie:* retirar una base dejaba sus recibos en
`copias/` y sus objetos en el bucket, y `recoger` no los veía (busca superadas BAJO un plan
vigente). En demo: 1 recibo de un plan sin vista y 2 artefactos sin recibo; poco hoy, y para
siempre. Cierre: `DELETE /paquetes/{n}` retira `copias/<n>_*.json` en el mismo commit y **encola la
pasada de la copia aunque no quede ninguna vista** (`VISTAS=""`), porque esa pasada es la que
limpia: `ore materialize --recoger` llama a `ore-store recoger-huerfanas` con los planes de
**todas** las vistas con copia del árbol (con o sin `--vista`) y el almacén borra recibo y
artefacto de lo que ningún plan reclama, más los artefactos sin recibo; y con `--informe` retira
los informes de vistas que ya no están (`la-pregunta` 9, `la-copia` 10). *Credenciales de nadie:*
19 fuentes retiradas en demo (cota superior) y 2 en victor dejaron su `fuente-<n>` vivo en el
custodio. Cierre: la `037` da la potestad `secreto:retirar` (a los mismos roles que emiten) y
`iam.revocar_de_secreto`, gemela de la `021`; el cofre gana `DELETE /organizaciones/{org}/secretos/{n}`
— puede el `owner` (quien lo emitió) o quien tenga la potestad, nunca un agente; la fila queda con
`retirado_en` y quien (la `020` lo previó: «no un delete»), las concesiones revocadas con fecha, el
material fuera del almacén de la celda, y la huella sin el valor (`el-cofre` 10). Y `DELETE
/fuentes/{n}` lo pide con el testigo de quien pulsa, como el alta: la respuesta dice si la
credencial salió o por qué no. *Y lo que quedó de antes* —fuentes que ya no están en ningún
manifiesto, inalcanzables desde la consola— lo retira un verbo de operador que corre en el
inquilino como `mudar`: `ore-cofre retirar-huerfanos --declaradas <las de hoy>` (`--seco` primero),
con su propia atribución (`038`: `retiro_agente`, `revoco_agente`), nunca una persona que no lo
hizo (`el-cofre` 11).

**W1 contesta, y lo que hace que una base nueva parezca rota durante minutos, medido (18 de
septiembre, `medida-lo-que-parece-roto.py`, victor y demo).** `standard_postgre_3.products` → 20
filas, 22 columnas, 1,6 s desde la copia, en la consola: el criterio de W1 (0030 ⑤). Pero entre
«alta de una base» y ese *Run* pasaron **553 s** (y 568 en la anterior, 184 en la que sólo tenía 3
tablas) en los que `ask` contestaba 409 «not made yet» con un botón de copiar. Cuatro causas, cada
una con su medida:

| | medido | qué lo cierra (siguiente iteración) |
|---|---|---|
| **el aviso** | la forja del inquilino **no avisa a Flux desde que la cola vive en ella (0024 E3-(c), 14 de septiembre)**, por tres cosas a la vez: ① `allow-webhooks` (17-el-aviso) sólo deja entrar al namespace `forja`, así que `t-demo` entrega 142 veces con «context deadline exceeded» y 0 aciertos (la de la plataforma: 36/36); ② el aprovisionador no baja `receptor-url` a `/puesto` (baja `iam-url`, `forja-admin`, `idp-admin`, `aprovisionador-secreto`), `RECEPTOR=""`, y `POST /orgs/t-victor/hooks` da **422 cada 5 minutos**, que el guion marca ✓ por idempotencia: `t-victor` tiene 0 hooks; ③ cuando la URL sí llegó (a mano), el guion creó **5 hooks iguales** en `t-demo`, uno por pasada. Sin aviso, un empujón a la cola tarda en ser artefacto **130 s de media, 254 máx** (21 commits en 6 h) | ① la entrada al receptor desde los namespaces `ore.dev/rol: cargas` **y sólo el pod `ore.dev/rol: forja`** (el argumento de P4 se conserva: un pod de un inquilino no puede disparar nada); ② el aprovisionador baja `receptor-url`; ③ antes de crear, `GET /orgs/{o}/hooks` y sólo si ninguno apunta al receptor — y un 422 del hook deja de ser ✓ |
| **el frío** | `jobs-p` es `e2-standard-4` **a demanda** (no spot), mín 0, máx 3, perfil `OPTIMIZE_UTILIZATION`: el nodo se va en cuanto sobra. `copiar-ef5beced`: **102 s** desde crear el Job hasta que su contenedor trabaja (nodo 58 s · imagen 20 s + init 22 s), y el trabajo, 72 s. El catálogo pidió nodo a las 18:42 y la copia otra vez a las 18:52: **cada acto paga el frío** porque el nodo ya se había ido | lo que no cuesta: perfil `BALANCED` (el nodo se queda ~10 min: catálogo y copia de la misma alta pagan uno); lo que cuesta ~100 €/mes: mín 1 en `jobs-p`. Se decide con número, no aquí |
| **el panel** | ventana media **435 s** por alta (3 con informe) en la que la consola dice «not made yet» + *Copy*, mientras el Job está en la cola o corriendo; el 18:01 el botón se pulsó en esa ventana y encoló un `rehacer` que releyó Neon. `ore-serve` ya distingue tres estados para una FUENTE (`catalogada`/`encolada`/`pendiente`, clonando la cola) y **para una vista no mira la cola**: el 409 sale de `ore ask` tal cual | el 409 de `ask` gana `estado`: `encolada` (hay `48-la-copia*.yaml` en la cola que la nombra, con la fecha del commit) · `fallida` (el informe dice `error` y su `motivo`) · `pendiente`; el panel pinta «copying since 18:46 · Job 48-la-copia» sin botón, y *Copy* sólo en `fallida`/`pendiente` |
| **el bundle** | la cabecera del recibo lleva `bundle` = SHA-256(**árbol entero** ‖ versión OOS ‖ lock). En victor, **16 de 19 commits** lo cambian —cada alta, catálogo o retirada de *cualquier* base, y hasta `ontology.config.yaml`—; sólo los `copias/*.json` lo dejan igual. Cada uno deja sin recibo a **todas** las vistas del árbol: `postgre_standard` (10 tablas) se releyó de Neon en las 3 pasadas del día sin que nada suyo cambiara —eso fue la cuota—, y el testigo en Neon es `none` (sin `wal_level=logical`), así que el bundle era lo **único** que cambiaba. Y la versión de OOS dentro significa que **cada release relee todos los orígenes de todos los inquilinos** | fuera de la cabecera: `plan` + `esquema` + `clave` + `testigo` ya nombran lo que se copia y de dónde; el bundle va al **cuerpo** del recibo como procedencia (qué árbol lo pidió), no a la llave. `verificar`/`propuesta` siguen comparando el bundle donde lo comparan hoy |

Y de paso: el árbol de `demo` **no compila** desde OOS2009 (`olist` con `owner: cambiame`, más 8
OOS2010): `materialize` lo salta paquete a paquete, así que sus copias siguen, pero cualquier
medida sobre demo con `ore compile` sale vacía hasta que se le dé dueño (`discover --owner`).

**La pregunta del 17 de septiembre, medida** — *«¿por qué se genera una Entity desde la ingesta,
si eso es la abstracción ontológica? El Assets catalog no es la ontología; ¿por qué pedimos
clave obligatoria?»*. Tiene razón, y la medida dice de dónde viene la conflación:

| | medido |
|---|---|
| **de dónde viene** | `ore discover` nació el 30 de agosto (`1cfa284`) como **inducir la ontología desde un catálogo**: por cada tabla emite `Table` + `View` trivial + **`Entity`** (`nature: entity`, `backedBy`), y su cola de decisiones es de modelado. El Assets catalog (database › schema › table) nació el 10 de septiembre (`4066005`, la consola) **encima** de eso, y hereda las tres cosas sin haberlas pedido |
| **qué decide cada clase** | de las 10 clases del inductor, **3 son del objeto físico**: `dueno` (quién responde del paquete), `filas` (cero filas: ¿viva o resto?), `vista` (el origen la declara vista). **7 son de modelado**: `colision` (dos tablas dan la misma entidad), `clave` (la identidad de la fila), `tipo` (el tipo OOS de una columna — la `Table` guarda el `physicalType` sin preguntar), `vacio` (sin columna tipable no hay entidad), `concepto`, `relacion`, `familia`, `clasificacion`. En `demo/olist`, 16 de las 17 abiertas son de modelado |
| **quién consume la Entity** | el SDL de GraphQL y la exportación (`exporta.rs`), `diff`/`promote`, la Forge (`/documentos/Entity`), y **`GET /paquetes/{n}/esquema`**: la consola construye hoy el árbol del catálogo **desde `entities/`** (`backedBy → view → table → object`), no desde `tables/`. Es el único sitio donde el catálogo depende de la ontología, y es al revés de lo que debería: el catálogo físico tiene más que la entidad (todas las columnas, con su `physicalType`, tipables o no) |
| **qué pide clave** | sólo la cadena copia → View → **Entity** (OOS2021: una copia que sólo anexa no respalda una entidad mutable). Una `Table` + `View` sin entidad **compila y se copia sin clave** (medido en I2: el fixture sin entidades pasó). La copia sin clave es un *snapshot*: se lee entera y se sustituye, los borrados salen solos — lo que `materialize` hace hoy porque ningún lector sirve el rango. La clave sólo compra el refresco por diferencia |
| **Foundry / Databricks** | el mismo reparto: dataset sin clave, *object type* con `primaryKey`; bronze sin clave, silver con `KEYS`. La clave se pide **al modelar**, no al ingerir |
| **qué pasa con lo que ya hay** | `review` re-induce `entities/` (`GOBERNADOS`): si el catálogo deja de emitir entidades, una re-inducción de `olist` **las borraría**. Qué tablas están modeladas tiene que ser una regla en el alcance, como la clase de la base |

**⇒ El catálogo no modela.** Tres iteraciones, y van **antes** de I4c (que pintaría «waiting for a
key», que en este modelo no existe):

| | qué | acepta |
|---|---|---|
| **C1** · `ore-cli` | `discover` induce **en dos mitades**: el catálogo (`Table` + `View` por tabla del `only`; decisiones `dueno`, `filas`, `vista`) siempre; la ontología (`Entity` y sus 7 decisiones) sólo para las tablas que el alcance nombra en **`"entities": [...]`**. Ausente = todas (lo que era: `olist` no cambia y `review` no borra nada); el alta del catálogo escribe `[]`. Un verbo para modelar una tabla —`ore model <paquete> <tabla>`: la añade al alcance y re-induce— que es lo que Foundry llama *promote to object type*. Con `type: standard`, **todas** las vistas salen con `materialized`, con o sin clave; la clave (del origen o de `clave`) sigue poniendo la tabla en `upsert` — mejora, no requisito | una base nace sin entidades y con sus N copias; `ore model` trae la entidad y su cola; `review` conserva lo modelado |
| **C2** · `ore-serve` | `GET /paquetes/{n}/esquema` lee **`tables/`** (todas las columnas, `physicalType`, `object`) y no `entities/`; `decisionesPendientes` cuenta las del catálogo; `POST /paquetes/{n}/tablas/{t}/modelar` → `ore model` (el sitio de la Forge para «promote»); `GET /paquetes` gana `modeladas` | la ficha del catálogo enseña el esquema físico; la Forge ve la entidad al ascender |
| **C3** · consola | el Assets catalog pinta el esquema físico y **Model this table** (→ Forge); la cola de decisiones del catálogo se queda con las 3 físicas y la Forge gana las 7 de modelado | banco con una base sin modelar y otra modelada |

Lo que hay que coordinar: `Entity`, `/documentos/Entity` y las decisiones de modelado son terreno de la
sesión de Ontology Forge — C2/C3 se acuerdan con ella antes de tocar `documentos.rs`. Lo que se
guarda de I4b: todo — la regla en el alcance, `inducir_con_regla`, `--reinducir`, `tras_inducir`;
sólo cambia que la copia deja de esperar a la clave. Después: I4c e I5.

**P1 C1 hecha** (2026-09-17, `4d0b90b`). Corrección a la tabla de arriba: `vista` («el origen la
declara vista: ¿la entidad o un informe sobre ella?») es de modelado, no física — quedan **2
físicas** (`dueno`, `filas`) y **8 de modelado**. El alcance gana `"entities": [...]` (ausente =
todas: `olist` en `demo` no cambia y `review` no borra nada; `[]` = ninguna, lo que el alta
escribe). El inductor parte el catálogo (`Regla {estandar, modeladas}`): la tabla sin modelar da
`Table` + `View` —el nombre que tendría su entidad, o el físico entero si colisiona, sin
preguntar— y sólo `filas`; con la base estándar **se copia sin esperar a nada** (clave del origen
→ `upsert`; sin ella, como el origen la dijo: instantánea). La modelada sin clave sigue esperando
(OOS2021), y es coherente: modelar es pedir identidad, como en Foundry. `ore discover
--no-model`/`--model`, `ore model <paquete> <objeto>`; el alta de `ore-serve` pasa `--no-model`.
La prueba de fuego: `tienda` estándar nace con **0 entidades y las dos copias**, sólo `dueno` en la
cola; `ore model customers` trae su entidad y su clave y la copia de customers pasa a esperar;
contestada, vuelve en `upsert` y compila. Lo que queda: C2 (`/esquema` desde `tables/`, el verbo
de modelar en `ore-serve`, `modeladas` en `GET /paquetes`) y C3 (consola) — con la Forge.

**P1 C2 hecha** (2026-09-17, `857f2df`). `GET /paquetes/{n}/esquema` trae **`tables`** desde
`tables/` —objeto, fuente, columnas con el `physicalType` del origen, la vista que la expone,
`modeled` y con qué entidad— y deja `entities` hasta que la consola lea `tables` (C3).
`POST /paquetes/{n}/tablas/{objeto}/modelar` → `ore model` (201 con `copias`/`encolado`; 409 si ya
lo está; 404 fuera del alcance); `model` entra en la lista de verbos herméticos (no consulta a
nadie: es `review`). `GET /paquetes` dice `tablas` y `modeladas`. Prueba de fuego 0–6.

**P1 C3, lo pedido** (2026-09-17, consola `9642d5d`, local): el árbol de Assets se construye
desde `tables` —esquema físico › tabla › columnas con el tipo del origen— y no desde
`entities`; la ficha de la tabla enseña las columnas de ORE con su `physicalType` y dice «—»
donde el catálogo no sabe. Clippy en CI (`descubrir` con 8 argumentos) arreglado en `084ead1`;
CI verde y `demo` desplegado.

**Decidido el 17 de septiembre: modelar no se pulsa desde la tabla.** El Assets catalog no modela,
así que tampoco lleva el botón: modelar es un acto de la ontología y nace en **Ontology Forge ›
Entities › New entity**, eligiendo la tabla del catálogo que la respalda — como Foundry crea el
*object type* desde la Ontology Manager eligiendo el dataset. El verbo ya está (`POST
/paquetes/{n}/tablas/{objeto}/modelar`, 201 con `copias`/`encolado`); quien lo pulsa es la
Forge, y es de su sesión: que la respuesta diga que las decisiones de modelado (clave,
relaciones, conceptos) van a la cola, y que en una base estándar la copia de esa tabla pasa a
esperar la clave. La ficha de la tabla en Assets sólo lo **dice** («Modeled as `Customers`», con
enlace), no lo hace. Queda de C3 en Assets, para cuando se pida: el modal mandando `type`, la
ficha de la base estándar («N/N copied · last copy») y **Copy into this cluster** sólo en la
foránea (dónde viven los datos sí es del catálogo). Y después, I5.

**Copy into this cluster, por tabla** (2026-09-17, ORE `c11195d`; consola `dff6dc8`, local).
Pedido en la ficha de la **tabla**, sólo para tablas de una foreign database: es la
**excepción a la clase**. El alcance gana `copies` (qué tablas se copian una a una; en una
estándar no hace falta y no se escribe) y la `Regla` lo aplica: se copia si la base es
estándar **o** la tabla está en `copies`. `ore copy <paquete> <objeto>`; `POST
/paquetes/{n}/tablas/{objeto}/copiar` (201 con `copias`/`encolado`; 409 si ya se copia o la base
es estándar; 404 fuera del alcance); el esquema dice `copied` por tabla; `tras_inducir` encola
también en una foránea con copias sueltas. Consola: la entrada en *Actions* (modal de
confirmación; 409 aviso, 404 error) y la fila *Storage* («Copied into this cluster» / «At
source»). Prueba de fuego 6.

**P1 I5 hecha** (2026-09-17, `demo`, `pruebas-de-fuego/la-copia-en-demo.py`). Una base estándar
`olist_copia` (3 tablas pequeñas de olist) creada por el API con el agente de la celda en 5 s;
el Job `copiar-822ce320` en `t-demo` (Flux → Kueue → pod en `jobs-p`, ~2 min de arranque):
cofre → origen → `ore-store-gcs` → **el bucket del inquilino**, con CMEK. Resultado:
`product_category_name_translation` 71 filas · 3 608 B; `products` 32 951 · 1 165 659 B; `sellers`
3 095 · 119 439 B; 3 artefactos Parquet + 3 recibos; el informe empujado al árbol (`2c2fa93`,
`copiador`); `GET /paquetes/olist_copia/copias` lo dice y `GET /paquetes` cuenta 3/3. **Lo que
destapó, y se arregló sobre la marcha** (cada uno un commit en `main`): `materialize` validaba el
árbol entero y otra base con `dueno` abierto (OOS2009) bloqueaba ésta → sólo su paquete y la
raíz, y un paquete roto se lleva sólo sus vistas (`eaeae41`, `5735521`); el error del almacén
tragaba su stderr (`9c4d51e`); **un NULL viajaba como `""`** en el protocolo del driver desde M4
y un `Integer` nulo no podía cargarse → un nulo es la propiedad ausente (`fc66a35`); el Job se
encolaba antes de que el conducto existiera (`2d1690e`); el convergedor borraba de la cola lo
que `ore-serve` acababa de encolar (`10a4035`); y en `victor`, `test-standard` con guion no
puede ser un espacio de nombres (OOS2030) → el alta y `discover` lo rechazan, y `DELETE
/paquetes/{n}` retira una base (`5735521`); y retirarla dio 422 «el árbol empeora» por los
`OOS2009`/`OOS2010` de las otras dos bases: el validador va por fases y se para en la primera
(`validate_package`), así que `test-standard` (OOS2030, pertenencia) TAPABA lo que las otras
tenían en el enlazado, y quitarla lo destapó → **retirar sólo empeora por lo que nombra a la
base retirada** (`empeora_salvo`, `la-copia-se-decide` 7b). **Lo que I5 mide y no cierra**: la segunda pasada en
Postgres **relee** (el LSN se mueve entre pasadas: cualquier escritura del servidor) — el recibo
funciona, pero «releer no lee el origen» espera al lector del rango del changelog
(`medida-el-rango-por-posicion.py` §D); hoy el refresco en Postgres es proporcional al tamaño.

**Lo que se aparca:** E4 (endpoint público) y E5 (dedicado) van después de P1–P4 — nadie de fuera
necesita llamar a un modelo que todavía no corre sobre datos—; B2 (`bastion certify`) es precio,
no capacidad, y espera; B4 (digest) con B2.

### E4 · Lo público es el gateway

La pestaña *Endpoint* enseña la URL y las claves por inquilino que B3 emite, y el uso.
**Acepta:** un `curl` desde fuera con la clave contesta; sin clave, 401; una clave de otra
organización, 401; el uso aparece en la fila.

### E5 · El tier dedicado

`tier: dedicated`: hoy una VM G4 por celda lanzada con el perfil (B1, apagado verificado,
`bastion status` como estado); con cuota bajo demanda y pool `gpu` en GKE, el ③ entero —
`modelos.git`, `desplegar-<celda>`, la plantilla y su comprobación en `gen-inquilino.py`,
`requests.nvidia.com/gpu` en la cuota, el informador con `despliegues`—. **Acepta:** un
`Model` dedicado arranca sin que nadie toque `kubectl`, sirve sólo a su celda, y retirarlo
apaga la máquina y el proveedor lo confirma.

---

## Lo que este abordaje NO hace, y por qué

- **No mide un modelo en CPU en la celda.** La primera versión de este ADR lo proponía como E0;
  mediría un sustrato que el producto no ofrece (0028). Se retira.
- **No pone una tabla de despliegues en `ore-iam`.** Sería el plano de control poseyendo lo que
  el árbol declara. `iam` guarda quién puede.
- **No ensancha `cola-<celda>`.** Cuando el dedicado viva en GKE, otra cuenta con otro verbo.
- **No deja a `ore-serve` inventar una configuración.** Nombra un perfil; el perfil lo certifica
  quien lo mide. Ni una máquina, ni un motor, ni un flag salen del árbol.
- **No construye una puerta pública propia.** Es B3.
- **No mete modelos propietarios por API.** Eso no es un `Model` del árbol: es una
  **conexión**, con clave del cliente y una excepción de salida explícita —«los datos salen del
  clúster»—, y va en otro ADR cuando *Training* exista y el caso maestro/obrero lo pida.
- **No decide escalar a cero, MIG ni entrenamiento.** Se nombran como huecos con su sitio.
