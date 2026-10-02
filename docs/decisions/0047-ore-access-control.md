# 0047 · ORE Access Control

**Estado:** aceptado y **en vivo** desde el 2026-09-29 (A1–A6, A7a y A9′). Abiertos: A7b, A8
(espera una decisión de producto) y A9 (aplazado). La deuda, al final.

Completa [0020](0020-el-plano-de-control.md) ③ («`ore-serve` no autoriza»), que no se revoca:
`ore-serve` sigue sin decidir, y ahora pregunta. Da el primer paso del hueco que
[0007](0007-enlazar-el-evaluador-de-cedar.md) dejó con nombre, y extiende la huella de `ore-iam`
(`008`) al plano de datos.

## Qué es

**ORE Access Control contesta dos preguntas para todo el plano de datos: ¿puede? y ¿qué hizo?**
Cada módulo que atiende a una persona (`ore-serve`, el custodio, lo que venga) **pregunta** antes
de actuar y **dice** lo que hizo. No decide él ni guarda él el rastro: decide `ore-iam` y guarda
`iam.huella`. Es lo que la industria llama *access management*: autorización más auditoría.

```
  persona ──token──► ore-serve (la celda) ──puede / hizo──► ore-iam ──► iam.huella
                     ore-acceso (PEP)        dos tokens      decide (PDP)   registra
```

Junto a [0048](0048-ore-idp.md), las tres piezas de la identidad de ORE:

| pieza | pregunta |
|---|---|
| **ORE IdP** (0048) | ¿quién eres? |
| **`ore-iam`** | ¿a qué organización perteneces, y qué potestades tienes en ella? |
| **ORE Access Control** (esto) | ¿puede hacer esto aquí?, y ¿qué ha pasado en mi organización? |

| papel (XACML / NIST / AuthZEN) | en ORE |
|---|---|
| **PEP**, lo que intercepta y pregunta | el crate **`ore-acceso`**: `puede` y `hizo`, y `quien` (el handle de quien crea) |
| **PDP**, lo que decide | `ore-iam`, en `/access/v1` (AuthZEN 1.0) |
| **PIP**, de donde salen los datos | el censo de `ore-iam` (`iam.pertenencia`, roles, concesiones, el handle de cada persona); los dueños, en el árbol (`owner`, 0027) |
| **PAP**, donde se escribe la política | las migraciones de `iam` (roles y potestades); `.arbol/` para lo que es del árbol (0044 B.1) |
| **el registro** | `iam.huella`: sólo inserción (la `039` lo impone a todos, al superusuario también), con `organizacion` y `celda` |

### Los principios

Los diez de la industria (§ «De dónde viene») que este diseño adopta:

1. **El token dice quién eres, nunca lo que puedes.** Los permisos se resuelven al decidir. Una
   revocación vale en ≤ 30 s, no cuando caduque un token.
2. **La organización sale de la celda que pregunta, nunca del cuerpo.** Una celda sólo puede
   preguntar por la suya.
3. **Sin respuesta, se niega.** Si `ore-iam` no contesta: 503, nunca 200. La única excepción es
   la pertenencia (A9′): 10 minutos de gracia para quien ya pasó.
4. **Saltarse una protección es una potestad con nombre**, se registra **antes** de actuar, y si
   no se puede registrar no se actúa.
5. **Gestionar no es leer los datos.** Las potestades de datos serán otro conjunto (A8). Ser admin
   de la organización no las da.
6. **El commit sigue siendo la auditoría fina del árbol.** La huella apunta al commit, no lo
   copia.
7. **Cada ruta declara lo suyo, junto a la ruta.** Una tabla escrita aparte se desincroniza.

## El contrato

### Dos tokens en cada llamada

| cabecera | qué lleva | qué saca `ore-iam` de él |
|---|---|---|
| `Authorization: Bearer …` | **la celda**: el token de Workload Identity de `ore-serve-<celda>`, `aud=ore-iam`, firmado por Google en el nodo | `(iss, sub)` → `iam.celda` → **la organización** |
| `Ore-Sujeto: …` | **quien pide**: el token del realm que la persona o el agente trajo a `ore-serve` | `(iss, sub)` → la persona o el agente, y `act` si es delegado (RFC 8693) |

- La celda no guarda ningún secreto. Su token nace en el nodo, dura una hora y sólo
  `t-<n>/ore-serve` puede ser `ore-serve-<n>`. La correspondencia cuenta → celda la escribe el
  aprovisionador (`POST /celdas/{c}/aprovisionada` con `{identidad: {emisor, sub}}`).
- Una celda que no está en `iam.celda`, o retirada: **401**. Un token del realm en
  `Authorization`: **403, clase equivocada**. `subject` del cuerpo distinto de `Ore-Sujeto`:
  **400**.
- Otro proveedor (AWS, Azure) traerá otro emisor, no otro diseño: la tabla guarda `(emisor, sub)`.

### `puede`: AuthZEN 1.0

```
POST /access/v1/evaluation
{ "subject":  { "type": "persona", "id": "<sub>" },
  "action":   { "name": "propuesta:fusionar-sin-revision" },
  "resource": { "type": "organizacion", "id": "-" },
  "context":  { "ruta": "POST /propuestas/{n}/fusionar", "peticion": "<id>" } }

200 { "decision": false,
      "context": { "id": "dec_…", "version": "…", "vale": 30,
                   "motivo": "no tienes `propuesta:fusionar-sin-revision` en esta organización" } }
```

- **Una denegación es un 200 con `decision: false`.** Para quien pregunta hay una sola regla:
  todo lo que no sea `200` con `decision: true` niega.
- `context.id` es el identificador de la decisión: viaja en el 403 que ve la persona y en el
  `hizo` que la sigue. `context.vale` es lo que se puede guardar la respuesta (techo: 30 s).
  `context.version` es el estado de la política de la organización, para poder pasar un día a
  copia local sin cambiar a quien pregunta. `context.motivo` no distingue «no perteneces» de «no
  puedes».
- `context.consulta: true` pregunta «¿podría?», para pintar botones. Su denegación no va a la
  huella.
- `POST /access/v1/evaluations`: el lote (`execute_all`).
- Una potestad fuera del catálogo niega («potestad desconocida»).

**En `ore-serve`:** `true` → lo de siempre; `false` → **403** `{error, decision}`; sin respuesta
en **2 s**, 5xx o 401 de la celda → **503** `{error: "no hay quien decida"}`; 401 del sujeto →
**401**.

### `quien`: el handle de quien crea

```
POST /access/v1/quien
{ "subject": { "id": "<sub>" } }

200 { "subject": { "id": "<sub>" }, "handle": "ana-garcia", "owner": "user:ana-garcia" }
```

Lo que una celda escribe en el `owner` de lo que alguien crea (0027, «el dueño es quien lo crea»).
Cada persona tiene **un handle**, en `iam.persona.handle` (la 048): único entre todas las
personas, con la forma de `OOS2009`, y **asignado una vez** —la primera vez que `ore-iam` ve su
token— a partir del nombre de usuario que eligió al registrarse (`preferred_username`, 0048; si no
lo hay, su correo; empate con `-2`, `-3`…). No cambia aunque cambie el usuario o el correo: es lo
que queda escrito en el árbol.

- `Ore-Sujeto` es opcional. Si es el token de esa misma persona, el handle sale de su usuario si
  aún no tenía; si no —desde un puesto llama el agente y la persona es quien lo abrió—, se lee el
  que tiene, y si aún no tiene, sale de su correo.
- **404** si `subject.id` no es una persona de la organización de la celda: un agente no es dueño
  de nada, y una celda no pregunta por gente de otra.
- No deja huella: no es un acto, es el sujeto quedando dicho.

**En `ore-serve`:** `Servidor::dueno_de_quien_crea` → `user:<handle>`; 404 → **403**, sin
respuesta → **503** (lo que se iba a crear no nace con un dueño inventado). `ore-acceso` lo guarda
sin plazo: un handle no cambia.

### `hizo`: la huella, con la organización dentro

```
POST /access/v1/eventos
{ "id": "<uuid>", "operacion": "fuente:crear", "sobre": "fuente/ventas",
  "resultado": "hecho", "decision": "dec_…", "commit": "<sha>", "cuando": "…", "detalle": { … } }
201 { "id": "<uuid>" }        (200 con el mismo id si ya estaba: idempotente)
```

- `resultado`: `hecho`, `negado` o `fallido`. Las denegaciones de `puede` las anota `ore-iam` al
  decidir. `negado` en `hizo` es lo que niega el propio módulo: **423** (rama protegida) y el 403
  de sus guardas. El 409 no, porque casi siempre es «ya existe».
- **Antes o después.** Lo normal va **después**, y sin hacer fallar lo hecho. Lo que se salta una
  protección o entrega un secreto va **antes**, como `en-curso`, y se cierra con `hecho` o
  `fallido` (`abre: <id>`); si esa primera escritura no entra, 503.
- **Los reintentos** no traen `Ore-Sujeto` (el token vive 300 s), traen `decision`. `ore-iam`
  toma el sujeto de la decisión que él tomó, y por eso guarda las de escritura **24 h**
  (`iam.decision`). Un evento sin sujeto y sin decisión viva: 400. Una celda no puede atribuirle a
  nadie lo que `ore-iam` no autorizó.
- **Qué se registra:** siempre la gestión, las escrituras y las denegaciones. Las lecturas del
  catálogo, nunca. Las lecturas de datos, cuando la organización lo encienda (A8). Un `puede` con
  `true` que no acaba en nada no deja fila: deja fila **el acto**, con su decisión dentro.

### El catálogo

`ore-iam` es el dueño del catálogo (`iam.potestad`, por migración). Hoy tiene **20 potestades**.
Las del plano de datos son cuatro:

| potestad | quién la tiene | qué guarda |
|---|---|---|
| `organizacion:leer` | todo miembro (y el agente registrado en la organización) | la entrada a la celda (A9′) |
| `propuesta:fusionar-sin-revision` | ORGADMIN, ACCOUNTADMIN | fusionar en `main` protegida sin revisión |
| `rama:proteger` | ORGADMIN, ACCOUNTADMIN | cambiar la protección de una rama |
| `fuente:crear` | ORGADMIN, ACCOUNTADMIN, SECURITYADMIN | dar de alta un origen |

En `ore-serve`, `actividad.rs` declara la operación de cada una de sus **61 escrituras**
(`ESCRITURAS`), y la prueba `toda_ruta_que_escribe_se_declara` lee el enrutador: una ruta que
escribe sin declararse hace fallar CI.

## En vivo

- **`ore-serve --acceso`** en cada celda (`malla/40-ore-serve.yaml`). Hace tres cosas:
  - **la pertenencia, en cada petición con sujeto:** `Servidor::pertenece` en `quien()`, la única
    puerta, con caché de 30 s y 10 minutos de gracia;
  - **las tres potestades de P2** (0044 B.7), con el merge y la liberación de `main` registrados
    antes;
  - **`hizo` de toda escritura, desde un sitio:** `recuento::atendiendo` echa el evento al
    `Buzon` de `ore-acceso`. Es un hilo: la respuesta no espera a `ore-iam`. El token vive sólo
    en memoria, 240 s; después el evento va a disco si tiene decisión, o a `muertos/` sin
    credencial.
- **`ore-iam`** sirve:
  - `/access/v1/evaluation`, `/evaluations`, `/eventos` y `/quien`;
  - `GET /organizaciones/{org}/actividad`: toda la actividad con `actividad:leer-toda`, y si no
    sólo la propia, sin el sondeo del sistema, con cursor y filtros por clase y celda.

  Lo suyo propio (invitar, conceder, secretos, celdas) lo anota también con organización.
- **La consola** pinta Governance / Activity sobre esa ruta (`rubix-platform` `9115343`).
- **La red:** `salida-a-ore-iam`, de `control` a `iam-servidor:8090`. Las llaves de Google las
  trae el CronJob `refresco-jwks-celdas` cada hora, comprobando el emisor, y `ore-iam` las relee
  ante un `kid` desconocido, sin reiniciar.
- **Medido el 2026-10-02:** 712 decisiones en 24 h, y los actos de las celdas llegando a la
  huella (`puesto:abrir`, `rama:crear`, `repositorio:crear`, `arbol:escribir`, `funcion:invocar`…).

**Pruebas de fuego:**
- `los-verbos.sh` 14–18: el puente, las clases, la idempotencia, los reintentos, los agentes y
  el handle (asignado una vez, desempate, desde el agente de un puesto, 404 a agentes y a gente de
  fuera, y la base que niega la forma y el duplicado);
- `la-propuesta.sh` 10: P2, la actividad y la pertenencia, que da 403 a una cuenta de fuera y
  aplica la gracia sin `ore-iam`;
- `el-cofre.sh` 12–13: un custodio por organización.

## De dónde viene

### La necesidad (2026-09-28)

`ore-iam` sabía quién puede qué **en la organización** y recordaba lo que se hacía **en él**.
`ore-serve` sabía quién llama y nada más. Entre los dos no había nada, y cinco sitios esperaban
la respuesta a «¿puede?»:
- la rama protegida (0044 B.7);
- el alta de orígenes;
- el puesto, que debía leer sólo lo permitido (0031);
- los equipos (0027);
- los ficheros (0046).

«¿Qué hizo?» quedaba en el commit de la forja de cada celda, o en ningún sitio. Un sexto
consumidor, el custodio, se lo resolvía solo leyendo tablas de `iam`.

⇒ **Una pieza de producto, no un arreglo de `ore-serve`**: un contrato, un sitio que decide y
guarda, y un cliente que cualquier módulo enlaza. Si no, cada módulo tendría sus reglas y su
formato de actividad.

### Lo que hace la industria

Se miraron:
- las nubes: Google, AWS y Azure;
- las plataformas de datos: Databricks, Snowflake, Foundry, Confluent, Redpanda, GitHub y GitLab,
  Atlas, Supabase y Grafana;
- los estándares y motores: AuthZEN, Zanzibar, Cedar, OPA/OPAL y Cerbos.

Lo que hacen todas:

**Decidir:**
1. El token es identidad. El UMA de Keycloak, que mete permisos en el token, tuvo fallos de
   confianza en 2025.
2. Un catálogo fijo de verbos por servicio, y los roles como conjuntos de ellos.
3. Gestionar ≠ leer datos (`Actions` frente a `DataActions` en Azure).
4. Primero se niega, y las barreras de la organización nunca conceden.
5. Saltarse una protección es un permiso con nombre (GitHub `policy_override`, Google
   `exceptionPrincipals`).
6. La política de aprobación es un dato del contenedor (Foundry, GitHub).
7. Aprobación del dueño por ruta (CODEOWNERS).
8. Al motor, credenciales cortas y recortadas (Unity Catalog).
9. Sin respuesta se niega, y toda denegación se explica (el identificador de autorización de
   AWS).
10. La propagación se reconoce: de 2 a 7 minutos en Google, hasta 10 en Azure.

**Dejar huella:**

11. La auditoría es otro servicio, con la decisión dentro (`authorizationInfo` en Cloud Audit
    Logs).
12. La gestión se registra siempre; leer datos, si se pide, por el volumen.
13. Las denegaciones, por defecto.
14. Una vista por organización, que no se edita y que se lee con su propia potestad.

**El contrato y el motor:**
- **AuthZEN 1.0** (final, enero de 2026) deja cambiar el motor sin cambiar a quien pregunta.
- **Cedar** (`permit`/`forbid`, análisis simbólico en Rust) es el candidato para cuando haya que
  decidir por recurso.
- **Zanzibar** obliga a escribir cada dato dos veces.
- Para la huella no hay estándar: la forma de referencia es Cloud Audit Logs, y `iam.huella` ya
  se le parece.

### Las medidas que lo decidieron

| medida | lo que dijo |
|---|---|
| **M1** (`medida-el-acceso.py`) | 101 rutas: 45 leen y 56 escriben. 48 de las 56 ya dejaban un commit con la persona dentro (el árbol o la cola), y las 8 sin rastro son leer y computar datos (el puesto, ejecutar una vista). Una potestad por ruta daba 61, demasiadas: agrupadas por lo que deciden salen unas quince. Las tres de P2 son de la organización, no del recurso, así que el motor de conjuntos basta |
| **M2** (`medida-el-salto.sh`) | ~210 escrituras de personas en 30 días entre tres celdas: guardar 24 h las decisiones son decenas de filas. El salto `ore-serve` → `ore-iam` por dentro: **2,1 ms p50, 5,1 ms p99** (con punto final en el nombre; la cola de 80 ms de la primera medida era la CPU del pod de medida, no el DNS). La consulta de potestades, 0,36 ms. Los 41 ms del balanceador son el camino de fuera |
| **M3** (`medida-el-camino.sh`) | No había red de las celdas a `ore-iam`. La celda ya tenía una identidad sin secreto (Workload Identity), y el agente no servía como identidad de la celda: su credencial la leen tres cuentas, el puesto incluido |
| **M4** | El custodio pregunta lo mismo que el puente, pero **administra concesiones**, que no cabe en `puede` ni en `hizo` y pide su propia ruta. Y los tres custodios compartían un login que veía el censo de todos |
| **M6** (`medida-la-huella.sh`) | La huella se **podía editar** (`ore_iam` tenía `UPDATE` y `DELETE`); no tenía columna de organización; el 98 % era sondeo de la máquina |
| **M7** (`medida-las-vias.sh`) | El puesto **no** se salta la puerta: lee con la credencial que `ore-serve` le presta, acotada a la tabla (`loadTable`). La promesa de 0031 tiene un sitio único donde cumplirse: antes de prestar |

### Cómo se hizo (28 y 29 de septiembre)

- **La huella cumple su promesa** (`039`): sin `update`, `delete` ni `truncate` para nadie.
- **A7a, un custodio por organización** (`040`–`042`): cada celda entra a la base con su papel
  (`cofre_<celda>`), con seguridad por fila en 8 tablas y las vistas en `security_invoker`. Fuera
  el login compartido y su secreto. Los eventos E1–E6 se provocaron y midieron antes de quitar la
  vuelta atrás.
- **A1, el contrato.**
- **A2**: `044` (`organizacion` y `celda` en la huella, la identidad de la celda e
  `iam.decision`), el puente en `ore-iam` y las llaves que se releen.
- **A3**: la red, el CronJob de llaves y el `uniqueId` desde el aprovisionador. Primero el binario
  y después la malla.
- **A4**: el crate `ore-acceso`.
- **A5**: P2 (`045`), en vivo; las tres fusiones sin revisión de `victor` están en la huella,
  abiertas y cerradas.
- **A6.1–A6.5**: la actividad, de punta a punta.
- **A9′, el hallazgo grave.** Con registro abierto, toda cuenta con audiencia `ore-serve` y las
  celdas en internet, `ore-serve` no comprobaba pertenencia: **cualquier cuenta habría entrado en
  cualquier celda**. Ahora la celda pregunta `organizacion:leer`. Los agentes que ninguna celda
  usaba salieron de `iam.agente` (`046`) y, en 0048, del realm.

### Las hipótesis, resueltas

| | hipótesis | resultado |
|---|---|---|
| H1 | el token sigue siendo sólo identidad | ✓ |
| H2 | cada módulo declara sus verbos; `ore-iam` guarda el catálogo | ✓ afinada: verbos agrupados, no uno por ruta; `ESCRITURAS` junto a las rutas |
| H3 | gestión y datos, dos conjuntos | ✓ medida (23 rutas de datos); el conjunto de datos llega con A8 |
| H4 | decide `ore-iam`, en AuthZEN, con `version` | ✓ |
| H5 | el sujeto del token, la organización de la celda | ✓ |
| H6 | caché corta, 503 sin respuesta, identificador de decisión | ✓ |
| H7 | saltarse la protección es una potestad propia | ✓ |
| H8 | el custodio pregunta por el puente | **abierta** (A7b). El cruce entre inquilinos se cerró antes, con A7a |
| H9 | los conjuntos bastan para P2; Cedar para decidir por recurso | ✓ para P2; Cedar, con A8 |
| H10 | un token por celda (Keycloak 26.2) | ✗ **sustituida por A9′**: la pertenencia la sabe `ore-iam`, no Keycloak |
| H11 | `hizo` después, con la forma de la huella | ✓ |
| H12 | siempre gestión, escrituras y denegaciones; datos si se enciende | ✓ lo primero; el interruptor, con A8 |
| H13 | el commit sigue siendo la auditoría fina | ✓ |
| H14 | la actividad se lee en `ore-iam`, por organización | ✓ A6.2 |
| H15 | `hizo` no hace fallar lo hecho; lo que se salta algo, antes | ✓ |

## Lo que se decidió no hacer

- **Subir Keycloak a 26.2 para tokens por celda (A9): aplazado.** Con A9′, lo que cerraría es
  poco: una celda comprometida que reenvía el token de alguien a otra celda de sus propias
  organizaciones. Ver 0048.
- **Copia local de la política en cada celda.** Un `puede` cuesta ~2 ms y `context.version` deja
  la puerta abierta.
- **Reutilizar conexiones a `ore-iam`.** Abrir una cuesta 0,3 ms.
- **Registrar las lecturas del catálogo.** Volumen sin lector.
- **Seguir la regla de `008` en el puente** («toda ruta deja huella, leer incluido»): sería una
  fila por pregunta. Las demás rutas de `ore-iam` la siguen cumpliendo.

## Lo que este ADR no decide

- **El motor definitivo.** Cedar es el candidato por encaje; entra cuando haya que decidir por
  recurso (A8).
- **Quién lee qué vista**, ni los datos en ramas: es la capa de 0031, que usará este puente.
- **La elevación temporal** (Google PAM, Azure PIM). Queda nombrada.
- **Exportar la huella** a un almacenamiento del cliente o a un SIEM. Queda nombrada.

## Deuda y pistas

| # | deuda | por qué importa | pista |
|---|---|---|---|
| 1 | **A7b · el custodio no pasa por el puente.** Sigue leyendo `iam` por SQL. Con la seguridad por fila sólo ve su organización, pero son dos caminos a la misma decisión | una regla cambiada en `potestad::exige` vale en los dos sólo porque comparten biblioteca | `puede` con recurso para `resolver` (`{type: secreto, id}`), `hizo` antes de contestar, una ruta de `ore-iam` para conceder sobre un recurso (M4), y quitar al papel el `select` sobre el censo |
| 2 | **A8 · leer datos no pregunta.** El préstamo de credenciales (`loadTable`, `loadView`, ejecutar una vista, los datos del puesto) no consulta potestad | es la promesa de 0031 («una vista que no puede leer no llega al DataFrame») | espera la decisión de producto: el cliente gestiona quién lee qué desde la consola. El sitio es único: antes de prestar (M7). Con ella llegan el conjunto de datos (H3), Cedar (H9) y el interruptor de lecturas (H12) |
| 3 | **🔥 Las llaves del realm en `ore-iam` no las refresca nadie.** El ConfigMap `identidad/jwks` se creó a mano el 2026-09-08; `refresco-jwks-celdas` trae sólo las de Google | hoy coincide con lo vivo (medido el 2026-10-02); si Keycloak rota su llave de firma, `ore-iam` deja de validar tokens: **cada `puede` da 503 y la gracia de A9′ dura 10 minutos** | el mismo CronJob que `50-jwks` en las celdas, en `identidad`, para el realm. `ore-iam` ya relee ante un `kid` desconocido |
| 4 | **El puesto monta el secreto del agente de su celda** (`/puesto/agente-secreto`; los tres agentes, Python, Node y JVM, lo leen) | el código de un cuaderno puede sacar tokens de agente cuando quiera; con A9′ sólo valen en su organización, pero es una credencial larga donde bastaría un token corto | que `ore-serve` emita al puesto un token corto y acotado, o pasar el agente a Workload Identity |
| 5 | **M2 Q1 y Q4 sin medir**: cuántas preguntas hace una pantalla y cuántas lecturas de datos hay. `recuento` lleva días en vivo | decide `vale` (hoy 30 s) y si el plazo del 503 baja de 2 s a 500 ms; y dimensiona el interruptor de lecturas | `medida-el-salto.sh 4 --desde <hora>` tras provocar los cuatro eventos (consola con rama, propuesta fusionada, puesto con `loadTable` y SQL, pasada del catalogador). Para series largas, Cloud Logging `WORKLOADS` (coste, decisión aparte) |
| 6 | **`ore-driver`, cuenta huérfana** con `bigquery.dataViewer` y `jobUser` en todo el proyecto | alcance a datos sin dueño | comprobar en los registros de auditoría de Google que nadie la usa, y borrarla |
| 7 | **La base `iam` vive en el Postgres del IdP**, dueña `keycloak`, superusuario | el censo de acceso y el proveedor de identidad comparten base y superusuario | una base propia, o al menos un dueño que no sea superusuario. La `039` ya protege la huella también de él |
| 8 | **La salida de red de `identidad` está abierta** (ninguna NetworkPolicy de salida) | `ore-iam` y el IdP pueden salir a cualquier sitio | salidas declaradas: DNS, Google APIs, el relay de correo (587) y la base |
| 9 | **El ruido de la huella.** En un día, 640 `organizacion:listar` y 635 `celda:listar` (la consola al cargar), frente a decenas de actos | se filtra al servir, pero la tabla crece con lo que nadie lee | dejar de escribir el sondeo del plano de control: es otra decisión, con su medida (A6) |
| 10 | **El mapa de arranque se escribe aparte** (`rutas::mapa`, a mano) | M1 midió que no anunciaba 34 rutas; es la desincronización que `ESCRITURAS` evita | derivarlo del enrutador, como `ESCRITURAS` |
| 11 | **Dos guiones de medida leen `cofre-url`**, que ya no existe (`medida-el-acoplamiento-del-inquilino.py`, `medida-el-cofre-y-su-almacen.py`) | medidas que ya no corren | apuntarlos a `t-<n>-base-del-cofre`, o retirarlos |
| 12 | **`ore-iam`, una conexión a la base tras un `Mutex`** | cabe de sobra hoy (0,36 ms por consulta) | un pool cuando Q1 diga que hace falta |

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

El detalle de cada medida (sus tablas, sus correcciones y los pasos A2, A6, A7a y A9′ por
piezas) está en la historia de este fichero: `git log -- docs/decisions/0047-el-acceso.md`.
