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

**El emisor.** `ore view add --from <tabla|vista> --owner … --field p=col … --where col=v …
<nombre>` es *el* emisor de `View`, el mismo que el inductor; lo que escribe compila. Pero
escribe **sólo el fragmento invertible sin copia** —`owner`, `from`, `fields`, `where`,
`oos.maturity: DRAFT`— y **no** `freshness`, `materialized`, `groupBy`, `having`, `moved`,
`reserved`, `description`, otras `labels` ni `x-rubix-*`; y **se niega a reescribir** un nombre
que ya existe (*«sobrescribirla sería perder lo que dijera, y este mando no lo decide»*). Es un
verbo de crear, no de escribir. No hay `ore table add`: la tabla es un hecho, la escribe el
inductor o una persona.

**Quién nombra a quién** (lo que el 409 de `DELETE` tiene que decir): a una vista la nombran
`Entity.backedBy` y `View.from.view`; a una tabla, `View.from.table`. Una `Function` escribe por
`effects.writes: hr.Employee.estado` —nombra la **entidad**— y la vista la alcanza por `backedBy`.
Renombrar una tabla es borrarla para su vista (`OOS2018`); quitarle una columna que una vista lee
es `OOS2018` con el nombre de la columna; dos tablas sobre el mismo `object` son legales (el
`object` es una cadena opaca, no una identidad); una tabla y su vista que nadie más nombra se
borran sin más.

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
3. **El PUT de una View no puede pasar por `ore view add`**: sólo escribe el fragmento sin
   copia ni agrupación y se niega a reescribir. El verbo emite el documento entero con el
   **mismo emisor que `Entity`** (`Node` → YAML en `documentos.rs`): un emisor por servidor, no
   uno por kind. Que no diverja del inductor se acepta midiendo, no prometiendo: una vista
   escrita por PUT y la misma por `view add` dan el **mismo `plan sha256`** en `ore view .`.
   `owner` lo exige el verbo, como `backedBy` en Entity.
4. **Reescribir desde JSON pierde los comentarios** del YAML (`acme-retail` está lleno). El PUT
   admite las dos entradas: el documento en JSON (un formulario) o `yaml` tal cual (el texto);
   las dos pasan por la misma puerta. Quien edita el texto no pierde lo que escribió.

### 4.6 · Concept e Interface, antes de sus filas (`medida-forge-concept-e-interface.py`, 2026-09-17)

**La forma** (v1alpha4). `Concept.metadata`: `name · namespace · labels · description`; `spec`:
`type` (obligatorio) `· labels · description · enum · aiContext · requiresGovernance ·
confidence`. `Interface.metadata`: `name · namespace · description`; `spec`: `requires`
(obligatorio) `· description`. Ninguno de los dos esquemas declara `patternProperties x-`, y
aun así **`x-rubix-displayName` pasa** en los dos: la extensión la admite el validador, no el
esquema.

**Dónde vive.** El directorio no decide nada; el kind es el discriminante. Un Concept en
`packages/hr/concepts/` o en `concepts/` de la raíz compila igual; una Interface en
`interfaces/` de la raíz (que es lo que `ore init` crea; `concepts/` no lo crea) o en
`packages/hr/interfaces/` también. **`OOS2030` no salta en la raíz**: un documento fuera de
`packages/` no tiene paquete que lo contradiga.

**El compilador**:

| rotura | `ore validate` |
|---|---|
| un Concept nuevo que nadie habla | **`OOS9004`** — y en `DRAFT` también |
| `is` a un concepto que no está · borrar el concepto que una propiedad habla | `OOS2001` en la propiedad |
| la propiedad rebaja la etiqueta del concepto · la eleva | `OOS4012` · pasa |
| el concepto lo habla sólo una entidad de otro paquete | pasa: `OOS9004` mide el árbol, aunque diga «del paquete» |
| Concept sin `type` | `OOS1004` |
| `requires` a lo que no está · borrar el concepto que una Interface requiere | `OOS2001` en la interfaz |
| `implements` sin satisfacer · borrar la interfaz implementada | `OOS9001` · `OOS2001` en la entidad |
| una Interface que nadie implementa | pasa |
| **dos ficheros que declaran el mismo concepto** | **pasaba** — y no era de Concept: dos Entity, View, Table, Lattice o Package iguales compilaban igual, y cada referencia resolvía la primera que encontraba. **Arreglado** el mismo día: `OOS2035` (`identidad.rs`, antes que nada en `validate_package`; spec `90-canonical` §5.2 y un caso de conformidad) |

**Los importados.** `ore pack` de un vocabulario da un `.oob` (forma canónica en JCS, con sus
`Concept`, su `Lattice` y su `Package`); en `vendor/*.oob` el cargador lo lee como cualquier
documento, `is: gdpr.personalEmail` resuelve, y **`OOS9004` no alcanza a lo importado**: es
publicar vocabulario. No hay `ore concept add` ni `ore interface add`.

**El boceto**: `acme-retail` tiene cero `is:` y cero `Concept`; las 18 propiedades del boceto
que «hablan» `gdpr.*` o `iso.*` se inventaron en `acme.ts`. En la celda, Concepts sale de
`/documentos/Concept` y de `vendor/*.oob`.

Cuatro decisiones para las dos filas:

1. **Un PUT de Concept nunca podría entrar solo**: nadie lo habla todavía, `OOS9004` es un
   diagnóstico nuevo, y la puerta «no empeora» lo rechaza. Y al revés tampoco: el `is` primero
   es `OOS2001`. La decisión era tolerar el `OOS9004` que nombra al documento recién escrito,
   **y el fire test la corrigió** (`los-documentos.sh` caso 18): el espejo también bloquea —
   quitar el `is` de la única propiedad que lo habla hace nuevo el `OOS9004` y retirar el
   concepto antes es 409 porque está hablado; ningún orden entraba. `OOS9004` es **el estado
   entre dos escrituras**, no un defecto de una, así que lo tolera **el motor entero** (no la
   fila) y la respuesta dice qué queda sin hablar: `sinHablar: ["hr.personalEmail"]`.
2. **Quién nombra**: a un Concept, `Entity.properties.*.is` e `Interface.requires`; a una
   Interface, `Entity.implements`. El 409 de `DELETE` los cuenta desde aquí.
3. **El motor recorre también la raíz** (`ore init` pone `interfaces/` ahí): los `.yaml`
   sueltos y los directorios que no son `packages/` ni `vendor/`; lo que no está en un paquete
   sale **sin `paquete`** (la ficha no inventa uno); uno nuevo se escribe en
   `packages/<ns>/<carpeta>/`. El verbo no exige nada propio: lo que falta ya es `OOS1004`.
4. **`GET /conceptos`** = los `Concept` del árbol (`importado: false`) más los de `vendor/*.oob`
   (`importado: true`, con el `paquete` del sobre), cada uno con **quién lo habla** (`hablado`:
   las propiedades con `is`, `hr.Employee.email`) y **quién lo exige** (`exigido`: las
   interfaces con `requires`). El fire test siembra un vocabulario `iso` empaquetado; el `gdpr`
   de conformidad ya no vale para eso: trae el lattice `gdpr.sensitivity` que acme-retail
   declara, y desde `OOS2035` eso son dos.

✓ Hechas (ORE, `documentos.rs`): dos filas más de `KINDS`, `/conceptos`, casos 15–18.

### 4.7 · Function, antes de su fila (`medida-forge-function.py`, 2026-09-17)

**La forma.** Tres `apiVersion`, tres formas: v1alpha2 exige `entrypoint` y `datasourceRef`
en cada efecto; desde v1alpha8 `datasourceRef` es `OOS1005` (el destino se deriva por
`backedBy`); v1alpha9 añade `runtime: model` con `model: modelo/<n>` y `prompt`, y bajo
v1alpha8 esas claves son `OOS1005`. `metadata` no admite `labels` (`OOS1005`, y es normativo:
la integridad no se declara sobre uno mismo); `x-rubix-displayName` pasa.

**Una función es la copia.** Sobre acme-retail una función sola no entra, y no por la
integridad: antes salta que la ontología escribiría por una vista **virtual** (`OOS2025`) cuya
raíz no declara `changes.key` (`OOS2024`). Escribir exige `View.materialized`,
`Table.changes.key` y el conducto `materialization.payload` autorizado, y en `hr.empleados`
eso está cerrado a propósito (`OOS4011`: acme-retail no declara ese conducto). El camino
completo, medido sobre `supply.Shipment.status`:

| paso | queda |
|---|---|
| la función sola | `OOS2020 · 2024 · 2025` |
| + Lattice de eje `integrity` | igual (y `OOS2013` latente: el esquema Cedar comprometido no conoce los niveles) |
| + `ore export --format cedarschema` **ahora** | no puede: exige el árbol válido (rc 65) |
| + etiqueta en la propiedad | igual |
| + `Table.changes.key` | `OOS2020 · 2025` |
| + `View.materialized` | `OOS2013` |
| + conducto `materialization.payload` **con el retículo nuevo** | `OOS2013` (sin el retículo en el conducto, `OOS4002`) |
| + esquema Cedar regenerado, al final | **compila** |

Siete escrituras de seis kinds para una función, y la última es una **acción**
(`cedarschema`), no un documento. Con la puerta «no empeora» cada una entra si no añade nada
nuevo, así que el orden importa: lattice (deja `OOS2013`) → conducto → etiqueta → key →
materialized → función → regenerar.

**Las reglas de integridad**, ya sobre ese árbol: sin endosos `OOS7002` (untrusted); un `when`
no cierra; `humanApproval` incondicional llega al techo igual que `attested`; `teamReview`
`OOS7004`; derivada `OOS4008` antes que `OOS7006`; dos fuentes cae por `OOS2024/2025` de la
otra vista; `OOS7001` arrastra por **precondiciones** (`target.x`), no por `input`. Quién
nombra a una función: `Ruleset.duties[].call` (`OOS2001` al retirarla). Quién nombra una
función: la propiedad (`writes`), el `Model` (`OOS2005` sin él; el que vale es el de
`POST /modelos`, con perfil certificado y suscripción) y la política (`authorization`).

**Hueco del compilador.** `effects[].writes` a una propiedad **o a una entidad** que no
existe **compilaba**; retirar la entidad con la función puesta también. La spec (`02-function`
§8) dice `OOS2005`. Nadie resolvía `writes`: `effect.rs::propiedad` devolvía `None` y se
saltaba el efecto. Es el mismo tipo de hueco que `OOS2035`, y se cerró el mismo día en
`ore-core/src/actuar.rs`, para todas las versiones.

**Lo que salió de aquí: v1alpha10.** La medida y la comparación con las funciones de una
ontología de objetos (leer, editar, puente con modelos, action types) llevaron a cambiar la
naturaleza de `Function` en la spec en vez de escribir la fila: `oos` `spec/v1alpha10/`
(«actuar»: `lee(f) ⊆ reads(f)` y `causa(f) ⊆ effects(f)`; `over` y `reads` son **vistas**,
porque la ontología no tiene objetos sino preguntas; `effects` opcional; el puente con el
modelo es la misma función; `OOS7014` la lectura no declarada) y el kind `Action` (la
invocación sin código: `over`, `input`, `preconditions`, exactamente uno de `sets` o `call`).
El compilador lo lee (`actuar.rs`, `effect.rs`) y la suite `conformance/v1alpha10` tiene quince
casos en verde. La fila de Forge para `Function` y `Action` se escribe sobre eso.

Lo que decide la fila:

1. **El verbo no exige nada propio**: todo lo dice el compilador con código. Escribe v1alpha9
   (wasm compila igual bajo v1alpha9) y **no escribe Models**: los nombra.
2. **`quien_nombra`**: Function ← `Ruleset.duties[].call`; y **Entity ← `Function.effects[].writes`**
   (hoy ni el compilador lo ve; el 409 lo tiene que contar el verbo).
3. **La fila sola no sirve para escribir una función en la celda**: hacen falta Lattice,
   ConduitPolicy, Table y la acción `cedarschema` (I2 gobierno + I4 `/acciones`). La consola
   tiene que decir el camino, no un formulario de siete campos.
4. **Antes de la fila, el hueco**: `OOS2005` para `writes` que no resuelve (ore-core + spec +
   conformidad), como se hizo con `OOS2035`.

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
| **I2** | `/documentos` para `View`, `Table` (✓ hechas: `documentos.rs` es un motor y una tabla de kinds; Views real 3/3), `Concept`, `Interface` (✓ hechas, con `GET /conceptos`: importados + locales, quién los habla y quién los exige), `Function`, y los de gobierno | Views ✓, Concepts ✓, Interfaces ✓, Functions, Policies |
| **I3** | `/derivados`: `diagnosticos`, `topologia`, `clasificacion`; `GET /arbol` | Explore, Links (topología) |
| **I4** | `/acciones` y `GET /dependencias`; «Proponer cambios» como ciclo real | Ontology config |
| **I5** | escribir desde Forge: los «Nuevo …» dejan de ser borradores | todas |

Lo que no entra: `kind: Link` (§3), `spec.titleKey` (§4.2), datos de ejemplo en una celda real.
