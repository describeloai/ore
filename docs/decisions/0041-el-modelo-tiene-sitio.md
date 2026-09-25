# 0041 · El modelo tiene sitio: vive en la ruta del catálogo que el cliente elige

**Estado:** decidido y hecho (2026-09-25) · **Decide:** dónde se escribe el `Model` que la
consola despliega, y con qué pieza. Sigue a [`0027`](0027-el-modelo-vive-en-el-arbol.md) (el
modelo en el árbol), [`0034`](0034-el-catalogo-de-assets.md) (el catálogo es el índice del
árbol) y [`0038`](0038-los-tres-niveles.md) (`base.schema.nombre`). Gramática:
[OOS v1alpha15](../../vendor/oos/spec/v1alpha15/00-scope.md).

## Lo medido

- **El `Model` se escribía en la raíz del árbol** (`modelos/<n>.yaml`) porque v1alpha9 lo hizo
  vocabulario del árbol, sin `namespace`, y `POST /modelos` tenía la ruta fija.
- **El catálogo no lo veía**: el índice lo daba con `paquete: null` y la consola sólo pinta lo
  que tiene base (`comoDatabaseDesdeAssets`).
- **La pieza para escribir en una ruta del catálogo ya existía**: el motor de `/documentos`
  escribe cualquier kind de su tabla en `packages/<base>[/<schema>]/<carpeta>/<n>.yaml`, con la
  figura de siempre (clonar, escribir, no empeorar, empujar con quien lo pide), único por
  `(kind, base, schema, nombre)`. El `TrainedModel` ya era una fila y ya salía en Assets.
- **Lo único que faltaba era la gramática**: un `Model` con `namespace` era `OOS1005`.

## Lo decidido

1. **v1alpha15**: el `Model` es contenido del catálogo, como el `TrainedModel`:
   `metadata.namespace` (la base) y `metadata.schema`. **Nombre único por ruta**, como todo lo
   del catálogo. Una `Function` lo nombra por partes: `modelo/<n>` en su mismo schema,
   `modelo/<base>.<n>` en `default`, `modelo/<base>.<schema>.<n>` completo.
2. **El `Model` es una fila más del motor de `/documentos`** (`carpeta: modelos`). Se **lee**
   por ahí —la ficha del catálogo, su YAML— y **no se escribe** por ahí (405): escribirlo es el
   documento Y la suscripción de la celda en el gateway, en un acto (0027 ②), y eso es de
   `/modelos`.
3. **`/modelos` no busca ni escribe ficheros**: escribe y retira con el motor
   (`escribir_en_su_sitio`, `retirar_de_su_sitio`), lee con él (`modelos_de`), y le añade lo
   suyo: el encaje con la lista de perfiles y la suscripción. `POST /modelos` lleva `paquete` y
   `schema`; `GET`/`DELETE /modelos/{ref}` usan la referencia `<base>[.<schema>].<n>`.
4. **La suscripción es de la celda y por id servido**: retirar un modelo la deja si otro del
   árbol sirve el mismo id.
5. **Los de antes** (v1alpha9–14, en la raíz) se siguen leyendo, resolviendo y retirando por su
   nombre. No se migran solos: se redespliegan en su ruta.

## Aceptación

`pruebas-de-fuego/los-modelos.sh`, 0–8 más 5b–5f: el alta en `packages/ventas/modelos/`
(v1alpha15, `ref ventas.v2-lite`); sin `paquete`, 422; un schema sin declarar, 422 (`OOS2037`
del motor); el mismo nombre en `ventas.espana`, otro modelo, y en la misma ruta, 409; retirar
uno de dos con el mismo id deja la suscripción; `/documentos` lee el `Model` y no lo escribe
(405); retirar uno que una `Function` nombra, 409 con quién (el motor). `ore-core`: el índice
da al modelo su base y su schema, y la función usa el de su schema.
