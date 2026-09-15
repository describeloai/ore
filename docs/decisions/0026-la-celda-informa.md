# 0026 · La celda informa

**Estado:** en curso (E0–E4 hechas y medidas; E5 pendiente) · **Fecha:** 2026-09-15 · **Decide:** que el estado vivo de una celda —cuota
usada, jobs, salud del control— **lo observa un informador que vive en la celda y lo empuja al
plano de control**, con identidad de agente y privilegio acotado a su namespace; que `ore-iam` lo
guarda como **el último snapshot medido** y `GET /celdas` lo devuelve; que la consola pinta la
overview del clúster **desde el plano de control** y compone con lo que el árbol ya dice; y que
`ore-serve` **no gana ni red ni privilegios** por esto

---

## El problema

La overview de un clúster tiene nueve conceptos estándar (Redpanda, Atlas, Neon, Databricks: salud
agregada, conexión, capacidad frente a cuota, productos, actividad, red, coste). Los medimos en
`victor` el 2026-09-15 (`pruebas-de-fuego/medida-el-estado-de-la-celda.py`) y salieron en tres
columnas:

```
A  el árbol      versión, salud, último commit, fuentes, paquetes, cola     ore-serve YA lo alcanza
B  kubernetes    cuota 250m/10 CPU · 896Mi/36Gi · 4/50 jobs                 sólo el API server
                 jobs 0 activos / 4 ok / 0 fallidos · control desde 09:23, 0 reinicios
C  nadie         vistas materializadas (no hay R2 en la celda), coste, latencia
```

**B existe y `ore-serve` no puede leerlo**: la `NetworkPolicy` de la celda deniega todo egress
salvo DNS, forja, cofre, control e IdP —un `wget` al API server desde el pod muere por timeout
(código 143)— y el proceso **no tiene TLS a propósito** (Dockerfile: «tres cerraduras»). Eso no es
un obstáculo: es la garantía de que el plano de control de la celda no habla con nadie. La
pregunta es cómo llega B a la overview **sin tocar esa garantía**.

---

## Lo que se miró antes de decidir

- **Convenciones de la API de Kubernetes**: `spec` lo escribe el usuario; `status` lo escribe **un
  controlador con privilegio** en el subrecurso `/status`, con `observedGeneration` y
  `conditions` (`Ready`/`Progressing`/`Degraded`, `True|False|Unknown`, `Reason` en CamelCase).
  El consumidor no observa el mundo: lee el status. `iam.celda.aprovisionada` (0025-⑦) **ya es
  esto**.
- **Redpanda BYOC**: un agente en el data plane, con *tokens opacos y efímeros*, **tira** de las
  specs y **empuja** «telemetría, endpoints, readiness, status» al control plane. El control plane
  nunca entra en el data plane; el cluster sigue vivo si el enlace cae; la consola lee **sólo** el
  control plane. Confluent y Databricks, igual.
- **kube-state-metrics**: `kube_resourcequota{type="used"|"hard"}`, `kube_job_status_*`, acotado
  por tenant con `--namespaces` + `Role`. Es el estándar para *series en el tiempo* (los charts de
  Atlas/Neon), no para «ahora».
- **capsule-proxy**: el tenant lee el API server por un proxy que filtra. Para quien usa `kubectl`;
  nosotros no exponemos Kubernetes.
- **Lo que no aparece en ninguna referencia**: un fichero (`ConfigMap` montado) como vehículo de
  estado. Es un mecanismo de configuración reutilizado, con el retraso del kubelet, escrituras en
  etcd por minuto y celda, y obliga a la consola a preguntar a `ore-serve` si `ore-serve` está
  vivo. Se descarta.
- **Lo que ya tenemos de esto**: la celda tiene identidad de máquina —el cliente
  `ore-agente-<celda>`, `rubix_tipo=agente`, secreto en Secret Manager `t-<celda>-agente-*`—, un
  init `driver` que la trae (Workload Identity, `44-el-catalogo.yaml`), y `ore-iam` acepta verbos
  por **clase de sujeto** (`es_aprovisionador`, 0025-⑦). El API server de la malla está en
  `10.10.0.2:443` (`kubectl get endpoints kubernetes`).

---

## La decisión

> ### ① B lo observa un informador que vive en la celda.

Un `Deployment` mínimo por celda, `informador`, rendido por `gen-inquilino.py` en el
compartimento como `ore-serve` y `ore-cofre`. Con **su** `ServiceAccount` y un `Role` de sólo
lectura **en su namespace**: `resourcequotas` get/list, `jobs` list, `pods` list. Nada de clúster.
Egress **sólo** al API server (`10.10.0.2/32:443`), al IdP (por el token) y a `ore-iam`
(`identidad:8090`). Init `driver` que trae el agente del almacén —la misma receta que los Jobs—;
contenedor principal sobre una etapa nueva del Dockerfile, `informador` (`alpine` + `curl` +
`ca-certificates` + `jq`): **sin `gcloud`, sin `ore`, sin `git`**.

Cada 60 s: token de agente si el que tiene caduca en <60 s (`client_credentials`, vida 300 s);
tres `GET` al API server con el token de la `ServiceAccount` y `--cacert` del pod; un snapshot
con `jq`; un `POST` a `ore-iam`. Si algo falla, lo dice en el log y **sigue**: un informador que
muere por una respuesta rara deja de informar justo cuando más falta.

> ### ② El snapshot es un contrato, y es pequeño.

```json
{ "v": 1, "medido_en": "2026-09-15T11:20:03Z",
  "cuota":   { "cpu": ["250m","10"], "memoria": ["896Mi","36Gi"], "jobs": [4,50] },
  "jobs":    { "activos": 0, "ok": 4, "fallidos": 0,
               "ultimo": { "nombre": "el-arbol", "estado": "ok", "inicio": "…", "fin": "…" } },
  "control": { "listo": true, "desde": "2026-09-15T09:23:28Z", "reinicios": 0 } }
```

`[usado, duro]` tal cual los dice Kubernetes (cantidades como cadenas; la consola las parsea, no
el informador). ≤ 8 KB. `v` sube cuando cambie la forma; `ore-iam` rechaza lo que no reconoce. La
medida `medida-el-estado-de-la-celda.py` **rinde este mismo snapshot** desde fuera con `kubectl`:
es la implementación de referencia contra la que se coteja el informador.

> ### ③ `ore-iam` lo recibe, lo guarda como último estado, y lo devuelve.

```
POST /celdas/{celda}/estado      cuerpo = el snapshot        sólo el AGENTE de la organización de esa celda
GET  /organizaciones/{org}/celdas  …, "estado_medido": { …snapshot… }   omitido si nunca informó
```

Migración `036`: `iam.celda_estado (celda references iam.celda, medido_en timestamptz, cuerpo
jsonb)` — **una fila por celda, upsert**. Es telemetría, no un acto: **sin huella por
snapshot** (`Tx::confirmar_observacion`, la única excepción a la regla de `base.rs`, con nombre);
mil cuatrocientas huellas al día por celda enterrarían las que importan. Lo que **sí** es un
hecho y se anota (`celda:informa`): que la celda **empieza** a informar, y que **vuelve** tras
más de tres minutos sin que llegara nada (`recibido_en`, no `medido_en`: el silencio es de
recepción). La autorización es por clase
y por pertenencia: `rubix_tipo=agente` **y** `(emisor, sub)` es el `iam.agente` de la organización
dueña de la celda. Una persona: 403. El agente de otra organización: **lo mismo que una celda
inexistente** (422, «no hay ninguna celda»: no se revela que existe). El aprovisionador: 403 —tiene
sus dos verbos y ninguno más—.

> ### ④ La consola pinta desde el plano de control, y compone con el árbol.

La overview lee `estado_medido` de `GET /celdas` —que ya pide— y de ahí salen **Capacidad**
(usado/duro con barra), **Actividad** (jobs) y la mitad de **Salud** (control listo, desde,
reinicios). Lo de A —fuentes, paquetes, versión, commit— lo sigue pidiendo a `ore-serve` como
hoy. Con `medido_en` pinta «medido hace 40 s»; si pasan **más de 3 min** sin snapshot, la celda
pasa a *Needs attention* con el motivo «el informador no informa desde …», y se enseña **el
último estado conocido con su hora**, no un spinner ni guiones.

> ### ⑤ `ore-serve` no cambia.

Ni `GET /estado`, ni `ServiceAccount`, ni una regla de egress más. Las tres cerraduras siguen
cerradas. Lo que este ADR añade a la celda es **otro proceso** con **otro** privilegio, y ninguno
de los dos puede hacer lo del otro.

---

## Lo que se acepta a cambio

- **Un pod más por celda** (10m CPU · 16Mi pedidos), que cuenta contra su cuota. En el snapshot
  aparece su propio coste; es honesto.
- **Un token de agente vivo en cada celda todo el tiempo.** Ya lo era: los Jobs de catálogo lo
  piden igual. Vida 300 s, sin refresh token, cliente confidencial del IdP.
- **La IP del API server en una `NetworkPolicy`** (`10.10.0.2/32`). Cambia si se recrea el
  clúster; la guarda (`medida-por-celda.py`) la coteja con `kubectl get endpoints kubernetes`
  en cada pasada, y `gen-inquilino.py` la lleva como constante nombrada.
- **`ore-iam` recibe escrituras cada minuto por celda.** Un upsert de 1 KB; con mil celdas, 17
  por segundo. Sin huella, sin crecimiento: una fila por celda.
- **Frescura de un minuto, no tiempo real.** Es la de Atlas y Neon en sus overviews.

---

## El abordaje — y por qué es así y no de golpe

Cada etapa deja el sistema entero y medido; ninguna depende de que la siguiente llegue. Se
empieza por **quien recibe** y no por quien informa: un informador contra un verbo que no existe
no se puede medir; un verbo sin informador se mide con `curl`.

### E0 · La medida y el contrato — ✓ 2026-09-15

`medida-el-estado-de-la-celda.py` pasa de tabla a **snapshot**: `--snapshot` imprime el JSON
de ② rendido con `kubectl`, byte a byte como lo rendirá el informador. Es la referencia.
**Acepta:** el snapshot de `victor` valida contra el esquema de ② y cuadra con lo que dice
`kubectl` a mano. *Medido: `victor` a las 12:20Z — cpu 250m/10, memoria 896Mi/36Gi, jobs 4/50,
0/4/0, control listo desde 09:23 sin reinicios; `validar()` sin faltas.*

### E1 · `ore-iam` recibe (036 y el verbo) — ✓ 2026-09-15 (`los-verbos` 13 en local y en CI; en producción, con el token real de `ore-agente-victor`: `POST /celdas/victor/estado → 200`, `--cotejar`: recibido hace 1 s, sin diferencias; huella `celda:informa · empieza`)

`036-el-estado-de-la-celda.sql`; `POST /celdas/{celda}/estado`; `GET …/celdas` con
`estado_medido`; la puerta de clases de `rutas.rs` pasa de «aprovisionador sí/no» a **una tabla
clase → verbos** (persona, aprovisionador, agente). `los-verbos.sh` caso 13: se acuña un token
tipo `agente` con el `sub` del agente de `acme` → 200 y el `GET` lo devuelve; una persona → 403;
el agente de `nova` → 404; el aprovisionador → 403; 9 KB → 422; `v: 2` → 422; dos `POST` seguidos
→ una fila. **Acepta:** los-verbos verde en local y en CI; desplegado por CI; con `curl` y el
token real de `ore-agente-victor` (traído por `gcloud`, desde fuera) un snapshot hecho a mano
entra y `GET /celdas` lo devuelve con `medido_en`.

### E2 · El informador — ✓ 2026-09-15 (`--cotejar` en `demo`, `prueba` y `victor`: snapshot fresco —7 a 57 s— y sin diferencias con la referencia; 0 `✗` en los logs de la primera hora; huellas `empieza` ×3 y una `vuelve`; MAESTRO = el API server real)

Etapa `informador` en el Dockerfile (CI la publica como `ore-informador:main`); `informar.sh`
(el bucle de ①, ~60 líneas, con `set -u` y sin `set -e`: cada paso decide si sigue); en
`13-el-inquilino-reconciliado.yaml` (bloque `demo`, que es la plantilla): `ServiceAccount`
`informador`, `Role` + `RoleBinding`, `Deployment` con init `driver` y `NetworkPolicy`
`salida-del-informador` (API server, IdP, `ore-iam`); `gen-inquilino.py` lo rinde por celda y
lo comprueba (⑫: el `Role` sin verbos de escritura, sin `ClusterRole`, la IP del maestro = MAESTRO);
`--cotejar` coteja MAESTRO con `kubectl get endpoints kubernetes`. *Lo que la primera pasada real
enseñó: `.dockerignore` excluía `malla/` y la etapa no encontraba su guion; y la lista de Jobs de
`demo` (110 KB) no cabía en la línea de órdenes de `jq` — las lecturas van a fichero.* Flux lo lleva a
`demo`, `prueba` y `victor`. **Acepta:** `medida-el-estado-de-la-celda.py --cotejar` es la sección D:
`estado_medido` de `GET /celdas` frente al snapshot que la propia medida rinde ahora — mismos
números, `medido_en` < 90 s, en las tres celdas; y el log del informador de `victor` sin un
solo `✗` en una hora.

### E3 · La overview lee el plano de control — ✓ código 2026-09-15 (consola `b5b23d4`: Capacidad usado/duro, Actividad, control; «medido hace»; regla de 3 min por `recibido_en`); la aceptación con el informador parado, pendiente de mirar en pantalla

`ClusterOverviewView`: Capacidad con usado/duro real, Actividad con los jobs, Salud con el
control y la regla de los 3 min (④). **Acepta:** los números en pantalla son los de la medida
(mismo snapshot, misma hora), y con el informador parado a mano (`kubectl scale --replicas=0`)
la overview dice *Needs attention · el informador no informa desde hh:mm* y conserva el último
estado; al levantarlo, vuelve sola.

### E4 · Salud agregada — ✓ 2026-09-15 (consola: `lib/cloud/salud.ts` con las reglas y `scripts/salud.mjs` con 15 casos —uno por regla, dos de orden— en cada build)

Un solo veredicto arriba, con las reglas escritas en un sitio (`lib/cloud/salud.ts`): *healthy*
si el árbol contesta, la entrada lleva a la puerta, el control está listo, el snapshot es
reciente y no hay jobs fallidos en la última hora; si no, *needs attention* **con el motivo
primero**. **Acepta:** cada regla tiene su caso en la medida (árbol caído, DNS torcido,
informador silente, job fallido) y la pantalla dice el motivo correcto en cada uno.

### E5 · Lo que queda fuera, dicho

Series temporales (C) → kube-state-metrics por namespace y una pantalla de Observability, **otro
ADR**. Vistas materializadas → cuando la materialización viva en la celda. Coste → cuando haya
precio por vCPU-hora. La overview lo dice como «sin medir», no con guiones.

---

## Lo que este abordaje NO hace, y por qué

- **No pone `GET /estado` en `ore-serve`.** Sería darle red o privilegios al proceso que no debe
  tenerlos, o hacer que la consola dependa del data plane para saber si el data plane está vivo.
- **No lo hace el reconciliador central.** Podría (ya escribe `aprovisionada`), pero cada 5 min,
  con cluster-admin, y mezclando la pasada de aprovisionar con mirar cada minuto. El informador es
  por celda, con el privilegio de su namespace, y si una celda enferma sólo deja de informar ella.
- **No guarda series en `ore-iam`.** Una fila por celda. Las series son métricas y van por
  métricas.
- **No expone Kubernetes al cliente**, ni por proxy: el cliente ve capacidad, jobs y salud con las
  palabras del producto, no `ResourceQuota` ni `Pod`.
- **No da a la consola acceso al clúster.** Lee `ore-iam` y `ore-serve`, como hoy.
