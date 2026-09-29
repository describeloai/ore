# 0047 · El acceso: decidir y dejar huella, el puente entre el plano de la organización y el del producto

**Estado:** **propuesto, en medición** (2026-09-28) · **Decide:** cómo un módulo del plano de
datos —`ore-serve`, el SDK del puesto, el custodio, lo que venga— **pregunta** si una persona
puede hacer algo y **dice** lo que hizo, sin decidir él ni guardar él el rastro. La pieza se llama
**`ore-acceso`** (*access management*: autorizar y auditar). Completa
[0020](0020-el-plano-de-control.md) ③ («y no autoriza»), que no se revoca: `ore-serve` sigue sin
decidir, y ahora pregunta. Da el primer paso del hueco que
[0007](0007-enlazar-el-evaluador-de-cedar.md) dejó con nombre, y extiende la huella de
`ore-iam` (`008`) al plano de datos. **No decide todavía** el motor definitivo, el catálogo entero
de potestades ni el camino de red: los deciden las medidas de § «Las medidas», en el orden de
§ «Los pasos».

## La necesidad

`ore-iam` es el plano de identidad y acceso: organizaciones, personas, roles como **conjuntos de
potestades** (`014`, `016`), invitaciones, concesiones, y **la huella** (`iam.huella`: cada acto
de ese plano, leer incluido). Sabe quién puede qué **en la organización** y recuerda lo que se
hizo **en él**. `ore-serve` sabe quién pregunta —verifica el token del realm (0020, enmienda del
2026-09-08)— y nada más: 0020 ③ lo dejó así a propósito, porque *la política se aplica en un
punto único, nunca en el consumidor* (`DESIGN` §3.8).

Entre los dos no hay nada, y faltan **dos** preguntas, no una:

- **¿Puede?** Ya hay cinco sitios esperando la respuesta:

  | quién espera | qué pregunta | dónde está escrito |
  |---|---|---|
  | la rama protegida (P2) | ¿puede fusionar sin revisión, o aprobar su propia liberación de `main`? ¿puede cambiar la política? | [0044](0044-ramas-globales.md) B.7 |
  | los orígenes | ¿puede dar de alta una conexión? (van a `main` sin mediación, se gobiernan con potestad) | 0044 B.7 |
  | el puesto | `over("hr.empleados")` lee «con la potestad de `ore-iam`: una vista que no puede leer no llega al DataFrame» | [0031](0031-el-puesto.md) |
  | los equipos | transferir la propiedad «cuando IAM los tenga» | [0027](0027-el-modelo-vive-en-el-arbol.md) |
  | los ficheros | un objeto se sirve «con URL firmada tras Cedar» | [0046](0046-documentos-objetos-y-ficheros.md) |

- **¿Qué hizo?** «X creó una instancia de Code Repositories», «X guardó este activo», «X dio de
  alta un origen», «X lanzó la copia de `ventas`», «a X se le negó fusionar». Hoy una parte queda
  en el commit de la forja de su celda y el resto en ningún sitio (§ «Lo que hay» 6 y 7), y la
  organización no tiene un sitio donde leer qué ha pasado en ella.

Y un sexto que ya resolvió la primera por su cuenta: **el custodio** (`ore-cofre`) enlaza
`ore-iam` como biblioteca y lee sus tablas para decidir si alguien abre un secreto. Se hizo así
para que no hubiera dos motores de autorización (`ore-iam/src/lib.rs`: «tendríamos DOS motores
contestando la misma pregunta — que es exactamente lo que la `0023` rechaza de Vault»).

⇒ **Es una pieza de producto, no un arreglo de `ore-serve`.** Cada módulo nuevo del plano de
datos va a hacer las dos mismas preguntas. Si cada uno se escribe las suyas, el día que cambie una
regla habrá que cambiarla en N sitios y alguno se quedará atrás, y la actividad de la organización
quedará repartida en tantos formatos como módulos. Las dos preguntas tienen que tener **un
contrato, un sitio donde se contestan y se guardan, y un cliente que cualquier módulo enlaza**.

### El nombre, y los papeles

La industria llama a esto *access management*: la autorización (*authorization service*,
*externalized authorization*) y la auditoría (*audit log*, *activity log*) son dos servicios en
las nubes —IAM y Cloud Audit Logs en Google, IAM y CloudTrail en AWS, RBAC y el Activity Log en
Azure— con **una forma común**: cada entrada de auditoría lleva dentro la decisión que la dejó
pasar. Aquí:

| papel (XACML / NIST / AuthZEN) | en ORE |
|---|---|
| **PEP**, lo que intercepta y pregunta | **`ore-acceso`**: el crate que enlaza cada módulo, con dos verbos, `puede` y `hizo` |
| **PDP**, lo que decide | `ore-iam` |
| **PIP**, de donde salen los datos | el censo de `ore-iam`; los dueños, en el árbol |
| **PAP**, donde se escribe la política | las migraciones de `iam` (roles y potestades); `.arbol/` para lo que es del árbol (0044 B.1) |
| el registro | `iam.huella`, de sólo inserción, extendida al plano de datos |

## Lo que hay (medido el 2026-09-28)

1. **Las potestades son todas de gestión.** `iam.potestad` tiene diecisiete, todas del plano de
   control: `celda:crear`, `celda:retirar`, `invitacion:emitir`, `rol:conceder`,
   `secreto:emitir`… **Ninguna es sobre el árbol ni sobre los datos.** El motor es
   `potestad::exige` (`ore-iam/src/potestad.rs`): unión de conjuntos, «no perteneces» y «no
   puedes» dan el mismo mensaje, y otorgar exige que lo otorgado esté contenido en lo propio.
2. **`ore-iam` ya acepta el token de `ore-serve`.** Los dos verifican el mismo emisor
   (`login.paladio.io/realms/rubix`) con la **misma audiencia, `ore-serve`**
   (`malla/40-ore-serve.yaml`, `malla/67-iam-servir.yaml`). Un token reenviado valdría tal cual.
   La otra cara: ese token vale igual contra el `ore-serve` de **cualquier** celda y contra
   `ore-iam`. No hay audiencia por celda.
3. **`ore-serve` sabe de qué organización es por un flag** (`--organizacion demo`), no por
   `ore-iam`. `ore-iam` sabe de quién es cada celda (`iam.celda`, 0025).
4. **No hay camino de red.** La salida de `ore-serve` (`salida-del-control`, en el namespace del
   inquilino, con `deny-all-egress` debajo) llega a DNS, a la forja, al custodio, al metadata
   server y a internet por 443; **no al namespace `identidad`**. El informador de 0026 sí llega
   (`malla/47-el-informador.yaml`), con el token de un agente.
5. **El custodio de cada inquilino lee el censo entero.** `ore-cofre` corre en `t-demo` y su
   papel `ore_cofre` tiene `select` sobre `iam.persona`, `iam.potestades_de_persona`,
   `iam.concesion_viva`, `iam.rol_de_recurso` e `iam.organizacion` **de todas las
   organizaciones**: no hay seguridad por fila en ninguna migración (`020`). Es lo que el
   comentario de `ore-iam/src/lib.rs` quería evitar para `ore-serve` («un pod de un inquilino
   dueño del censo de los demás»), y ya pasa en lectura. Esta pieza puede cerrarlo.
6. **La huella de `ore-iam` existe y es buena.** `iam.huella` (`008`): `quien` y `agente` (el
   `sub` + `act` de RFC 8693), `operacion`, `sobre`, `detalle`; sin `update` ni `delete` para la
   aplicación («una huella que se puede editar es un borrador»). **Todas** las rutas de `ore-iam`
   la dejan, las de leer también, y CI lo hace valer. Hay una potestad para leer la de los demás,
   `actividad:leer-toda` (SECURITYADMIN), y **ninguna ruta que la sirva**.
7. **En `ore-serve` la auditoría es el commit.** Las escrituras del árbol (`escribiendo_en`) y
   las operaciones de la celda (`escribiendo`) acaban en un commit con la persona como autor y el
   agente como committer, y un commit vacío no se hace porque «la historia **es** la auditoría»
   (`git.rs`). **Lo que no es un commit no deja rastro en ningún sitio**: las lecturas, el SQL
   del puesto, lanzar una copia o un trabajo, las denegaciones (`401`, `409`, `423`). Y lo que sí
   lo deja está en la forja de cada celda, en formato git, no como actividad de la organización.
8. **`ore-serve` monta 155 brazos de ruta y ninguno pregunta nada.** Quien tiene sesión puede
   todo lo que la ruta hace. Las guardas que hay son de otra clase: la rama (0044 B.2, `423`), el
   agente del puesto (sólo `GET`), mover datos sólo en `main` (`409`).
9. **El IdP es Keycloak 26.0.7** (`malla/60-idp.yaml`). El intercambio de tokens estándar
   (RFC 8693) es soportado desde la 26.2.

## Lo que hace la industria

Se miraron tres familias (fuentes al final): las nubes (Google Cloud, AWS, Azure), las
plataformas de datos gestionadas (Databricks, Snowflake, Foundry, Confluent, Redpanda, GitHub y
GitLab, MongoDB Atlas, Supabase, Grafana Cloud) y los estándares y motores (el modelo PDP/PEP,
OpenID AuthZEN, Zanzibar, Cedar, OPA/OPAL, Cerbos, OAuth).

### Decidir: lo que hacen todas

1. **El token dice quién eres, nunca lo que puedes.** En Google, AWS y Azure el token lleva
   identidad (y grupos a lo sumo); los permisos se resuelven al decidir. Con permisos dentro del
   token, una revocación espera a que caduque, el token crece y el censo viaja con él. El
   sistema de autorización de Keycloak (UMA, RPT) hace eso mismo y en 2025 se le encontraron
   fallos de confianza entre servidores de recursos del mismo realm.
2. **Un catálogo fijo de verbos por servicio.** `bigquery.tables.getData`, `s3:GetObject`,
   `Microsoft.Storage/…/blobs/read`. Cada servicio declara los suyos, y un rol es un conjunto de
   ellos.
3. **Gestionar no es leer los datos.** Azure separa `Actions` de `DataActions`: un Owner de la
   suscripción no lee los blobs. Igual Databricks (el admin del metastore gestiona permisos y
   tiene que darse `SELECT`), Atlas (su API de administración no lee datos), Redpanda y Supabase.
4. **Primero se niega y luego se permite, y las barreras de la organización nunca conceden.**
   Denegaciones de Google, SCP/RCP de AWS, marcados de Foundry (se exigen todos, siguen al
   linaje, y quitarlos es una potestad aparte que el dueño no tiene).
5. **Saltarse una protección es un permiso con nombre, explícito y registrado.** GitHub: lista
   de quién se la salta, con «sólo mediante PR», y el evento `policy_override`. Google:
   `exceptionPrincipals` en cada denegación. Azure: la elevación tiene su evento y un aviso
   mientras dura. Snowflake: ni `ACCOUNTADMIN` toca lo que no cuelga de él.
6. **La política de aprobación es un dato del contenedor.** Foundry: quién puede revisar,
   cuántas aprobaciones, si el autor aprueba lo suyo; sólo el dueño la cambia, y cambiarla
   reevalúa las propuestas abiertas. GitHub: la última subida la aprueba otro, y las
   aprobaciones viejas caducan.
7. **Aprobación del dueño por ruta o paquete**: CODEOWNERS en GitHub y GitLab.
8. **Al motor, credenciales cortas y recortadas, nunca acceso permanente**: Unity Catalog se las
   da al motor que las pide.
9. **Si no hay respuesta se niega, y toda denegación se explica.** AWS da un identificador con el
   que ver qué política decidió; Azure dice la acción y el ámbito que faltaron.
10. **La propagación se reconoce y se acota**: Google de 2 a 7 minutos, Azure hasta 10.

### Dejar huella: lo que hacen todas

11. **La auditoría es otro servicio, con la forma de la decisión dentro.** Una entrada de Cloud
    Audit Logs lleva quién, en nombre de quién, qué método, sobre qué recurso, el resultado y
    `authorizationInfo` (permiso, recurso, concedido). CloudTrail lo mismo, con el `errorCode`.
12. **La gestión se registra siempre; leer datos, si se pide.** Google: *Admin Activity* siempre
    encendido, *Data Access* apagado por defecto (salvo BigQuery). AWS: eventos de gestión
    siempre, de datos opcionales. Azure: el Activity Log siempre, los registros del plano de datos
    son diagnósticos que se activan. La razón es el volumen.
13. **Las denegaciones se registran por defecto** (Google *Policy Denied*): quién intentó qué y
    por qué no.
14. **Una vista por organización, que no se edita y que se lee con su propia potestad.**
    `system.access.audit` en Databricks, `ACCESS_HISTORY` en Snowflake, el *audit log* de la
    organización en GitHub; se exporta a almacenamiento o a un SIEM. Y lo que es un saltarse
    algo tiene su propio tipo de evento, para poder alertar sobre él.

### Dónde se decide: dos familias

| | decide un servicio central | cada servicio evalúa una copia |
|---|---|---|
| quién | Google (Zanzibar: ~3 ms en la mediana, 20 ms en el p99), AWS Verified Permissions | AWS IAM por región, Confluent (las reglas viajan por un topic y el broker no sirve hasta tenerlas), Azure en el plano de datos |
| a favor | la revocación vale al instante; el censo no sale del plano de control | microsegundos; sigue funcionando si el centro cae |
| en contra | un salto de red; el centro es dependencia crítica | hay que construir la sincronización, sus versiones y el «nuevo enemigo» (una copia vieja deja pasar a quien acaban de quitar) |

### El contrato, y el motor

- **OpenID AuthZEN, Authorization API 1.0**, especificación final desde enero de 2026:
  `POST /access/v1/evaluation` con `{subject, action, resource, context}` → `{decision,
  context}`; `/evaluations` por lotes; búsquedas de sujeto, recurso y acción. Lo implementan
  Cerbos, OpenFGA, AWS Verified Permissions, motores sobre OPA y otros. Hablarlo deja cambiar el
  motor sin cambiar a quien pregunta.
- **Cedar**: `permit` y `forbid` (un `forbid` gana siempre, sin política se niega), jerarquía de
  entidades para roles, atributos para dueños y contexto. El crate `cedar-policy` es la
  implementación de referencia, en Rust, con validación por esquema y análisis simbólico
  (`cedar-policy-symcc`) que **demuestra** propiedades del conjunto de políticas. El árbol ya
  emite su esquema (`ore export cedar`) y nadie lo evalúa (0007).
- **Zanzibar (SpiceDB, OpenFGA)**: potente para grafos de compartición, pero obliga a escribir
  cada dato dos veces (Postgres y las tuplas) y sus clientes en Rust son de la comunidad.
- **OPA/OPAL, Cerbos**: la familia de «copia local» hecha producto; Go, o WASM.
- **Para la huella no hay un estándar de la talla de AuthZEN**; la forma de referencia es la
  entrada de Cloud Audit Logs (o CloudTrail), y `iam.huella` ya se le parece.

## La derivación a ORE: hipótesis, cada una con su medida

Lo de arriba es lo que hacen otros. Lo que sigue es lo que **creemos** que toca aquí, y cada
línea dice qué medida la confirma o la tumba. Ninguna es decisión hasta que su medida hable.

### Decidir

| | hipótesis | por qué | se decide con |
|---|---|---|---|
| H1 | el token sigue siendo sólo identidad | § 1; ya lo es | — (ya medido) |
| H2 | cada módulo **declara** sus verbos (`recurso:verbo`, la forma de `iam.potestad`), y `ore-iam` los guarda como catálogo | § 2 | **M1**: cuántos verbos salen de las rutas de `ore-serve` |
| H3 | las potestades de gestión y las de datos son **dos conjuntos**, y ser admin no da las de datos | § 3 | **M1**: qué rutas son de gestión, de árbol, de celda y de datos |
| H4 | decide `ore-iam`, en forma AuthZEN; la respuesta lleva `version` para poder pasar un día a copia local sin cambiar a quien pregunta | § «dos familias»: el volumen de preguntas decide | **M1** (cuántas rutas preguntan) y **M2** (cuántas veces por minuto, y cuánto cuesta el salto) |
| H5 | el sujeto sale del token que verifica `ore-iam`, **nunca del cuerpo**; la organización, de la celda a la que pertenece quien pregunta, no de lo que diga | aislamiento entre inquilinos: un punto de decisión común a todos deriva la organización de lo que verifica él, no de lo que le dicen | **M3**: cómo se identifica el `ore-serve` de una celda ante `ore-iam` |
| H6 | `puede` guarda la respuesta unos segundos, niega con **503** si no hay respuesta (no 403) y cada denegación lleva un identificador con el que ver por qué | § 9 | **M2** |
| H7 | saltarse la protección es una potestad propia (`propuesta:fusionar-sin-revision`), dicha en el merge como ya se hace | § 5; 0044 B.3 | **M1** (el catálogo) |
| H8 | el custodio deja de leer `iam` por SQL y pregunta por el puente; su papel pierde el `select` sobre el censo | «lo que hay» 5 | **M4**: qué lee exactamente el custodio y cuántas veces |
| H9 | el motor de hoy (conjuntos) basta para P2; **Cedar** entra con las decisiones por recurso (dueños, vistas, «nadie aprueba lo suyo» como `forbid` demostrable) | § «el contrato» | **M1** (qué preguntas necesitan el recurso y no sólo la organización) |
| H10 | un token por celda (audiencia recortada, RFC 8693) para que el de una celda no valga en otra | «lo que hay» 2 y 9 | **M5**: qué cuesta subir Keycloak a 26.2 |

### Dejar huella

| | hipótesis | por qué | se decide con |
|---|---|---|---|
| H11 | `hizo` emite un evento **después** de actuar, con la forma de `iam.huella` más la organización, la celda, el resultado y la decisión que lo dejó pasar | § 11 | **M1** (qué evento emite cada ruta) y **M6** (la huella de hoy) |
| H12 | se registran **siempre** la gestión, las escrituras y las denegaciones; las lecturas de datos, sólo si la organización lo enciende | § 12 y 13: el volumen | **M2** (cuántas lecturas) y **M6** (cuánto cabe en Postgres) |
| H13 | **el commit sigue siendo la auditoría fina del árbol**: el evento apunta a él (`commit`), no copia el cambio | «lo que hay» 7: la historia ya es la auditoría | **M1**: qué rutas ya cubre un commit |
| H14 | la actividad de la organización se lee en `ore-iam` (la propia, sin potestad; la de todos, con `actividad:leer-toda`), y leerla deja huella, como ya es regla | § 14; `008` | **M6** |
| H15 | `hizo` no hace fallar lo que ya se hizo: si el registro no está, el evento espera en la celda y se reintenta; lo que **no** puede quedarse sin rastro (saltarse una protección) lo registra antes de actuar | una huella perdida es un control perdido, y un fallo del registro no puede deshacer un commit | **M2** y **M3** |

## Las medidas

### M1 · El inventario de preguntas y de eventos (hecha el 2026-09-28)

**Qué:** cada ruta que monta `ore-serve`, con lo que hace. Estático, sobre el código:
`pruebas-de-fuego/medida-el-acceso.py`, sin cluster, sin red.

1. Los brazos del enrutador de `rutas.rs`: método, camino y la función que atiende.
2. Por función, lo que toca: escribe el árbol en una rama o en `main` (`escribiendo_en`), opera
   la celda (`escribiendo`), habla con la forja, lanza un Job, abre el custodio, entra en un
   puesto.
3. La clase de cada ruta: **lectura**, **árbol**, **celda**, **propuesta o rama**, **puesto**, u
   otra que haya que mirar a mano.
4. La potestad candidata (`recurso:verbo`: el primer segmento en singular, y el verbo del
   camino o del método), y cuántas distintas salen.
5. Si la ruta necesita el **recurso** para decidir (lleva un nombre en el camino) o le basta la
   organización.
6. El evento que emitiría (`hizo`), si **ya lo cubre un commit**, y cuántas rutas que escriben
   **no dejan hoy ningún rastro**.
7. Las potestades y los roles que ya existen, leídos de `iam/migraciones`.
8. Lo que el enrutador monta y `rutas::mapa` no anuncia al arrancar.

**Qué decide:** el tamaño y la forma del catálogo (H2), la frontera gestión/datos (H3), cuántas
rutas preguntan y cuántas son lectura (H4), cuántas necesitan recurso (H9), y el hueco de la
huella (H11, H13).

**Su límite:** mira la función que atiende, un nivel; lo que se decida más abajo sale como «otra».
Se dice en el informe.

#### M1 · Lo medido (2026-09-28)

`medida-el-acceso.py` sobre `main`, y las diez rutas «otra» y las que no dejan rastro, miradas a
mano. Dos correcciones del primer pase, ya en el script: un brazo con alternativas
(`(...) | (...) =>`) comparte el cuerpo del siguiente (las de `/documentos` salían sin rastro y lo
dejan), y **encolar es un rastro propio**: un commit en la cola de trabajos (`trabajo.git`) con la
persona de autora (`forja.publicar(dir, sujeto, …)`).

**101 rutas** (método × camino, sin `salud` ni `version`): **45 leen**, **56 escriben**, y **73**
llevan un recurso en el camino.

| clase | rutas | qué son |
|---|---|---|
| lectura | 28 | el catálogo: árbol, assets, paquetes, datasets (su ficha), fuentes, modelos, trabajos |
| árbol | 25 | editar el árbol: ficheros, documentos, paquetes, schemas, proyectos, repositorios |
| puesto | 17 | abrir, ejecutar, SQL, LSP, salidas, *transforms* |
| propuesta/rama | 12 | proponer, revisar, fusionar, ramas y su protección |
| celda | 9 | fuentes, modelos, copias, decisiones, confirmar datasets |
| otra | 10 | ocho **encolan un trabajo** (catalogar, comprobar, invocar, rehacer una copia, abrir un trabajo, el entorno) y dos **ejecutan una vista**: una lectura de datos hecha con `POST` |

**El rastro de las 56 que escriben:**

| rastro | rutas | cuál |
|---|---|---|
| commit en el árbol | 31 | todo lo del árbol y de la celda |
| commit en la cola | 10 | los trabajos encolados. **Idempotente**: repetir lo mismo no deja un segundo rastro |
| la forja | 7 | la PR, la revisión, el merge, la rama |
| **ninguno** | **8** | **seis del puesto** (ejecutar, SQL, LSP y su salida, la salida de una celda, declarar un *transform*) y **las dos de ejecutar una vista** |

**Lo que dice:**

1. **H13 se confirma, y se afina.** El commit ya es la huella fina de 31 escrituras, y la cola de
   otras 10: 48 de 56 dejan rastro con la persona dentro. `hizo` **no** tiene que copiarlo: tiene
   que **apuntarlo** (el commit, el de la cola, el número de PR) y sacarlo de la forja de la celda
   a la actividad de la organización. Hay dos cosas que el commit no da: lo **repetido** (la cola
   es idempotente) y lo **negado**.
2. **El hueco de la huella está donde se tocan los datos.** Las 8 sin rastro son el puesto y
   ejecutar una vista: **leer y computar datos**. Es la clase que las nubes registran «si se pide»
   (H12), y aquí es la única que no deja nada. Con M7, es el sitio de la promesa de 0031.
3. **H3 se confirma: hay una frontera de verdad.** Unas **23 rutas** leen o computan datos (el
   puesto entero, ejecutar una vista, invocar una función y leer sus resultados: 17 + 2 + 2 + 2);
   el resto gestiona el árbol, la celda o las propuestas. Son las `DataActions` de
   Azure.
4. **H2 se afina: el catálogo no es una potestad por ruta.** El nombre ingenuo (`recurso:verbo`
   por ruta) da **61 potestades**: demasiadas para repartir en roles, y la industria no lo hace
   así (Google agrupa por recurso, y un rol reúne decenas). Agrupadas por lo que deciden salen
   unas **quince**: leer el catálogo; escribir el árbol en una rama; proponer, revisar y
   fusionar; fusionar sin revisión y cambiar la protección; dar de alta y retirar fuentes y
   modelos; lanzar copias y decidir sobre ellas; abrir un puesto; leer datos; invocar funciones.
   El catálogo final sale del paso A1.
5. **H9 se confirma para P2.** Las tres preguntas de P2 (fusionar sin revisión, cambiar la
   protección, dar de alta una fuente) son **de la organización**, no del recurso: el motor de
   conjuntos de hoy basta. Lo que necesita el recurso (los 73 caminos con nombre) es lectura de
   datos y dueños: A8.
6. **H4 depende de las lecturas, y queda para M2.** 45 de las 101 son `GET` del catálogo. Si
   cada una pregunta a `ore-iam`, el salto se paga en cada pantalla. Hoy «pertenecer ya da
   lectura» (`iam.por_defecto`): leer el catálogo es una pregunta de pertenencia que se puede
   guardar por sesión, no por ruta. Cuántas son por minuto lo dice M2.
7. **De paso: el mapa de arranque no dice la verdad.** `rutas::mapa`, lo que `ore-serve` anuncia
   al arrancar, **no anuncia 34** de las rutas montadas (entre ellas `PUT /ramas/{r}/proteccion`,
   `POST /puestos/{id}/sql` y `POST /trabajos`). Es el argumento de H2 visto al revés: si la
   potestad de cada ruta se declara aparte de la ruta, se desincroniza igual. La declaración tiene
   que estar **en la ruta misma**, y el mapa y el catálogo salir de ella.

### M2 · Frecuencia y coste del salto (por preparar)

Cuántas peticiones por minuto recibe cada clase de ruta en `t-demo` y `victor`, y la latencia de
una petición de `ore-serve` a `ore-iam` dentro del cluster. `ore-serve` no registra sus
peticiones: la medida necesita, primero, contarlas.

Desde A1 cierra además los tres valores marcados **⟨M2⟩** en § «El contrato»:
- `vale`: cuánto se guarda una respuesta (de partida, 30 s);
- el tiempo de espera antes del 503 (de partida, 2 s);
- cuánto guarda `ore-iam` las decisiones de escritura para los reintentos de `hizo` (de partida,
  24 h), y cuántas son al día.

#### M2 · Cómo se mide (preparada el 2026-09-29)

Cuatro preguntas. Cada una con la fuente que la contesta, y de dónde sale esa fuente: si ya está,
si se lee o si hay que crearla.

| | pregunta | fuente | ¿existe? |
|---|---|---|---|
| Q1 | cuántas peticiones por minuto recibe `ore-serve`, por clase de ruta (las de M1), por inquilino y por clase de sujeto (persona, agente); la media y el pico | **una línea por petición en `ore-serve`** (M2.3) | **no**: hace falta código |
| Q2 | cuánto cuesta preguntar a `ore-iam` desde dentro | los registros del balanceador de `ore-iam` (M2.1), y el viaje y la consulta medidos dentro (M2.4) | en parte |
| Q3 | cuántas decisiones de escritura hay al día (lo que `ore-iam` tendría que guardar para los reintentos) | los commits de la forja de cada celda (M2.2): 48 de las 56 escrituras dejan uno (M1) | **sí**, veinte días de historia |
| Q4 | cuántas lecturas de datos hay (el interruptor de H12) | la línea de M2.3, las clases «puesto» y «datos» | no: la misma línea |

**M2.1 · La latencia de `ore-iam` vista por el balanceador. Hecha: los registros ya existen.** El
backend de `ore-iam` registra el 100 % de las peticiones y el de `ore-serve` ninguna. Se leyeron
las últimas 2.000 entradas, del 27 al 29 de septiembre:

| ruta | peticiones | p50 | p95 |
|---|---|---|---|
| `GET /organizaciones` | 1.007 | 41 ms | 97 ms |
| `GET /organizaciones/{id}/celdas` | 993 | 41 ms | 86 ms |

Eso es **lo que tarda hoy una pregunta a `ore-iam` de punta a punta**: el balanceador, la
transacción, la consulta de potestades y la fila de la huella. Es el techo: desde dentro no hay
balanceador. Y da una medida indirecta de la consola, que pide `/organizaciones` cada vez que
carga, unas 22 veces por hora.

**M2.2 · Las escrituras de verdad, por la forja. De lectura.** Por celda, los commits de
`ontologia.git` (el árbol y la celda) y de `trabajo.git` (la cola), agrupados por día y según su
autor sea persona o agente. Salen de `git log` dentro del pod de la forja, y sólo se imprimen
cuentas. Las PR, las revisiones y las fusiones salen de la API de la forja, también como cuentas.
Contesta Q3 con veinte días de historia real, sin esperar a nada.

**M2.3 · Contar las peticiones de `ore-serve`. Código: necesita tu go.** Una línea por petición,
a la salida estándar, al terminar de atenderla:

    acceso · GET /paquetes/{}/vistas/{} · 200 · 12 ms · persona

- **El camino va sin nombres.** Cada segmento que no es un literal de las rutas se escribe `{}`.
  El registro no guarda qué paquete, qué tabla ni quién: lo que mide es **cuántas y de qué
  clase**. El vocabulario de literales sale del propio enrutador. Una prueba comprueba que cada
  brazo de `rutas.rs` da un patrón con sentido, así que una ruta nueva no sale como `{}`.
- **Sin flag y sin malla.** Va por la salida del proceso y se lee con `kubectl logs`. No hay nada que
  desplegar aparte del binario, así que no choca con la regla de no empujar juntos un flag nuevo
  y la malla que lo usa.
- **Es el primer trozo de A4.** El sitio donde se escribe esa línea es el mismo por donde pasará
  `puede`, al entrar en cada ruta.

Y **`pruebas-de-fuego/medida-el-salto.sh`** la agrega: lee las líneas de los tres `ore-serve`,
asigna la clase con la tabla de M1 (`medida-el-acceso.py`, que ganará una salida JSON), y da por
clase y por inquilino el total, la media por minuto y el minuto pico.

**M2.4 · El salto dentro del cluster. De lectura, con un pod efímero.**
- **El viaje:** un pod con las etiquetas del informador, el único que hoy tiene camino, pide 500
  veces `/salud` a `ore-iam` (p50, p95, p99).
- **La consulta:** en la base, `EXPLAIN ANALYZE` de la consulta de `potestad::exige` y de una
  fila de la huella dentro de una transacción que se deshace.

Juntos dan el coste de `puede` sin el balanceador.

**Cuándo habla Q1, sin esperar a un «día cualquiera».** El recuento tiene que haber visto **cada
clase de ruta al menos una vez en uso real**:
- una sesión de consola que navegue el catálogo y edite en una rama;
- una propuesta abierta y fusionada;
- una sesión de puesto que lea una tabla (`loadTable`) y ejecute SQL;
- una pasada del catalogador (el agente).

Se provocan en `t-demo`, como los eventos de A7a.6, y se cuentan. El ritmo de esos minutos es el
pico con una persona. Q2 × pico da lo que `puede` añade a una pantalla.

**Cómo cierra los valores ⟨M2⟩:**
- **`vale`:** el más corto que mantiene las preguntas por minuto de una sesión por debajo de lo
  que `ore-iam` atiende sin notarlo (M2.1 da su techo actual).
- **El tiempo de espera antes del 503:** el p99 de M2.4 con margen. Si queda muy por debajo de
  2 s, baja.
- **La retención de decisiones:** M2.2 dice cuántas son al día. 24 h cuesta eso por celda.

#### M2 · Lo medido (2026-09-29, Q2 y Q3; Q1 y Q4 esperan a M2.3 en vivo)

`pruebas-de-fuego/medida-el-salto.sh`, secciones 1 a 3. La 4 lee las líneas de M2.3 cuando
existan.

**Q3 · las escrituras, en 30 días de la forja:**

| celda | árbol (persona / sistema) | cola (persona / sistema) | máx. al día | máx. en una hora | PR |
|---|---|---|---|---|---|
| demo | 74 / 34 | 11 / 50 | 33 | 18 | 0 |
| prueba | 1 / 1 | 0 / 39 | 8 | 2 | 0 |
| victor | 73 / 40 | 53 / 66 | 29 | 15 | 7 |

- **Las escrituras de personas son pocas.** Unas 210 en 30 días entre las tres celdas, y el día
  con más no pasa de 33. Guardar 24 h las decisiones de escritura son **decenas de filas por
  celda**. ⟨M2⟩ queda cerrado: **24 h**, y sobra.
- «Sistema» es lo que no pasó por `ore-serve`: la semilla y el aprovisionador en el árbol, y en la
  cola los Jobs periódicos.

**Q2 · el salto, desde dentro.** 500 peticiones desde `t-demo` a `ore-iam:8090/salud`, cada una
con conexión nueva:

| tramo | p50 | p95 | p99 |
|---|---|---|---|
| DNS | 3,6 ms | **81,5 ms** | 83,2 ms |
| conexión | 0,3 ms | 0,7 ms | 70,8 ms |
| respuesta | 0,7 ms | 1,7 ms | 67,7 ms |
| total | 4,7 ms | 83,1 ms | 84,3 ms |

Y en la base, 1.000 veces cada una: **la consulta de `potestad::exige`, 0,36 ms**, y **una fila
de la huella, 0,014 ms**.

**Lo que dice:**

1. **Decidir es barato. Lo caro es el camino si se hace mal.** La respuesta de `ore-iam` y la
   consulta suman alrededor de 1 ms. La cola de 80 ms es **el DNS**. El pod tiene `ndots:5` y
   `ore-iam.identidad.svc.cluster.local` sólo lleva cuatro puntos, así que antes de dar con el
   nombre bueno se prueban los dominios de búsqueda.
   ⇒ **`ore-acceso` reutiliza la conexión y resuelve una vez** (o nombra con punto final). Pasa
   a ser una condición de A4, no una optimización.
2. **Los 41 ms de M2.1 son el balanceador**, el TLS y el camino de fuera, no `ore-iam`. Desde
   dentro, un `puede` ronda los milisegundos.
3. **El tiempo de espera antes del 503 puede bajar.** Con 2 s el margen es de veinte veces el
   p99, contando el DNS. Queda en 2 s hasta que Q1 diga cuántas preguntas hace una pantalla. Si
   con conexión reutilizada el p99 queda por debajo de 5 ms, **500 ms**.
4. **`vale` espera a Q1.** Con un `puede` de alrededor de 1 ms, la caché sirve para quitar carga
   a `ore-iam`, no para esconder latencia. Su valor depende de cuántas preguntas por minuto haya.

**M2.3 · en código.** `crates/ore-serve/src/recuento.rs` escribe una línea por petición, salvo
`/salud`, por la salida de error. ✏️ Cloud Logging **no** la recoge: el cluster sólo le manda
los componentes del sistema (`SYSTEM_COMPONENTS`), así que la sección 4 lee con `kubectl logs`,
y un pod reemplazado se lleva sus líneas. Para los cuatro eventos basta. Para una serie larga
habría que encender `WORKLOADS`, que tiene coste y es una decisión aparte:

    acceso · GET /paquetes/{}/vistas/{} · 200 · 12 ms · persona

- El vocabulario de literales se lee de `rutas.rs` y de `catalogo.rs` al compilar. Tres pruebas
  lo sujetan: cada brazo vuelve a su patrón, un nombre no sale, y en el vocabulario no entran
  cabeceras, argumentos ni los nombres de ejemplo de las pruebas.
- Q1 y Q4 hablan cuando corra en las celdas y hayan pasado los cuatro eventos.
- `medida-el-acceso.py` gana `--json`, la tabla de M1 que usa la sección 4.
- ✏️ De paso: **M1 no veía el catálogo Iceberg.** Sus rutas cuelgan de `(_, ["v1", resto @ ..])`,
  un brazo sin método, y `loadTable` es una lectura de datos. La sección 4 lo clasifica aparte
  (`iceberg`, e `iceberg (datos)` para `GET …/tables/{}`).

### M3 · El camino (hecha el 2026-09-29)

Qué regla de red hace falta, con qué identidad se presenta `ore-serve` ante `ore-iam` (el token
reenviado de la persona, más la cuenta de servicio de la celda) y cómo sabe `ore-iam` qué celda
pregunta. El informador de 0026 es el precedente.

#### M3 · Lo medido

`pruebas-de-fuego/medida-el-camino.sh`, en el cluster. Lanza un pod efímero por inquilino con
las etiquetas de `ore-serve` (`ore.dev/rol=control`), para que le apliquen sus mismas
NetworkPolicy, y lo borra al terminar. De cada token pide sólo los *claims*: nunca lo imprime.

- **No hay camino, desde ninguna celda.** Desde `t-demo`, `t-prueba` y `t-victor`, `ore-iam:8090`
  no contesta en 5 s y el IdP da 200. Lo corta la **salida**: el namespace del inquilino sólo deja
  salir a `identidad` por el 8080 (`salida-al-emisor`), y al 8090 sólo al informador. La
  **entrada** de `ore-iam` es más ancha de lo que usa. Deja pasar desde **cualquier pod de un
  namespace con `ore.dev/tenant=demo`**, una regla de antes de las celdas que nada usa, y desde el
  balanceador, porque la consola llega a `ore-iam` por fuera. ⇒ **La red no es la frontera de
  identidad:** `ore-iam` ya es público y decide por el token. La regla que falta es una salida
  (`control` → `iam-servidor:8090`) y su entrada, recortada a `ore.dev/rol=control`.
- **La celda ya tiene una identidad que no es un secreto.** Cada `ore-serve` corre con Workload
  Identity como `ore-serve-<celda>`, y el servidor de metadatos le da **un token de Google firmado
  con la audiencia que pida**. Medido con `aud=ore-iam` en los tres: `iss
  https://accounts.google.com`, el correo de su cuenta, su `sub` numérico y una hora de vida.
  **Sólo la cuenta de Kubernetes `t-<n>/ore-serve` puede ser esa cuenta**
  (`workloadIdentityUser`, un único miembro). Y en ninguno de los tres namespaces puede nadie
  crear pods ni pedir tokens de otra cuenta (ni `ore-serve`, ni el puesto, ni el driver, ni el
  informador, ni el custodio). No hay nada guardado que robar: el token nace en el nodo, dura una
  hora y sólo vale para la audiencia que dice.
- **El agente de 0026 no sirve como identidad de la celda.** Es un cliente del realm por celda, y
  el realm le estampa `rubix_tipo=agente` y `rubix_celda=<celda>`. Pero su credencial la leen
  **tres** cuentas: `ore-driver-<n>`, `ore-informador-<n>` y **`ore-puesto-<n>`**, y el puesto
  ejecuta código de la persona. Si la celda preguntara como el agente, cualquier código de un
  puesto podría presentarse como la celda. Y en `iam` el agente es **de la organización**, no de
  la celda (`iam.agente` no tiene columna de celda). Encima, en los datos de hoy:
  - `ore-agente`, el cliente de antes de los agentes por celda, está **registrado en dos
    organizaciones** (demo y prueba) con el mismo `sub`, y sigue habilitado en el realm.
  - `ore-agente-prueba-dos` sigue habilitado y registrado **con su celda retirada**.
- **`ore-serve` todavía no habla con `ore-iam`.** Ninguna ruta lo llama, y su organización es un
  flag (`--organizacion`, § «Lo que hay» 3).

**Lo que dice:**

1. **H5 se confirma, y se concreta.** La celda se presenta con **su token de Google**
   (`aud=ore-iam`). La persona va aparte, con su token reenviado. `ore-iam` verifica los dos: la
   organización sale de la celda, y la celda de un dato que escribe la plataforma. **Nunca del
   cuerpo, ni del nombre de la cuenta.** La correspondencia es por `(emisor, sub)`, como la de las
   personas: un correo se puede volver a crear después de borrado, y un `sub` no.
2. **`ore-iam` gana un segundo emisor, que sólo sirve para celdas.** Hoy verifica un emisor (el
   realm) con una audiencia (`ore-serve`). La identidad de la celda trae otro
   (`accounts.google.com`, `aud=ore-iam`), y con él una **clase** nueva, `celda`, que sólo puede
   pedir las rutas del puente (la tabla de clases de 0025 E5 y 0026). Sus llaves se traen como las
   del realm (`50-jwks.yaml`): por un Job, a un fichero, no por el servidor en vivo.
3. **La correspondencia cuenta → celda la escribe el aprovisionador**, que es quien crea la cuenta.
   Va en `iam.celda`, dentro del verbo que ya tiene (`aprovisionada`), y no se deduce del nombre.
4. **Otro proveedor traerá otro emisor, no otro diseño.** Hoy todas las celdas activas son de
   `gcp` y corren en `ore-mesh`. La identidad de carga de trabajo de AWS y de Azure también es un
   token OIDC firmado, con su emisor. Por eso la tabla guarda `(emisor, sub)`, no «la cuenta de
   Google».
5. **Hallazgos, fuera de este paso:**
   - el cliente viejo `ore-agente` sigue vivo y registrado en dos organizaciones;
   - retirar una celda no retira su agente (`prueba-dos`);
   - el puesto lee la credencial del agente de su celda, y llega al IdP y a `ore-serve` (las
     salidas que valen para todos los pods del namespace). Con eso puede pedir un token de agente
     y hacer `GET` en su `ore-serve`, que es lo que ya se le deja. A `ore-iam` no llega: no tiene
     salida al 8090 ni a internet.

   Se apuntan para A7b, y el primero se puede cerrar ya.

### M4 · El custodio (hecha el 2026-09-28)

Qué consultas hace `ore-cofre` sobre `iam`, en qué rutas, y si todas caben en el contrato del
puente. Estática, sobre `crates/ore-cofre`, y con el cluster para contar los custodios.

#### M4 · Lo medido

- **Tres custodios, un solo papel.** Corren en `t-demo`, `t-prueba` y `t-victor`, y los tres
  entran a la base como **`ore_cofre`**: el mismo papel de Postgres. Ese papel tiene `select` sobre
  el censo de **todas** las organizaciones (§ «Lo que hay» 5) y sobre todo el esquema `cofre`. El
  custodio de un inquilino puede leer los metadatos de los secretos, las concesiones y las
  personas de los demás. Los **valores** no: desde 0024-⑤ viven en el Secret Manager de cada celda,
  con su KEK como CMEK (lo que M7 comprueba).
- **Cuatro rutas** (`emitir`, `listar`, `resolver`, `retirar`) y dos verbos de operador por la
  línea de órdenes (`mudar`, `huerfanos`).
- **Lo que pregunta a `iam`:**

  | pregunta | cómo | en el puente |
  |---|---|---|
  | ¿puede emitir, listar o retirar secretos? | `potestad::exige` (`secreto:emitir`, `secreto:listar`, `secreto:retirar`) | `puede`, **de la organización** |
  | ¿puede usar **este** secreto? (`resolver`) | `iam.concesion_viva`: una concesión `usar` u `owner` sobre `secreto/{nombre}` | `puede`, **con recurso**: la primera pregunta que no es de conjuntos, y cabe en AuthZEN (`resource: {type: secreto, id}`) |
  | ¿quién es? ¿es persona o agente? | `verbos::persona_id`, `verbos::sujeto_id` | lo contesta el token verificado |
  | ¿de qué organización y celda es el secreto? | `iam.organizacion`, `iam.celda` | lo sabe `ore-iam` por la celda que pregunta (H5) |

- **Lo que escribe en `iam`, que no son preguntas:** al emitir, concede `owner` a quien emite y
  `usar` a los agentes de la organización (`iam.conceder_de_secreto`); al retirar, revoca
  (`iam.revocar_de_secreto`). **Eso es administrar concesiones**, el papel que el ADR llama PAP, y
  no cabe en `puede` ni en `hizo`: pide una ruta de `ore-iam` para conceder sobre un recurso.
- **La huella va en la misma transacción que el acto**, y en `resolver` **antes de contestar**
  («si no queda la huella, no sale el valor»). Por HTTP esa atomicidad se pierde: es exactamente
  la excepción de H15 (lo que no puede quedarse sin rastro se registra antes de actuar, y si no se
  puede, no se actúa).
- **Frecuencia:** 139 `secreto:resolver` en 20 días (M6). La latencia del salto no importa aquí.

**Lo que dice:** H8 se confirma, con dos matices. Las preguntas y la huella caben en el puente;
**las concesiones no**, y piden su ruta en `ore-iam`. Y el cruce entre inquilinos no espera a A7:
**un papel de base por celda, o seguridad por fila en `iam` y `cofre` por organización**, cierra
hoy lo que el puente cerraría mañana. A7 pasa a tener dos tiempos: A7a, el papel por celda (un
arreglo de seguridad, pequeño); A7b, el custodio por el puente.

### M5 · El token por celda (por preparar)

Qué cambia al subir Keycloak de 26.0.7 a 26.2 y qué hace falta en el realm para el intercambio
estándar.

### M6 · La huella de hoy (hecha el 2026-09-28)

Cuántas filas tiene `iam.huella` y a qué ritmo crece, qué operaciones hay, si sus columnas y sus
índices aguantan la actividad del plano de datos (la organización y la celda no son columnas), y
qué haría falta para servirla por organización. `pruebas-de-fuego/medida-la-huella.sh`: de
lectura, en el Postgres de `identidad`, sólo agregados (ni personas, ni `sobre`, ni `detalle`;
de `detalle`, sólo sus claves).

#### M6 · Lo medido

- **16.299 filas en 20 días** (del 8 al 28 de septiembre), **6,8 MB**; entre 700 y 2.300 al día.
- **El 98 % es la máquina y el sondeo:** `celda:aprovisionada` 9.592 (el 59 %: el aprovisionador,
  cada cinco minutos y por celda), `organizacion:listar` 3.310 y `celda:listar` 3.017 (la consola,
  al cargar). Los actos de gestión de personas —invitar, conceder, emitir o retirar secretos,
  crear celdas— **no llegan a 150 filas**.
- **La organización no es columna.** Sólo 206 filas la llevan (en `detalle`); la celda, 9.735.
  Los índices son por `cuando`, por `quien` y por `operacion`: **ninguno por organización**. Hoy
  no se puede servir «la actividad de mi organización» sin recorrer la tabla.
- `agente` va en 21 filas. `sobre` va siempre, sin tipo delante.
- ⛔ **La huella se puede editar.** `008` promete «sin `update` y sin `delete` en el papel de la
  aplicación», y **`ore_iam` tiene `UPDATE` y `DELETE`** sobre `iam.huella`: el `grant` general de
  `020` (`select, insert, update, delete on all tables in schema iam to ore_iam`) lo dio, y no hay
  trigger ni regla que lo impida. Y la base `iam` vive en el Postgres del IdP (`idp-db-0`), dueña
  de sus tablas el papel `keycloak`, que es **superusuario**.

**Lo que dice:**

1. **Postgres aguanta H11 y H12.** Gestión y escrituras del plano de datos son del orden de lo de
   hoy: decenas de MB al mes. El volumen que no aguantaría es el de leer datos, que H12 ya deja
   para cuando la organización lo encienda.
2. **Antes de extender la huella hay que cumplir su promesa.** Hecho aparte, antes de A2:
   `iam/migraciones/039-la-huella-no-se-edita.sql` quita `update`, `delete` y `truncate` a
   `ore_iam` y pone un trigger que los niega **a cualquiera**, el dueño superusuario incluido
   (quitarlo es otra migración, con nombre); `los-verbos.sh` 9b lo prueba en las dos capas. Una
   huella que se puede editar es un borrador.
3. **H14 necesita dos columnas y un índice**: `organizacion` y `celda`, con
   `(organizacion, cuando desc)`.
4. **El ruido tiene nombre.** La regla de `008` («leer deja huella») aplicada al sondeo de la
   consola y al aprovisionador llena la tabla de lo que nadie va a leer. Las nubes lo separan: lo
   que hace el sistema no es actividad de la organización. La actividad que se sirva (A6) filtra
   por clase —persona, agente, sistema— y el sondeo del plano de control deja de ser una fila por
   petición.
5. **El dueño superusuario y el IdP compartiendo base** no son de este ADR, pero se nombran: el
   censo del plano de identidad vive en la base del proveedor de identidad, y su dueño lo puede
   todo.

### M7 · Las vías a los datos que no pasan por `ore-serve` (hecha el 2026-09-28)

M1 mide la puerta: la malla sólo expone el `ore-serve` de cada celda (`43-la-entrada.yaml`),
`ore-iam` y el IdP, y la forja no sale. Pero hay acceso a datos que no cruza esa puerta: el puesto
lee las copias del bucket **con el token de su pod** (`objectViewer` del bucket entero, 0031 §4),
y ahí ningún `puede` lo vería —es justo donde 0031 prometió que «una vista que no puede leer no
llega al DataFrame»—. **Qué:** cada cuenta de servicio de la malla (y de GCP) con permisos sobre
datos del inquilino —buckets, BigQuery, secretos—, desde qué pod se usa y si su uso pasa antes
por una ruta de `ore-serve`. De lectura, sobre la malla y la política IAM del proyecto. **Qué
decide:** si `ore-acceso` basta en la puerta o necesita también credenciales cortas por decisión
(§ 8, la credencial que Unity Catalog da al motor), y en qué paso entra eso.

#### M7 · Lo medido

`pruebas-de-fuego/medida-las-vias.sh`, de lectura: las cuentas del cluster con identidad de GCP,
las cargas que las usan, y sus roles en el proyecto, en cada bucket y en cada secreto (nombres,
nunca valores).

- **21 cuentas con identidad de GCP**: seis por inquilino (`cofre`, `driver`, `forja`,
  `informador`, `ore-serve`, `puesto`), las copias de la forja y del IdP, y el aprovisionador.
- **El bucket de cada inquilino** (`…-t-<n>-copia`), sin ninguna cuenta de otro inquilino:

  | cuenta | puede | para qué |
  |---|---|---|
  | `ore-driver-<n>` | `objectAdmin` | los Jobs escriben las copias |
  | `ore-serve-<n>` | `objectViewer` + `objectCreator` | lee y crea (el catálogo, el `metadata.json`); ni borra ni sobrescribe |
  | `ore-puesto-<n>` | `objectViewer` **sólo bajo `ore/puesto/`** (condición `solo-la-capa`) | baja la capa del entorno por su nombre; **no ve las copias** |

- **El puesto no se salta la puerta.** Desde W3.7 ②b (`aprovisionar-inquilino.sh`) lee los
  datos con **la credencial que `ore-serve` le presta**, y el catálogo Iceberg la da **acotada a
  la tabla** al cargarla (`loadTable` con `vended-credentials`, `catalogo.rs`): es el patrón de
  Unity Catalog (§ 8), y ya existe. La promesa de 0031 («una vista que no puede leer no llega al
  DataFrame») tiene **un sitio único donde cumplirse**: antes de prestar la credencial.
- **Los secretos, aislados por inquilino.** Cada custodio es `secretmanager.admin` con una
  condición por prefijo (`t-<n>-cofre-`): no alcanza los secretos de los demás. `ore-serve` y el
  driver de `victor` leen el token de su forja, y nada más.
- **Lo que queda fuera de la puerta, y está bien que quede:** los Jobs (`driver`) actúan como
  agente, lanzados por una ruta que ya pregunta (M1); el aprovisionador es plataforma
  (`projectIamAdmin`: la identidad más poderosa del proyecto, y lo que haga lo cuentan los Cloud
  Audit Logs de Google, no la huella); y quien tenga Owner o Editor en el proyecto lee todos los
  buckets (los papeles *legacy* del bucket), que es la entrada de operador.
- ⚠️ **Una cuenta huérfana con alcance a datos:** `ore-driver`, sin sufijo de inquilino, tiene
  `bigquery.dataViewer` y `bigquery.jobUser` **en todo el proyecto**, y ninguna cuenta del cluster
  la usa. Es de antes de las cuentas por inquilino.

**Lo que dice:** `ore-acceso` en la puerta basta; no hace falta un mecanismo nuevo de credenciales,
porque el préstamo acotado ya existe. Lo que falta es **preguntar antes de prestar**: `puede`
entra en `loadTable` / `loadView` (y en `POST /vistas/…/ejecutar` y `/puestos/{id}/datos`), que
son los sitios donde el plano de datos se abre (A8). Y `ore-driver` se retira.

## El contrato (A1, escrito el 2026-09-29)

Sale de M1 (qué se pregunta y qué rastro hay), M3 (quién pregunta y por dónde) y M6 (dónde se
guarda). Lo que depende de M2 lleva su marca, **⟨M2⟩**, y un valor de partida que M2 confirma o
cambia. `ore-iam` lo sirve (A2) y el crate `ore-acceso` lo habla (A4). Ningún módulo lo habla a
mano.

### Quién pregunta: dos tokens en cada llamada

| cabecera | qué lleva | quién lo emite | qué saca `ore-iam` de él |
|---|---|---|---|
| `Authorization: Bearer …` | **la celda**: el token de Workload Identity de `ore-serve-<celda>`, `aud=ore-iam` | Google, en el nodo (M3) | `(iss, sub)` → la fila de `iam.celda` → **la organización** |
| `Ore-Sujeto: …` | **quien pide**: el token del realm que la persona (o el agente) trajo a `ore-serve`, tal cual | el realm | `(iss, sub)` → la persona, y `act` si es delegado (RFC 8693, como ya hace la huella) |

- **La organización no viaja nunca.** Ni en el cuerpo, ni en una cabecera, ni en un `resource`.
  Una celda sólo puede preguntar por la suya, porque es lo único que `ore-iam` deduce de ella.
- **El cuerpo lleva `subject` porque AuthZEN lo pide**, y tiene que coincidir con el token de
  `Ore-Sujeto`. Si no coincide, 400: es un fallo de quien pregunta, no una denegación.
- **Una celda que no está en `iam.celda`, o está retirada: 401.** Un token del realm en
  `Authorization`, el de una persona también: **403, clase equivocada**. Las rutas del puente son
  de la clase `celda` y de ninguna otra, con la tabla de clases que ya existe.
- Hasta A9 el token de la persona tiene la audiencia `ore-serve` y vale en cualquier celda. **Eso
  no abre nada aquí:** la pregunta siempre es «en la organización de esta celda», y la celda no la
  elige quien trae el token.

### `puede`: AuthZEN 1.0, sin extensiones en la forma

```
POST /access/v1/evaluation
{ "subject":  { "type": "persona", "id": "<sub>" },
  "action":   { "name": "propuesta:fusionar-sin-revision" },
  "resource": { "type": "organizacion", "id": "-" },
  "context":  { "ruta": "POST /propuestas/{n}/fusionar", "peticion": "<id de la petición>" } }

200 { "decision": false,
      "context": { "id": "dec_…", "version": "…", "vale": 30,
                   "motivo": "no tienes `propuesta:fusionar-sin-revision` en esta organización" } }
```

- **`decision` es un booleano, y una denegación es un 200.** El código de error es para cuando no
  se pudo decidir (400, 401, 403 de clase, 5xx). Es lo que dice AuthZEN, y deja a `ore-serve` una
  sola regla: **todo lo que no sea `200` con `decision: true` niega**.
- **`resource`.** Para las preguntas de la organización (las de P2, M1 § 5) es `{type:
  organizacion, id: "-"}`: la organización es la de la celda. Las de recurso (el `resolver` del
  custodio, M4; los datos, A8) llevan su tipo y su nombre (`{type: secreto, id: ventas-pg}`).
  Hoy el motor de conjuntos las contesta con las concesiones, y Cedar cuando una pida más (H9).
- **`context.id`** identifica la decisión. Viaja en el 403 que ve la persona y en el `hizo` que
  la sigue. Es el identificador de autorización de AWS (§ 9).
- **`context.version`** es el estado de la política de esa organización: cambia cuando cambian
  sus roles, pertenencias o concesiones. Hoy no lo lee nadie. Existe para poder pasar un día a
  copia local sin cambiar a quien pregunta (H4).
- **`context.vale`** son los segundos que `puede` puede guardar la respuesta. **⟨M2⟩, de partida
  30**, y la misma regla para las denegaciones. Con 30 s, una revocación tarda como mucho medio
  minuto en valer. Las nubes reconocen minutos (§ 10).
- **`context.motivo`** es para la persona y dice lo que le falta. Como hoy, «no perteneces» y «no
  puedes» dan el mismo mensaje: el motivo no destapa quién está dentro.
- **`POST /access/v1/evaluations`**, el lote de AuthZEN, con la semántica por defecto
  (`execute_all`): para que una pantalla sepa de una vez qué botones enseñar.
- **Una potestad que no está en el catálogo niega**, con motivo «potestad desconocida», y se
  registra. Una ruta mal declarada falla cerrada, y se ve.

### En `ore-serve`: tres respuestas, ninguna nueva salvo el 503

| lo que pasa | lo que contesta la ruta |
|---|---|
| `decision: true` | lo de siempre |
| `decision: false` | **403** `{error: <motivo>, decision: <id>}` |
| `ore-iam` no contesta en **⟨M2⟩ 2 s**, o contesta 5xx, o 401 **de la celda** | **503** `{error: "no hay quien decida", reintentar: true}`. Nunca se deja pasar |
| 401 **del sujeto** (su token caducó entre la puerta y la pregunta) | **401**, como si lo hubiera visto la puerta |

### `hizo`: la huella, con la organización dentro

```
POST /access/v1/eventos
{ "id": "<uuid del evento>", "operacion": "fuente:crear", "sobre": "fuente/ventas",
  "resultado": "hecho", "decision": "dec_…", "commit": "<sha>",
  "cuando": "2026-09-29T10:00:00Z", "detalle": { … } }
201 { "id": "<uuid del evento>" }      (y 200 con el mismo id si ya estaba: se puede reintentar)
```

- **Va a `iam.huella`**, con las dos columnas que M6 pidió, `organizacion` y `celda`, sacadas del
  token de la celda. `quien` y `agente` salen del token de `Ore-Sujeto`, como en el resto de
  `ore-iam`.
- **`resultado`** es `hecho`, `negado` o `fallido`. Las denegaciones de `puede` las registra
  `ore-iam` al decidir (`acceso:negado`), sin esperar a que `ore-serve` lo diga (§ 13). `hizo`
  con `negado` es para lo que niega el propio módulo: la rama protegida (`423`), mover datos
  fuera de `main` (`409`).
- **`commit`** apunta, no copia (H13). Es el commit del árbol, el de la cola o el de la PR. La
  historia fina sigue en la forja de la celda.
- **`id` lo pone quien emite**, y dos eventos con el mismo `id` son uno. Es lo que permite
  reintentar sin duplicar (H15).
- **Qué se registra (H12):** siempre la gestión, las escrituras y las denegaciones. Las lecturas
  del catálogo **no**. Las de datos (el puesto, ejecutar una vista, prestar una credencial: las
  8 rutas sin rastro de M1), **⟨M2⟩, sólo si la organización lo enciende**.
- **Los permisos no se registran por su cuenta.** Un `puede` con `true` que no acaba en nada no
  deja fila. Uno que acaba en un acto deja **el acto**, con el `id` de su decisión dentro, que es
  la forma de `authorizationInfo` (§ 11). Por eso **el puente no sigue la regla de `008`** («toda
  ruta deja huella, leer incluido»): las demás rutas de `ore-iam` la siguen cumpliendo. Aquí
  cumplirla sería una fila por pregunta, el ruido que M6 § 4 ya nombró.

### Cuándo `hizo` va antes, y cuándo después (H15)

- **Después, y sin hacer fallar lo hecho:** si `ore-iam` no contesta, el evento espera en la
  celda y se reintenta con su mismo `id`. Un commit que ya está no se deshace porque falte su
  fila.
- **Antes, y si no se puede no se actúa:** lo que se salta una protección
  (`propuesta:fusionar-sin-revision`, `rama:proteger`) y lo que el custodio entrega (`resolver`,
  M4). Se registra `resultado: "en-curso"` y se actúa. Luego se cierra con otro evento, `hecho` o
  `fallido`, que apunta al primero (`abre: <id>`). Si la primera escritura no entra: 503.
- **Quién es el sujeto de un reintento.** El token de la persona puede haber caducado para
  entonces (vive 300 s). Por eso un reintento **no trae `Ore-Sujeto`, trae `decision`**, y
  `ore-iam` toma el sujeto de la decisión que él mismo tomó. Eso le obliga a **guardar las
  decisiones de escritura** un tiempo: **⟨M2⟩, de partida 24 h**, en una tabla aparte de la
  huella, que se poda. Un evento sin `Ore-Sujeto` y sin una decisión viva se rechaza (400). Una
  celda no puede atribuirle a nadie algo que `ore-iam` no autorizó.

### El catálogo: cada ruta declara lo suyo

- **La potestad se declara en la ruta**, en el mismo sitio que la monta, y de ahí salen el mapa
  de arranque y el catálogo. Es la lección de M1 § 7: un mapa escrito aparte ya no anuncia 34
  rutas.
- **`ore-iam` es el dueño del catálogo** (`iam.potestad`, por migración: es el PAP). Una ruta que
  declara una potestad que `ore-iam` no conoce la ve negada (arriba), y **CI lo comprueba antes**:
  el catálogo que sale de las rutas tiene que estar contenido en el de las migraciones.
- **Las tres primeras, para P2 (A5):** `propuesta:fusionar-sin-revision`, `rama:proteger` y
  `fuente:crear`. Son de gestión (H3) y de la organización (H9). **Ser admin de la organización
  no da las de datos** cuando lleguen (A8): van en otro conjunto (`DataActions`).

### La red (A3)

- **Una salida en el namespace de cada inquilino:** `salida-a-ore-iam`, de `ore.dev/rol=control`
  a `identidad` / `ore.dev/rol=iam-servidor`, puerto 8090. La rinde `gen-inquilino.py`, como las
  demás.
- **La entrada de `ore-iam`:** se añade `ore.dev/rol=cargas` + `ore.dev/rol=control`, y se quita
  la regla de `ore.dev/tenant=demo` (M3). Como siempre, el binario antes que la malla: la salida
  no se abre hasta que `ore-iam` sirva las rutas.

### Lo que A1 deja fijado, y lo que no

| fijado | abierto, y quién lo cierra |
|---|---|
| los dos tokens; la organización sale de la celda | `vale`, el tiempo de espera y lo que se guardan las decisiones: **M2** |
| AuthZEN 1.0 para `puede`, con `id`, `version`, `vale` y `motivo` en `context` | registrar las lecturas de datos, y con qué interruptor: **M2** y A6 |
| `hizo` a `iam.huella`, con `organizacion` y `celda`, idempotente por `id` | la forma exacta de `version`: **A2** |
| antes o después según el acto; los reintentos por `decision` | la audiencia por celda del token de la persona: **M5** y A9 |
| cada ruta declara su potestad; el catálogo es de `ore-iam`; CI los compara | Cedar: A8 |
| la salida `control` → `iam-servidor:8090` | |

## Los pasos

Por orden, y cada uno se abre con su go. Un paso que depende de una medida no empieza hasta que la
medida haya hablado; si la medida tumba la hipótesis, el paso se reescribe aquí antes de hacerse.

| paso | qué | depende de | sale |
|---|---|---|---|
| **A0** | **Medir**: M1, luego M6, M4 y M7 (estáticas o de lectura), luego M3 y M2 (en el cluster, de lectura). Hechas todas menos M2 y M5 | — | este ADR con las hipótesis confirmadas o tumbadas, y el catálogo inicial |
| **A1** | **El contrato**, escrito: `puede` en forma AuthZEN (sujeto del token, organización de la celda, `version`, identificador de decisión), `hizo` con la forma de la huella, los códigos (403 no puedes, 503 no hay respuesta) | M1, M3, M6 | una sección «El contrato» en este ADR. **Escrito el 2026-09-29**, con tres valores pendientes de M2 |
| **A2** | **`ore-iam` contesta**: primero las columnas `organizacion` y `celda` de la huella con su índice (M6; que no se edite ya lo hace la `039`); luego `POST /access/v1/evaluation` (y `/evaluations`) sobre `potestad::exige`, con su huella; y recibe eventos | A1 | `ore-iam` con las rutas nuevas y su prueba de fuego |
| **A3** | **El camino**: la regla de red de `ore-serve` a `identidad` y cómo se identifica la celda. Binario antes que malla: el flag nuevo y la malla que lo usa no se empujan juntos | A2 desplegado, M3 | la malla, empujada aparte |
| **A4** | **El crate `ore-acceso`**: `puede` (con caché corta, 503, identificador) y `hizo` (con espera y reintento en la celda) | A2 | el crate con sus pruebas, sin consumidores todavía |
| **A5** | **El primer consumidor, P2 de 0044**: las potestades `propuesta:fusionar-sin-revision`, `rama:proteger` y `fuente:crear` (o lo que M1 diga), en roles; `ore-serve` las pregunta; el merge y la liberación de `main` lo dicen; la consola obedece | A3, A4 | 0044 B.7 hecho |
| **A6** | **La actividad**: `ore-serve` emite `hizo` en todo lo que escribe y en las denegaciones, apuntando al commit cuando lo hay; `ore-iam` la sirve por organización (`actividad:leer-toda`); la consola la enseña | A4, M1 | «qué ha pasado en mi organización», de verdad |
| **A7a** | **Un papel de base por celda** (o seguridad por fila por organización en `iam` y `cofre`): el custodio de un inquilino deja de ver a los demás. No espera al puente | M4 | el cruce entre inquilinos, cerrado |
| **A7b** | **El custodio pasa por el puente** (`puede` con recurso para `resolver`, `hizo` antes de contestar, y una ruta de `ore-iam` para las concesiones) y su papel pierde el `select` sobre el censo | A4, M4 | «lo que hay» 5, cerrado del todo |
| **A8** | **Leer datos pregunta, y Cedar** cuando una pregunta necesite el recurso: `puede` antes de prestar la credencial (`loadTable`, `loadView`, ejecutar una vista, los datos del puesto, M7); dueños por paquete (aprobaciones por dueño); objetos (0046) | M1, M7 y el primer consumidor que lo pida | la promesa de 0031 y el hueco de 0007, cerrados |
| **A9** | **El token por celda** (Keycloak 26.2, intercambio estándar) | M5 | «lo que hay» 2, cerrado |

### A7a, por pasos (medido el 2026-09-28)

Lo que el custodio tiene hoy: los tres (`t-demo`, `t-prueba`, `t-victor`) entran con **el mismo
login**, `cofre_app` (miembro de `ore_cofre`), cuya URL es **un** secreto de plataforma
(`cofre-url`). Lee `cofre.secreto`, `iam.agente`, `iam.celda`, `iam.concesion_viva` y
`iam.potestades_de_persona` (todas con `organizacion`), `iam.organizacion` e `iam.persona` (sin
ella), e `iam.rol_de_recurso` (el catálogo de roles, que es de todos). Y dos cosas que deciden la
forma del arreglo:

- **Las vistas son del superusuario** (`keycloak`, que se salta la seguridad por fila): una política
  sobre `iam.concesion` no vale si se lee por `iam.concesion_viva`. Tienen que pasar a
  `security_invoker`, y entonces `ore_cofre` necesita `select` sobre sus tablas de debajo.
- **`iam.conceder_de_secreto` y `iam.revocar_de_secreto` son `security definer` del superusuario**
  y reciben la organización como argumento: hoy el custodio de un inquilino puede conceder sobre
  un secreto de otro. Tienen que comprobar que la organización es la de quien llama.

| paso | qué | riesgo |
|---|---|---|
| **A7a.1** ✓ | Hecho (`040-el-papel-de-cada-celda.sql`, `los-verbos.sh` 14). Migración: `iam.papel_de_celda (papel, celda)` y `iam.mi_organizacion()` (la del login que llama, por `session_user`); una función `security definer` que da de alta el papel de una celda (login con clave nueva, miembro de `ore_cofre`) y devuelve la URL, para que el aprovisionador no necesite `CREATEROLE`. **Sin seguridad por fila todavía**: no cambia nada de lo que corre | nulo |
| **A7a.2** ✓ | Escrito (`aprovisionar-inquilino.sh`, tras `cofre-url`). El aprovisionador llama a esa función y guarda la URL en Secret Manager como **`t-<n>-base-del-cofre`**, con `secretAccessor` para `ore-cofre-<n>`. Lo corre la convergencia de cada cinco minutos, así que llega a `demo`, `prueba` y `victor` en la pasada siguiente, y a toda celda nueva al nacer. Sólo da el papel si el secreto no tiene versión viva (darlo otra vez rota la clave). ⛔ **No `t-<n>-cofre-base`**, aunque la cuenta del custodio ya alcanzara ese prefijo: es el espacio de nombres de los secretos de la gente (`almacen.rs`: `t-<inq>-cofre-<nombre>`), y un secreto emitido con el nombre `base` pisaría la URL de la base, o la entregaría por `resolver` | bajo: crea, no cambia |
| **A7a.3** | La malla: el custodio trae `t-<n>-base-del-cofre` en vez de `cofre-url`. Se comprueba que los tres arrancan y resuelven con su papel. **Medido antes:** sólo cambia el `initContainer` de `41-el-cofre.yaml` (plantilla de `demo`: `gen-inquilino.py` la rinde para cada celda); el binario no cambia y no hay flag nuevo. El custodio conecta **al arrancar** y sale con 69 si no puede, así que un login malo se ve en seguida (CrashLoop), no en la primera petición; `/salud` no toca la base. Despliega con `Recreate`: unos segundos sin custodio por inquilino (139 `resolver` en 20 días). `cofre_app` no tiene nada propio que `cofre_<celda>` no herede de `ore_cofre` (ni permisos directos, ni ajustes, ni otras pertenencias). Llega con la convergencia, a los tres: ✏️ se midió mal que `demo` iba por la kustomization `malla`; su Deployment es de `inquilino-demo` (`kustomize.toolkit.fluxcd.io/name`), como los otros dos, y lo cambia la pasada del aprovisionador que rinde y empuja cada compartimento. ⛔ **`45-la-mudanza-del-cofre.yaml` no se toca:** es un `Job` completado el 14-sep en los tres inquilinos, inmutable, y Flux (`force: false`) fallaría al aplicar otra plantilla; sigue nombrando `cofre-url` y no vuelve a correr. Se resuelve en A7a.6 | medio: si falla, un custodio no arranca; se vuelve atrás con la malla |
| **A7a.6′** | Con A7a.6: la mudanza sale de las plantillas (`PLANTILLAS` de `gen-inquilino.py`), porque un inquilino nuevo la crearía nombrando `cofre-url`, que ya no existirá (y no tiene nada que mudar) | bajo |
| **A7a.4** ✓ | Escrita y probada (`041-cada-custodio-su-organizacion.sql`; vuelta atrás en `iam/vuelta-atras/`, fuera del runner). Al probarla salió un fallo de la guarda: `pg_has_role(…, 'member')` es verdad para un superusuario con cualquier papel, y trataba al operador como a un custodio sin celda; mira `pg_auth_members`. Migración: seguridad por fila en las tablas de arriba —una política que deja todo a `ore_iam` y `ore_aprovisionador`, y otra que a `ore_cofre` sólo le deja su organización (`iam.persona`: las que pertenecen a ella)—; las vistas a `security_invoker` con sus `grant`; y la guarda en las dos funciones | medio: una política que falte deja a alguien sin ver nada; lo cubre la prueba de fuego del paso siguiente, contra una base con dos organizaciones |
| **A7a.5** ✓ | `el-cofre.sh` 12 (y el custodio de toda la prueba corre ya como `cofre_acme`); `el-cofre.sh` y `los-verbos.sh` enteras en verde con los binarios de Linux contra `postgres:16`. El 6 y el 10 medían «Zoe recibe el mismo error que Ada con uno inventado»; desde la 041 el custodio no ve a Zoe y le dice que no la conoce, así que miden lo que protegen: a quien no pertenece, el mismo error para un secreto que existe y para uno inventado. Prueba de fuego (`el-cofre.sh` o `los-verbos.sh`): dos organizaciones, dos papeles; el custodio de una no ve ni concede nada de la otra, y `ore-iam` sigue viéndolo todo | — |
| **A7a.6** | Se retira `cofre_app` y el secreto `cofre-url`. ✏️ No «cuando A7a.3 lleve un día sano», que no dice qué tiene que pasar: cuando hayan salido bien, medidos, los eventos de § «A7a.6, lo que tiene que pasar». **Hecho el 2026-09-29** | bajo |

#### A7a.4, medido (2026-09-28)

**Lo que lee el custodio, entero** (su SQL y lo que hereda de la biblioteca de `ore-iam`:
`potestad::exige`, `verbos::persona_id`, `verbos::sujeto_id`): `cofre.secreto`, `iam.agente`,
`iam.celda`, `iam.organizacion`, `iam.persona`, `iam.rol_de_recurso`, las vistas
`iam.concesion_viva` (de `concesion`) e `iam.potestades_de_persona` (de `pertenencia`,
`pertenencia_rol`, `rol_potestad` y la vista `por_defecto`); inserta en `iam.huella`; y llama a
`conceder_de_secreto`, `revocar_de_secreto` y `revocar_de_secreto_operador`, las tres con
`p_organizacion` de argumento.

**Quién más lee esas tablas**, y se quedaría ciego sin su política: `ore_iam` (login `iam_app`),
todo; `ore_aprovisionador` (login `aprovisionador`), `iam.organizacion` por columnas (`nombre`,
`kek`) y la vista `celda_de`, que es del superusuario y no pasa por la seguridad por fila. Nadie
más entra (`keycloak` es superusuario y se la salta).

**La migración:**

| pieza | qué |
|---|---|
| seguridad por fila en 8 tablas | `cofre.secreto`, `iam.agente`, `iam.celda`, `iam.concesion`, `iam.organizacion`, `iam.persona`, `iam.pertenencia`, `iam.pertenencia_rol`. **No** en los catálogos (`rol_potestad`, `potestad`, `rol_de_recurso`), que son de todos, ni en `iam.huella` (el custodio sólo inserta) |
| `ore_iam` | una política por tabla que lo deja todo (`using (true)`): no cambia nada de lo que hace |
| `ore_aprovisionador` | `select` sobre `iam.organizacion` con `using (true)`: sus `grant` por columnas siguen siendo el límite |
| `ore_cofre` | su organización, y nada más: `organizacion = (select iam.mi_organizacion())` en agente, celda, concesión, pertenencias y `cofre.secreto` (este para leer y escribir); `id = …` en `organizacion`; en `persona`, las que pertenecen a ella. `(select …)` para que se evalúe una vez por consulta |
| las dos vistas | `security_invoker = true` en `concesion_viva` y `potestades_de_persona`, porque son del superusuario y, si no, leerían por encima de las políticas; y `select` a `ore_cofre` sobre sus tablas de debajo (`concesion`, `pertenencia`, `pertenencia_rol`, `rol_potestad`) |
| las tres funciones | al empezar: si quien llama es de `ore_cofre` y `p_organizacion` no es la suya, se niega |

**Lo que cambia para el custodio**, y es lo buscado: preguntado por otra organización (el `org`
del camino lo pone quien llama), no ve a nadie en ella, y contesta lo mismo que hoy a quien no
pertenece. Un login de `ore_cofre` sin celda —`cofre_app`— no ve nada.

**Precondición:** ninguna sesión de `cofre_app` en `pg_stat_activity` (A7a.3 entero en los
tres). **Vuelta atrás, escrita antes:** una migración que desactiva la seguridad por fila de las
8 tablas y devuelve las vistas a como estaban; las políticas pueden quedarse, sin efecto.

**La prueba (A7a.5), sobre `el-cofre.sh`**, que ya funda `acme` (con celda) y `otra` (sin ella):
`otra` con celda; un papel para cada una; el custodio corre como `cofre_acme`; como `cofre_acme`,
de cada tabla sólo salen filas de `acme`, y conceder o revocar para `otra` se niega; `iam_app`
sigue viéndolo todo; el aprovisionador sigue leyendo `nombre` y `kek`; `cofre_app` no ve nada.

#### A7a.6, lo que tiene que pasar (medido el 2026-09-28)

`cofre_app` y `cofre-url` eran la vuelta atrás de la A7a entera. Se quitan cuando los eventos que
harían falta sin ellos han salido bien, y se provocaron en vez de esperarlos:

| | evento | cómo se midió | resultado |
|---|---|---|---|
| E1 | un custodio se reinicia, trae su secreto y conecta | `rollout restart` de los tres; y la convergencia los volvió a desplegar sola (victor tiene once ReplicaSets hoy) | los tres Ready, sin reinicios ni `✗`, cada uno con una sesión de su login |
| E2 | cada ruta sobre datos reales bajo la 041 | emitir y resolver ocurrieron de verdad (20:38, victor); listar y retirar, con el login de cada celda en una transacción deshecha | demo y victor: listar 8 (5 vivos), retirar 1 fila y sus concesiones; de otra organización, 0 filas vistas, 0 tocadas, revocar negado; 10 vivos antes y después |
| E3 | los verbos de operador | `mudar` y `retirar-huerfanos --seco` dentro de cada pod | los tres, sin error, cada uno con lo suyo |
| E4 | la convergencia | dos pasadas completas tras la 041 | sin `✗`; las tres celdas aprovisionadas |
| E5 | nadie más usa `cofre_app` ni `cofre-url` | `cofre_app` sin login y la versión de `cofre-url` desactivada; una pasada completa y el registro de la base | 0 intentos de `cofre_app`; el único que tocaba `cofre-url` era el propio aprovisionador (su permiso), que se quita en este paso |
| E6 | la vuelta atrás sin `cofre_app` | simulacro en demo: rotar su clave, guardarla (versión 2) y reiniciar | entra con la nueva; la vieja da `password authentication failed`; la versión 1 desactivada |

**El orden de A7a.6**, porque el aprovisionador **fallaba** si `cofre-url` no existía: primero
el código (sin el paso de `cofre-url`; sin la mudanza en las plantillas, que Flux poda de los tres
inquilinos —su rastro está en la huella, `secreto:mudar`—); después la `042`, que retira
`cofre_app`; y al final se borra `cofre-url`, lo único que no tiene vuelta.

**Hecho el 2026-09-29**, en ese orden:

1. **El código (`bac1dbc`).** Las pasadas del aprovisionador con el guion nuevo terminan con «las
   5 celdas están al día», sin `✗` y sin el paso de `cofre-url`. `mudar-el-cofre` ya no existe en
   `t-demo`, `t-prueba` ni `t-victor`.
2. **La `042`.** `migrar-iam` aplicó una migración. `cofre_app` ya no existe, en `ore_cofre`
   quedan `cofre_demo`, `cofre_prueba` y `cofre_victor`, y hay una sesión de cada uno.
3. **`cofre-url` borrado.** Después se reinició el custodio de `prueba`: trae su base (132 bytes)
   y conecta con su login. Los tres custodios están Ready y sin errores.

El orden importa: la seguridad por fila (A7a.4) no entra hasta que ningún custodio use
`cofre_app`, porque ese login no tiene celda y con las políticas puestas no vería nada: los tres
custodios caerían a la vez.

A0–A5 es lo que hace falta para P2. A6 es lo que hace falta para la auditoría. A7–A9 cierran
huecos que esta pieza deja a la vista, y no bloquean nada de lo anterior.

## Lo que este ADR no decide

- **El motor definitivo.** Cedar es el candidato por encaje, no por decisión; entra cuando una
  medida pida decidir por recurso (A8).
- **Los datos en ramas**, ni quién lee qué vista: es la capa de 0031, que usará este puente.
- **La elevación temporal** (Google PAM, Azure PIM): dar la potestad de saltarse algo por un rato
  y con justificación. Queda nombrada.
- **Exportar la huella** a un almacenamiento del cliente o a un SIEM. Queda nombrada.

## Fuentes

- Google: [deny](https://docs.cloud.google.com/iam/docs/deny-overview) ·
  [Principal Access Boundary](https://docs.cloud.google.com/iam/docs/principal-access-boundary-policies) ·
  [propagación](https://docs.cloud.google.com/iam/docs/access-change-propagation) ·
  [Zanzibar](https://research.google/pubs/zanzibar-googles-consistent-global-authorization-system/) ·
  [Cloud Audit Logs](https://docs.cloud.google.com/logging/docs/audit)
- AWS: [evaluación de políticas](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_evaluation-logic.html) ·
  [RCP](https://docs.aws.amazon.com/organizations/latest/userguide/orgs_manage_policies_rcps.html) ·
  [aislamiento de los servicios globales](https://docs.aws.amazon.com/whitepapers/latest/aws-fault-isolation-boundaries/global-services.html) ·
  [identificador de autorización](https://docs.aws.amazon.com/IAM/latest/UserGuide/troubleshoot_access-denied-authorization-id.html) ·
  [Verified Permissions](https://docs.aws.amazon.com/verifiedpermissions/latest/userguide/what-is-avp.html)
- Azure: [definiciones de rol: Actions y DataActions](https://learn.microsoft.com/en-us/azure/role-based-access-control/role-definitions) ·
  [propagación](https://learn.microsoft.com/en-us/azure/role-based-access-control/troubleshooting) ·
  [elevar el acceso](https://learn.microsoft.com/en-us/azure/role-based-access-control/elevate-access-global-admin)
- Databricks: [Unity Catalog](https://docs.databricks.com/aws/en/data-governance/unity-catalog/) ·
  [privilegios](https://docs.databricks.com/aws/en/data-governance/unity-catalog/manage-privileges/privileges) ·
  [credenciales al motor](https://docs.databricks.com/aws/en/external-access/credential-vending)
- Snowflake: [control de acceso](https://docs.snowflake.com/en/user-guide/security-access-control-overview) ·
  [consideraciones](https://docs.snowflake.com/en/user-guide/security-access-control-considerations)
- Foundry: [proyectos y roles](https://www.palantir.com/docs/foundry/security/projects-and-roles) ·
  [marcados](https://www.palantir.com/docs/foundry/security/markings) ·
  [proteger recursos](https://www.palantir.com/docs/foundry/global-branching/protecting-resources)
- Confluent: [MDS](https://docs.confluent.io/platform/current/kafka/configure-mds/index.html) ·
  [RBAC en Cloud](https://docs.confluent.io/cloud/current/security/access-control/rbac/overview.html);
  Redpanda: [autorización](https://docs.redpanda.com/redpanda-cloud/security/authorization/)
- GitHub: [ramas protegidas](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches) ·
  [rulesets](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets) ·
  [eventos de auditoría de la organización](https://docs.github.com/en/organizations/keeping-your-organization-secure/managing-security-settings-for-your-organization/audit-log-events-for-your-organization)
- Estándares: [AuthZEN 1.0](https://openid.github.io/authzen/) ·
  [aprobada como final](https://openid.net/authorization-api-1-0-final-specification-approved/) ·
  [Cedar](https://docs.rs/cedar-policy) · [el artículo de Cedar](https://arxiv.org/pdf/2403.04651) ·
  [SpiceDB, consistencia](https://authzed.com/docs/spicedb/concepts/consistency) ·
  [OPAL](https://docs.opal.ac/) · [RFC 8693](https://www.rfc-editor.org/rfc/rfc8693) ·
  [Keycloak 26.2, intercambio estándar](https://www.keycloak.org/2025/05/standard-token-exchange-kc-26-2) ·
  [los fallos de UMA en Keycloak](https://www.gabriel.urdhr.fr/2025/07/08/keycloak-uma-vulnerabilities/)

Dos huecos en lo leído: nadie publica cómo llama por dentro un servicio como BigQuery o S3 a su
IAM, y de Foundry sólo es público Multipass como servicio de autenticación; su punto de decisión
no aparece en ninguna documentación pública.
