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

Y una tercera, `conformidad.default.escrita`, vacía, para `put`.

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
- `op`: una de las siete (`list`, `stat`, `open`, `url`, `put`, `verify`) o `sesion` (la
  credencial) o `medida`.
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

| invariante | qué |
|---|---|
| `recorrido_completo` | recorrer el cursor da cada ítem una vez, ninguno repetido, y el total de la muestra |
| `una_transaccion` | todas las páginas de un recorrido llevan el mismo `as_of` |
| `coste_por_pagina_estable` | la última página no tarda más de 2× la primera (se registra además la medida) |
| `version_fijada` | la cabecera `ORE-Media-Version` coincide con la versión cuyos bytes llegaron |
| `lote_parcial` | el resultado de cada ítem va en su posición y el fallo de uno no cambia los demás |
| `idempotente` | repetir la operación da el mismo resultado y no crea nada nuevo |

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
| [`casos/medidas.json`](casos/medidas.json) | lo que se mide para fijar los umbrales de B6 |

## Un ejecutor

Lee `muestra.json`, la instala en una celda de prueba (las tres colecciones y el origen de la
virtual), corre cada caso de `casos/` en las colecciones de su `en`, y escribe un informe: por
caso, `pasa`, `falla` con el motivo, o `no-aplica` con el porqué (un ejecutor de SQL no corre
`open`). Una superficie es conforme cuando no tiene ningún `falla` y cada `no-aplica` está en su
lista declarada.

El primero es el de Python (B4); el de la ruta HTTP, con B2 y B3.
