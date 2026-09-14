# 0024 · Dónde corre el inquilino

**Estado:** aceptado (2026-09-14, con la E3 hecha y medida) · **Fecha:** 2026-09-13 · **Decide:** que el plano del árbol de un
inquilino puede correr en **tres sitios** bajo **un solo plano de control**; que el modelo de IA
va **con el árbol, en el mismo clúster y en otro pool**; que la ingesta de terceros aterriza
**en el plano de datos del inquilino y nunca en el de control**; y que **el clúster pasa a ser un
hecho del plano de control** — la consola lo emite desde ahí, no desde el árbol. Y desde la E3:
que **el material de un secreto vive en la celda** y **la puerta del inquilino también**; el plano
de control guarda quién puede y tiene **una** puerta, sólo para que tiren de ella

---

## El problema

La plataforma ha integrado un producto cuyo centro de gravedad es **un modelo de IA
autohospedado dentro del perímetro del cliente**, que infiere sobre la ontología y el catálogo.
Para eso hay que darle al cliente un clúster: uno «serverless» dedicado, o el suyo (BYOC). La
consola ya lo ofrece en *Cloud Management / GPU Clusters / Nuevo*.

Y eso abre dos preguntas que no son de infraestructura sino de producto:

1. **¿Hasta dónde tiene que seguirle el árbol?** Si el modelo está en casa del cliente y el árbol
   sobre el que razona está en la nuestra, ¿es soberanía o es la mitad?
2. **¿Modelo y datos en el mismo clúster, o en dos?** Sobre todo cuando, además de punteros, la
   plataforma empiece a **ingerir** datos de terceros.

Y una tercera que sale al mirar la consola: la vista de clústeres pinta **clústeres de mentira**.
Para emitir el de verdad hay que saber **de dónde sale** — y hoy no sale de ningún sitio:
`iam.organizacion` no tiene ni una columna que diga en qué clúster vive. El clúster es implícito:
todo está en `ore-mesh`.

---

## Lo que se miró antes de decidir

**Lo que somos hoy no es serverless: es un clúster compartido, aislado por namespace.**
[`medida-el-coste-del-cluster.py`](../../pruebas-de-fuego/medida-el-coste-del-cluster.py):
un GKE Standard, un nodo spot, y cada inquilino es `t-<n>` con su `ore-serve`, su cofre, sus
`NetworkPolicy` y tres repositorios en **una** forja compartida. Lo único que escala a cero son
los Jobs de catálogo. El aislamiento es correcto para separar inquilinos y **no es un
perímetro**: comparten nodo, kernel, forja e IdP.

**Lo que tenemos de cada cliente es menos de lo que parece.** Punteros, esquema, decisiones y la
credencial cifrada. **Ni una fila**: [`0006`](0006-el-artefacto-de-topologia.md) y
[`0018`](0018-la-ontologia-es-el-sistema-de-registro.md) las dejan en el lago del cliente,
escritas por quien tiene el driver. ⇒ El «plano de datos» ya es del cliente. Lo que tenemos es
el **plano de metadatos** — y un esquema **es** información de negocio: `orders`, `refunds`,
`churn_risk` cuentan la empresa sin una sola fila.

**Lo que hace Redpanda, mirado con cuidado.** Al registrarse te da «un clúster serverless».
La pantalla dice `Topics 1/300 · Partitions 1/5K · Network: Public` y *«live throughput charts
are not available for serverless clusters»*: cuotas fijas, red sin opción, métricas que no
existen para ese tier. Es un **inquilino lógico sobre infraestructura compartida** en su cuenta
de AWS, con nombre de clúster. Es exactamente nuestro `t-demo`. Lo que Redpanda hace bien no es
«un clúster por cliente»: es **la escalera** — Serverless / Dedicated / BYOC — con **un solo
plano de control** para los tres. Sólo se mueve el plano de datos.

**El corte ya está escrito.** En la consola, `lib/server/query.ts`, cada consulta declara su
plano y no hay valor por defecto: `control` (IdP, `ore-iam`, organizaciones, miembros) o `arbol`
(fuentes, paquetes, esquema, estado). Lo que falta no es el corte: es que los dos planos puedan
vivir en sitios distintos.

---

## La decisión

> ### ① Un plano de control; el árbol, en uno de tres sitios.

| tier | dónde corre `arbol` | quién lo opera | aislamiento | suelo por inquilino |
|---|---|---|---|---|
| **compartido** | nuestro clúster, namespace `t-<n>` | nosotros | lógico | ≈ $0 fijo |
| **dedicado** | un clúster nuestro por inquilino | nosotros | físico | ≈ $74/mes de plano de control + lo que corra |
| **BYOC** | la cuenta del cliente | nosotros, por un agente que **tira** | su perímetro | $0 para nosotros |

`control` no se mueve nunca: quién eres, a qué organización perteneces, la facturación, la
consola. Es la misma figura que separa **identidad** de **camino** en el resto del árbol — el
emisor central, la dirección en casa del cliente.

⭐ **El compartido ya existe.** Lo que la consola llama *new cluster paradigm* son el dedicado y
el BYOC. Se puede llamar «serverless» al compartido desde hoy sin mentir: lo que Redpanda vende
con esa palabra es una promesa de operación y facturación —sin nodos, pago por uso—, no una
arquitectura.

> ### ② El árbol va con el modelo. Y el modelo va en el clúster del inquilino, en otro pool — no en otro clúster.

Si el argumento es «el modelo nunca sale de tu perímetro», **el contexto sobre el que razona
tampoco puede salir**. Un modelo soberano preguntando a un `ore-serve` en nuestra casa es
soberanía a medias, y es exactamente la mitad que un cliente que compra soberanía va a mirar.

Y en el **mismo clúster**, porque para separar perfiles de máquina existen los node pools:

```
clúster del inquilino (dedicado o BYOC)
├── pool sistema   ore-serve · cofre · forja · Flux          pequeño, siempre
├── pool datos     Jobs · ingesta · motor de consulta        escala con el volumen
└── pool gpu       el modelo                                 escala con la inferencia, de 0
```

Dos clústeres darían un salto de red en el camino más caliente —modelo ↔ ontología ↔ datos—,
otra cuota de gestión, y dos cosas que importar en BYOC, a cambio de nada que un pool con su
taint no dé. Clúster aparte **sólo** si: otra región u otra nube para el modelo, una exigencia
regulatoria de separación, o una GPU tan efímera que se levanta y se tira entera. Ninguna hoy.

> ### ③ La ingesta de terceros aterriza en el plano de datos del inquilino. `0006` y `0018` se sostienen.

Es la decisión que la ingesta pone a prueba. Si los datos aterrizaran en el plano de control,
ORE pasaría a ser un lago y la historia de soberanía entera se rompería por debajo. Si aterrizan
en el clúster del inquilino —un bucket en **su** cuenta y un motor en el pool de datos—, el
lago del cliente ahora **es** su clúster, y la ingesta es **un driver más que escribe allí**.

⇒ El clúster del inquilino es *árbol + cofre + Jobs + ingesta + modelo*, y el plano de control
sigue **sin tocar una fila**. Es lo que hace que BYOC sea el mismo manifiesto en otro sitio.

> ### ④ El clúster es un hecho del plano de control, y la consola lo emite desde ahí.

Hoy no está escrito en ningún sitio. Para emitirlo hace falta que exista, y el sitio es
`iam`: **`iam.celda`** — de qué organización, qué tier, qué proveedor y región, y en qué
estado. La escribe el aprovisionador al fundar, que es cuando se decide.

⛔ **No se emite desde el árbol**, aunque las fuentes sí. El clúster es lo que **hospeda** a
`ore-serve`; preguntarle a `ore-serve` en qué clúster está es circular — cuando esté caído es
justo cuando la respuesta importa, y no habrá respuesta.

⭐ Pero la figura es la misma que la de las fuentes, en el otro plano: `/fuentes` dice qué se
**declaró** y `/fuentes/{n}/estado` qué está **pasando**. Aquí, `control` dice qué celda tiene
la organización —tier, región, estado administrativo— y **si contesta** se pregunta por el
camino, al `/salud` de su `ore-serve`, desde el servidor de la consola que ya lo alcanza.
Identidad de un plano; vida, del otro. **Ninguno de los dos finge saber lo del otro.**

> ### ⑤ El material de un secreto vive en la celda. El plano de control guarda quién puede.

Tomada en la E3 (2026-09-13), después de medir el cofre y de mirar qué hacen los que viven de
BYOC. La medida (`pruebas-de-fuego/medida-el-cofre-y-su-almacen.py`) dice dónde está hoy cada
pieza de un secreto de `demo`:

```
en claro     sólo en memoria del pod del cofre, EN el inquilino (KMS por stdin/stdout)   ✓
cifrado      en Postgres CENTRAL (`idp-db.identidad`), alcanzado por SQL desde `t-demo`  ✗
llave        en KMS del proyecto de la PLATAFORMA, una por organización (`ore/demo`)      ✗
quién puede  en `iam.concesion`, central — y eso es metadato                              ✓
```

Y lo que hace el sector es unánime y está escrito: Redpanda guarda los secretos estáticos en el
Secret Manager de la nube donde corre el plano de datos y *«never leave the data plane account
or network»*; WarpStream, *«only metadata is transferred from your environment»*. El plano de
control sabe **que** existe y **quién** puede; nunca **qué** es.

⇒ **El material cifrado deja `cofre.material` y pasa al Secret Manager de la celda.** En
`compartido` la celda es nuestro proyecto y el secreto se llama `t-<n>-…`, con acceso sólo para
`ore-cofre-<n>` por Workload Identity — el mismo camino que la `0023` ya abrió para el testigo de
la forja y el agente. En `dedicado` es igual; en `byoc` es **su** proyecto, y la llave (CMEK
sobre el secreto) es suya sin que escribamos una línea: la KEK por organización que hoy aplica
`kms.rs` a mano pasa a ser la clave CMEK del secreto, que es la misma llave en el mismo KMS
puesta donde el Secret Manager la aplica solo.

⛔ **Lo que NO se mueve: `cofre.secreto` e `iam.concesion`.** Nombre, clase, quién lo emitió y
quién puede usarlo son metadato y se quedan en el plano de control. Así `resolver` conserva su
invariante —*«no hay un camino en el que el código sepa que el secreto existe antes de saber si
quien pregunta puede»*—: el `join` de `cofre.secreto` con `iam.concesion_viva` sigue siendo una
consulta y sigue devolviendo nada si no puedes; **sólo después** el cofre va al almacén de la
celda a por el valor. La medida lo cobra: es la única invariante que el traslado toca.

⚠️ Lo que cuesta, medido: 2 tablas + 1 vista, 3 funciones (`emitir`, `listar`, `resolver`), 8
filas de 230 bytes como mucho en `demo`. Y lo que se gana además del principio: el cofre de un
inquilino **deja de necesitar la base central para leer material** — le basta `iam` para
preguntar, que en BYOC será por HTTP y no por SQL (la `020` ya separó los papeles; lo que cambia
es el camino).

⛔ **Lo que yo había propuesto era lo contrario** —«central y cifrado hasta que alguien lo
pida»— y era exactamente lo que ninguno de ellos hace, porque el material cruza al proveedor
aunque sea ilegible. Se escribe para que no vuelva.

> ### ⑥ La puerta vive en la celda. El plano de control tiene una, y es para que tiren de ella.

Son **dos tráficos** y la pregunta abierta los mezclaba:

**Plano de control → plano de datos.** Todos los medidos coinciden: *sólo saliente desde el plano
de datos*. El agente de Redpanda *«pulls its work from the control plane»* con testigos opacos y
efímeros, y el clúster *«remains available even if the network connection to the control plane is
lost»*. **Ese agente ya lo tenemos y se llama Flux**: sondea la forja. Luego el plano de control
necesita **una** puerta pública —la forja, con testigo, como hoy— y la celda **sólo necesita
salida**. Ahí queda resuelta la mitad «¿alcanza Flux la forja desde BYOC?»: sí, porque es la
dirección que cualquier cortafuegos deja pasar.

**Consola y clientes → plano de datos.** Redpanda **no** hace de proxy: hospeda la zona DNS
(`<cluster>.byoc.prd.cloud.redpanda.com`), el registro apunta a un endpoint **en la nube del
cliente**, TLS por Let's Encrypt, pública o privada (PrivateLink/PSC), y el navegador en
`cloud.redpanda.com` habla con el plano de datos directamente. La razón es ③: si el proxy pasa
datos, el proveedor está en el camino del dato **y en el de la disponibilidad**, que es lo que
`0006` y `0018` prohíben.

⇒ **Cada celda tiene su puerta**: Gateway, certificado y `<n>.ore.paladio.io` apuntando a su IP.
La zona es nuestra y **el registro DNS lo escribe el plano de control**, que es quien sabe en qué
celda vive cada organización (`iam.celda`). Y el Gateway por el que hoy llega `demo` **no es «el
compartido de la plataforma»**: es el Gateway de la celda `ore-mesh`, que además hospeda el plano
de control. Para `demo` en la E3 eso es cero movimiento de manifiesto; lo que falta es que
`iam.celda` **diga su puerta** y que la consola construya la dirección de `ore-serve` desde la
celda y no desde un host fijo.

⛔ **Y lo que había propuesto también era lo contrario** —una sola puerta en el plano de control
haciendo de proxy— con el argumento de «un certificado, un DNS». El argumento era de comodidad;
la promesa es de soberanía, y las dos no caben.

---

## Lo que se acepta a cambio

- ⛔ **La forja deja de ser una.** Si el árbol es soberano, su forja también: una por inquilino
  dedicado o BYOC, en su clúster. Dejarla central rompería ② por debajo. Es una `StatefulSet` y
  10 GB, y es N cosas que parchear en vez de una — el precio ya aceptado en `0022`.
- ⚠️ **Flux invierte el sentido, y es lo correcto.** Hoy Flux en nuestro clúster reconcilia el
  compartimento. En dedicado y BYOC, Flux corre **allí** y **tira de nuestra forja**: el
  compartimento sigue siendo nuestro y describe qué corre en su casa. **Nunca necesitamos una
  credencial hacia su clúster** — ellos tiran de nosotros. Es el modelo de agente de todo BYOC
  serio, y es `0022`-① sin cambiar una palabra.
- ⚠️ **Una puerta por celda es un certificado y un registro DNS por celda.** Decidido en ⑥. El
  plano de control escribe el registro; el certificado lo emite la celda (cert-manager, como hoy
  `ore-puerta`). En BYOC privado —sin IP pública— es PSC/PrivateLink, y es E5.
- ⚠️ **El cofre cambia de almacén, no de llave.** Decidido en ⑤: el material va al Secret
  Manager de la celda y la KEK por organización pasa a ser la CMEK del secreto. `kms.rs` deja de
  cifrar a mano. BYOC en otra nube u on-prem sigue exigiendo abstraer el almacén — pero es una
  interfaz de tres verbos, medida, y no se empieza hasta que haya un cliente que lo pida.
- ⚠️ **«Serverless dedicado» no es cero.** GKE Autopilot cobra los pods **y** $74/mes de plano
  de control por clúster; el free tier cubre **uno** por cuenta de facturación, no uno por
  cliente. El suelo por inquilino existe y va en el precio, no en la sorpresa.
- ⚠️ **Tres tiers son tres cosas que probar.** Cada cambio de plantilla se comprueba contra los
  tres, o el tier que nadie usa es el que está roto.

---

## El abordaje, por etapas

La misma propiedad que en `0022`: cada etapa deja el sistema mejor aunque la siguiente no llegue.

**E1 · El clúster de verdad en la consola, y fuera los de mentira.** `iam.celda` con una
migración que la crea y **siembra** `demo` y `prueba` —`ore-mesh`, compartido, `gcp`,
`europe-west1-b`—; el aprovisionador la escribe en las altas nuevas; `GET
/organizaciones/{org}/celdas` en `ore-iam`; la consola lista **eso** y pregunta `/salud` por el
camino para pintar si responde. `MOCK_CLUSTERS` se borra: a diferencia del catálogo, aquí el mock
no convive con lo real — sólo hay **un** clúster por organización y es éste.

⭐ Lo que esta etapa paga por sí sola: el clúster deja de ser implícito. Hoy no hay ningún
sitio donde esté escrito en qué clúster vive `demo`.

**E2 · El compartido se llama por su nombre.** El tier `compartido` en la consola con la
promesa de Redpanda —sin nodos, pago por uso— y sus cuotas dichas (`ResourceQuota` ya existe).

**E3 · Nuestro clúster como primer «clúster de cliente».** Medido primero
(`medida-el-acoplamiento-del-inquilino.py`): de 13 acoplamientos, 11 viajan o se parten solos y
2 había que **decidir** — y son ⑤ y ⑥. Lo que la E3 hace, en este orden: **(a) ✓** `iam.celda`
dice su puerta (`027`), el aprovisionador converge el DNS o dice qué registro falta, y la
consola coteja que la entrada **llegue** a la celda — la dirección ya salía de la organización
(`022`), lo que faltaba era la relación con la celda; **(b) ✓** el
cofre guarda el material en el Secret Manager de la celda como `t-<n>-cofre-<nombre>`, con la KEK
como CMEK y una **condición IAM por prefijo** medida desde dentro del pod
(`medida-el-almacen-por-inquilino.py`: el `create` también la obedece); `cofre.secreto` e
`iam.concesion` se quedan, `ore-cofre mudar` lleva lo viejo y la `028` no borra hasta que esté
vacío; **(c) ✓** `t-demo` y `t-prueba` con **su forja propia** (`46`: se funda sola y deja su
admin en el almacén; `ontologia` y `trabajo` mudados `--mirror`; `trabajo` legible sin testigo
dentro de la celda para que Flux no necesite un `Secret`), y Flux tirando del compartimento
en la nuestra. **Medido con las tres medidas de acoplamiento, cofre y forja: 0 por decidir.**
Lo que queda dicho y no hecho: cerrar la salida del inquilino hacia `forja/`, borrar las copias
muertas de la central, y delegar `ore.paladio.io` a Cloud DNS. Con esto, dedicado y BYOC son
*el mismo manifiesto en otro sitio*.

**E4 · Dedicado.** Un GKE por inquilino, aprovisionado por el mismo guion, con los tres pools.

**E5 · BYOC.** El agente en su clúster, tirando. El almacén del cofre en **su** proyecto con
**su** CMEK, que tras ⑤ es configuración; y la puerta privada (PSC) si no quieren IP pública.

**E6 · El pool de GPU y el modelo.** Que es lo que todo esto sostiene, y por eso va al final:
sin E3 el modelo estaría en casa del cliente mirando un árbol en la nuestra.
