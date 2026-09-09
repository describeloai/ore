# 0022 · El inquilino es un repositorio

**Estado:** aceptado · **Fecha:** 2026-09-09 · **Decide:** que aprovisionar un inquilino **se
escribe, no se aplica**; que la unidad de ese repositorio es **el inquilino y no el clúster**; y
que quién aprueba el alta es **un ajuste de cada cliente**, no una decisión de la plataforma

---

## El problema

`ore-serve` va **por organización** — decidido, y no por gusto: los clientes son enterprise y
administración pública, y ahí el aislamiento no se cuenta, se enseña. Cada inquilino con su
namespace, su cuota, su política de red y su credencial es lo que un auditor quiere ver.

⇒ Y eso convierte fundar en algo mucho mayor que escribir una fila. Un inquilino es esto:

```
Namespace · ResourceQuota · NetworkPolicy x7 · ServiceAccount x3
Deployment ore-serve + Service · ConfigMap de llaves + CronJob que las refresca
LocalQueue + ClusterQueue
Secret con el testigo de la forja · Secret del agente
y en la forja: el repositorio, con su `ore init` hecho
```

**¿Quién aplica eso?** La respuesta obvia es un proceso con permisos de ámbito de clúster sobre
`Namespace`, `Deployment`, `Secret` y `NetworkPolicy`. Y esa respuesta es la mala:

> ⛔ **Quien puede crear el Secret de un inquilino puede leer el de todos.**

Ese proceso sería la pieza más peligrosa del sistema — más que cualquier `ore-serve`—, y es
exactamente la figura que este árbol ya rechazó una vez: en CI se hizo que la construcción
corriera **como** `ore-ci` en vez de concederle suplantar a la cuenta de cómputo, *«que tiene medio
proyecto»*.

---

## Lo que se miró antes de decidir

[`pruebas-de-fuego/medida-el-aprovisionador-de-inquilinos.py`](../../pruebas-de-fuego/medida-el-aprovisionador-de-inquilinos.py).

- **Parametrizar es lo barato.** 27 menciones de `demo` en tres ficheros. Lo que separa esto de un
  estado del arte no es el aprovisionador.
- **El namespace del inquilino nacía dentro del manifiesto de la cola** (`10-kueue.yaml`). Con uno
  daba igual; con dos, dar de alta a un cliente no puede significar reaplicar Kueue. Ya está
  separado en `11-el-inquilino.yaml`, que dice arriba las tres sustituciones que lo definen.
- **Y midiendo salió una credencial que nadie había mirado:** el testigo que usaba `ore-serve`
  para empujar era de `ore-admin`, **administrador de la forja entera**. Un pod de inquilino con
  una llave que alcanzaba los árboles de todos. Ya está estrechado — un usuario por inquilino,
  colaborador de su árbol y de nada más: un repositorio ajeno le da **404**, no 403.

---

## La decisión

> ### El aprovisionador NO aplica: **escribe**. Y escribe en un repositorio **por inquilino**, nuestro, con lectura para el cliente.

**① No hay credenciales de clúster en el camino del alta.** El aprovisionador emite los
manifiestos del inquilino y **empuja** — que es lo único que este sistema ya sabe hacer—, y un
agente de GitOps, que tiene esos permisos una vez y auditado, los aplica. De ahí salen cuatro
cosas que no hay que programar:

- el alta queda en un **commit**, con quien la pidió dentro. Para gobierno eso no es un lujo: es
  la prueba;
- revisar antes de aplicar es un **pull request**, no un procedimiento escrito en un documento;
- deshacer es un `revert`;
- y la deriva deja de ser invisible: lo que hay y lo que se declaró se comparan solos.

⭐ Es el argumento de [`0018`](0018-la-ontologia-es-el-sistema-de-registro.md) aplicado un piso
más arriba. **Un inquilino también es un documento.**

**② La unidad es el INQUILINO, no el clúster.** Un repositorio por clúster llevaría dentro los
nombres de todos los clientes ⇒ no se le podría enseñar a ninguno, y la propiedad que justifica
todo esto —que el aislamiento se vea— se perdería en la primera pregunta.

Y así la unidad vuelve a ser la misma que arriba: **una unidad de gobierno, un repositorio**. El
árbol de la ontología ya lo era.

**③ Es nuestro, con lectura para el cliente — y la revisión es un ajuste POR INQUILINO.**

Esto es lo que hace que la decisión no haya que tomarla dos veces:

```
sin revisión      fundar → commit → aplicado. Minutos, sin humanos.
con revisión      fundar → pull request → lo aprueba SU gente → aplicado.
```

Lo segundo se activa poniendo revisión obligatoria en **ese** repositorio. No cambia el
aprovisionador, no cambia el formato, no cambia el agente. ⇒ **No hay que decidir hoy quién
aprueba en 2028**, ni elegir lo mismo para el cliente que quiere ir rápido y para el que tiene un
pliego.

⭐ Y si un cliente exige la propiedad, se le **transfiere** el repositorio. Mismo formato, mismo
contenido: es un `transfer`, no un rediseño.

---

## Lo que se acepta a cambio

- ⛔ **N repositorios en vez de uno.** Un cambio transversal —subir `enforce` a `restricted`, una
  CVE en la imagen base— pasa de ser un commit a ser N. Se automatiza porque son **nuestros**, que
  es justo la diferencia con dárselos al cliente: allí serían N pull requests que hay que
  perseguir, y acabarías con inquilinos en versiones distintas sin poder forzar.
- ⛔ **El cliente no escribe.** Ve su compartimento y, si se le da, lo aprueba. Cambiar su cuota
  pasa por nosotros. Esto **no es autoservicio**, y decirlo así evita venderlo como lo que no es.
- ⚠️ **La plantilla y la instancia se separan, y hay que sostener el corte.**
  `ore/malla/` dice qué **es** un inquilino y se versiona con el producto; el repositorio del
  inquilino dice qué **hay** y se versiona con la operación. Mezclarlos haría que dar de alta a un
  cliente fuera un commit en el árbol del producto.
- ⛔ **Y el repositorio de instancia NO puede vivir sólo en la forja que él mismo define.** Es
  circular: si la forja se cae, no se pueden leer los manifiestos que reconstruyen la forja.

---

## El abordaje, por etapas

La propiedad que se ha buscado al ordenarlas: **cada una deja el sistema mejor aunque la
siguiente no llegue nunca.** Ninguna es un andamio que sólo sirva para la de después.

**E0 · lo que ya está.** [`017`](file) declara cómo se llama el árbol de cada organización;
`11-el-inquilino.yaml` es la plantilla con sus tres sustituciones dichas; el testigo de la forja
alcanza un repositorio; y la forja tiene copia **que se restaura en cada vuelta**.

**E1 · Renderizar, sin aplicar nada.** Un mando que emite los manifiestos de un inquilino a la
salida estándar, desde la plantilla y las tres sustituciones.

⭐ No necesita **ninguna** credencial: es texto. Y trae su propia prueba — renderizar `demo` tiene
que dar lo que ya está aplicado en el clúster. Si no coincide, el renderizador miente, y se sabe
antes de que nadie dependa de él.

**E2 · El repositorio de instancia de `demo`, a mano.** ✔ 2026-09-09 ·
`describeloai/inquilino-demo`, privado. En GitHub y **no en la forja**, por lo circular: la
forja está descrita en parte por manifiestos como ésos.

La comprobación es `pruebas-de-fuego/la-deriva-del-inquilino.sh`, y usa `kubectl diff` en vez de
comparar por nuestra cuenta — porque la hace el SERVIDOR: pregunta *«¿qué cambiaría si aplicase
esto?»*, que es exactamente la pregunta. Comparar un `get` con lo escrito obligaría a ir tachando
valores por defecto, `status` y `managedFields` hasta que la prueba dejara de decir nada.

**Resultado: ni una diferencia** en el inquilino entero — salvo una, y encontrarla era el motivo
de esta etapa.

> ### ⛔⛔ El `ConfigMap` `jwks` no lo puede reconciliar un agente
>
> El manifiesto declara la semilla —`{"keys":[]}`— y lo vivo lleva las llaves del realm, que
> escribe el `CronJob` en cada refresco. Un agente de GitOps que reconciliase ese objeto
> **devolvería el juego a vacío cada pocos minutos**, y `ore-serve` dejaría de validar tokens en
> su siguiente arranque — sin que nada fallara mientras tanto.
>
> ⇒ Es la primera aparición de una clase entera: **objetos que un manifiesto SIEMBRA y un
> proceso posee en tiempo de ejecución.** Los secretos de la E4 son el mismo animal.
>
> Se resuelve en la E3 diciéndoselo al agente (`ignoreDifferences` sobre `.data`). Hasta
> entonces está exento en la comprobación, **con nombre y sólo ese objeto**: si mañana difiere
> otro, se pone rojo. Un objeto exento y nombrado es una decisión; una prueba que tolerase
> «alguna diferencia» no sería una prueba.

**E3 · El agente de GitOps.** ✔ 2026-09-09 · **Flux**, y sólo dos de sus controladores —
`source-controller` y `kustomize-controller`—, vendidos y pinchados en v2.9.5 en `malla/12-flux.yaml`.
Sin `helm`, sin `notification`, sin los de imágenes: cada uno sería superficie que alguien tiene
que mantener y parchear.

El enganche está en `malla/13-el-inquilino-reconciliado.yaml`, y vive en `malla/` **y no en el
repositorio del inquilino** a propósito: es la parte que decide qué se obedece. Si viviera dentro
de lo que se obedece, quien pudiera escribir ahí cambiaría a qué apunta el agente.

El testigo es una **clave de despliegue de sólo lectura** de ese único repositorio: si se filtrase,
no alcanza a ningún otro y no puede escribir en éste.

⭐ **Probado:** se subió `ore-serve` a 3 réplicas a mano y la primera reconciliación lo devolvió a 1.
La deriva dejó de ser invisible **y dejó de durar**.

Y la pregunta que la E2 dejó escrita tiene respuesta exacta: `kustomize.toolkit.fluxcd.io/ssa:
IfNotPresent` — Flux crea el objeto si no está y no lo toca nunca más. La documentación de Flux
nombra el caso con estas palabras: *«Flux crea los recursos con campos que otros controladores
mutan después»*. Comprobado: tras la primera reconciliación, los dueños de ese `ConfigMap` siguen
siendo `kubectl` y el `curl` del CronJob — Flux no lo tocó.

⛔ **Y lo que E3 cambia en la costumbre:** `t-demo` ya no se aplica a mano. Para cambiar el
compartimento de un inquilino se cambia la PLANTILLA, se renderiza y se empuja a su repositorio.
El resto de `malla/` se sigue aplicando a mano, porque nadie lo reconcilia todavía.

⚠️ Y una cosa que sale de frente: `kustomize-controller` recibe **`cluster-admin`**. No es una
traición a ① — el permiso no desaparece, se CONCENTRA: en vez de repartirlo por cada pieza que
necesite crear algo, lo tiene un componente con versión pinchada, manifiesto en revisión y una sola
forma de decirle qué hacer. Acotarlo por inquilino (`spec.serviceAccountName`) choca hoy con que el
manifiesto del inquilino CREA su propio `Namespace`, que es un recurso de clúster. Va con la E7.

⭐ Y esto ya paga por sí mismo aunque no haya un segundo cliente nunca: hoy `kubectl apply -f
malla/` lo hace una persona, y **lo que nadie aplicó no se distingue de lo que nadie escribió**.

**E4 · El aprovisionador escribe.** ✔ 2026-09-09 · `malla/aprovisionar-inquilino.sh`, corrido de
verdad contra `prueba` y desmontado a continuación.

⛔ La pregunta que esta decisión dejaba abierta —**los secretos no van en un repositorio**— tiene
respuesta, y es la `0023`: el valor va al **almacén de la plataforma** y el manifiesto sólo lo
referencia; un contenedor de inicio lo trae a un `emptyDir` de memoria. Ni `SealedSecret` ni
`External Secrets`: **el aprovisionador sigue sin una sola credencial de clúster.**

⇒ Queda **una** excepción, dicha en el paso ⑦ del guion y no escondida: la clave de despliegue que
Flux necesita para leer el repositorio del inquilino. `source-controller` la lee de etcd, así que
se emite y se aplica a mano — una línea con nombre en vez de un permiso general.

### ⭐⭐ Y lo que la corrida de verdad enseñó, que la prueba en seco no podía

El guion vivió una iteración entera probado **en seco** y parecía bueno. La primera corrida real
destapó **tres** defectos en diez minutos, y los tres tienen la misma forma: *una línea verde
encima de algo que no pasó*.

| | qué | por qué en seco era invisible |
|---|---|---|
| ① | `curl -o /tmp/r` dentro de la forja, cuyo sistema de ficheros es de solo lectura: salía con **23** en las tres llamadas | el llamante redirigía a `/dev/null` y no miraba el código |
| ② | `correr "el secreto …" -- true`, que no es una orden | `command not found`, y la salida siguió |
| ③ | `git init` sobre lo recién rendido: la segunda pasada muere con «rejected — fetch first» | en seco no hay primera pasada, así que no hay segunda |

⇒ El defecto de fondo no es ninguno de los tres: es que **el guion no comprobaba**, y en seco eso
es invisible por construcción. Arreglado: `forja_api` distingue el éxito del fallo aquí dentro
—`409` y `422` son éxito, porque son «ya existía»—, y ⑥ **clona y converge** en vez de ocurrir una
vez.

⭐ Y de ahí sale la pieza que faltaba: **`malla/desaprovisionar-inquilino.sh`**. Un aprovisionador
sin su inverso sólo se corre en serio una vez —y por eso se prueba en seco, y por eso esos tres
defectos vivieron una iteración entera—. Con el inverso se corre, se mira, se desmonta y se
repite, que es la única forma de que esto sea una propiedad y no una anécdota.
`pruebas-de-fuego/medida-la-corrida-de-verdad.py` fija las cuatro propiedades.

**Lo medido en la corrida** (`prueba`, luego borrado): dos claves de KMS con una cuenta cada una y
disjuntas; `serve-prueba` colaborador de `t-prueba/ontologia` con **204** y de `t-demo/ontologia`
con **404**; el secreto del almacén alcanzable por una sola cuenta; y **ni un rastro del nombre
`demo` en los cinco manifiestos rendidos**.

⚠️ Lo que sigue sin hacer de la E4: esto es todavía **un guion que se corre a mano**, y su forma
final es un **Job** —con su testigo de forja y su identidad de Google, y **sin ni un permiso de
RBAC**—, porque la forja sólo se alcanza desde dentro del clúster.

**E5 · El árbol.** `ore init --name <org>`, primer commit y push, con la imagen de `serve` —la
única con `git`—. El nombre del manifiesto sale de la organización, y no es cosmético: se propaga
a cada `connectionEnv`.

**E6 · La entrada por inquilino.** Hoy hay **un** Ingress en toda la malla y es el del IdP. Con
`ore-serve` por organización hay que decidir cómo llega la consola a cada uno, y en este mercado
cada cliente suele querer su propio nombre DNS. La consola resolverá `organización → URL` desde
`iam.organizacion`, que es para lo que existe esa columna.

**E7 · Lectura para el cliente, y revisión donde se pida.** Lo último a propósito: es un ajuste
del repositorio, no trabajo de plataforma. Que sea barato es la mitad del valor de ③.

---

## Lo que esto NO decide

- **Qué agente de GitOps.** Flux o Argo; la decisión de arriba no depende de cuál.
- **Cómo viajan los secretos.** Nombrado en E4 y sin respuesta.
- **Si un cliente tiene su propio clúster.** Esta forma lo admite —el repositorio se transfiere—
  pero cuándo se ofrece es comercial, no técnico.
- **Quién aprueba en cada cliente.** Por diseño: es un ajuste, y esa es la propiedad que se
  compró en ③.
