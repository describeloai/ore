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

### M3 · El camino (por preparar)

Qué regla de red hace falta, con qué identidad se presenta `ore-serve` ante `ore-iam` (el token
reenviado de la persona, más la cuenta de servicio de la celda) y cómo sabe `ore-iam` qué celda
pregunta. El informador de 0026 es el precedente.

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

## Los pasos

Por orden, y cada uno se abre con su go. Un paso que depende de una medida no empieza hasta que la
medida haya hablado; si la medida tumba la hipótesis, el paso se reescribe aquí antes de hacerse.

| paso | qué | depende de | sale |
|---|---|---|---|
| **A0** | **Medir**: M1, luego M6, M4 y M7 (estáticas o de lectura), luego M3 y M2 (en el cluster, de lectura) | — | este ADR con las hipótesis confirmadas o tumbadas, y el catálogo inicial |
| **A1** | **El contrato**, escrito: `puede` en forma AuthZEN (sujeto del token, organización de la celda, `version`, identificador de decisión), `hizo` con la forma de la huella, los códigos (403 no puedes, 503 no hay respuesta) | M1, M3, M6 | una sección «El contrato» en este ADR |
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
| **A7a.1** | Migración: `iam.papel_de_celda (papel, celda)` y `iam.mi_organizacion()` (la del login que llama, por `session_user`); una función `security definer` que da de alta el papel de una celda (login con clave nueva, miembro de `ore_cofre`) y devuelve la URL, para que el aprovisionador no necesite `CREATEROLE`. **Sin seguridad por fila todavía**: no cambia nada de lo que corre | nulo |
| **A7a.2** | El aprovisionador (`aprovisionar-inquilino.sh`) llama a esa función por celda y guarda la URL en Secret Manager como `t-<n>-cofre-base`, que la cuenta del custodio ya alcanza por su prefijo. Se corre para `demo`, `prueba` y `victor` | bajo: crea, no cambia |
| **A7a.3** | La malla: el custodio trae `t-<n>-cofre-base` en vez de `cofre-url`. Se comprueba que los tres arrancan y resuelven con su papel | medio: si falla, un custodio no arranca; se vuelve atrás con la malla |
| **A7a.4** | Migración: seguridad por fila en las tablas de arriba —una política que deja todo a `ore_iam` y `ore_aprovisionador`, y otra que a `ore_cofre` sólo le deja su organización (`iam.persona`: las que pertenecen a ella)—; las vistas a `security_invoker` con sus `grant`; y la guarda en las dos funciones | medio: una política que falte deja a alguien sin ver nada; lo cubre la prueba de fuego del paso siguiente, contra una base con dos organizaciones |
| **A7a.5** | Prueba de fuego (`el-cofre.sh` o `los-verbos.sh`): dos organizaciones, dos papeles; el custodio de una no ve ni concede nada de la otra, y `ore-iam` sigue viéndolo todo | — |
| **A7a.6** | Se retira `cofre_app` y el secreto `cofre-url` | bajo, cuando A7a.3 lleve un día sano |

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
