# Conformidad · la media en código

La suite que una superficie (el SDK de Python, el de SQL, después JVM y Node) tiene que pasar
para decirse conforme con [`docs/media.md`](../../docs/media.md) y con OOS v1alpha17. Es de
0049, B0: **se escribe antes que la implementación**, y la base entra como pieza (B6) cuando la
pasa en vivo.

Los casos son datos, no código: un ejecutor por lenguaje los lee y los corre contra una celda.
Ningún caso depende del lenguaje que lo ejecuta.

## La muestra

[`muestra.json`](muestra.json) declara dos colecciones con **los mismos ítems**:

- `conformidad.default.mantenida` — mantenida, en el lago;
- `conformidad.default.virtual` — virtual, en un origen versionado que el ejecutor controla
  (para los casos que cambian el origen a mitad).

Y una tercera, `conformidad.default.escrita`, vacía, para `put`. Y una cuarta,
`conformidad.default.derivar`, virtual y pequeña (`docs/a.pdf`, `docs/c.pdf` en `v1` e `img/b.png`),
la entrada de los casos de `put` derivado (0049 B9): sus pasos cambian, quitan y copian ítems de su
origen, y cada caso la reinstala.

Los bytes de cada ítem se dan en la muestra (texto o base64) o se generan (`generar`), así que
el ejecutor conoce el contenido exacto y **calcula él mismo** el `sha256` que espera: la suite no
escribe ningún digest a mano.

## Un caso

```json
{
  "id": "open-003",
  "norma": "docs/media.md §2 open · Range",
  "en": ["mantenida", "virtual"],
  "op": "open",
  "pide": { "path": "docs/a.pdf", "range": [0, 3] },
  "espera": { "status": 206, "bytes": { "de": "docs/a.pdf", "rango": [0, 3] }, "cabeceras": ["ETag", "Repr-Digest"] }
}
```

- `en`: en qué colecciones se corre; un caso con dos se corre dos veces. **Que la misma
  expectativa valga en la mantenida y en la virtual es el punto de 0049 D1.**
- `op`: una de las operaciones (`list`, `stat`, `open`, `url`, `put`, `verify`), `sesion` (la
  credencial), `medida`, o `apply` (`put` derivado, 0049 B9: un caso **por pasos**, abajo).
- `pide`: los argumentos, con los nombres del contrato.
- `antes` / `durante`: lo que el ejecutor hace a la muestra antes o a mitad del caso
  (`sobrescribir`, `caducar_credencial`, `put`).
- `espera`: el vocabulario de abajo, y nada más.

## Los huecos

Un valor entre llaves lo rellena el ejecutor con lo que la muestra instalada le dio:

| hueco | qué |
|---|---|
| `{coleccion}` | el nombre cualificado de la colección en que corre el caso |
| `{version:<ruta>:<id>}` | la versión que el origen dio a esa versión de la muestra (`v1`, `v2`) |
| `{t0}` | un `as_of` recordado con `antes.recordar_as_of` |
| `{version_en_t0}` | la versión que el ítem tenía en ese `as_of` |
| `{salida}` | en un caso de `apply`, una colección escrita **nueva** que el ejecutor crea para ese caso |

## Un caso por pasos (`apply`)

Un caso de `apply` lleva `pasos`, en orden, y cada uno puede llevar su `espera`. Un paso es uno de:

| paso | qué hace el ejecutor |
|---|---|
| `apply` | `collection.apply(fn, version=, output="{salida}", …)` sobre la colección del caso, con la función de ese nombre (abajo) y las opciones que traiga (`retry_errors`, `save_every_s`); `falla_en`: la función lanza en esos ítems; `cortar_tras_guardar: n`: el ejecutor corta la pasada tras el `n`-ésimo guardado |
| `sobrescribir`, `borrar`, `copiar` | cambian el **origen** de la colección (una nueva versión de un ítem, quitarlo, otra ruta con los mismos bytes), y el ejecutor lo ingiere |
| `derivations` | `GET /media/…/derivations` de `{salida}` |

Las funciones, con nombre para que ningún caso dependa del lenguaje:

| función | qué da |
|---|---|
| `lineas_pdf` | por cada ítem `.pdf`, un fichero por línea de su contenido: `l<n>.txt` con la línea (sin el salto), anclado `{kind: page, page: n}`; nada para lo demás |
| `nombre_repetido` | por cada ítem `.pdf`, dos ficheros `x.txt`; nada para lo demás |

## El vocabulario de `espera`

| clave | se cumple si |
|---|---|
| `status` | el estado HTTP (o su equivalente en el SDK) es ese |
| `error` | el error es un problem+json de ese `type` (y en el SDK, su excepción o valor de error) |
| `bytes` | el cuerpo es exactamente el contenido de ese ítem de la muestra (o de ese rango, o de esa versión) |
| `ref` | la `MediaRef` tiene esos campos con esos valores; `"*"` es «presente y no nulo», `null` es «nulo» |
| `digest_de` | `digest` es `sha256:` del contenido de ese ítem, calculado por el ejecutor |
| `cabeceras` | esas cabeceras están |
| `sin_claves` | ninguna de esas claves aparece, a ningún nivel, en la respuesta (p. ej. `url` en un listado) |
| `sin_cadenas` | ninguna de esas cadenas aparece en el cuerpo ni en las cabeceras (p. ej. una firma, o lo que un 403 no debe revelar) |
| `sin_error` | todos los pasos de una `sesion` terminan sin error |
| `paths` | las rutas devueltas son exactamente esas, en cualquier orden |
| `current` | el `current` de `stat` es ese |
| `digest_de_lo_enviado`, `size_de_lo_enviado` | el `digest` y el `size` de lo que devolvió `put` son los de los bytes enviados, calculados por el ejecutor |
| `repr_digest_de` | la cabecera `Repr-Digest` es `sha-256` del contenido de ese ítem (RFC 9530) |
| `despues_en_list`, `despues_no_en_list` | tras el caso, un `list` de la colección los tiene o no los tiene |
| `caduca_en_s` | `expires_at` menos el instante de la petición no pasa de `max` |
| `errores_en` | en un lote, la posición `i` trae un error de ese `type` |
| `todos` | todos los resultados de un lote son ese (`ok`) |
| `linaje_incluye` | el linaje del trabajo registra esa colección, con su `as_of` |
| `invariante` | una propiedad con nombre, de la tabla de abajo |
| `mide` | no pasa ni falla: registra una medida con ese nombre (los umbrales se fijan midiendo) |
| `resumen` | el resumen que devolvió `apply` tiene esas claves con esos valores (las demás no se miran) |
| `ficheros` | los ítems actuales de `{salida}` son exactamente esas rutas |
| `ficheros_incluye`, `no_ficheros` | `{salida}` tiene, o no tiene, esas rutas |
| `origen` | el `source` de ese fichero es ese ítem de la entrada: su ruta en `uri`, y `digest` el del contenido, calculado por el ejecutor |
| `ancla` | el `source.anchor` de ese fichero es ese |
| `derivacion` | el `derivation` de ese fichero tiene esos campos con esos valores |
| `marcas` | la entrada del registro de ese origen tiene ese `state` (`empty`, `error`); `null`: no es una marca |
| `sin_escribir` | el `as_of` de `{salida}` no cambió con el paso |
| `bytes_de` | el contenido de ese fichero es ese texto |
| `stat_de` | un `stat` de esa ruta en `{salida}` da eso |
| `estados`, `ficheros_de` | en el registro leído, el `state` de cada origen, y cuántos ficheros nombra |

| invariante | qué |
|---|---|
| `recorrido_completo` | recorrer el cursor da cada ítem una vez, ninguno repetido, y el total de la muestra |
| `una_transaccion` | todas las páginas de un recorrido llevan el mismo `as_of` |
| `coste_por_pagina_estable` | la última página no tarda más de 2× la primera (se registra además la medida) |
| `version_fijada` | la cabecera `ORE-Media-Version` coincide con la versión cuyos bytes llegaron |
| `lote_parcial` | el resultado de cada ítem va en su posición y el fallo de uno no cambia los demás |
| `idempotente` | repetir la operación da el mismo resultado y no crea nada nuevo |
| `sin_recalculo` | en este paso, la función no se llamó sobre ningún ítem que una pasada anterior ya dejó confirmado (lo calculado y sin confirmar al cortarse sí se rehace) |

## Los casos

| fichero | qué cubre |
|---|---|
| [`casos/list.json`](casos/list.json) | sin bytes ni URLs; el cursor; una transacción; `as_of`; el prefijo; la referencia entera |
| [`casos/stat.json`](casos/stat.json) | `current`; lo que no existe |
| [`casos/open.json`](casos/open.json) | los bytes, iguales en mantenida y virtual; rangos; fijado; el cambio a mitad; el `sha256` al paso; el tipo por los bytes |
| [`casos/url.json`](casos/url.json) | el `ttl`; el lote parcial |
| [`casos/put.json`](casos/put.json) | el digest y el tipo los calcula la celda; idempotente; `abort`; el digest que no casa; donde no se escribe |
| [`casos/errores.json`](casos/errores.json) | problem+json; sin permiso; no declarada |
| [`casos/sesion.json`](casos/sesion.json) | la credencial que caduca a mitad de una lectura y de una celda |
| [`casos/derivar.json`](casos/derivar.json) | `put` derivado (0049 B9): la primera pasada, repetir, cambia y encoge, se va, la copia, la marca, el error y su reintento, el corte, otra versión, el nombre repetido, el registro; el linaje que no cuadra; `retire` |
| [`casos/medidas.json`](casos/medidas.json) | lo que se mide para fijar los umbrales de B6 |

## Un ejecutor

Lee `muestra.json`, la instala en una celda de prueba (las tres colecciones y el origen de la
virtual), corre cada caso de `casos/` en las colecciones de su `en`, y escribe un informe: por
caso, `pasa`, `falla` con el motivo, o `no-aplica` con el porqué (un ejecutor de SQL no corre
`open`). Una superficie es conforme cuando no tiene ningún `falla` y cada `no-aplica` está en su
lista declarada.

El primero es el de Python (B4); el de la ruta HTTP, con B2 y B3.
