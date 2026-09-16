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
