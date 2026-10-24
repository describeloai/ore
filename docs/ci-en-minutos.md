# El CI en minutos

**Objetivo:** de *push* a desplegado, **3 minutos** el caso normal y **5 como techo**, también cuando se
tocan binarios. Hoy son ~40 (medido el 2026-10-23 en `1cb8e0c1`).

## 1. Dónde se va hoy (`1cb8e0c1`, todos los jobs a la vez, el despliegue espera al último)

| job | dura | en qué |
|---|---|---|
| `conectores` | **37 min** | 10 min construir el laboratorio de Phoenix + 9 el de HBase (desde el archivo de Apache), y ~15 conectores **en serie** |
| `hbase-kerberos` | 19 min | un HBase con Kerberos montado de cero, en tres protecciones, **en serie** |
| `construir` | 18 min | 11,5 compilar binarios *release* en un runner de 4 vCPU, 4,3 construir y subir imágenes, 1 el frontend |
| `suite` | 4 min | 3 la suite (`cargo test`) |
| los demás | 1–4 min | |
| `imagen` | 3 min | espera a **todos** |

Y cuando uno falla se repiten los 40: el 2026-10-23 fueron tres vueltas (dos fallos de `conectores`).

## 2. Los factores

1. **Se prueba lo que no cambió.** Los laboratorios y los conectores corren en cada commit, toque o no
   lo suyo. *(Arreglado en la rama `ci-3-minutos`: huella por prueba, `cambios` + `puerta`.)*
2. **Se construye lo que no cambió.** Los laboratorios salen del archivo de Apache cada vez. *(Arreglado:
   `ci/laboratorio.sh`, por su huella en GHCR.)* Los binarios ya van por huella (0060 B2).
3. **Compilar *release* en una máquina pequeña.** 11,5 min en 4 vCPU; tocar `ore-core` recompila 22 de 24
   binarios (camino crítico `ore-code → ore-core`).
4. **Todo en serie dentro de un job.** Los conectores no dependen entre sí y van uno tras otro.
5. **Esperas fijas.** `sleep 30`, `sleep 20` tras cada laboratorio, además del sondeo.
6. **El despliegue espera a todo.** El job más lento manda aunque no tenga nada que ver con el cambio.
7. **Nadie lo vigila.** Ningún job tiene presupuesto: crecer de 8 a 37 min no rompió nada.

## 3. Cómo se llega

| paso | qué | queda en |
|---|---|---|
| **C1** | huella por prueba + laboratorios en GHCR (rama `ci-3-minutos`) | ~20 min si se toca `ore-core`; ~7 si sólo `ore-serve` |
| **C2** | `construir`: runner grande (16 vCPU) y caché de `target/` persistente por huella; compilar sólo los binarios cuya huella cambió (ya), en paralelo con las imágenes | ~5 |
| **C3** | lo que queda de `conectores` y `hbase-kerberos`, partido en jobs en paralelo (uno por conector) | cada uno ≤ 5 |
| **C4** | cortar el abanico de `ore-core`: que tocarlo no recompile 22 binarios | 3 |
| **C5** | las reglas de §4, cumplidas por el propio CI | se queda |

C2 cuesta dinero (un runner grande de GitHub, o uno propio en GKE, que choca con la cuota de 12 vCPU):
se decide aparte.

## 4. `construir` en 5 minutos sin pagar (diseño, 2026-10-23)

**Lo que hay.** El repositorio es público: los runners normales (4 vCPU) son gratis y sin límite de
minutos, y GitHub deja ~20 jobs a la vez. Uno grande se paga siempre; uno propio en GKE choca con la
cuota de 12 vCPU. `construir` compila 30 binarios en UN runner (perfil `imagen`: sin LTO, 16 unidades,
estáticos) con las dependencias de fuera ya cocinadas (`ore-deps:h-…`, cargo-chef), pero **los crates
del workspace se compilan enteros cada vez**; y construye las imágenes que cambiaron con `docker build`
de etapas del Dockerfile, de tres en tres.

**La idea: muchas máquinas gratis en vez de una cara, y no repetir nada.**

| pieza | qué | por qué ahorra |
|---|---|---|
| **K0 · medir** (el previo) | una corrida de `construir` con `cargo build --timings` y el tiempo de cada imagen: cuánto es compilar el workspace, cuánto enlazar y cuánto cada imagen | decide el orden de K1–K4 con números y no con intuición |
| **K1 · el `target/` del workspace en caché** | guardar `target/imagen` (sin `incremental`) por la huella de la receta, y restaurar el último de `main` (`restore-keys`). Cargo sólo recompila los crates cuya huella cambió y lo que depende de ellos | ya medido en H0: sólo `ore-serve` → 304 s en vez de 409; con caché caliente, menos. Hoy se pierde todo en cada corrida |
| **K2 · repartir la compilación** | `construir` pasa a una **matriz** de 3–4 jobs; cada uno compila un grupo de binarios (los grupos, por huella: sólo los que faltan) sobre la misma `ore-deps` y su propia caché de K1, y los deja como artefacto. Un job final junta y construye las imágenes | 4 runners × 4 vCPU = 16 vCPU **gratis**. Tocar `ore-core` sigue recompilándolo, pero en paralelo y sólo con los binarios de cada grupo |
| **K3 · enlazar rápido** | `mold` como enlazador (`-C link-arg=-fuse-ld=mold`, en `.cargo/config.toml` del CI) | 30 enlaces estáticos grandes; `mold` es varias veces más rápido que `ld` |
| **K4 · imágenes sin rehacer etapas** | las imágenes de binarios sólo `COPY` del binario (ya); las del puesto (`python`, `spark`, `notebooks`), su base con lo de `pip`/Spark como **imagen propia por huella** de lo que la produce (como los laboratorios), y encima sólo el SDK. Todas en paralelo, no de tres en tres | hoy cada cambio del SDK rehace la etapa entera del puesto |
| ~~**K5 · un binario en vez de 30**~~ *(descartado el 2026-10-24: con `mold` enlazar ya no manda; el nombre K5 pasa a la puerta por imagen, §4.1)* | un `ore` que despacha por nombre (`ore-serve` → enlace simbólico); cada binario pasa a ser una función de su crate | un solo enlace. Es un cambio grande: se decide con K0 |

**Lo que se espera** (a confirmar con K0): sólo `ore-serve` → 2–3 min de `construir`; tocar `ore-core`
→ 4–5 min (K1 + K2 + K3); cambiar el SDK → 1–2 min de imágenes (K4). Con C1 (pruebas por huella) y la
`puerta`, de *push* a desplegado: 3–5 min.

**Orden:** K0 → K1 → K3 → K2 → K4, cada uno ensayado en una rama con
`gh workflow run ci --ref <rama> -f ensayo=true` (sólo `construir`, no despliega) y medido; K5 si hace falta.
Sin coste: todo corre en runners gratis, y la caché de GitHub (10 GB por repositorio) guarda K1.

### 4.1 Medido y fusionado (2026-10-24, `726a8170`)

| pieza | medido |
|---|---|
| K1 + K3 | cambio normal: `construir` 3 min 13 s. Tocar `ore-core`: compilar 6 min 32 s (estáticos) + 1 min 55 s (HBase, dinámico, en serie) |
| K4 | imágenes de puesto con la caché de GHCR: 25–46 s (eran 29–130) |
| K2 | **no se hace**: la cadena `ore-core → ore-store → … → ore-view → ore-serve` es serie, y cada runner de una matriz la repetiría |

**K5 · una puerta por lo que despliega.** `puerta` (la plataforma: todo menos los drivers) y
`puerta-conectores` (`conectores` + `hbase-kerberos` + la de la plataforma), cada una con su relevo:
`imagen` mueve `:main`/`:1` y releva ore-serve, ore-cofre, ore-iam, informador, ore-medios, ore-teselas,
ore-motor y ore-postgres; `imagen-conectores` mueve `ore-drivers:main` y releva `ore-federation`. Así
`ore-serve` no espera a pruebas que no prueban nada suyo. Regla: el protocolo entre `ore-serve` y los
drivers sigue hablando con la versión anterior (unos minutos conviven).

**K6 · `conectores` en cuatro grupos a la vez** (matriz: `documentos` = MongoDB + Cassandra, `phoenix`,
`hbase`, `objetos` = S3, GCS, Azure, SFTP, SharePoint y BigQuery). En serie eran ~10 min. MongoDB y
Cassandra dejan de ser `services` (arrancarían en los cuatro) y los levanta su grupo. La marca de verde
la deja `conectores-verde` cuando pasan los cuatro.

⚠️ Una caché de GitHub guardada en una rama no la ve `main`: el primer CI de `main` tras fusionar fue en
frío para K1 **y** para las marcas de verde de C1 (por eso `conectores` y `hbase-kerberos` corrieron el
2026-10-24 aunque el commit sólo tocara `ore-store`).

### 4.2 K7 y K8 · que un empuje no compile en frío (2026-10-24, por medir)

Lo que pasó el 2026-10-24: `8ba5417c` añadió un crate (`ore-k8s`), que cambia `Cargo.lock` y con él la
receta de las dependencias. Las dos cachés de `construir` iban por esa receta: dependencias de terceros
y lo compilado del workspace, perdidas las dos. Se compiló todo desde cero: **26 min**. Y los dos empujes
siguientes (`9494992a`, `07e602b4`) arrancaron antes de que el primero guardara su caché, así que
compilaron en frío lo mismo.

| pieza | qué | espera |
|---|---|---|
| **K7 · `construir` en cola** | `concurrency: construir-<rama>`, sin cancelar: un `construir` de `main` espera al anterior. GitHub deja uno solo en espera, el último (sólo se despliega la punta) | el segundo de dos empujes seguidos, ~3 min en vez de en frío |
| **K8 · la caché sobrevive a una receta nueva** | la huella de las dependencias, en dos: **herramientas** (el compilador y cómo se llama) y **receta**. Con otra receta y las mismas herramientas se restaura la última imagen de dependencias y el último objetivo, y cargo, que decide por huella, compila sólo lo nuevo. La imagen se cocina (~20 min) sólo con herramientas nuevas | un crate o una dependencia nueva cuesta lo que cambia, no un arranque en frío |

Nunca se mezclan compiladores: el respaldo exige la misma huella de herramientas.

**Por medir** (el primer empuje que cambie `Cargo.lock` con K8 dentro): el paso «los binarios que
faltan, compilados» en minutos, contra los 26 de hoy.

**Lo que queda (C4):** cortar el abanico de `ore-core`. 28 de los 31 binarios dependen de él y casi todos
usan una parte pequeña: tocar lo que cambia a menudo (el esquema de executions, el 2026-10-24) recompila
28. Primero se mide quién usa qué de `ore-core`; luego se parte.

## 5. Las reglas fijas

1. **Ningún job pasa de 5 minutos.** `timeout-minutes: 5` por defecto en todos; una excepción va en una
   lista de este documento, con motivo y fecha de caducidad.
2. **Nada se construye dos veces.** Binario, imagen, laboratorio o frontend: por la huella de lo que lo
   produce, y si el registro ya la tiene, se baja.
3. **Nada se prueba si no cambió lo suyo.** Cada prueba larga tiene huella (lo que nombra: su bloque, sus
   paquetes con sus dependencias, sus rutas) y un verde por huella no se repite.
4. **El despliegue espera sólo a lo que el cambio tocó** (`puerta`).
5. **En paralelo por defecto.** Dos pasos van en serie sólo si uno usa lo que el otro deja, y se dice.
6. **Nada de esperas fijas.** Sólo sondeos de «ya está listo», con su tope.
7. **Cada CI dice cuánto tardó cada job** (resumen del run), y uno que se pasa de su presupuesto falla.
