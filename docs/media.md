# La media en código · el contrato de ejecución

**Estado:** B0 de [0049](decisions/0049-media-paradigms-in-code-repositories.md) · contrato
escrito, sin implementar. Lo que la gramática fija de los valores (la referencia, el ancla, la
tabla anclada, el listado) está en OOS
[`v1alpha17`](../vendor/oos/spec/v1alpha17/00-scope.md); esto fija **las operaciones**: qué hace
cada una, cómo se pide por HTTP, cómo se llama desde cada lenguaje, y qué tiene que pasar una
superficie para decirse conforme ([`conformidad/media`](../conformidad/media/README.md)).

Una sola regla lo ordena todo: **el código nunca habla con el almacén ni con el origen**. Habla
con la celda (0049, D1), y la celda decide, fija, firma y sirve.

## 1. Los nombres

| en | colección | ítem | referencia |
|---|---|---|---|
| gramática | `MediaCollection` | — | el valor de `Media<c>` |
| HTTP | `/media/{base}/{schema}/{coleccion}` | `?path=` o `?digest=` | JSON con los campos de v1alpha17 `01` §3 |
| Python ([SDK](sdk.md#collections)) | `ore.collection("b.s.c")` | `Item` | `MediaRef` (dataclass inmutable) |
| SQL | `b.s.c` en un `FROM` (su listado) | una fila | `_item` |

`MediaRef` se serializa en JSON con los nombres de la gramática (`uri`, `collection`, `path`,
`version`, `digest`, `size`, `content_type`, `content_type_detected`, `checksum`,
`annotations`). Un campo desconocido se ignora al leer: así entra un campo nuevo sin romper a
nadie.

## 2. Las siete operaciones

### `list` · el listado

```
GET /media/{b}/{s}/{c}/items?prefix=&as_of=&cursor=&limit=
→ 200 { "as_of": "<transacción>", "items": [MediaRef…], "cursor": "<opaco>" | null }
```

- **Sin bytes y sin URLs.**
- **Una transacción entera**: la primera página fija `as_of` (la actual si no se pide), y las
  siguientes la heredan por el cursor. Nunca se mezclan dos transacciones en un recorrido.
- **Por cursor, no por desplazamiento**: el coste de una página no depende de cuántas van antes
  (hoy sí, 0049 «Lo que hay hoy»). `limit` hasta 1000; por defecto, 1000.
- Python: `collection.items(prefix=None, state=None, limit=1000)` → iterador perezoso de `Item`s
  sin bytes, por cursor; `limit` es el tamaño de página.

### `stat` · lo fresco

```
GET /media/{b}/{s}/{c}/item?path=|digest=&version=
→ 200 MediaRef + { "current": true|false }
```

`current` dice si la versión pedida sigue siendo la actual. Un ítem que no está en `as_of` ni en
la actual, `404`.

### `open` y `read_range` · los bytes, fijados

```
GET /media/{b}/{s}/{c}/content?path=|digest=&version=
→ 307  Location: <dónde están los bytes>
       { "url", "version", "item", "ttl_s", "expires_ms", "desde": "lago" | "medios" }

GET <url>
Range: bytes=0-1023            (opcional)
→ 200 | 206, el cuerpo en flujo
  ETag: "<validador fuerte de esa versión>"
  Repr-Digest: sha-256=:<base64>:     (RFC 9530, si se conoce el digest)
  Accept-Ranges: bytes | none
```

- **Fijado**: sin `version`, la celda fija la actual **al abrir** y la devuelve en la cabecera
  `ORE-Media-Version`; todo el flujo es de esa versión. Si el origen cambia a mitad, se corta con
  un error (`media/cambiado`), nunca con bytes de otra.
- **Cómo se fija una virtual** (0049 B3·0, medido): `versionId` —**también `null`**, la versión
  de un objeto subido antes de activar el versionado, que en un bucket versionado sobrevive a una
  sobrescritura— e `If-Match` con el ETag que anotó el manifiesto. Lo que el origen ya no tiene en
  esa versión, o tiene con otro ETag, es `media/cambiado`.
- **La celda dice dónde, no pasa los bytes** (0049 B3·3): `content` contesta `307`. De una
  mantenida, a la URL firmada de su blob en el lago; de una virtual, a
  `ore-medios:8098/contenido?permiso=…` —el puerto del puesto, que sólo sirve eso; el índice, la
  firma y los permisos están en el 8097, sólo para ore-serve—, un permiso opaco que vale para ese ítem en esa versión
  (todos sus rangos) durante `ttl_s` —5 min, nunca más de lo que le queda a la credencial de
  la fuente—. Quien no sigue redirecciones (el SDK) lee `url` del cuerpo; caducada, pide otra
  con la misma `version`. **No se reenvía el token de ORE** a esa URL: no lo necesita.
- **Da igual la clase**: una mantenida se sirve del lago; una virtual, del origen con la
  credencial de la celda. Para el código es la misma ruta (0049, D1). La celda **calcula el
  `sha256` al paso** de una lectura entera de un ítem que no lo tenía, y lo registra.
- **`Repr-Digest`**, no `Content-Digest`: el digest es de la representación entera, también en
  un `206` (RFC 9530 §3).
- Un origen sin rangos responde `Accept-Ranges: none` y un `Range` es `media/sin-rangos`.
- Python: `item.open()` → objeto fichero binario de solo lectura con `read`, `seek` y `tell`
  (`io.RawIOBase`). `seek` se traduce en un `Range`; cerrar a medias **corta** la conexión, no
  descarga el resto. Al leer hasta el final verifica `size` y, si lo hay, el digest
  (`media/corrupto`).
- Python, lo demás (0049 B3·5, `puesto/python/ore/medios.py`): la URL de `content` se lee **sin el
  token de ORE**; un `read` no es una petición —se lee en flujo desde el cursor y sólo un `seek`
  abre otra—; un permiso caducado se renueva con la misma versión. `item.read_bytes()` baja un
  ítem grande por rangos en paralelo; `ore.read_many(items, threads=16)` muchos a la vez, dando
  `(item, data, error)` —el error de uno no para los demás—. Excepciones por `type`
  (subclases de `MediaError`): `MediaNotFound`, `MediaForbidden`, `MediaChanged`, `MediaCorrupt`,
  `MediaRangeError`.

### `url` · para quien necesita HTTP

```
POST /media/{b}/{s}/{c}/urls   { "items": [{path|digest, version?}…], "ttl_s": 300 }
→ 200 { "urls": [{ "url", "expires_at", "headers": {…} } | { "error": {…} }…] }
```

- Para un navegador o un modelo externo que solo sabe descargar. **El código de ORE no la
  necesita**: tiene `open`.
- `ttl_s` entre 30 y 3600; por defecto, 300. Recortado a lo que dure la credencial de origen.
- **Al portador**: nunca se escribe en una tabla, un log ni un resultado.
- Hasta 1000 ítems por petición; el error de uno va en su posición y no tumba el lote.
- Una virtual con `open` no pasa por aquí. Su URL, si se pide, es la del origen, y solo la alcanza
  quien tenga salida a él (no un puesto: 0049, «Lo que hay hoy»).

### `put` · escribir un ítem

```
POST /media/{b}/{s}/{c}/transactions  { "ttl_s"? }
→ 201 { "transaction": "<t>", "upload": "<url>", "ttl_s", "expires_ms" }
PUT  <upload>&path=<camino>               (el cuerpo en flujo, con Content-Length)
     Content-Type: <declarado, opcional>
     Repr-Digest: sha-256=:…:  (opcional: si llega, se coteja)
→ 201 MediaRef + { "transaction", "stored" }
POST /media/{b}/{s}/{c}/transactions/{t}/commit
→ 200 { "transaccion", "metadata_location", "items", "cambios", "procedencia", "commit" }
POST /media/{b}/{s}/{c}/transactions/{t}/abort  → 204
```

- **Los bytes no pasan por la puerta** (0049 B4b·2): como `open` dice dónde leer, abrir una
  transacción dice dónde subir —`upload`, `ore-medios:8098/subida?permiso=…`—, y el código sube
  cada ítem ahí añadiendo `&path=`. `upload` es un **portador** de esa transacción mientras viva:
  nunca se escribe en una tabla, un log ni un resultado.
- **Quién puede** (B4b·2): la colección tiene que ser escrita (si no, `media/no-escribible`); desde
  un puesto, la clase de su repositorio puede quitar (`media/sin-permiso`) y dentro de un transform
  sólo se escribe su `output` (`media/no-declarada`), al abrir y al confirmar. Confirmar o abortar
  sólo lo hace quien abrió (si no, `media/transaccion`).
- **Confirmar escribe el puntero** (`datasets/<b>/<s>/<c>.json`, en la rama del puesto) en un
  commit, con la **procedencia**: `{puesto, transform, inputs, fijadas}` dentro de un transform
  —las transacciones que leyó, B4·2—, `{puesto, leidas}` en una sesión, y `transaccion` y `por`.
  Si otro confirmó a la vez, `409` y la transacción **sigue abierta**: se confirma otra vez, sobre
  la base nueva.
- **Y el linaje en el documento** (B4·4, v1alpha19 `01` §2): en el mismo commit, la colección recibe
  `spec.derivedFrom` con lo que esta transacción leyó —los `inputs` del transform, o lo que leyó la
  sesión—, sin repetidos ni ella misma. Cada transacción lo reescribe; si no leyó nada, no lo lleva.
  Un documento v1alpha16–18 sube a v1alpha19. La respuesta del commit dice el `derivedFrom`.

- Solo en una colección **escrita** (v1alpha16 `02` §3); en una mantenida es `media/no-escribible`.
- SQL (0049 B4·4, el guion del puesto): `create media collection [if not exists] b.s.c media
  <document|image|…> formats (pdf, …) [comment '…']` crea la misma colección escrita y vacía —es
  `create_collection` del SDK—; el guion la coteja en orden (la base y el schema, o creados antes en
  él; un nombre una cosa; una mantenida no se crea encima). Llenarla es del código.
- Python (0049 B4b·3, `puesto/python/ore/medios.py`; [SDK](sdk.md#writing-into-a-collection)): `ore.create_collection(name, media, formats)`
  escribe el documento (v1alpha19, sin `from`) sin `owner`: la colección es de quien la crea —la
  persona que abrió el puesto—, y lo pone el servidor (0052 · Ownership); `owner=`
  sólo para dársela a otro; `with collection.transaction() as t: t.put(path, data)`
  confirma al salir y aborta con una excepción. `data` son bytes (con su `Repr-Digest`), una ruta (en
  flujo) o un fichero (si no se rebobina, se copia antes). Una subida cortada se reintenta; un commit
  que pierde la carrera de la forja (`409` sin `type`) se vuelve a confirmar. `upload` no se enseña y
  no lleva el token de ORE. Dentro de un transform, `inputs`/`output` admiten una colección, y sólo se
  escribe su `output`. Errores: `MediaNotWritable`, `MediaTransactionError`, y `MediaCorrupt` si el
  digest no casa. `t.put_many(pairs, threads=8)` sube muchos a la vez, como `read_many`.
- La celda calcula el `sha256` al paso, detecta el tipo por los bytes, y guarda el blob **por su
  digest**. Subir lo que ya está es gratis (idempotente por digest).
- El ítem existe cuando la transacción se confirma; un `abort` no deja nada.
- Una transacción no tiene tope de ítems (0049, D5: sin el de 10 000 de Foundry); se confirma
  entera o no.
- **Cómo lo hace la celda** (0049 B4b·1, `crates/ore-medios/src/escritura.rs`). `ore-medios` es
  el único que escribe en el lago; el puesto no recibe ninguna credencial suya:
  - abrir da una transacción y un **permiso de subida** (como el de leer: opaco, en memoria, vive
    lo que la transacción, una hora como mucho —menos que la gracia de la recogida de blobs—);
  - los bytes van del puesto a `ore-medios:8098/subida?permiso=…&path=…` en flujo
    (`Content-Length` obligatorio, hasta 5 GiB): el `sha256` y el `crc32c` al paso, lo grande a
    disco, el blob por su digest; si ya estaba, se toca y no se sube. Un `Repr-Digest` que no casa
    es `media/digest-no-casa` y no deja nada;
  - el tipo que se sirve es **el de los bytes** (`content_type_detected`); el declarado vale
    sólo si los bytes no dicen nada;
  - confirmar sella el manifiesto —la tabla de una mantenida: `clave` = camino, `version` = el
    `sha256`— **sobre la base que nombra el puntero en ese momento**: escribir un camino con otro
    contenido retira su fila actual; con el mismo, no cambia nada (`cambios: {entran, cambian,
    iguales}`). El puntero lo escribe `ore-serve`;
  - una transacción que no está abierta, o es de otra colección, es `media/transaccion` (404 o
    409).

### `verify` · recalcular

```
POST /media/{b}/{s}/{c}/verify { "items": [...] } → 200 { "results": [{ "ok" | "error" }…] }
```

Lee los bytes y compara con el digest. Para auditar, no para el camino caliente.

## 3. Los errores

Todos en **RFC 9457** (`application/problem+json`): `type`, `title`, `status`, `detail`, más
`item` cuando es de uno. Los tipos son estables y cada superficie los convierte en su excepción o
en su valor de error:

| `type` | status | cuándo |
|---|---|---|
| `media/no-existe` | 404 | la colección o el ítem |
| `media/sin-permiso` | 403 | la concesión lo niega; no dice si existe |
| `media/no-declarada` | 403 | un transform lee una colección que no declaró (0049, D6) |
| `media/cambiado` | 412 | la versión fijada ya no se puede leer entera |
| `media/sin-rangos` | 416 | el origen no da rangos |
| `media/permiso` | 401 | el permiso de leer un ítem falta, caducó o no es de esta celda: se pide otro |
| `media/rango` | 416 | el rango no cabe en el ítem, o no es de una parte (`bytes=a-b`, `a-`, `-n`) |
| `media/corrupto` | 502 | los bytes no casan con `size` o `digest` |
| `media/no-escribible` | 409 | `put` en una colección que no es escrita |
| `media/digest-no-casa` | 422 | el `Repr-Digest` que trajo un `put` no es el de sus bytes |
| `media/transaccion` | 404 / 409 | la transacción de un `put` no está abierta (caducó, se cerró, un reinicio) o es de otra colección |
| `media/origen` | 502 | el origen falló (con su código dentro) |
| `media/limite` | 413 / 429 | un tope o un ritmo; con `Retry-After` |

## 4. La credencial

- El SDK no guarda una cabecera: pide la credencial a un **proveedor** del agente cada vez que la
  necesita, y el proveedor la renueva antes de que caduque (0049, D3). Un flujo abierto con `open`
  sobrevive a la renovación: la credencial se comprueba al abrir.
- Un transform presenta además su **ámbito** (el puesto y el trabajo): la celda comprueba que la
  colección está entre sus entradas declaradas y lo registra en el linaje.
- **Cómo lo hace la celda** (0049 B4·2). El puesto se reconoce por el **agente** que lo reclamó,
  no por la cabecera `x-ore-puesto`, que el código de la celda puede quitar. Mientras corre un
  transform (`POST /puestos/{id}/transform`, o una función lanzada):
  - una colección que no está en sus `inputs` da `media/no-declarada`;
  - una que sí está se lee **de la transacción que su puntero tenía al declararla**: `list`,
    `stat`, `url` y `open` ven lo mismo aunque la colección cambie mientras el trabajo corre. La
    respuesta de declarar y la ficha del puesto lo enseñan (`fijadas: {<colección>: <transacción>}`).
  - Sin transform, se lee la transacción de ahora y la colección queda anotada en el puesto
    (`colecciones_leidas`): es la procedencia de lo que esa sesión escriba.

## 5. Lo que no es de este contrato

- Cómo se deriva (el registro de derivación, `Collection.apply()`, `retry_errors`): es B5, con la
  tabla anclada de v1alpha17 `03` como forma de salida; desde el SDK, en
  [`sdk.md`](sdk.md#incremental-derivation-apply).
- Qué función saca qué.
- Permisos por ítem (0047).
