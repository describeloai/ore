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

### 4.4 · Después de I1 (2026-09-16): `ore-serve` sirve 19 rutas

`/documentos/Entity` (GET) y `/documentos/Entity/{ns}/{n}` (GET · PUT · DELETE). La medida
pasa **Entities a real (4/4)** y Links a parcial 2/3 (le queda `/derivados/topologia`). Lo
que se midió antes de escribir el verbo, contra `ore validate` sobre `acme-retail`:

| prueba | resultado |
|---|---|
| `Entity` sin `backedBy` | **pasa** — los bindings de v1alpha7 siguen siendo legales |
| `backedBy` a una vista que no existe | `OOS2018` |
| una propiedad que la vista no expone | `OOS2022` |
| `metadata.displayName` sin prefijo | `OOS1005` |
| `relations.*.target` a lo que no está | `OOS2005` |
| la cadena de consulta (`?kind=`) | **la puerta la descarta a propósito** (`http.rs`) |

Tres decisiones salen de ahí, y se quedan escritas en `documentos.rs`:

1. **`backedBy` lo exige el verbo, no el compilador.** Forge escribe entidades v1alpha8 y no
   escribe bindings: una entidad sin `backedBy` es una declaración sobre ninguna fila. El
   422 lo dice con su motivo y **sin inventarle un código**; §5 decía `OOS2022` y ese código
   significa otra cosa.
2. **El `kind` es un segmento, literal.** `/documentos/Entity`, no `?kind=Entity`, y en
   `rutas.rs` va escrito `"Entity"` y no `{kind}`, para que la medida no cuente como
   servido lo que no lo está: cada kind de I2 entra con su segmento.
3. **`If-Match: <commit>`** es como quien leyó dice sobre qué leyó. Si el árbol ya no está
   ahí, 409 antes de escribir nada. Sin la cabecera no se comprueba, como en `/modelos`.

Aceptado por [`pruebas-de-fuego/los-documentos.sh`](../pruebas-de-fuego/los-documentos.sh)
(en CI, tras `los-modelos.sh`): la lista con `metadata` y `spec` enteros (7 entidades, 7 con
labels, 5 con relations), la ficha con su YAML y el commit que la trajo, PUT válida → 201 y
commit del sujeto, sin `backedBy` → 422, propiedad sin campo → 422 `OOS2022` con `donde` y
`ayuda`, `displayName` sin prefijo → 422 `OOS1005`, dos PUT sobre el mismo commit → 200 y
409, DELETE referenciada → 409 con quien la nombra, DELETE libre → 200 y 404 después.

### 4.5 · View y Table, antes de I2 (`medida-forge-view-y-table.py`, 2026-09-16)

**La forma.** `View.metadata`: `name · namespace · labels · description`; `spec`: `owner · from ·
freshness · fields · where · materialized · moved · reserved · groupBy · having` (obligatorios
`owner`, `from`, `fields`). `Table.metadata`: `name · namespace · description`; `spec`:
`datasource · object · profile · columns · reads · changes` (obligatorios todos menos `profile`).
Las dos admiten `x-<proveedor>-*`; `x-rubix-displayName` **pasa** en las dos. `View.labels` sólo
admite `oos.maturity` (`OOS1005` con cualquier otra); `Table` no admite `labels`.

**El compilador**, contra `acme-retail` (`hr.empleados` sobre `hr.workday_worker`):

| rotura | `ore validate` |
|---|---|
| borrar o renombrar la vista que `Employee.backedBy` nombra | `OOS2018` en Employee |
| borrar la tabla que `empleados.from.table` nombra · `from.table` a una que no existe | `OOS2018` en la vista |
| un `field` o un `where` sobre una columna que la tabla no tiene · un campo que la vista de abajo no expone | `OOS2018` |
| una vista sobre otra vista (`from.view`) | pasa |
| `Table` sin `changes` · `datasource` sin declarar · `reads: none` con una vista virtual encima · copia sin conducto | `OOS1004` · `OOS2004` · `OOS2020` · `OOS4011` |
| `count(col)` | `OOS1004`, con la frase que manda escribir `count()` |
| agrupar sobre una **tabla** (`count()`, `groupBy`, `having`) | pasa |
| agrupar sobre una **vista** (`sum(baseSalary)` de `empleados`) | **fallaba** con `OOS2018` y un nombre vacío: la rama `from.view` resolvía los campos con la función que excluye los agregados, y la conformidad no lo cubría (9 casos sobre tabla, 0 sobre vista). **Arreglado** en `vistas.rs` el mismo día, con cuatro casos de conformidad (spec §4 y §5.8): agrupar sobre una vista compila; agregar lo que abajo ya es un agregado es `OOS2018` con su motivo |
| `View` sin `owner` | **pasa** — `owner` lo exige el emisor (`cambiame` → `OOS2009`), no el compilador |

**El inductor.** `ore discover` escribe `tables/Clientes__public_clientes.yaml` con `name:
public_clientes` (`reads: {}`, `changes: { mode: none, witness: none }`: no se sondeó, no se
inventa) y `views/Clientes__public_clientes.yaml` con `name: clientes`, `oos.maturity: DRAFT` y
`owner: cambiame`. **El fichero no se llama como el documento**: se busca por `metadata.name`.
Y **el árbol inducido no compila** hasta `review` y `source add` (`OOS2009`, `OOS2010`,
`OOS2004`).

**El emisor.** `ore view add --from <tabla|vista> --owner … --field p=col … <nombre>` es *el*
emisor de `View`, el mismo que el inductor; lo que escribe compila. No hay `ore table add`: la
tabla es un hecho, la escribe el inductor o una persona.

**Lo derivado.** `ore view .` cuenta por vista `plan · raíz · caras · esquema · linaje · refresco ·
empuje · cotejo · flujo · restricciones`. Es el material de `/derivados` (I3), no de `/documentos`.

Cuatro decisiones salen de aquí, y son las de I2:

1. **La puerta del PUT es «el árbol no empeora», no «el árbol compila».** Sobre un árbol
   inducido sin revisar, «compila» rechaza cualquier escritura por errores ajenos al documento.
   Se compila antes y después, y 422 sólo si aparecen diagnósticos nuevos, identificados por
   `(código, mensaje)` sin la posición; el 422 trae **sólo los nuevos** y cuenta los `previos`.
   **Hecho en `Entity` (I1) el mismo día** (`documentos.rs::empeora`, caso 10 de
   `los-documentos.sh`): el motor de View y Table lo hereda.
2. **El 409 explícito con los nombres** hace falta en `DELETE` de View y Table por lo mismo que
   en Entity: `OOS2018` cuenta la verdad desde quien la nombra. Quién nombra a una vista:
   `Entity.backedBy`, `View.from.view`; a una tabla: `View.from.table`.
3. **Escribir una View pasa por `ore view add`** cuando es nueva, o se emite la misma forma:
   un emisor propio en `ore-serve` sería el segundo, y divergirían. `owner` lo exige el verbo.
4. **Reescribir desde JSON pierde los comentarios** del YAML (`acme-retail` está lleno). Un
   PUT que trae `yaml` tal cual y se valida conserva lo que la persona escribió; un PUT que trae
   el documento en JSON lo reescribe. Los dos caben; el que edita un formulario manda JSON y el
   que edita el texto manda YAML.

## 5. Los verbos que faltan son tres familias, no once rutas

Todos los `kind` son documentos del mismo árbol y se escriben con la misma figura que ya usan
`/fuentes` y `/modelos` (`escribiendo`: clonar, escribir, compilar, empujar con quién lo pidió,
409 si alguien empujó antes):

1. **`/documentos`** — `GET /documentos/K` · `GET|PUT|DELETE /documentos/K/{ns}/{n}` (el
   kind es un segmento: la puerta no lee la cadena de consulta, §4.4); validación por kind
   antes de empujar (una `Entity` sin `backedBy` → 422 con su motivo, igual que un `Model`
   sin perfil; el resto son los diagnósticos del compilador tal cual, `OOS2022`, `OOS1005`…).
   Desbloquea siete secciones. `Entity` hecho en I1.
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
| **I1** ✓ | `/documentos` para `Entity` (lectura con `labels` y `relations`, `PUT`, `DELETE`, §4.4); Forge lee el árbol de la celda en Entities y Links | Entities, Links (lectura) |
| **I2** | `/documentos` para `View`, `Table`, `Concept`, `Interface`, `Function`, y los de gobierno; `GET /conceptos` (importados + locales, quién los habla) | Views, Concepts, Interfaces, Functions, Policies |
| **I3** | `/derivados`: `diagnosticos`, `topologia`, `clasificacion`; `GET /arbol` | Explore, Links (topología) |
| **I4** | `/acciones` y `GET /dependencias`; «Proponer cambios» como ciclo real | Ontology config |
| **I5** | escribir desde Forge: los «Nuevo …» dejan de ser borradores | todas |

Lo que no entra: `kind: Link` (§3), `spec.titleKey` (§4.2), datos de ejemplo en una celda real.
