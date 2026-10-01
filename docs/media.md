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
| Python | `ore.coleccion("b.s.c")` | `Item` | `MediaRef` (dataclass inmutable) |
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
- Python: `coleccion.items(prefijo=None, as_of=None)` → iterador perezoso por lotes, filtrable
  (`.where(tipo=…)`) antes de pedir nada más.

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
  `ore-medios/contenido?permiso=…`, un permiso opaco que vale para ese ítem en esa versión
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
  ítem grande por rangos en paralelo; `ore.leer_varios(items, hilos=16)` muchos a la vez, dando
  `(item, datos, error)` —el error de uno no para los demás—. Excepciones por `type`:
  `MediaNoExiste`, `MediaSinPermiso`, `MediaCambiado`, `MediaCorrupto`, `MediaRango`.

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
POST /media/{b}/{s}/{c}/transactions                     → 201 { "transaction": "<t>" }
PUT  /media/{b}/{s}/{c}/transactions/{t}/items?path=     (el cuerpo en flujo)
     Content-Type: <declarado, opcional>
     Repr-Digest: sha-256=:…:  (opcional: si llega, se coteja)
→ 201 MediaRef
POST /media/{b}/{s}/{c}/transactions/{t}/commit | /abort
```

- Solo en una colección **escrita** (v1alpha16 `02` §3); en una mantenida es `media/no-escribible`.
- La celda calcula el `sha256` al paso, detecta el tipo por los bytes, y guarda el blob **por su
  digest**. Subir lo que ya está es gratis (idempotente por digest).
- El ítem existe cuando la transacción se confirma; un `abort` no deja nada.
- Una transacción no tiene tope de ítems (0049, D5: sin el de 10 000 de Foundry); se confirma
  entera o no.

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
| `media/origen` | 502 | el origen falló (con su código dentro) |
| `media/limite` | 413 / 429 | un tope o un ritmo; con `Retry-After` |

## 4. La credencial

- El SDK no guarda una cabecera: pide la credencial a un **proveedor** del agente cada vez que la
  necesita, y el proveedor la renueva antes de que caduque (0049, D3). Un flujo abierto con `open`
  sobrevive a la renovación: la credencial se comprueba al abrir.
- Un transform presenta además su **ámbito** (el puesto y el trabajo): la celda comprueba que la
  colección está entre sus entradas declaradas y lo registra en el linaje.

## 5. Lo que no es de este contrato

- Cómo se deriva (el registro de derivación, `aplicar`, `reintentar_errores`): es B5, con la
  tabla anclada de v1alpha17 `03` como forma de salida.
- Qué función saca qué.
- Permisos por ítem (0047).
