# 0023 · Dónde vive un secreto

**Estado:** aceptado · **Fecha:** 2026-09-09 · **Decide:** que `iam` dice **quién puede** y no
guarda ningún valor; que el material vive **cifrado de sobre** y la llave que lo abre **no vive
con él**; que **quien emite queda `owner`** de lo que emitió; y que **`usar` no es `leer`**

---

## El problema

La plataforma va a ofrecer un almacén de secretos como producto: contraseñas, tokens de acceso,
claves de API, credenciales de bases de datos y de orígenes. Una superficie donde una persona con
rol suficiente los crea, y donde otras —y otras **máquinas**— los usan.

Y llega con una pregunta que parecía de compra —*«¿Vault? ¿los `Secret` de Kubernetes?»*— y que
al mirarla resulta ser tres preguntas apiladas:

```
quién dice QUIÉN PUEDE       ← ya está escrito, y esta decisión lo usa
dónde vive el MATERIAL       ← esto es lo que se decide aquí
cómo LLEGA al que lo usa     ← la entrega, y va aparte
```

⛔ Y hay una urgencia concreta detrás: la **E4** de [`0022`](0022-el-inquilino-es-un-repositorio.md)
está bloqueada por esto. El aprovisionador **escribe, no aplica** — pero un `Secret` no cabe en un
repositorio, así que sin un sitio donde poner un valor, la E4 obligaría al aprovisionador a tener
credenciales de clúster, que es justo lo que la `0022` quitó.

---

## Lo que se miró antes de decidir

[`pruebas-de-fuego/medida-el-almacen-de-secretos.py`](../../pruebas-de-fuego/medida-el-almacen-de-secretos.py),
contra las migraciones de `iam` y no contra la memoria.

- **El modelo de permisos ya está entero**, y vacío de secretos: `iam.potestad` con sus once filas,
  `iam.rol_de_recurso`, la unión de conjuntos de la `016` y la guarda de contención. Hay dónde
  meterlas.
- ⭐ **Y la frase que abrió esto era el modelo:** *«cualquier usuario con suficiente rol puede
  emitir un secreto; ahora leerlo o usarlo ya es otra historia»*. Es exactamente el corte que la
  [`011`](file) peleó — arriba la organización, abajo el recurso, y **sin herencia entre los dos**.
- ⭐ **`iam.concesion.sujeto` es texto sin clave ajena a `persona`**, y la
  [`0021`](0021-una-persona-en-varias-organizaciones.md) dice por qué: *«para que quepa un agente
  (RFC 8693)»*. Un Job puede ser el titular de una concesión.
- **`concesion` ya regala** caducidad, revocación con fecha y autor, quién la dio, el aislamiento
  por organización y `concesion_viva`.
- Y en el proyecto **no hay KMS ni Secret Manager habilitados**. La elección es real.

---

## La decisión

> ### `iam` dice QUIÉN PUEDE y no guarda nada. El material vive cifrado de sobre, y la llave no vive con él.

**① Emitir es una POTESTAD; leer y usar son CONCESIONES. Y quien emite queda `owner` de lo que
emitió — y de nada más.**

Emitir no habla de ningún secreto concreto: todavía no existe. Leer y usar hablan de **ése**.

⭐ Y de ahí sale sola la asimetría que hace seguro el producto: quien puede crear secretos **no
queda con derecho sobre los que ya había**, porque emitir vive arriba y leer vive abajo. Un
almacén donde el permiso de crear arrastra el de leer es un almacén con una sola cerradura.

⚠️ El `owner` sobre lo emitido **no es una concesión gratis: es la que evita el huérfano.** Un
secreto que nace sin nadie que pueda darlo no se lo puede dar nadie nunca — es la organización sin
administrador otra vez, y se arregla igual: en el mismo acto.

**② `usar` es un rol de recurso NUEVO, y no es `lector`.**

```
lector   VE EL VALOR            una persona que copia una contraseña
usar     lo RESUELVE SIN VERLO  un Job que se conecta
owner    lo rota, lo revoca, y lo concede
```

⛔ Y **la mayoría del acceso tiene que ser `usar`**. Un almacén donde para conectarte hay que poder
leer la contraseña es un cajón con una puerta: cada permiso de ejecución arrastra uno de lectura, y
la auditoría no puede distinguir *«se conectó»* de *«se la llevó»*.

⭐ Cabe sin tocar nada porque `iam.rol_de_recurso` **no tiene ordinal a propósito**: meter un
tercer rol es insertar una fila. `usar` no implica `lector` por la misma razón por la que `owner`
no lo implica — *dos hechos, no una regla*.

**③ El material: una clave por secreto, la maestra en un KMS, el cifrado en nuestra base.**

```
el VALOR    cifrado con una DEK propia de ese secreto
la DEK      cifrada con una KEK que vive en un KMS y no sale de él
el CIFRADO  en nuestra base, al lado de `iam`
```

La propiedad: **el material y la llave nunca están en el mismo sitio.** Un volcado de la base es
ruido sin el KMS, y el KMS no guarda ningún secreto de nadie — sólo lo que los abre.

Y una DEK **por secreto**, que no es ceremonia: rotar la maestra es re-envolver claves sin
descifrar nada, y comprometer un secreto no compromete a los demás.

⭐ Con esto, **que el cliente traiga su propia KEK** —requisito en banca y sector público— es una
configuración por organización y no un rediseño.

**④ No hay un segundo motor de autorización. Nunca.**

⛔ Y por eso **Vault no**. No por su coste operativo —que existe—, sino porque trae su propio
lenguaje de políticas, sus propias identidades y su propia auditoría: *«¿quién puede leer el
secreto X?»* quedaría escrito en **dos sitios que tienen que coincidir**.

Es lo mismo que este árbol ya decidió con el IdP —*«El IdP dice quién ENTRA. Nosotros decimos quién
PERTENECE»*— y por lo que la pertenencia no se metió en Keycloak aunque cupiera.

⇒ Si algún día hace falta Vault, un HSM o el KMS del cliente, entran **por debajo, como
custodios**. Lo que no se les da es decidir quién puede.

---

## Lo que se acepta a cambio

- ⛔ **El KMS es una dependencia dura.** Si no responde, no se resuelve ningún secreto. Falla
  cerrado, que es lo correcto, pero hay que decirlo y hay que medir a qué deja sin servicio.
- ⚠️ **Rotar la KEK exige un trabajo de re-envoltura**, y hay que escribirlo **antes** de
  necesitarlo. Una rotación que no se ha ensayado es la copia que nadie ha restaurado.
- ⛔ **Se renuncia hoy a las credenciales dinámicas** —de vida corta, emitidas al vuelo— que es lo
  que Vault hace mejor que nadie. Guardar cadenas estáticas no las necesita; el día que un cliente
  las pida, se pagan entonces y por debajo de la misma interfaz.
- ⚠️ **`concesion.recurso` deja de poder ser texto libre.** Es el asidero del secreto: lo que se
  concede, lo que se audita y lo que un manifiesto referencia. Sin forma cerrada, dos escrituras
  del mismo secreto no son el mismo recurso y la concesión no alcanza a lo que creías.
- ⛔ **Y una regla que no se puede romper una sola vez:** el valor en claro no se registra, no sale
  en ningún listado y no viaja en la respuesta de nada que no sea una resolución explícita y con
  huella. Una excepción «temporal» aquí y el producto deja de ser un almacén de secretos.

---

## Por dónde se empieza

⭐ La mitad barata es independiente del KMS y se puede hacer ya: **`iam` no necesita saber nada del
material para saber quién puede.** Tres cosas, todas dentro de tablas que existen:

```
1  `secreto:emitir` y `secreto:listar` en `iam.potestad`   ← y SECURITYADMIN deja de ser una carcasa
2  `usar` en `iam.rol_de_recurso`                          ← una fila
3  una FORMA cerrada para `concesion.recurso`
```

Y sólo después el custodio: el KMS, las tablas del material, el verbo que resuelve y la huella con
su **para qué**.

⇒ El orden no es capricho: con 1-3 hechos, la consola ya puede pintar quién puede qué sobre un
secreto que todavía no existe — que es exactamente lo que la E1 de la `0022` hizo con los
manifiestos, y por el mismo motivo: **lo que no necesita una credencial se hace primero, porque su
prueba es inmediata.**

---

## Lo que esto NO decide

- **Qué KMS.** Cloud KMS, uno del cliente, o un HSM. La forma de sobre los admite a los tres, y
  ésa era la mitad de su gracia.
- **Cómo llega el valor al proceso que lo usa.** Es la tercera pregunta de arriba —la entrega— y
  tiene su propia respuesta: montado como fichero, resuelto por identidad, nunca un `Secret`
  perpetuo. Va aparte porque se puede cambiar sin tocar nada de esto.
- **El «para qué» de la huella.** Queda nombrado como necesario —a un auditor no le basta *quién
  lo leyó*, pregunta *por qué*— y no se diseña aquí.
- **Si los secretos de PLATAFORMA viven en el mismo sitio que los del cliente.** El testigo de la
  forja y la clave de Flux no son de nadie que administre una organización, y meterlos donde el
  cliente administra le daría superficie sobre la credencial con la que su propio `ore-serve`
  empuja. Se mide antes de decidirlo.
