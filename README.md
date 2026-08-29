# ORE · Ontology Runtime Engine

**El motor que convierte un repositorio ontológico en un paquete vivo.**

> Un `Ontology Repository` es texto declarativo en Git: exacto, revisable y **inerte**.
> ORE lo compila, lo coteja, lo firma y lo sirve — gobernado, tipado y en tiempo real —
> a tus aplicaciones y a tus agentes.

Apache-2.0 · implementación de referencia de [**OOS**](https://github.com/oos-dev/oos) ·
**pre-alfa, no existe todavía**

---

## Qué es, y qué no es

ORE **no es un estándar**. [OOS](https://github.com/oos-dev/oos) lo es; ORE es su
implementación de referencia. Y ORE **no permite versionar tu ontología en Git** — Git ya
versiona.

> **ORE permite ejecutar.** Ese es el hueco que nadie había cubierto, y es todo su motivo
> de existir.

| No es | Eso es |
|---|---|
| un data warehouse ni un motor de almacenamiento | tu almacén de siempre |
| un ETL | tus pipelines de siempre |
| un LLM, ni un proxy de LLM | tu modelo de siempre |
| un catálogo de metadatos | describe; ORE ejecuta |

**Sí es un índice secundario distribuido**: mantiene la topología del grafo localmente y
deja la carga útil en origen. No mueve tus datos a ningún sitio nuevo — mueve las aristas
que los conectan, y solo cuando el repositorio lo declara.

---

## El arco

```
   FUENTE            ARTEFACTO           GRAFO             SUPERFICIE
   Ontology     →    Ontology      →     en memoria   →    MCP · GraphQL · SDK
   Repository        Bundle
   texto en Git      firmado             mmap + índice     lo que consume un agente

   commit SHA        sha256:…            digest + marca    endpoint versionado
                                         de agua
```

Un consumidor nunca ve un repositorio ni un bundle: ve una superficie.
Un operador nunca despliega un repositorio: despliega un bundle.
Un humano nunca edita un bundle: edita la fuente.

---

## Tres caras, tres fronteras de confianza

Bajo un solo binario hay tres productos, y confundirlos es lo que atasca una revisión de
seguridad.

| Cara | Momento | Comandos | Qué toca |
|---|---|---|---|
| **Scaffolder** | autoría | `init` `source add` `discover` `review` `drift-detect` | metadatos de producción y, si se pide, un LLM |
| **Compilador** | CI | `lint` `validate` `test` `diff` `plan` `compile` `promote` `export` | **nada. Sin red, sin credenciales, sin reloj** |
| **Runtime** | producción | `dev` `serve` + Helm | credenciales vivas de todas las fuentes |

**Nueve de catorce comandos no abren un socket.** De ahí sale un argumento que no es
marketing:

> El paso que decide **qué significan las cosas** es el único que no puede filtrar nada.

Y el scaffolder queda **fuera del camino de ejecución de confianza**: escribe ficheros,
nunca escribe en el grafo. Lo que produce es una propuesta en `DRAFT`, y la única vía de
propuesta a verdad es un commit revisado. **La IA aporta velocidad sin aportar autoridad.**

---

## Lo que hace que valga la pena

### La gobernanza se demuestra al compilar

```console
$ ore compile

error[OOS4001]: flujo de información no autorizado

  hr.Employee.baseSalary ──derivación──▶ totalCompensation ──binding──▶ materialization.cache

  etiqueta del origen      : gdpr.sensitivity = critical   (declarada)
  etiqueta de la derivada  : gdpr.sensitivity = critical   (computada, join)
  autorización del conducto: gdpr.sensitivity = low

  → declarado en   packages/hr/entities/Employee.yaml:22
  → propagado por  packages/hr/entities/Employee.yaml:31
  → alcanza        packages/hr/bindings/warehouse.yaml:12

  ayuda: baja el modo a `passthrough`, aplica un desclasificador autorizado
         (`mask`, `aggregate`), o eleva la autorización del conducto en
         conduits.yaml — lo último requiere revisión de CODEOWNERS.
```

**Nadie clasificó `totalCompensation`.** El compilador lo hizo propagando desde sus
orígenes, y por eso la etiqueta sigue siendo cierta dentro de seis meses.

Sin conexión a la base de datos. Sin credenciales. Sin un solo dato leído. **Un auditor
externo lo verifica clonando el repositorio.**

### Nadie escribe un binding a mano

```console
$ ore source add --name crm_prod postgres://acme:••••@db.internal:5432/crm
  ✓ conectado · PostgreSQL 16.2 · 47 tablas
  ✓ credencial en .env.local (añadido a .gitignore)
  ✓ residency: <sin declarar>              ← decisión pendiente

$ ore discover --source crm_prod --out packages/customers
  12 entidades · 38 relaciones desde claves foráneas · 4 tablas puente
  6 critical · 11 high · 3 sin decidir (conf. 0.31)
  ⚠ 3 decisiones te esperan:  ore review

$ ore review
  notes : text — confianza 0.31. El texto libre suele contener PII incidental.
  ¿Sensibilidad?
  › high     puede contener PII   (recomendado)
    none     confirmo que no
```

**No escriben ficheros: contestan preguntas.** Y las políticas tampoco —
`ore policy add --template gdpr.purpose-limitation` pide cuatro respuestas y emite Cedar
correcto. Escribir Cedar a mano es la vía de escape del 10%, no el camino normal.

### El despliegue es reconciliación

```
merge a main → webhook → ORE descarga el delta → recompila → hot reload sin downtime
```

Estado deseado en Git, estado real en el grafo, un reconciliador que los converge.
**Es ArgoCD aplicado a la semántica.**

Y su corolario es la razón de compra en sector regulado: si el grafo es siempre función
determinista de un commit, *«¿qué sabía el agente el martes a las 14:32?»* se responde con
**un commit y una marca de agua**.

---

## Arquitectura

### Dos planos, y no se mezclan

| Plano | Qué sirve | Dónde vive | Latencia |
|---|---|---|---|
| **Contexto** | entidades, relaciones, tipos, políticas, linaje | **artefacto mapeado en memoria** | µs–ms |
| **Datos** | filas y valores | consulta federada al origen | la de la fuente |

Casi todo lo que un agente necesita para no alucinar vive en el plano de contexto, que es
local y compilado. **El plano de contexto no quiere una base de datos**: son megabytes que
no cambian entre despliegues, así que la forma correcta es una estructura de solo lectura
mapeada en memoria — acceso sin deserialización, arranque en milisegundos, sin
calentamiento.

### Sin estado por defecto

> **ORE es *stateless* por defecto y *stateful* por declaración.**

Un repositorio con todos sus bindings en `passthrough` **no necesita base de datos alguna**:
arranca, mapea el bundle y sirve. El almacenamiento aparece solo cuando un binding declara
`mode: index` o `cache`.

Tres consecuencias: el binario abierto es **apto para producción** en el caso mayoritario;
la complejidad operativa es proporcional a la ambición y no un peaje de entrada; y un pod
de ORE es un contenedor que se puede matar y recrear libremente.

### Superficie de servicio

Tres, y ninguna más en v1: **MCP** —donde está la distribución del ecosistema agéntico, y
que sustituye a seis adaptadores de framework—, **GraphQL** para aplicaciones, y un
**protocolo nativo** para alto rendimiento. JSON-LD y Cypher son `ore export --format`, no
camino caliente.

---

## Niveles, y por qué ORE no tiene privilegios

| Nivel | Qué hace | ¿Acceso a datos? |
|:---:|---|:---:|
| **L0** · Validador | valida, normaliza, comprueba el flujo, emite digest | **no** |
| **L1** · Servidor de contexto | entidades, relaciones, tipos, políticas, linaje | **no** |
| **L2** · Ejecutor | resuelve bindings, aplica obligaciones, federa | sí |
| **L3** · Actor | ejecuta funciones y hace cumplir `autonomy` | sí, con escritura |

**ORE ejecuta la suite de conformidad de OOS como un consumidor externo cualquiera**, por
su CLI pública y sin acceso privilegiado a sus estructuras internas.

No es escrúpulo: es lo único que impide que la especificación acabe teniendo la forma de su
implementación de referencia. **ORE debe ser reemplazable para que OOS valga algo, y OOS
debe ser adoptable sin ORE para que ORE valga algo.**

---

## Instalación

```bash
brew install oos-dev/tap/ore
cargo install ore-cli
curl -fsSL https://get.ore.dev | sh
pnpm add -g @oos-dev/ore            # binario Rust envuelto en npm
docker run ghcr.io/oos-dev/ore
```

Binario estático nativo en Rust, distribuido con la misma simplicidad que `docker`,
`kubectl` o `terraform`.

---

## Estado

**Nada de lo anterior existe todavía.** Este repositorio arranca hoy.

| Fase | Qué | Criterio de éxito |
|:---:|---|---|
| **0** | esquemas, `ore validate`, `ore compile`, el runner de conformidad | compila el ejemplo de referencia y emite un digest estable |
| **1** | `source add` · `discover` · `review` sobre PostgreSQL | apuntar a un esquema sucio de ~50 tablas y que un arquitecto diga *«está un 80% bien»* tras contestar cinco preguntas |
| **2** | retículos, conductos, propagación, chequeo de flujo, Cedar embebido | `ore validate` falla con la cadena causal completa ante PII que alcanza un conducto no autorizado |
| **3** | `ore dev` + servidor MCP + obligaciones en lectura | un agente pregunta por MCP y el PII vuelve enmascarado **sin que el agente haya hecho nada** |

Objetivo de la fase 0, medible desde el primer commit: **73 casos de conformidad rojos → verdes.**

### Frontera abierta

El binario es **Apache-2.0 y plenamente apto para producción** en un nodo y una región,
sin límite de uso, para siempre. Lo que se construye encima —alta disponibilidad
multi-región, sincronización de índices entre nodos, conectores enterprise, plano de
gobernanza— es otra cosa y vive en otro sitio.

Un motor abierto que solo sirviera en un portátil no capturaría ningún estándar, y sin
estándar no hay nada que construir encima.

---

## Relación con OOS

| | |
|---|---|
| **OOS** define el artefacto | régimen de identidad, modelo de compilación, contrato de conformidad, vocabulario de gobernanza |
| **ORE** define la ergonomía y la ejecución | cómo un humano llega a un paquete conforme, y cómo se sirve |

Que sean dos cosas es deliberado: **otro proveedor podría construir una experiencia de
autoría mejor que esta y seguir produciendo paquetes conformes.** Eso es exactamente lo que
un estándar hace posible, y la razón de que el motor no se guarde nada.
