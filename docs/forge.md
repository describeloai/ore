# Ontology Forge — el gestor de la ontología, y de qué sale

> Lo que la consola llama *Data → Ontology Forge*: la interfaz central desde la que se crea y
> se gestiona la ontología sobre los conjuntos de datos. Este documento dice **de qué
> abstracción sale**, **qué secciones tiene y por qué**, **qué verbos de `ore-serve` le
> faltan** y **cómo se itera**: cada iteración se mide antes con
> [`pruebas-de-fuego/medida-forge-contra-serve.py`](../pruebas-de-fuego/medida-forge-contra-serve.py).
> El repositorio corre por debajo y no se ve: aquí se modela y se gestiona.

---

## 1. La naturaleza de la abstracción, en una frase

```
Table    un hecho físico          el objeto tal cual está, con dos caras (reads · changes)
View     UNA PREGUNTA declarada   Q(Table), y compone — sólo el fragmento invertible
Entity   qué es una fila          la única que lleva significado
```

**El objeto de peso es la vista**: la pregunta sobre el hecho. La ontología es *el conjunto de
preguntas que una organización ha acordado hacerse, más qué significan las respuestas, quién
puede verlas y qué se puede causar con ellas* — y eso es un repositorio (`ontologia.git` de la
celda), con `apiVersion` por documento, compilación pura y digest determinista
([`ontologia-como-repositorio.md`](ontologia-como-repositorio.md)).

Tres clases de pantalla, que Forge no mezcla nunca:

| clase | qué es | qué hace Forge con ella |
|---|---|---|
| **documento** | lo que se declara: los catorce `kind` | se edita → commit con quién lo pidió |
| **derivado** | linaje, clasificación efectiva, topología, digest, diagnósticos, emisiones | se lee; **jamás** se edita |
| **decisión** | la cola de `discover` (once clases de pregunta) | se contesta |

## 2. Lo que se define · lo que se deriva · lo que se gestiona

**Se define** (catorce documentos, cuatro alturas más el gobierno): `OntologyConfig`,
`Package`, `Table`, `View`, `Entity`, `Lattice`, `ConduitPolicy`, `Ruleset`, `Concept`,
`Interface`, `Function`, `Resolution`, `RequestPolicy`, Cedar, `Model` (0027); y un artefacto,
la `Propuesta`. Cuatro dueños distintos con cuatro documentos distintos: dominio, seguridad,
cumplimiento, identidad. La tabla no tiene dueño.

**Se deriva** (y por eso no se declara): linaje, clasificación efectiva (etiqueta ·
procedencia · concepto), grafo de consumidores, jerarquía de interfaces, integridad de cada
`Function`, topología (el índice es una vista de aristas), plan, manifiesto de caché,
diagnósticos `OOS1xxx`–`9xxx`, digest · bundle · `.oob` · lock, emisiones (GraphQL, ODCS,
Ossie, Cedar schema), `ore diff`.

**Se gestiona**: `source add → catalog (Job) → discover → review → validate/verify → lock →
pack → diff → compile/export → promote`; el paquete con `new / move / split / merge`.

## 3. Las secciones, y de dónde sale cada una

| grupo | sección | documento detrás | por qué así |
|---|---|---|---|
| ontología | **Explore** | — (derivados) | la portada: cuántas cosas, el grafo, las ocho reglas que cumple al compilar, lo que se deriva |
| | **Entities** | `Entity` | las seis partes, ni una más |
| | **Concepts** | `Concept` | *Properties* no es sección: una propiedad vive dentro de su `Entity`; lo compartido es un `Concept` (lo que Foundry llama *shared property types*). El inventario transversal de propiedades es una pestaña de búsqueda |
| | **Links** | `Entity.spec.relations` | objeto de primera clase **en la interfaz**, fila de la entidad **en el árbol**: la inversa y la topología se derivan. `kind: Link` con `backedBy` entra en `oos` el día que haga falta un many-to-many o un link entre dos paquetes que ninguno posee |
| | **Interfaces** | `Interface` | |
| sustrato | **Views** (+ Tablas) | `View`, `Table` | la vista con dueño; la tabla sin él |
| cinética | **Functions** | `Function`, `Model` | propone, no aplica |
| gobierno | **Policies** | Cedar · `Ruleset` · `ConduitPolicy` · `Lattice` · `RequestPolicy` | cinco pestañas, cuatro dueños |
| gestión | **Ontology config** | `OntologyConfig`, `Package`, lock | paquetes, orígenes, dependencias, el ciclo |

Forma: lista + ficha (Foundry), no editor de ficheros. «Nuevo …» enseña los campos del
documento y dice que no escribe hasta que exista el verbo.

## 4. Lo que se midió antes de escribir

### 4.1 · Frente a Foundry: entidad ≠ object type

Un *object type* es «the schema definition of a real-world entity or event» **con sus
instancias**: Object Storage V2 indexa el backing datasource **y las ediciones** («modifications
layer atop underlying dataset information»), y un object type puede no tener dataset («use
actions to populate every property»). Un *link type* es un objeto propio, bidireccional, con
nombre en cada lado; los many-to-many se respaldan con un dataset.

Nuestra `Entity` es **una declaración sobre la fila de una pregunta**: no tiene instancias
propias; lo físico está fuera (`backedBy → View → Table → datasource`); la clasificación es un
tipo que se propaga y se comprueba **al compilar, sin datos**. La diferencia real no es de
spec: es de motor — Foundry tiene la copia editable y nosotros la decidimos (0018) y no la
tenemos (F5). Dos cosas suyas que nos faltan y son de presentación: *display name* y *title
key*.

### 4.2 · `displayName` y `titleKey` contra `ore validate` (v1alpha8, `acme-retail`)

| prueba | resultado |
|---|---|
| `Entity.metadata` | cerrado: `name · namespace · labels · description · aiContext` (`View`: sin `aiContext`; `Table`: sólo `name · namespace · description`) |
| `metadata.displayName`, `spec.titleKey` sin prefijo | **`OOS1005`** |
| `x-rubix-displayName`, `x-rubix-titleKey` en `metadata` (entidad y vista) | **pasan** |
| `x-rubix-titleKey: noExiste` | **pasa** — una extensión es opaca; nadie la coteja |

**Decisión.** `displayName` es presentación (la misma clase que `description`), tiene un
consumidor hoy (Forge) → **extensión `x-rubix-displayName` ya**, en `metadata` de `Entity`;
se promueve a `metadata.displayName` en `oos` cuando aparezca el segundo consumidor (GraphQL,
MCP). `titleKey` es **referencial** —nombra una propiedad que debe existir, como `primaryKey`—
y la prueba de `noExiste` enseña por qué no vale como extensión; su sitio es `spec.titleKey`
con `OOS2018`, **y entra en `oos` con la copia (F5)**, que es su único consumidor. No antes.

### 4.3 · Forge contra `ore-serve` (`medida-forge-contra-serve.py victor`, 2026-09-16)

`ore-serve` sirve 15 rutas: `fuentes` (GET/POST, `{n}/estado`), `paquetes` (GET/POST,
`{n}/esquema`, `{n}/decisiones` GET/POST), `modelos` (GET/POST/GET n/DELETE), `perfiles`,
`salud`, `version`. En `victor`, con el token de agente, todas contestan 200: 1 paquete
(`ventas`), 1 fuente, 1 modelo, 0 decisiones. **`/esquema` emite `name, namespace,
primaryKey, backedBy, properties` — sin `labels` ni `relations`**: Entities no puede pintar
sensibilidad y Links no puede pintar aristas hasta que las emita.

| sección | estado | le falta |
|---|---|---|
| Explore | parcial 2/4 | `GET /arbol`, `GET /derivados/diagnosticos` |
| Entities | parcial 1/4 | `/documentos?kind=Entity`, `PUT`/`DELETE /documentos/Entity/…` |
| Concepts | falta 0/2 | `GET /conceptos`, `PUT /documentos/Concept/…` |
| Links | parcial 1/3 | `PUT /documentos/Entity/…`, `GET /derivados/topologia` |
| Interfaces | falta 0/2 | `/documentos?kind=Interface` |
| Views | falta 0/3 | `/documentos?kind=View`, `?kind=Table`, `PUT` |
| Functions | parcial 3/5 | `/documentos?kind=Function` |
| Policies | falta 0/5 | `/documentos?kind=Ruleset · ConduitPolicy · Lattice · RequestPolicy`, `GET /politicas` |
| Ontology config | parcial 6/9 | `GET /dependencias`, `POST /acciones/validar`, `POST /acciones/diff` |

## 5. Los verbos que faltan son tres familias, no once rutas

Todos los `kind` son documentos del mismo árbol y se escriben con la misma figura que ya usan
`/fuentes` y `/modelos` (`escribiendo`: clonar, escribir, compilar, empujar con quién lo pidió,
409 si alguien empujó antes):

1. **`/documentos`** — `GET /documentos?kind=K` · `GET|PUT|DELETE /documentos/K/{ns}/{n}`;
   validación por kind antes de empujar (una `Entity` sin `backedBy` → 422 `OOS2022`, igual
   que un `Model` sin perfil). Desbloquea siete secciones.
2. **`/derivados`** — sólo lectura: `diagnosticos`, `linaje`, `clasificacion`, `topologia`.
3. **`/acciones`** — `validar`, `lock`, `diff`, `pack`: los verbos del repositorio que hoy son
   CLI.

`/fuentes`, `/paquetes`, `/decisiones`, `/modelos` se quedan: son los que **derivan algo** al
escribir (un Job, una suscripción).

## 6. Las iteraciones, en el orden que la medida da

Cada una se mide antes (§4.3) y cierra filas de la tabla; ninguna pinta lo que no llega.

| | qué | cierra |
|---|---|---|
| **I0** ✓ | el boceto en la consola sobre `acme-retail` (`components/ontology/`), con cada pantalla diciendo qué verbo le falta; `x-rubix-displayName` en la ficha y en el borrador de `Entity` | — |
| **I1** | `/documentos` para `Entity` (lectura con `labels` y `relations`, `PUT`, `DELETE`); Forge lee el árbol de la celda en Entities y Links | Entities, Links (lectura) |
| **I2** | `/documentos` para `View`, `Table`, `Concept`, `Interface`, `Function`, y los de gobierno; `GET /conceptos` (importados + locales, quién los habla) | Views, Concepts, Interfaces, Functions, Policies |
| **I3** | `/derivados`: `diagnosticos`, `topologia`, `clasificacion`; `GET /arbol` | Explore, Links (topología) |
| **I4** | `/acciones` y `GET /dependencias`; «Proponer cambios» como ciclo real | Ontology config |
| **I5** | escribir desde Forge: los «Nuevo …» dejan de ser borradores | todas |

Lo que no entra: `kind: Link` (§3), `spec.titleKey` (§4.2), datos de ejemplo en una celda real.
