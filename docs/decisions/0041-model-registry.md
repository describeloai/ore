# 0041 · Model Registry

**Estado:** **decidido y hecho** (2026-09-25) · **Decide:** que el registro de modelos de ORE **es
el catálogo**: un `Model` vive en `base.schema.nombre` como cualquier otro activo, y de ahí le vienen
su identidad, sus versiones, su linaje y su dueño. Sigue a [`0027`](0027-model-serving.md)
(el modelo es un documento del árbol), [`0034`](0034-el-catalogo-de-assets.md) (el catálogo es el
índice del árbol) y [`0038`](0038-los-tres-niveles.md) (`base.schema.nombre`). Gramática:
[OOS v1alpha15](../../vendor/oos/spec/v1alpha15/00-scope.md) y, para el dueño,
[v1alpha21](../../vendor/oos/spec/v1alpha21/00-scope.md).

## Qué es

Un **`Model`** es la referencia a un modelo **servido**: un id que la puerta de la celda atiende, y
que una `Function` llama (`model`, `models`). El Model Registry es el sitio donde esos modelos se
dan de alta, se encuentran, se versionan y se retiran.

**No es un sistema aparte.** Como en Unity Catalog, el registro es el propio catálogo: un `Model`
es un documento en la ruta que el cliente elige, y hereda lo que el catálogo da a todo lo que
contiene. No hay una segunda base de datos de modelos que pueda discrepar del árbol.

## La naturaleza del registro

| | de dónde le viene |
|---|---|
| **identidad** | `metadata.namespace` (la base) y `metadata.schema`: **un nombre único por ruta**, como todo el catálogo. Una `Function` lo nombra por partes: `modelo/<n>` en su mismo schema, `modelo/<base>.<n>` en `default`, `modelo/<base>.<schema>.<n>` completo |
| **versiones** | la historia de la forja, por fichero: cada alta o cambio es un commit con autor y fecha. El índice de assets da la última (`version: {commit, cuando, sujeto}`), `GET /arbol/historia/{ruta}` la lista entera y `GET /arbol/version/{hash}/{ruta}` el documento tal como era |
| **linaje** | las relaciones tipadas del índice, en las dos direcciones: una `Function` que lo nombra da `usa` / `usado_por`, así que el registro dice **quién lo usa**; un `TrainedModel` da `sale_de` / `produce` por su `trainedFrom`, es decir, **de qué datos salió** |
| **dueño** | `owner: user:<handle>` de quien lo da de alta (v1alpha21; [0052](0052-ownership.md)). Uno de v1alpha15 sin `owner` sigue valiendo |
| **integridad** | **no se retira lo que se usa**: retirar un `Model` que una `Function` nombra es 409, con quién lo nombra |

Lo que se versiona es **el documento**: a qué id servido apunta el modelo, con qué perfil, quién lo
cambió y cuándo. **No hay versiones numeradas del artefacto** ni alias de etapa (`@champion`,
staging/producción): un `Model` referencia lo que la puerta sirve, y el cambio de lo servido es el
commit. Si algún día hacen falta alias, es otra decisión.

## Cómo funciona

- **Alta y retirada, por `/modelos`.** `POST /modelos {paquete, schema, …}` escribe el documento en
  `packages/<base>[/<schema>]/modelos/<n>.yaml` **y** suscribe la celda en la puerta, en un solo
  acto (0027 ②). `GET` y `DELETE /modelos/{ref}` usan la referencia `<base>[.<schema>].<n>`.
  `/modelos` no toca ficheros por su cuenta: escribe, lee y retira con el motor de documentos
  (`escribir_en_su_sitio`, `modelos_de`, `retirar_de_su_sitio`), con su figura de siempre —clonar,
  escribir, no empeorar, empujar con quien lo pide—, y añade lo suyo: el encaje con la lista de
  perfiles y la suscripción.
- **Lectura, por el catálogo.** El `Model` es una fila del motor de `/documentos`
  (`carpeta: modelos`): sale en Assets con su base y su schema, y su ficha enseña el YAML. Por
  `/documentos` **no se escribe** (405): escribir un modelo es también suscribirlo, y eso es de
  `/modelos`.
- **La suscripción es de la celda, por id servido.** Retirar un modelo la deja en pie si otro
  `Model` del árbol sirve el mismo id.
- **Los de antes** (v1alpha9–14, en la raíz del árbol, `modelos/<n>.yaml`) se siguen leyendo,
  resolviendo y retirando por su nombre. No se migran solos: se redespliegan en su ruta.

## Por qué así

Hasta v1alpha14 un `Model` se escribía en la raíz del árbol, sin base ni schema, y `POST /modelos`
tenía la ruta fija: el catálogo lo daba con `paquete: null` y la consola, que sólo pinta lo que tiene
base, no lo enseñaba. La pieza para escribir en una ruta del catálogo ya existía (el motor de
`/documentos`, donde el `TrainedModel` ya era una fila), y lo único que faltaba era la gramática: un
`Model` con `namespace` era `OOS1005`. Hacerlo contenido del catálogo, y no un registro propio, le
da gratis lo que un registro tiene que tener, con las mismas reglas que todo lo demás.

## Aceptación

`pruebas-de-fuego/los-modelos.sh`, 0–8 más 5b–5f: el alta en `packages/ventas/modelos/`
(v1alpha21, `owner: user:ana`, `ref ventas.v2-lite`); sin `paquete`, 422; un schema sin declarar,
422 (`OOS2037`, del motor); el mismo nombre en `ventas.espana` es otro modelo, y en la misma ruta,
409; retirar uno de dos con el mismo id deja la suscripción; `/documentos` lee el `Model` y no lo
escribe (405); retirar uno que una `Function` nombra, 409 con quién. En `ore-core`, el índice da al
modelo su base y su schema, y la función usa el de su schema.
