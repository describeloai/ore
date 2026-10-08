# 0049 · Media paradigms in code repositories

**Estado:** propuesto · visión definida y estado del arte recogido (2026-09-30); **B0–B3 hechos**
—B2 y B3 en vivo en victor el 2026-10-01—: la gramática en OOS v1alpha17, el contrato de ejecución en
[`docs/media.md`](../media.md), la suite en [`conformidad/media`](../../conformidad/media/README.md),
`ore-medios` sirviendo y la puerta de lectura: una colección virtual se lee desde un puesto. B4,
B4b, **B5 y B7 hechos** —en vivo el 2026-10-03; B7 es su forma SQL—; **B9 hecho** —ficheros que
dan ficheros, en vivo el 2026-10-08—; B6, por construir. Nace de E10 C de 0046, que se promueve aquí: no es una pantalla de la consola sino el
uso de la media desde código, con su escritura, y toca el SDK, el puesto, ore-serve y la gramática.

## La pregunta

¿Qué tiene que ser verdad para que un code repository de ORE sea **el mejor lugar para trabajar
con media** —el más cómodo y el más flexible—? Trabajar con media es, sobre todo, **convertirla en
activos tabulares**: inferencia en lote, OCR, texto con maquetación, trozos y vectores para RAG,
clasificación, detección y segmentación, transcripción, eventos en audio y vídeo, entidades. En
cuatro paradigmas: SQL, Python, JVM y Node.

La respuesta de este documento es una **base común** —un modelo, un contrato y la plataforma que
los sostiene— sobre la que cada paradigma es una superficie idiomática. Se define **la base ideal**
primero; se construye sin mirar atrás; y entra como una pieza cuando es una realidad comprobada.

## Cómo se lee

De lo más abstracto a lo más concreto. Cada nivel solo depende de los de arriba:

| nivel | qué fija | quién lo tiene que leer |
|---|---|---|
| **0 · principios** | qué es la media para el código | todos |
| **1 · el modelo** | la referencia, las anclas, la tabla de resultados, la identidad | todo lo que escribe o lee resultados |
| **2 · el contrato** | las siete operaciones y su semántica | cada superficie |
| **3 · la plataforma** | las siete decisiones que lo hacen posible | ORE |
| **4 · las superficies** | Python y SQL primero; JVM y Node después | quien programa |

Detrás: lo que hay hoy (medido), cómo se construye y cómo entra, qué se acepta a cambio, y el
estado del arte en que se apoya (resumido aquí; entero en
[`docs/investigacion/e11-media-en-codigo-estado-del-arte.md`](../investigacion/e11-media-en-codigo-estado-del-arte.md)).

## La visión

Una persona abre un repositorio, arrastra la colección `legal.default.contratos` a una celda y
escribe:

```python
@ore.modelo(gpus=1, lote=16)
class Paginas:
    def __init__(self):            # una vez por trabajador
        self.m = cargar("docling")
    def __call__(self, item):      # 1 ítem → N anclas
        with item.open() as f:     # fijado a su versión; rangos si hace falta
            for bloque in self.m.convertir(f):
                yield ore.Ancla.region(bloque.pagina, bloque.bbox, texto=bloque.texto)

@transform(inputs=[ore.coleccion("legal.default.contratos")], output="legal.default.bloques")
def bloques(contratos):
    return contratos.items().where(tipo="application/pdf").aplicar(Paginas(), en_error="registrar")
```

Y ocurre esto, sin que lo pida:

- Solo se procesan los ítems **nuevos o cambiados**, y los que cambiaron de modelo o de parámetros;
  los que fallaron se reintentan con `reintentar_errores`, y su error es una fila, no un job caído.
- La tabla `bloques` tiene **tipos de verdad** —página, caja con su sistema de coordenadas, texto,
  confianza, procedencia— y se consulta en SQL como cualquier otra.
- Su **linaje** dice qué colección, en qué transacción, con qué función y modelo.
- La colección es **virtual** (vive en el S3 del cliente) y da igual: el código lee igual una
  virtual que una mantenida.
- Una celda de dos horas no pierde el acceso; recorrer cien mil ítems no se degrada.
- `estimar()` dijo antes cuántos ítems, páginas y cuánto coste.

En SQL, la misma colección es una tabla —su listado— y un resultado se une a su ítem por la
referencia. En Java y Node, lo mismo con el handle de cada lenguaje.

## Nivel 0 · Los principios

Lo que el estado del arte repite en los cuatro paradigmas (fuentes en el anexo):

1. **Tres capas: referencia → handle → bytes.** Listar no lee contenido. La referencia viaja por
   la tabla; el handle se abre cuando se pide; los bytes llegan al final. Quien las aplana (Spark
   `binaryFile`, `read_files` con `content`) paga un techo de ~2 GiB y todo en memoria.
2. **La referencia es un valor tipado**, no una cadena ni una URL.
3. **La identidad es del contenido, y la da la plataforma.** Ningún almacén garantiza que su
   referencia siga al origen; ni el ETag ni un CRC son identidad.
4. **Se lee fijado, o se falla.** Mejor un error que otro contenido.
5. **La autorización la decide el catálogo; la URL solo transporta.** Una URL firmada es
   derivada, corta y al portador: nunca se guarda.
6. **Un resultado sobre media está anclado** a una parte del medio (página, región, intervalo,
   rango de texto), con su geometría declarada.
7. **El error de un ítem es un dato del ítem**, no del job.
8. **Lo derivado sabe de qué deriva**: contenido, función, versión, modelo, parámetros. Si algo de
   eso cambia, se recalcula; si no, no.
9. **La media se trata según su tipo** (lo que admite un PDF no es lo que admite un vídeo), y el
   tipo se detecta por los bytes, no por la extensión.

## Nivel 1 · El modelo

### La referencia: `MediaRef`

La forma es la del tipo lógico **`FILE` de Parquet** (`uri`, `offset`, `size`, `content_type`,
`checksum`, `inline`), que Databricks ya usa (`FILE`, Beta) e Iceberg propone para v4
(apache/iceberg#17919). ORE la adopta con sus campos:

| campo | estándar | regla |
|---|---|---|
| `uri` | Parquet `FILE.uri` | `ore://<base>.<schema>.<colección>/<ruta>?v=<versión>` |
| `collection`, `path` | — | el localizador lógico; la ruta **no** es identidad |
| `version` | S3 `VersionId`, GCS generation | fija los bytes |
| `digest` | descriptor OCI, RFC 9530 | `sha256:<hex>`; la identidad del contenido (abajo) |
| `size` | Parquet `size`, OCI | obligatorio |
| `content_type` | RFC 6838 | el tipo efectivo |
| `content_type_detected` | WHATWG MIME Sniffing | por firma; si contradice al declarado, se marca, no se corrige solo |
| `checksum` | Parquet `checksum`, S3 `x-amz-checksum-*` | CRC64NVME: validador y deduplicación, **no** identidad |
| `annotations` | OCI `annotations`, Dublin Core, Exif/XMP | lo técnico y barato (ancho, alto, duración, páginas); la posición GPS, no por defecto |

Nunca en una `MediaRef`: la URL firmada ni quién autoriza.

### La identidad

- **Identidad = `digest`** cuando se conoce. Dos referencias con el mismo `sha256` son el mismo
  contenido, cambien la ruta, el ETag o el checksum.
- **Mientras no se conoce** (una virtual que aún no se ha leído entera), la identidad es el
  **localizador fijado** `(colección de origen, clave, VersionId)`: en un bucket versionado es
  inmutable. El `digest` se calcula **al paso** la primera vez que los bytes cruzan la celda
  (D1), sin lectura extra.
- Un bucket sin versiones no da identidad fuerte: se dice, y la referencia lleva `checksum` y
  `etag` como validadores.

### Las anclas: un selector tipado

Alineado con **W3C Web Annotation** (target = fuente + selector + estado) y sus equivalentes de
URL (Media Fragments, RFC 8118 para PDF, rangos RFC 9110):

| ancla | campos | equivale a |
|---|---|---|
| `Item` | — | el medio entero |
| `Pagina` | `n` | `#page=n` |
| `Region` | `pagina?`, `bbox {x, y, w, h}`, `poligono?`, `sistema` (px, pt, normalizado + ancho y alto) | `#xywh=`, COCO, Docling `prov` |
| `Intervalo` | `t_ini`, `t_fin` (s) | `#t=`, WebVTT, segmentos de Whisper |
| `Fotograma` | `t`, `indice?` | — |
| `Texto` | `char_ini`, `char_fin` (sobre un texto derivado con su id) | `TextPositionSelector` |
| `Bytes` | `offset`, `length` | `Range`, Parquet `offset/size` |

Cada herramienta ancla con su geometría (Unstructured, polígono antihorario; marker, horario;
Docling, `bbox` por página): **se normaliza al entrar**, y el sistema de coordenadas va declarado.

### La tabla anclada

La forma canónica de **todo** resultado sobre media. Una fila por ancla:

| grupo | columnas |
|---|---|
| referencia | `item: MediaRef` |
| ancla | `ancla_id` (determinista: sha256 de ítem + tipo + ancla + función), `ancla_padre`, `ancla: Ancla` |
| carga | `etiqueta`, `texto`, `valor` (struct tipado por el esquema de la función), `vector: Vector<Float32, n>`, `confianza` |
| procedencia | `fn`, `fn_version`, `modelo`, `modelo_rev`, `params_hash`, `ejecucion`, `creado` |
| estado | `estado` (`ok`, `error`), `error_tipo`, `error_msg`, `intentos` |

COCO, WebVTT, ALTO, hOCR y PAGE XML son **exportaciones** de esta tabla, no su almacén. El
documento entero que devuelva un parser (p. ej. el JSON de Docling) puede guardarse como ítem de
una colección de salida, referenciado desde sus filas.

### Los tipos que esto exige

`Struct`, `List<Struct>`, `Vector<Float32, n>` (dimensión fija), `MediaRef` y `Ancla`, en la
gramática (`ore-core`), en la escritura del lago y en su lectura por SQL. Sin ellos, cualquier
resultado es JSON en un string: el defecto que el estado del arte señala en Foundry.

## Nivel 2 · El contrato

Siete operaciones, las mismas en los cuatro paradigmas; el nombre y el handle, idiomáticos:

| operación | semántica |
|---|---|
| `list(colección, prefijo?, as_of?)` | el listado como filas de `MediaRef`, **sin bytes**, por cursor; `as_of` es la transacción de la colección |
| `stat(ref)` | metadatos frescos; dice si la versión sigue siendo la actual |
| `open(ref)` | un flujo **fijado** a la versión; verifica tamaño y, si se conoce, `digest` al terminar; cerrar a medias no descarga el resto |
| `read_range(ref, offset, length)` | 206 con validador fuerte; si el origen no da rangos, lo dice |
| `url(ref, ttl)` | derivada, corta, al portador; para quien necesite HTTP (un navegador, un modelo externo) |
| `put(bytes \| flujo, ruta, tipo?)` | sube, calcula `sha256` al paso, detecta el tipo; idempotente por `digest`; dentro de una transacción |
| `verify(ref)` | recalcula el `digest` |

Y cuatro reglas que valen para todas:

- **Fijado:** un transform lee una colección **en una transacción** (`as_of`) y la registra.
- **Error por ítem:** una operación sobre un ítem que falla devuelve su error como valor
  (`{valor, error}` en SQL; `en_error="registrar"` en código). Solo un fallo de la plataforma
  aborta.
- **Derivación:** todo lo que produce la tabla anclada lleva la clave
  `(identidad, fn, fn_version, modelo_rev, params_hash)` y el estado `ok | error | pendiente`.
- **Lo que no se hace en SQL**: ordenar o agrupar por la referencia, servir rangos, transcodificar.
  Eso es del handle.

## Nivel 3 · La plataforma: las siete decisiones

Cada una dice qué se decide, qué se descarta y qué se acepta a cambio.

### D1 · La media se lee a través de la celda, no del origen

El código lee cualquier ítem —mantenido o virtual— por una **puerta de lectura de la celda** que
el puesto alcanza: para una mantenida, firma el blob del lago; para una virtual, lee el origen con
la credencial de la celda (el rol de E9b), **en flujo, fijado a la versión**, sirve rangos, calcula
el `sha256` al paso y puede dejar el ítem en el lago como caché (lo que Foundry llama *access
patterns* con persistencia).

- **Descartado:** abrir la salida del puesto a S3 (el puesto dejaría de estar aislado y el código
  del usuario tocaría la credencial del cliente); materializar toda virtual (dejaría de serlo).
- **A cambio:** los bytes de una virtual cruzan la celda —coste de red y CPU, y un servicio más que
  dimensionar—.

### D2 · Servir es un servicio

Un **índice de ítems** consultable por ruta, identidad y cursor sin leer el manifiesto entero; la
firma, en proceso; sin procesos ni `git fetch` por petición; lotes grandes; réplicas.

- **Descartado:** seguir con `ore` como subproceso por petición (1,4–2,1 s medido, crece con el
  cuadrado del recorrido).
- **A cambio:** el índice es estado nuevo que mantener coherente con las transacciones de la
  colección.

### D3 · La credencial del código se renueva sola

El SDK pide su token cuando va a caducar, también en mitad de una celda, y una celda larga da
latido.

- **Descartado:** alargar la vida del token (más ventana si se filtra).
- **A cambio:** el agente expone al SDK un proveedor de credencial, no una cabecera fija.

### D4 · Tipos nativos antes que cualquier función de media

`Struct`, `List<Struct>`, `Vector<Float32, n>`, `MediaRef`, `Ancla`: en la gramática, en la
escritura del lago (Iceberg), en DuckDB y en la consola.

- **Descartado:** JSON en un string como forma transitoria (se queda para siempre).
- **A cambio:** una versión de la gramática y el trabajo de mapear cada tipo en el lago y en SQL
  antes de ver un solo OCR.

### D5 · Un solo modelo incremental por ítem

Un **registro de derivación** por clave `(identidad, fn, fn_version, modelo_rev, params_hash)` con
estado `ok | error | pendiente`: procesar solo lo pendiente, `reintentar_errores`, recalcular al
cambiar de versión, y mover una ruta sin cambiar el contenido no recalcula nada.

- **Descartado:** el checkpoint opaco (Databricks: da por visto lo que falló; identidad por ruta)
  y dos políticas de transacción (Foundry: tope de 10 000 frente a sin snapshot).
- **A cambio:** el registro es otra tabla por salida, y la versión de una función la declara quien
  la escribe (o se deriva del código).

### D6 · La colección es una entrada declarada

`inputs=[ore.coleccion(...)]`: el linaje la registra con su transacción y el permiso se comprueba
en el catálogo al abrir el trabajo; toda lectura de media desde código pasa por ahí.

- **Descartado:** leer por ruta o desde dentro de una función sin declarar (Databricks: se sale del
  linaje); que el permiso dependa de declarar la entrada (Foundry: acceso y linaje mezclados).
- **A cambio:** una lectura no declarada se rechaza en un transform (se permite en una sesión
  interactiva, y se registra).

### D7 · Cómputo para IA

Perfiles de recurso **por trabajo** (CPU, memoria, GPU), paralelismo dentro del puesto (varios
trabajadores con el modelo cargado una vez), y **los pesos de un modelo como activo del lago** (el
puesto no sale a internet, y así debe seguir).

- **Descartado:** un tamaño de puesto para todo.
- **A cambio:** la GPU es infraestructura y coste nuevos (hoy la cuota de la región es 0); **cuándo
  entra es una decisión aparte**. La base no depende de ella: todo lo anterior funciona en CPU.

## Nivel 4 · Las superficies

### Python (primero)

- La colección como entrada; `items()` perezoso, por lotes, filtrable sobre el listado.
- El handle: un objeto fichero con `seek` y rangos (`io.RawIOBase`) que pide su acceso al leer y lo
  renueva.
- `@ore.modelo`: estado por trabajador (`__init__`/`__call__`), recursos, lote, reintentos,
  `en_error`; el esquema de salida, de los tipos o de Pydantic.
- `aplicar()`: generador 1 → N, incremental por D5, que escribe la tabla anclada.
- **Operaciones por tipo de medio**, listas: texto con maquetación y páginas a imagen (documento),
  OCR y embeddings (imagen), transcripción con segmentos (audio), escenas y fotogramas (vídeo).
  Cada una escribe la tabla anclada con sus columnas fijas.
- `estimar()`: ítems, páginas, minutos y coste, en seco.
- Después, el bucle de Document Intelligence de Foundry: comparar estrategias sobre una muestra,
  medir calidad, tiempo y coste, y desplegar la ganadora como transform versionado.

### SQL (acotado)

- **Crear** una colección escrita (B4·4): `create media collection [if not exists] b.s.c media
  document formats (pdf) [comment '…']`, como `create volume` de Databricks con lo que una colección
  necesita además. Es el verbo de Python (`crear_coleccion`), de quien la crea (0052); llenarla es
  del código, y su `derivedFrom` lo escribe el servidor al confirmar cada transacción.
- La colección es una tabla: su listado, con `item: MediaRef` y sus metadatos.
- Escalares `stat(item)`, `url(item, ttl)`; funciones de tabla con `LATERAL` para lo que da N filas.
- `{valor, error}` siempre, sin opción.
- Materializar como vista mantenida incremental con la clave de D5; `reintentar_errores(t)`.

### JVM y Node (al final)

La misma `MediaRef`, la misma tabla anclada, el mismo contrato. El handle: `SeekableByteChannel`
con `close` y `abort` distintos en la JVM; `Blob` y `ReadableStream` web en Node, con el trabajo
pesado fuera del bucle de eventos.

## Lo que hay hoy (auditado el 2026-09-30)

| | hallazgo |
|---|---|
| ⛔ | una **virtual no se lee desde código**: la URL apunta al S3 del cliente y el puesto solo sale a Google (`malla/21-el-puesto.yaml`, a propósito); E9·4 funcionó con una mantenida |
| ⛔ | **servir** lanza `ore` → `ore-store pagina` + `blob-firmar` y un `git fetch`, y `pagina` lee el manifiesto entero (`ore-store/src/ciclo.rs:1802`): 1,4–2,1 s por llamada; ore-serve, 1 réplica y 500m |
| ⛔ | el **token** del agente dura 300 s (`malla/61-realms.yaml`) y se renueva solo entre celdas (`puesto/python/agente.py:403,431`) |
| ⛔ | **sin GPU** ni recursos por trabajo (1–2 CPU, 2–4 Gi fijos; una celda cada vez) |
| ⛔ | **sin tipos** de resultado: `write()` rechaza listas, structs y binario (`puesto/python/ore/__init__.py:637-663`) |
| △ | `@transform` no admite una colección; `media()` no pasa por el linaje; sin listado de ítems en el SDK |
| △ | sin incremental, sin error por ítem, sin memoización; `write()` rechaza una tabla vacía |
| △ | `over()` materializa la tabla entera; la huella es CRC64NVME (`sha256` solo en las mantenidas) |
| ✓ | la cola con cuota por inquilino; ore-serve no pasa bytes; blobs por `sha256` con 32 hilos; lectura fijada con `If-Match` (412); procedencia de lo escrito; DuckDB con tope y derrame; S3 por rol (E9b) |

## Cómo se construye, y cómo entra

La base se construye **completa y aparte**, con su contrato escrito primero y una **suite de
conformidad** que cualquier superficie tiene que pasar. No se parchea el camino de hoy
(`media`/`medias`), que sigue sirviendo hasta el relevo.

| paso | qué | decisión |
|---|---|---|
| **B0 · el contrato** ✅ | la `MediaRef`, las anclas, la tabla anclada y las siete operaciones, como especificación (OOS v1alpha17) y como suite de conformidad (36 casos, neutrales al lenguaje) | niveles 1–2 |
| **B1 · los tipos** ✅ | `Struct`, `List<Struct>`, `Vector`, `MediaRef`, `Ancla` en la gramática (v1alpha17, conformance 30/30), el lago (ids por hijo, upsert, cambio de forma; 0032 T6) y SQL (DuckDB los lee nativos) | D4 |
| **B2 · servir** ✅ | el índice de ítems y la firma en proceso; la credencial que se renueva | D2, D3 |
| **B3 · la puerta de lectura** ✅ | leer mantenidas y virtuales por la celda: flujo, rangos, fijado, `sha256` al paso | D1 |
| **B4 · la entrada** ✅ | la colección en `inputs`, `items()`, el handle, el linaje | D6 |
| **B4b · la colección escrita** ✅ | crear colecciones nuevas **desde la instancia**, en SQL (`create media collection …`) y en Python (`ore.crear_coleccion(…)`), de quien las crea (0027), y llenarlas: `put` con transacciones (`docs/media.md` §2), el `sha256` al paso, el tipo por los bytes, el linaje en el puntero | D4, D6 |
| **B5 · la derivación** | el registro por clave, `aplicar()`, `reintentar_errores`, la tabla anclada | D5 |
| **B6 · el relevo** | la suite pasa en vivo; la base entra como pieza y `media`/`medias` pasan a ser azúcar sobre ella | — |

**B4b, por qué aparte** (anotado el 2026-10-01): el kind ya existe —`MediaCollection` de
v1alpha16 tiene dos formas, la **mantenida** (`from`, la llena el sistema desde un `ObjectTable`,
y con `virtual` sirve del origen) y la **escrita** (sin `from`, la llena código)—, y la consola ya
crea mantenidas y virtuales. Lo que falta es la escrita **desde código**: declararla en el árbol
desde una celda (como `crear_dataset` y `crear_vista` declaran hoy su documento por Forge) y
escribir sus ítems, que es la mitad de lo que hace falta para que una derivación deje media
(páginas como imagen, recortes, audio troceado) y no sólo tablas. Era E10 de 0046 («crear una
colección desde SQL y código») y entra aquí, detrás de B4.

Después, sobre la base: las operaciones por tipo de medio, `estimar()`, SQL, el bucle de
evaluación y, cuando se decida, D7 con GPU. JVM y Node, al final.

**Entra como pieza cuando** (cada objetivo se fija midiendo en B0, no antes):

- una colección **virtual** se lee desde un puesto, por rangos, fijada a su versión;
- se recorre el listado de una colección grande por cursor sin que cada página cueste más que la
  anterior;
- una celda de más de una hora conserva el acceso;
- una tabla anclada con `Region`, `Vector` y `valor` tipado se escribe desde Python y se lee desde
  SQL;
- una segunda pasada sin cambios no procesa nada, y un error se reintenta solo;
- el linaje de la salida nombra la colección y su transacción.

### B2·0 · lo medido (2026-10-01, sonda desechable)

**Dónde vive**: un servicio nuevo en la celda, **`ore-medios`** (decidido): enlaza `ore-store`, es el
único que toca el lago y los orígenes, y ore-serve sigue sin TLS y decide quién puede. En B3 es
también la puerta de lectura.

| pregunta | medido | qué decide |
|---|---|---|
| **firmar** (hoy) | `ore-store-gcs blob-firmar` en el pod de ore-serve (500m): 1 URL 0,2 s; 10, 4,9 s; **100, 52 s**. En un pod de 2 CPU: 100, 14 s | firmar es el cuello de botella, no el listado |
| **por qué** | `ore_gcp::cliente()` crea un cliente nuevo en cada llamada: cada `signBlob` paga TCP + TLS (y la carga de los certificados del sistema) en 32 hilos, contra la CPU del pod. El token sí se reusa | — |
| **firmar con la conexión reusada** | `signBlob` desde victor con la misma cuenta: conexión nueva, 100 en 1,76 s; **reusada, 17 ms la una, 100 en 0,69 s, 500 en 1,55 s** (32 a la vez, ~320/s) | `ore-medios` firma con un cliente vivo y reusado; 75× sobre hoy. Cachear URLs hasta poco antes de caducar es un extra, no la solución |
| **cargar el índice** | el listado de una colección (las 14 columnas de hoy), en proceso, release, lago local: 10 k filas, 11 ms y 5 MB; 100 k, 70 ms y 54 MB; **1 M, 0,63 s y 451 MB**, más 1,15 s de índice. Una página por cursor y 1000 búsquedas por huella: **< 1 ms** | el índice cabe en memoria por transacción hasta el orden del millón; por encima, proyectar sólo las columnas que sirven y guardar posiciones en los arrays en vez de copiar cadenas, con un tope de memoria y desalojo por colección |
| **enterarse de una transacción nueva** | ya medido (0046 E5b·2): un `fetch` del espejo vivo 0,07–0,10 s, un `worktree` 0,08 s | `ore-medios` no toca git: ore-serve, que ya pone el árbol al día en cada petición, le pasa el `metadata_location` del puntero de esa rama, y el índice se guarda con esa clave. Siempre fresco, y vale para ramas |
| **la credencial que se renueva** | una celda corre en el mismo proceso que el agente (`exec`), y el agente ya renueva su token a 60 s de caducar | el SDK pide la cabecera al agente en cada llamada (un proveedor), en vez de copiarla al empezar la celda; sin red nueva |

Sin medir (por el entorno): el índice leído del lago real en GCS (sumará la descarga de sus
Parquet: 45 MB para 1 M filas); el rendimiento del proveedor de credencial con un token de 30 s,
que espera a B2·3 en vivo.

### B2 · hecho, y lo medido en vivo (2026-10-01, victor)

`ore-medios` corre en la celda (su Deployment, malla `45-ore-medios.yaml`; la imagen de ore-serve
con otro comando) y ore-serve le habla por `/media/…`. Medido de punta a punta por la entrada
pública, con el token del agente, desde fuera del clúster (incluye la ida y vuelta a europe-west1
y el `fetch` del árbol de ore-serve):

| operación | antes (0046, `ore` por petición) | ahora |
|---|---|---|
| `list`, una página | 1,4–2,1 s, y crecía con el recorrido | **0,23 s**; la segunda página, lo mismo (cursor) |
| `stat` | — | 0,23 s |
| `url`, 1 / **100** | 0,2 s / **52 s** | 0,35 s / **1,27 s** |
| `content` (B3·3) de una mantenida | — | 307 en 0,27 s al blob del lago, que da `206 %PDF-` en 0,31 s |
| `content` de una virtual | — | 307 a `ore-medios` con permiso: 0,90 s la primera (custodio + STS), 0,23 s después |

Y la puerta de lectura de una virtual, desde el pod de ore-serve (la red del puesto es B3·4): `206`
con `content-range: bytes 0-4/717` y `%PDF-`; tras una lectura entera, **`stat` da el `sha256` visto
al paso** (`open-007`) y es el mismo que el del blob de la copia mantenida de ese contrato
(`e17f225d…`): dos caminos, la misma identidad. Un permiso inventado, `401`.

**Lo que costó desplegarlo**: con `ore-medios` en el mismo `cargo build` que los demás binarios, la
máquina de Cloud Build (8 GB) se quedaba colgada en la fase de enlace —LTO completo, otro enlace
pesado en paralelo— y cada construcción moría a los 60 min (`INTERNAL_ERROR`; tres, y bloqueó el CI
de todas las sesiones). Se enlaza aparte, solo, después del resto: +4 min por construcción.

**Hallazgo**: el listado no traía `size` (cerrado en B3·6a, abajo).

### B3 · hecho: la puerta de lectura (2026-10-01, victor)

**El criterio** —*una colección virtual se lee desde un puesto, por rangos, fijada a su versión*—,
cumplido desde un puesto de victor abierto por una persona: `ore.coleccion("s3_foreign_contract…")`
y `read_bytes()` de sus cuatro contratos, que viven en el S3 del cliente (eu-north-1) sin copia
nuestra. El puesto no alcanza S3 (comprobado); los bytes pasan por `ore-medios` con la credencial
de la celda (el rol de 0046 E9b). Los cuatro `sha256` calculados al paso **son los de los blobs de
la copia mantenida** de los mismos ficheros: dos caminos, la misma identidad. El rango, en vivo:
`206`, `content-range: bytes 0-4/717`.

**Lo medido antes de construir (B3·0, pod en t-victor con el rol)**: asumir el rol 0,38 s (1 h);
**~125 ms por petición** a eu-north-1, que es lo que cuesta cada `seek`; un flujo 59 MB/s, ocho
rangos en paralelo 114 MB/s; el `sha256` al paso −8 % en Python; `If-Match` con el ETag fija
(`206`/`412`); los objetos subidos antes de activar el versionado tienen `versionId` `null`, y S3
lo acepta para fijar (medido). Sin checksums guardados en el origen.

**Lo construido:**

| paso | qué | commit |
|---|---|---|
| B3·1 | `ore-medios` lee el origen fijado (`versionId` —también `null`— e `If-Match`), entero o por rango, en flujo, con el `sha256` al paso y su verificación; `Salida::Bytes` en `ore-entrada`; `ore-firmar-s3` fija también `null` (E9 servía la actual bajo la referencia de la vieja) | `0529cb2` |
| B3·2 | el **permiso**: un identificador opaco y aleatorio que apunta a lo guardado en `ore-medios` (el ítem fijado y la credencial temporal), 5 min, todos los rangos de UN ítem; sin secreto compartido | `430d275` |
| B3·3 | `GET /media/…/content` en ore-serve: decide y contesta **307** —al blob firmado del lago (mantenida) o a `ore-medios` con el permiso (virtual)—; la fuente sale del árbol (colección → `objectTable` → `datasource` → `connectionEnv`) | `ec284bc` |
| B3·4 | la red: el puesto llega a `ore-medios` **sólo a su puerto del contenido** (8098), y el resto (8097: índice, firma, permisos) sigue siendo sólo de ore-serve | `1c9f538`, `8b51d45` |
| B3·5 | el SDK: `ore.coleccion`, `items()`, `stat()`, `item.open()` (flujo desde el cursor; sólo un `seek` abre otra petición; sin el token de ORE en la URL de los bytes), `read_bytes()` por rangos en paralelo, `leer_varios()`, excepciones por `type` | `697552b` |
| B3·6a | el `size`: el índice leía como texto columnas enteras y lo perdía en todos los ítems; el SDK además lo aprende de la respuesta; la media lleva la rama del puesto | `10ff627` |

**Lo que se encontró por el camino** (cada uno, medido y cerrado):

- **Un agujero, en la primera red**: abrir al puesto el 8097 le daba también `/indice/*`, que confía
  en quien llama; una celda con un `metadata_location` podía listar y firmar sin pasar por las
  concesiones. Encontrado con la sonda del puesto, cerrado a mano en minutos y luego por diseño:
  **dos puertos**, y la red los separa (la sonda: 8098 `/contenido` 401, `/indice/*` 404; 8097,
  cortado).
- **El CI**: `ore-medios` en el mismo enlace LTO que los demás colgaba la máquina de Cloud Build
  (B2, arriba).
- **El `size`**: un fallo de B2·1 que sólo una celda de verdad (`seek(-8, 2)`) sacó.

**Sin probar, y por qué**: `open-006` (cambiar el objeto a mitad de una lectura) necesita escribir en
el bucket del cliente, y la credencial de prueba es de sólo lectura; el `412` está probado contra
un S3 de mentira y en vivo con un ETag alterado. El `sha256` visto al paso vive en memoria de
`ore-medios` (se olvida al reiniciar): escribirlo en el manifiesto es de B5.

### B4 y B4b · hechos: la colección entra y se escribe desde código (2026-10-02/03, victor)

**El criterio** —*una colección se declara como entrada de un transform y se lee fijada; una
colección escrita se crea desde la instancia, en Python y en SQL, y se llena por transacciones con
su linaje*—, cumplido en puestos de victor abiertos por una persona, en la rama `test4`:

- **Python (P5, `ceb4a0e` del árbol):** `ore.crear_coleccion(…)` sin dueño da `owner: user:victor`;
  un transform con `inputs=[ore.coleccion("s3_foreign_contract.nueva_carpeta.contratos")]` copia
  sus cuatro PDF con `transaccion().put`; el puntero lleva la procedencia y `fijadas`
  `{contratos: "1"}`; los cuatro `sha256` leídos de vuelta coinciden. Negativas, 3/3: una colección
  no declarada (celda), un `409` del SDK y uno del servidor.
- **SQL + linaje (B4·4, `1f8beaa` del árbol):** `create media collection if not exists
  s3_standard.nueva_carpeta.contratos_sql media document formats (pdf) comment '…'` la crea
  (`created`); el transform `copiar` la llena (4 ítems, 6 969 B, transacción 1) y, al confirmar, el
  servidor escribe en el documento `derivedFrom: [s3_foreign_contract.nueva_carpeta.contratos]` y lo
  sube a `v1alpha19`, en el mismo commit.

**Lo construido:**

| paso | qué | commit |
|---|---|---|
| B4·1 | ore-core habla v1alpha19: la colección escrita deriva (`spec.derivedFrom`), y un dataset puede derivar de una colección | `722850e` |
| B4·2 | lo declarado manda: declarar fija la transacción de cada colección de `inputs`; una no declarada es `403 media/no-declarada`; fuera de un transform, libre y anotada | `0d48825` |
| B4b·1 | `put` en ore-medios: transacciones, subida en flujo (5 GiB), el tipo por los bytes; sólo la celda escribe en el lago | `b7cba14` |
| B4b·2 | `POST /media/…/transactions`: ore-serve decide y ore-medios sella; los bytes no pasan por ore-serve; el puntero va a la rama del puesto con su procedencia | `9c38c95` |
| B4b·2′ | ore-medios con cuenta propia (`ore-medios-<celda>@`: lee, crea y toca blobs, nada más) | `d9b5413`, `514fc64` |
| B4·3/B4b·3 | el SDK: la colección como input, `crear_coleccion`, `transaccion()`/`put`/`put_varios`; el banco de la media | `5edf81f`…`08f2016`, `57c9f23` |
| B4·4 | el linaje lo escribe el servidor al confirmar (lo leído por el transform o la sesión; sin lecturas, fuera) | `828e9cd` |
| B4b·4 | `create media collection` en el guion SQL, el editor no lo marca, y una celda con ella es del árbol y no de DuckDB | `a2b58e6`, `d089986`, `2e36f1b` |

**Lo que se encontró por el camino:** el dueño de lo creado tenía que ser la persona (0052, `user:<handle>`
de ore-iam, migración 048); un puesto de Functions podía escribir (cerrado con la identidad declarada
del puesto, R1, y el cierre de su credencial); y los puestos huérfanos de la cola, que agotaban la
cuota (cerrado de raíz: cierre por inactividad y barrido, `eafdfd0`).

**Pendiente, y de B5:** el `sha256` de un ítem virtual sólo se conoce al leerlo (vive en memoria de
`ore-medios`); la identidad por contenido de una virtual sin leer es su `path@version`.

### B5 · hecho: la derivación incremental por ítem (2026-10-03, victor)

**El criterio** —*una función sobre una colección se calcula una vez por ítem y por clave; lo que no
cambió no se recalcula ni se reescribe, y el resultado es una tabla anclada*—, cumplido en un puesto
de victor (`pytransformsv1`, rama `test4`) con `pruebas-de-fuego/b53-la-derivacion-en-vivo.py`, ya
con el SDK en inglés: `ore.collection(…).apply(paginas, version="1")` dentro de un `@transform` cuya
entrada es la colección escrita de B4 (4 PDF) y cuya salida es
`s3_standard.nueva_carpeta.paginas_b5`.

- **Primera:** 4 nuevos, 4 filas, escrita; la tabla trae las seis columnas de sistema de v1alpha17
  `03` (`_item`, `_anchor`, `_anchor_id`, `_anchor_parent`, `_derivation`, `_status`) y la carga.
- **Otra vez:** 4 saltados, **no se escribe** (ni un commit).
- **Una copia de un contrato con otra ruta:** el mismo ítem (la identidad es el `digest`): 4
  saltados, 0 recalculados, y **no se escribe**; la fila conserva su ruta. La primera pasada en vivo
  sí escribía —el ítem tomaba la primera ruta del listado y la copia salía antes—: arreglado en
  `60f1e44` (una ruta que la fila ya dice manda mientras siga; si se va, la copia la hereda) y
  vuelto a correr en vivo.

| paso | qué | commit |
|---|---|---|
| B5·0 | medida: la tabla anclada desde una celda (100 k filas: escribir 14,2 s, leer 2,7 s; ~6 s fijos por escritura) | `40daf49` |
| B5·1 + B5·2 | `write(…, anchored_to=)` declara el Dataset anclado (v1alpha19 `anchoredTo`, sólo la carga); `apply()`: la tabla de salida es el registro, por clave; un error es su fila; nada cambia, nada se escribe; guardado por tiempo | `6504902` |
| B5·3 | en vivo; una copia no mueve el ítem (caso 13 del banco) | `60f1e44` |

**Queda fuera, y anotado:** `apply()` en Node y en la JVM (D5 lo pide; van al final, con su
superficie); que `apply()` emita **ficheros** a una colección escrita, y no sólo filas; y su forma en
SQL —materializar una tabla anclada desde una colección—, que es el siguiente paso de este ADR.

### B7 · hecho: la tabla anclada desde SQL (2026-10-03, victor)

La forma SQL de `apply()`. **No es una vista**: el resultado de aplicar funciones a ficheros no se
recalcula con un plan, es un `Dataset` escrito por un transform y anclado a su colección. Por eso la
frase es la que ORE ya tiene para escribir un dataset, y lo único nuevo es qué cabe en el `FROM`:

```sql
-- transforms/paginas.sql
create or replace dataset legal.archivo.paginas as
select p.page, p.texto, p.anchor
from legal.archivo.contratos as c
cross join lateral legal.funciones.paginas(c.item) as p
where c.content_type = 'application/pdf';
```

- **Una colección es una relación en `FROM`**, sin decir su tipo, como una vista o un dataset:
  `item` (`Media<colección>`), `path`, `digest`, `size`, `content_type`, `modified`.
- **Las `Functions` (0050) se llaman desde SQL**: escalares, `f(c.item)`, y de tabla,
  `cross join lateral f(c.item)`; tipadas por su contrato.
- **Si el `FROM` lee una colección, el dataset sale anclado a ella** (`anchoredTo`) y se calcula
  por `apply()`: la consulta, ítem a ítem, con el mismo registro por clave. `or replace` dice la
  verdad: el resultado es el de recalcularlo todo; lo incremental sólo lo abarata. La versión es la
  consulta normalizada más la de cada función; el ancla, la columna `anchor` si la hay.
- **Límites de la primera versión**, cada uno con su error: una sola colección y ningún `join` con
  otra relación; sin agregados, ventanas, `order by` ni `limit`; funciones de Python sin `over` ni
  `models`; `insert into` desde una colección, no.

| paso | qué |
|---|---|
| B7·1 | la colección en `FROM`: ore-core la acepta en un `select`, ore-serve la resuelve, el SDK la registra en DuckDB, el editor la conoce |
| B7·2 | las `Functions` en SQL: resueltas contra el árbol, reescritas a nombres internos, registradas con los tipos de su contrato |
| B7·3 | `create or replace dataset … as select` que lee una colección: anclado, por `apply()`, con sus límites |
| B7·4 | en vivo, un `.sql` en `pytransformsv1` |
| B7·5 | docs, y esta sección pasa a «hecho» |

Fuera: ficheros que dan ficheros (`insert into media collection … select …`, hito 3), `join` con
otras relaciones (con la versión del dataset en la clave), Functions de TypeScript desde SQL.

**Lo comprobado en vivo** (puestos de victor, `pytransformsv1`, ramas `test4` y `test6`):

- **B7·1:** `select path, size, content_type from s3_standard.nueva_carpeta.contratos_sql` → las 5
  rutas, sin leer un byte.
- **B7·3, sólo SQL:** `create or replace dataset …contratos_meta as select c.path, c.size,
  c.content_type from …contratos_sql as c` → 4 ítems nuevos (la copia es el mismo ítem), 4 filas;
  otra vez, 4 saltados y nada escrito.
- **B7·2 + B7·3:** `test_project.pdf_pages` —un `@function` con `Media[…]` de entrada y
  `list[Page]` de salida, con una `@dataclass` `Anchor` dentro— en `cross join lateral` →
  `…paginas_sql`: 4 nuevos, 4 filas con su ancla `page`; otra vez, 4 saltados y nada escrito.

| paso | qué | commit |
|---|---|---|
| B7·1 | la colección en `FROM` (ore-core, ore-serve, SDK, editor) | `7d13de0`, `0c0ee32` |
| B7·2 | las Functions en SQL: reescritas con el tokenizador, registradas con su contrato | `d01d1c5` |
| B7·3 | el dataset escrito desde una colección, anclado, por `apply()`; `removed` cuenta ítems | `45210d5` |
| B7·4 | `get_function` lee en la rama de la sesión (el código de una función aún no en `main` daba 404) | `01857df` |
| B7·5 | docs (`docs/sdk.md`), «in this branch» en el error | este |

**Lo que se encontró por el camino:** el nombre con que se llama una Function lleva la base de su
proyecto (`test_project.pdf_pages`), y el producto dice que las Functions viven fuera de base y
schema; la decisión (un espacio `functions.<def>`, y el proyecto frente a la base) es aparte. Y
la imagen `puesto-node` no se construía en Node 24 (el informe de `node --test` cambió a `spec`
sin terminal): `74d83e7`.

### B9 · hecho: ficheros que dan ficheros (2026-10-08, victor)

**El criterio** —*una función sobre una colección que emite **ficheros** se calcula una vez por
ítem y por clave, como `apply()` (B5); lo que no cambió no se recalcula ni se reescribe; lo que
cambió reemplaza sus ficheros; lo que se fue se lleva los suyos; y cada fichero dice de qué ítem y
de qué versión sale*—. Hoy se hace a mano con `transaction().put()` (B4b, en vivo): sin registro,
cada pasada lo rehace todo o lleva la cuenta quien programa. Lee de colecciones del lago (mantenidas
o escritas): no pide la credencial del cliente.

**La superficie.** La misma `apply()`; lo que decide el modo es la **salida**: un `Dataset` da filas
(B5), una `MediaCollection` escrita da ficheros. La función devuelve (o va dando) `ore.File`:

```python
@transform(inputs=[ore.collection("s3_stuff.nueva_carpeta.contratos")],
           output="sandbox.default.paginas")           # una colección escrita
def paginas(contratos, out):
    return contratos.apply(a_png, version="1")

def a_png(item):
    with item.open() as f:
        for n, png in enumerate(render(f), 1):          # p. ej. pypdfium2
            yield ore.File(f"p{n:03}.png", png, anchor={"kind": "page", "page": n})
```

- `ore.File(name, data, content_type=None, anchor=None)`: `data` como en `put` (bytes, ruta o
  fichero abierto); `anchor`, el de v1alpha17 `02` (qué parte del ítem es).
- **La ruta de salida es la del ítem más el nombre**: `nueva_carpeta/a.pdf/p001.png`. Determinista
  —repetir escribe en el mismo sitio, y el blob igual ni se sube (`iguales`)—; dos ficheros de un
  ítem con el mismo nombre son un error de la función. Una copia del ítem con otra ruta es el mismo
  ítem (la identidad es el `digest`, como en B5): sus ficheros no se duplican ni se mueven.
- Filas y ficheros a la vez, no: una salida, un modo.

**El registro es el índice de la colección de salida**, como en B5 la tabla anclada es el suyo: no
hay otra tabla que mantener a la par.

- Cada fichero escrito así lleva dos campos nuevos en el índice y en su `MediaRef`: **`source`**
  (el ítem de origen: `uri` con versión y `digest`) y **`derivation`** (la clave: identidad del
  ítem, `version` —o el hash del código de la función—, `params`). Es el linaje a nivel de ítem: una
  página dice de qué contrato y de qué versión sale, y por ahí llega hasta el dato final (la fila de
  una tabla anclada sobre las páginas).
- Un ítem que **no da ficheros** (un filtro: «sólo los escaneados») o que **falla** deja una
  **marca** en el índice —una fila sin blob, `state: empty | error`, con el mensaje— que el
  listado no enseña como ítem. Sin ella, un ítem sin salida se recalcularía en cada pasada.

**Una pasada:**

1. El listado de la entrada y el registro de la salida (ficheros y marcas, por `source`).
2. Por ítem de la entrada: con la misma clave, **se salta**; nuevo, **se calcula**; con otra clave
   (cambió el ítem, la función o los parámetros), se calcula y, en la misma transacción, **se
   retiran sus ficheros que ya no emite** (las páginas 9–12 de un contrato que pasó a tener 8).
3. Un ítem que ya no está en la entrada: **se retiran sus ficheros** y su marca.
4. Un error es su marca y los demás siguen; `retry_errors=True` los reintenta. `threads` ítems a
   la vez, como B5.
5. Se confirma cada `save_every_s` y al final: una transacción por guardado, así que una pasada
   cortada conserva lo confirmado y la siguiente sigue de ahí. Nada que hacer, nada escrito.
6. El linaje de la colección (`derivedFrom`) lo escribe el servidor al confirmar, como ya hace (B4·4).

Devuelve el resumen de B5 con los ficheros: `{items, new, recomputed, skipped, errors, removed,
files_written, files_retired}`.

**Lo que hace falta debajo:**

| paso | qué |
|---|---|
| B9·0 | **medida** en un puesto de victor: los contratos a PNG (146 ms/página de media, 79–108 ms en caliente, ~36 KB por página a 150 ppp; leer, 0,27–0,51 s por contrato, latencia) y `pypdfium2` 5.14 + `pillow` 12.3 provistos en la imagen; la transacción de 1 000 ficheros, pendiente de su salida | `5aa25c7`, `9a36f31` |
| B9·1 | el contrato: [`docs/media.md`](../media.md) §2 «`put` derivado» —`source` y `derivation` del ítem, la entrada del registro por origen (`files`, `empty`, `error`), el linaje en el cuerpo del `commit` (una entrada reemplaza a la anterior del mismo origen; `retire_sources`, `retire`), `GET …/derivations`, `media/derivacion`— y 15 casos en [`conformidad/media/casos/derivar.json`](../../conformidad/media/casos/derivar.json). **Sin cambio de gramática**: el valor de `Media<c>` no cambia y v1alpha19 `01` §6 deja la escritura de ítems al contrato de ejecución | `3df3d63` |
| B9·2 | ore-medios: siete columnas más en el manifiesto; el sello reemplaza la salida de cada origen, reescribe el linaje de los mismos bytes, cambia o quita la marca, retira orígenes y caminos; lo que no cuadra no deja nada; `/indice/derivaciones` | `1d498b2` |
| B9·3 | ore-serve: el cuerpo del `commit` tal cual hasta ore-medios (`Json::de_node_fiel`: sin perder `null` ni decimales), y `GET /media/…/derivations` | `e4ccff9` |
| B9·4 | SDK: `ore.File`, `apply()` hacia una colección, `Transaction.delete`, `Collection.derivations()`, `MediaRef.source`/`derivation`; el banco modela la escrita como ore-medios y `la-derivacion-a-ficheros.py` corre los 15 casos | `2161a67`, `edbdd6c` |
| B9·5 | en vivo (abajo) | `b5f9581` |
| B9·6 | docs (`docs/sdk.md` «Files from files») y esta sección | este |

**Lo comprobado en vivo** (victor, repositorio Models, `pruebas-de-fuego/b95-ficheros-en-vivo/b95.py`):
tres contratos copiados a una escrita de entrada, y un `@transform` que da un PNG por página con
`pypdfium2` hacia `sandbox.default.b95_paginas_0947`.

- **Primera:** 3 nuevos, 3 ficheros, 7,2 s.
- **Otra vez:** 3 saltados, `written False`: ni una transacción.
- **c1 cambia, c3 se va, entra c4:** 1 nuevo (c4), 1 recalculado (c1), 1 saltado (c2), `removed 2`
  (c3 y la identidad vieja de c1: la identidad es el `digest`), 1 fichero retirado (el de c3; el de
  c1 se reescribe en su camino con el linaje nuevo). Quedan c1, c2 y c4; el registro, tres entradas.
- **El linaje desde el código:** `c1.pdf/p001.png` dice `source` (el contrato, su versión, su
  `digest` y `{kind: page, page: 1}`) y `derivation` (`a_png`, `1`, su clave).

**Lo que se encontró por el camino**, cada uno arreglado y con su prueba:

- dentro de un transform, `apply()` lee el registro de su propia **salida**, y ore-serve sólo dejaba
  leer `inputs` (403 `media/no-declarada`): la salida se lee, de ahora, como en `_lee` del SDK;
- `MediaRef` descartaba `source` y `derivation` (ignora lo que no conoce);
- una escrita **recién creada** no tiene transacción y su registro era 404: ahora, vacío;
- `pillow` provista dejó de poder ser sugerida (`librerias::las_sugeridas_de_python`, 29).

Los tres primeros los dejaba pasar el banco; lo destapó correrlo en vivo.

**Fuera, y anotado:** su forma SQL (`insert into media collection … select …`, M3, que verá
`source` como columna del listado); Node y la JVM; las marcas `empty` y `error` y el corte a mitad
sólo están probados en el banco y en Rust (los contratos de victor dan todos una página).

### B10 · diseño (propuesto, 2026-10-08): ficheros que dan ficheros, en SQL

La forma SQL de B9, como B7 lo es de B5. Vive en un repositorio **`transforms-sql`**: escribe
datos, así que es un Transform y se construye con Build, con su linaje y su salida declarada.

```sql
-- transforms/paginas.sql
create or replace media collection legal.archivo.paginas media image formats (png) as
select p.name, p.data, p.anchor
from legal.archivo.contratos as c
cross join lateral functions.pdf_a_png(c._item) as p
where c.content_type = 'application/pdf';
```

**Por qué `create or replace … as select`, y no `insert into media collection … select`** (lo
que B7 apuntó): `insert into` promete añadir, y lo que pasa es otra cosa —cada origen
**reemplaza** su salida, lo que ya no da se retira, lo que se fue se lleva lo suyo—. `or replace`
dice la verdad, como en B7: el resultado es el de recalcularlo todo; lo incremental sólo lo
abarata. Y la colección queda **definida por su consulta**, con su medio y sus formatos, como un
`create or replace dataset … as select` define un dataset.

**Lo que la consulta da**, una fila por fichero, por nombre de columna:

| columna | tipo | |
|---|---|---|
| `name` | `String`, obligatoria | el camino del fichero dentro del ítem de origen (`p001.png` → `<ruta>/p001.png`), como `ore.File.name` |
| `data` | `BLOB`, o un `Media<c>` | los bytes; un `Media` (p. ej. `c._item`) **copia** ese ítem tal cual —filtrar o copiar una colección sin escribir una función— |
| `content_type` | `String`, opcional | el declarado; mandan los bytes |
| `anchor` | `Anchor`, opcional | qué parte del ítem es |

Cualquier otra columna es un error (no hay dónde ponerla: el fichero no tiene columnas). Los
bytes los da una **Function** (0050): una `@function` de Python que devuelve
`list[Pagina]`, con `name: str`, `data: bytes` y `anchor: Anchor` —`bytes` es `Opaque` en su
contrato y `BLOB` en SQL (`ore-code` `derivar.rs`, `sql_functions.py`)—, llamada con
`cross join lateral functions.<def>(c._item)` como en B7 (0056: las funciones tienen su espacio propio, fuera de toda base).

**Cómo se calcula:** exactamente B9. El SDK corre la consulta **ítem a ítem** con la colección
reducida a ese ítem (lo que ya hace B7 con `_sql_per_item`), convierte cada fila en un `ore.File` y
se lo da a `apply()` hacia la colección: el mismo registro por clave, las marcas, el reemplazo por
origen, retirar lo que se fue. La versión es la consulta normalizada y el código de cada función
que llama (la de B7). Un error de un ítem —de la función o de la consulta— es su marca, y los
demás siguen.

**Los límites de la primera versión**, los de B7, cada uno con su error: una sola colección en el
`FROM` y ningún `join` con otra relación; sin agregados, ventanas, `order by` ni `limit`;
funciones de Python sin `over` ni `models`; dos filas de un ítem con el mismo `name`, error de ese
ítem. La colección, si ya existe, tiene que tener el mismo `media` y `formats` (cambiarlos es un
`alter`, aparte); si es mantenida (tiene `from`), no se escribe.

**Preview** (0055): la consulta sobre los primeros ítems, sin escribir nada; se ven las filas que
serían ficheros (`name`, `content_type`, el tamaño de `data`, `anchor`), no los bytes.

| paso | qué |
|---|---|
| B10·0 | **hecho en vivo** (2026-10-08, victor, repositorio functions-python): `functions.pdf_a_png` —`Media[…]` → `list[Pagina]` con `data: bytes` y una `@dataclass` de ancla— en `cross join lateral` sobre `s3_stuff.nueva_carpeta.contratos`: 4 contratos, 4 filas, `data` es `BLOB` (los bytes empiezan por la firma PNG) y `anchor` `STRUCT(kind VARCHAR, page BIGINT)`; ~25 s la consulta entera, en serie y en frío. De paso: `sql()` sobre una colección corre en una sesión de Functions, el ítem del listado es `_item` (no `item`, 0057 C3) y pyright ve `Media["c"]` como `MediaRef` (`c704d46`) |
| B10·1 | **hecho**: el guion (ore-core) analiza `create or replace media collection … media … formats (…) [comment '…'] as select …` (`Sentencia::ColeccionDerivada`, con su `select` como `Unidad`) y lo coteja contra el árbol: las columnas por nombre (`name` y `data`, y si acaso `content_type` y `anchor`; otra, un `*` o una sin nombre, error), una colección en el `FROM` y nada más, todo por ítem, las funciones publicadas y de código, la destino escrita y con el mismo `media` y `formats` (o nueva), y no la que lee. Sin `or replace`, con `if not exists` o con `from object table`, error; `or replace` sin `as`, también. La celda la sabe del árbol; correrla es B10·3 (hasta entonces, `NotImplementedError`). `a_media_collection_is_derived_from_a_query`, 21 casos |
| B10·2 | **hecho**: la sentencia es un `Transform` (`transformar::derivar_sql`): `entrypoint: <ruta>.sql:<n>`, salida la colección, entradas lo que lee su consulta; `ore` genera su documento y `OOS2013` lo coteja como a los demás. OOS v1alpha25 `01` §5.2 gana la fila (`c8b92e7`) y el caso `valid/a-collection-from-its-query`. Crear la colección si no está pasa a B10·3: lo hace la celda del build, que es quien escribe |
| B10·3 | **hecho**: la celda (ore-serve, `celda_de_sentencia`) crea la colección si no está —escrita, con su `media` y `formats`— **antes** de declarar el transform (que la nombra), y dentro `_sql_a_ficheros` (SDK): la consulta ítem a ítem (`_consulta_por_item`, sacada de `_sql_per_item`, que no cambia) y cada fila un `ore.File` (`_fichero_de_fila`: `data` bytes, o un ítem cuyos bytes se copian con su tipo; del ancla, sin sus nulos) por `apply()` hacia ella (B9). Su fila: `items`, `new`, `recomputed`, `skipped`, `errors`, `removed`, `files_written`, `files_retired` (`_resultado_de_derivar`). `pruebas-de-fuego/la-derivacion-a-ficheros-en-sql.py`, 6 casos con DuckDB de verdad contra el banco (que gana `POST /puestos/p1/sql`); la celda, en el corpus de `el_codigo_generado_casa_con_el_sdk` |
| B10·4 | Preview de la sentencia, y el editor la reconoce (no la marca, no va a DuckDB) |
| B10·5 | casos: el guion (ore-core) y el banco (`la-derivacion-a-ficheros` con su forma SQL); y en vivo, un `.sql` en un repositorio `transforms-sql` de victor: contratos → PNG por página, otra vez nada, la entrada cambiada |
| B10·6 | docs y esta sección pasa a «hecho» |

**Fuera, y anotado:** `join` con otras relaciones (la versión del dataset en la clave); Functions
de TypeScript o Java desde SQL; `annotations` del fichero desde columnas de la consulta.


## Lo que no se hace aquí

- Las funciones concretas de IA (qué OCR, qué modelo de transcripción): se eligen sobre la base,
  midiendo, y son F8 de 0046.
- La GPU (D7): decisión de coste aparte.
- Permisos **por ítem**: el estado del arte los echa en falta (Foundry, Databricks), pero son de
  0047; la tabla de listado ya permite filtrar por fila.
- C2PA y la procedencia criptográfica del medio: se registra si hay manifiesto; validarlo, después.

## Qué se acepta a cambio

- **Construir antes de ver resultados.** B0–B5 no enseñan un solo OCR; es el precio de no heredar
  JSON en strings ni un incremental que miente.
- **Un servicio más en la celda** (D1, D2): estado, réplicas y un camino de bytes que antes no
  cruzaba ORE.
- **Una versión de la gramática** (D4) y la migración de lo que ya declara `Media<c>`.
- **Una forma propia hasta que Iceberg tenga `FILE`**: la `MediaRef` se guarda como struct con los
  nombres de Parquet `FILE`, y adoptará el tipo cuando exista.
- **El esquema `ore://`** es propio y no está registrado.

## Anexo · El estado del arte, resumido

**SQL.** Snowflake: tipo `FILE` (referencia con `CONTENT_TYPE`, `SIZE`, `ETAG`), directory tables,
tres URLs (scoped, de stage, presigned). BigQuery: `ObjectRef` (`uri`, `version`, `authorizer`,
`details`), object tables con bytes en pseudocolumna oculta. DuckDB `read_blob` con proyección
perezosa. Ninguno garantiza que la referencia siga al origen; las funciones de IA reciben la
referencia, no los bytes.

**Python.** Daft (`daft.File` perezoso, UDF con `gpus`, `on_error`), Ray Data (actores con el modelo
cargado una vez, contrapresión, `ray.data.llm` con vLLM, checkpoint), Lance (blob v2, `take_blobs`
perezoso, versiones), Pixeltable (columnas calculadas incrementales, `errors_only`), HF `datasets`
(`decode=False`), WebDataset. Ninguno direcciona por contenido al escribir.

**JVM.** Beam `match → readMatches → ReadableFile` (las tres capas explícitas), Spark `binaryFile`
(el contraejemplo), Flink `FileSink` exactly-once, Tika, PDFBox con caché acotada, AWS SDK v2
(`close` drena, `abort` corta).

**Node.** `Blob`/`File`/`ReadableStream` iguales en Node, navegador y Workers; `fs.openAsBlob`
falla si el fichero cambia; `file-type` detecta por firma; sharp con límites; `fluent-ffmpeg`
archivado (2025).

**Estándares.** Parquet `FILE` e Iceberg v4 `file`; descriptor OCI (`mediaType`, `digest`,
`size`); RFC 9530 (`sha-256`; `crc32c` y `md5`, no como identidad); RFC 6838 y WHATWG MIME
Sniffing; RFC 9110 (rangos), Media Fragments, RFC 8118, IIIF; W3C Web Annotation; COCO, WebVTT,
ALTO/hOCR/PAGE; OpenLineage; C2PA 2.2.

**Foundry.** Lo mejor: la media reference como tipo de columna, operaciones por tipo de medio con
salida fija, `suppress_errors`, incremental `added/previous/current`, *access patterns*, Document
Intelligence. Lo peor: resultados en JSON string, permiso por media set, dos políticas de
transacción, streams sin acceso aleatorio, virtuales sin STS.

**Databricks.** Lo mejor: `FILE` (Beta), `ai_parse_document` con `bbox` y confianza, error por fila.
Lo peor: Auto Loader da por visto lo que falló e identifica por ruta, tres formas de error, las
funciones de IA rompen el incremental, el acceso por ruta se sale del linaje.
