# 0027 · Model Serving

**Estado:** **aceptado · en vivo** (el tier compartido, desde 2026-09-16) · **Decide:** cómo se
**sirven y se distribuyen** los modelos en la plataforma: un modelo lo sirve una máquina con **un
perfil certificado**, una **puerta** (el gateway) lo reparte entre las celdas con identidad, cuotas,
contabilidad y soberanía, y el árbol de cada celda dice **qué modelos puede llamar**. Dónde vive el
documento del modelo y su identidad es [`0041`](0041-model-registry.md) (Model Registry); el
sustrato que sirve es Bastion ([`0028`](0028-bastion-es-el-producto-sobre-el-stack.md)); quién lo
llama, [`0029`](0029-donde-corre-una-funcion.md) y [`0050`](0050-functions-in-code-repositories.md).

## Qué es

Model Serving lleva un modelo **de un perfil medido a las celdas que lo usan**. Un modelo no es un
servicio al lado de los datos: es un operador dentro del grafo. Una `Function` lo nombra
(`modelo/<n>`), no una URL; lo que escribe aterriza en la ontología; y el gobierno ve qué vistas lo
alimentan y qué produce.

Tres decisiones lo definen:
- **El árbol nombra un perfil, nunca una configuración.** Ni runtime, ni recursos, ni argumentos de
  motor: eso es del perfil, y el perfil lo certifica quien lo mide.
- **Hay una puerta, y es una.** Ninguna celda alcanza una máquina de modelo: todo pasa por el
  gateway, que sabe quién llama, qué puede llamar y cuánto gasta.
- **El plano de control observa y no posee.** El árbol declara, la puerta sirve y cuenta, y el
  estado se cruza al leerlo. No hay una tabla de despliegues aparte que pueda discrepar.

## Las piezas

| pieza | qué es | dónde |
|---|---|---|
| **el documento** | `kind: Model` con `profile`, `digest`, `tier`, `task` y `owner` | el árbol, en `base.schema.nombre` (0041) |
| **el perfil** | una máquina × un modelo × un motor, con los argumentos exactos de `vllm serve` y **los números medidos**: tok/s a 1, 8, 16 y 32 usuarios y TTFT. `bastion certify` lo da por bueno sólo si el banco de pruebas, en la clase de máquina de producción, cumple esos números. **Un perfil sin número no existe** | Bastion B2: `env/profiles/<máquina>/`; publicado en `gs://bastion-perfiles/perfiles.json` |
| **la máquina** | `g1`, `g4` y `g8`: 1, 4 u 8 RTX PRO 6000 de 96 GB, con la imagen `bastion/env` (vLLM fijado). Cada una lleva la **etiqueta de soberanía** de su proveedor: `eu-dc, dpa, kms` en GCP, `community` en Vast (sólo para uso interno). Se apaga sola: apagado verificado, plazo máximo y un vigilante que destruye la máquina que se queda sin latido | Bastion B1 |
| **la puerta** | el gateway: **datos** en `:8000`, la API OpenAI `/v1` (`chat/completions`, `completions`, `embeddings`, `models`), y **control** en `:9000` (`/admin/tenants`, sus modelos y claves, `/admin/backends`, `/admin/usage`, `/admin/health`, `/metrics`) | Bastion B3, en la VM `modelos-e0` (`10.10.0.100`) |
| **el reconciliador** | cada 60 s compara la **demanda** (las suscripciones) con la **oferta** (los backends y las máquinas): enciende una máquina con el perfil que se pide, la retira tras un periodo de gracia, deja enfriar un modelo que falla y no pasa del techo de gasto | Bastion R2, junto al gateway |
| **los verbos** | `POST /modelos` · `GET /modelos` · `GET` y `DELETE /modelos/{ref}` | `ore-serve`, `modelos.rs` |

## De la consola a un modelo servido

1. **El Hub enseña lo certificado**: cada modelo con su máquina, sus tok/s por concurrencia, su TTFT,
   su $/M y su $/h. No hay un catálogo de lo que *cabría*, sino la matriz de lo que **se ha medido**.
2. **Deploy** pide la base y el schema, y llama a `POST /modelos`. `ore-serve` comprueba **el encaje
   contra la lista de perfiles**: un perfil que no está es 422 (con los que sí están); un `digest`
   que no es el del perfil, 422; `tier: dedicated`, 422, porque la plataforma sirve el tier
   compartido.
3. **En el mismo acto**, escribe el documento en el árbol —con `owner: user:<handle>` de quien lo da
   de alta ([0052](0052-ownership.md))— **y suscribe la celda** en la puerta
   (`POST /admin/tenants/{celda}/models`). Si la suscripción no se hace, **el documento no se
   escribe**: un `Model` que la puerta no conoce promete algo que no existe (502, el árbol intacto).
4. **El reconciliador ve la demanda** y enciende una máquina con el perfil. Al registrarse, el
   backend sirve y la ficha pasa a `running`. Medido: una suscripción de `victor` a Llama 3.1 8B
   llevó una `g1` de nada a servir en unos 7 minutos, y la primera llamada devolvió 74 tokens en
   0,94 s.
5. **Retirar** (`DELETE /modelos/{ref}`) quita el documento y la suscripción. La suscripción es de
   la celda **por id servido**, así que se queda si otro `Model` del árbol sirve el mismo id. Sin
   demanda, el reconciliador retira la máquina tras su gracia.

**El tier compartido** es la multi-tenencia del producto: **un backend por modelo**, de la
plataforma, que comparten todas las celdas que lo nombran. La separación es lógica y la hace la
puerta: cada celda ve y llama sólo lo suyo, con su cuota y su cuenta.

## Quién lo llama

- **Una `Function` de `runtime: model`** (0029): `model: modelo/<n>` y un `prompt` sobre las filas
  de `over`. `POST /funciones/.../invocar` resuelve el modelo a su puerta y su id servido, y encola
  un Job en la cola de la celda (Kueue). El Job toma el token de agente de la celda, lee la copia de
  `over`, hace una llamada por fila a `/chat/completions` y sella el resultado en el lago de la
  celda, con su informe en el árbol. **La salida de un modelo es `untrusted`** mientras nadie la
  endose (`OOS7002`).
- **Una `Function` de Python que declara `models:`** (0050): `ore-serve` resuelve cada modelo al
  invocarla, y el código lo llama con `ore.modelo("<ref>").chat(…)` o `.pide(…)`. Fuera de una
  función que lo declare no hay modelo: `ore.modelo` lo niega, y la red tampoco llega.

## Red e identidad

- **La red:** tres reglas de salida en la celda, y sólo tres:
  - los Jobs (`rol: driver`) al plano de datos (`:8000`);
  - `ore-serve` (`rol: control`) al de control (`:9000`);
  - el Job de una función que declara modelos (`usa-modelo`, que sólo pone `ore-serve`) al de
    datos.

  Por el lado de la VPC, un cortafuegos con nombre (`malla/71-la-red-de-los-modelos.sh`) deja entrar
  a la máquina de la puerta sólo lo que viene de la malla. Un puesto interactivo no llega a un
  modelo.
- **La identidad:** la celda llama con **el token de su agente** (`ore-agente-<celda>`). El realm le
  pone la audiencia `modelos` y la claim `rubix_celda`. La puerta lo verifica contra el JWKS
  (RS256, emisor exacto, `aud: modelos`, `rubix_tipo: agente`) y **la celda es el tenant**. El
  token dice quién; **qué puede llamar lo dice la suscripción**, que vive donde se ejecuta. Una
  celda sin `Model` recibe 401 «not subscribed», y la de al lado también. Verificar el token cuesta
  0,13 ms por petición.

## Lo que se observa

**El estado lo trae quien sirve.** La puerta cuenta por celda, modelo y backend (tokens, peticiones y
$) y sabe qué backends están arriba. `GET /modelos` lo cruza con lo que el árbol declara,
una vez por petición:

| | fase |
|---|---|
| declarado, y un backend arriba sirve su id | `running` |
| declarado, y ninguno arriba | `provisioning` |
| declarado, y la celda no está suscrita | `error` (deriva) |
| suscrito, y sin documento | `retiring` |
| la puerta no contesta | `error`, con el motivo; la ficha sale igual |

**Deployments** pinta esa fila: estado, modelo y perfil, máquina, réplicas, uso de hoy y del mes
(peticiones · tokens · $), el endpoint interno y quién lo pidió y cuándo (del commit). La consola
nunca espera a la puerta ni inventa una píldora.

## Las reglas

- **Soberanía:** la etiqueta viaja con la máquina y se comprueba **en cada llamada**. Una celda
  `eu-dc` nunca se enruta a una máquina `community`: si no hay capacidad soberana, 503 «no sovereign
  capacity». Nunca una degradación silenciosa.
- **Cuotas:** concurrencia y tokens por minuto (429) y presupuesto mensual (402), por celda.
- **Los pesos no pasan por el árbol.** El árbol lleva el `digest`, la máquina trae y cachea los
  bytes, y **lo que corre es lo que el árbol dice, o no corre**. `ore-serve` no toca un byte de
  pesos.
- **El encaje se decide en el servidor**, contra la lista publicada. La consola no inventa el
  encaje: lo pinta.

## El precio

**$/M = $/h de la máquina ÷ tok/s del perfil a 32 usuarios.** Por ejemplo, Qwen3-235B FP8 en una
`g4`: 5,87 $/h ÷ 587 tok/s = **2,78 $/M**; DeepSeek-V2-Lite en una `g1`, 0,28 $/M. La puerta cuenta
cada llamada por celda, en tokens y en $, y suma el mes.

## Lo que Model Serving no es

- **Un modelo en CPU dentro de la cuota de la celda.** Lo que no tiene perfil en una máquina
  certificada no se sirve.
- **Una tabla de despliegues en `ore-iam`.** `iam` guarda quién puede, no lo que el árbol declara.
- **Una configuración inventada por `ore-serve`.** Nombra un perfil, y el perfil lo certifica quien
  lo mide.
- **Una segunda puerta.** Lo público —una URL y claves para llamar desde fuera— es la misma puerta
  (claves `bk_` por tenant).
- **Un modelo propietario por API.** Eso es una conexión, con la clave del cliente y una salida de
  datos explícita: otro producto.

## Aceptación

`los-modelos.sh` (los verbos, el encaje, las cuatro fases, el uso y el autor, contra una puerta de
banco), `la-celda-llega-al-gateway.py` (la celda llama y la de al lado no; retirar es 401),
`deployments-tiene-filas.py` (de la consola a la fila, con su autor) y `la-invocacion-se-decide.sh`
(una función sobre una copia, de punta a punta). En Bastion, las pruebas del gateway (identidad,
cuotas, soberanía; el 99,3–99,8 % de los tok/s directos a 32 usuarios) y la medida del reconciliador
(R1).

## Historia

E0–E3 se hicieron el 16 de septiembre de 2026 (la celda alcanza el modelo, el documento, la
puerta sin tocar nada a mano, Deployments con filas). Hasta el 3 de octubre este documento se
llamaba «El modelo vive en el árbol» y llevaba además el cuaderno de la copia en la celda (P1, que
hoy deciden 0015, 0033 y 0045) y la regla del dueño (hoy [0052](0052-ownership.md)). Lo que fue,
en `git log --follow -- docs/decisions/0027-model-serving.md`.
