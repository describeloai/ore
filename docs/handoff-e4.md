# Handoff · E4 — el plan consulta la caché

> **Este documento es desechable.** Se borra el día que el criterio de §6 se ponga en verde.
> Un plan que sobrevive a su ejecución deja de ser un plan y pasa a ser documentación de un
> pasado que ya nadie comprueba.
>
> Fecha: 2026-09-01 · Escrito después de E0 y E1, con lo que E1 dejó medido delante.

---

## 1. Qué falta, exactamente

E1 dejó la decisión construida y **sin consumidor**. `ore cache check` la contesta a mano;
`planificar` no la pregunta. Hoy la fase ③ va siempre a la fuente:

```text
① AUTORIZAR   ✅
② TRAVESÍA    ✅  el índice, en local
③ CARGA ÚTIL  ⚠️  una petición por (fuente, entidad) — SIEMPRE al origen
④ ENSAMBLAR   ✅
```

Y con ella falta la propiedad que el motor promete y que hoy no tiene forma de ser cierta:

> **Un plan cuya caché sirve no abre ninguna conexión al origen — y la respuesta dice de
> dónde salió cada lectura.**

La segunda mitad de esa frase no es adorno. Una respuesta que no distingue caché de origen no
se puede auditar, y *«¿esto vino del lago o del sistema de gestión?»* es la primera pregunta
de cualquiera que revise un número raro.

---

## 2. La decisión que ya estaba tomada y hay que leer

El [ADR 0006](decisions/0006-el-artefacto-de-topologia.md) §3, en la línea que decide E4 entero:

> *El escaneo columnar que reclamaba Arrow/Parquet lo hace el mismo camino que ya lee
> cualquier tabla del cliente: **la caché entra por la puerta que ya existe**.*

Con eso, **E3 se encoge**. Leer la caché no necesita ningún programa nuevo: una tabla Iceberg
en el lago del cliente es una tabla, y ya hay un protocolo para leer tablas. Lo que queda
delegado es **escribirla**, que es un materializador, no un lector.

> **Servirse de la caché es cambiarle a una lectura la fuente y el objeto. Nada más.**

Ese es el corazón de E4 y es lo que lo hace pequeño. Si acaba siendo grande, algo se ha
torcido.

---

## 3. Las tres decisiones que hay que tomar para que eso funcione

### 3.1 · Las columnas de la tabla de caché **son los nombres de propiedad**

Un binding mapea propiedad → columna **del origen**: `nom → NOMBRE_CLI`. La tabla de caché no
tiene por qué llamarlas igual, y hace falta una convención porque si no la proyección no se
puede reescribir.

**La convención es la identidad: la columna se llama como la propiedad.** Dos razones:

1. **La caché la escribimos nosotros**, así que elegimos los nombres. No es una tabla ajena
   que haya que aceptar como venga.
2. Una tabla cuyas columnas son propiedades **es autodescriptiva**, y su correspondencia con
   el modelo no puede desviarse de un binding que alguien edite mañana. Con nombres del
   origen habría dos mapas del mismo hecho y ninguno diría cuál manda.

Es una obligación para el materializador de E3, y se escribe aquí porque hoy no hay quien la
recuerde.

### 3.2 · El manifiesto dice **en qué fuente** vive la tabla

`Entrada` tiene `tabla` —una coordenada— y le falta de qué fuente declarada sale. Sin ese
campo no se puede elegir driver ni resolver la credencial, y meterla en la coordenada sería
volver a la cadena de conexión dentro del documento, que es exactamente lo que `source add`
separó.

El formato es de ayer y no ha salido de aquí: entra en `oreCache: 1`, no hay que versionarlo.

### 3.3 · El instante entra en la **consulta**, no en la respuesta

Comprobar si algo está rancio necesita saber cuándo se pregunta, y eso hoy solo lo tiene
`responder`. Pero la decisión *«esto sale de la caché»* es de planificación: es la que decide
si se abre una conexión.

**No rompe la pureza del plan.** El plan sigue siendo función de sus entradas — el instante es
una entrada más, como los atributos del principal, y `responder` ya lo decía con todas las
letras: *«el motor no lee el reloj: el instante llega con la petición»*. Lo que cambiaría la
pureza sería leerlo, no recibirlo.

---

## 4. La forma

### 4.1 · De dónde sale cada lectura, dentro del plan

```rust
pub enum Origen {
    /// Del origen declarado en el binding. `porque` dice por qué no de la
    /// caché — `None` si no había manifiesto que consultar.
    Fuente { porque: Option<String> },
    /// De la caché, con hasta cuándo era cierta.
    Cache { marca: String },
}
```

`porque` **no es opcional por comodidad**: distingue *«no hay caché»* de *«hay una y está
rancia»* de *«hay una y se escribió bajo otra regla»*. Las tres producen la misma lectura al
origen y **no significan lo mismo**, y la tercera es la que alguien tiene que ver.

Va en la forma canónica del plan. Es un artefacto: si dos ejecuciones leen de sitios distintos
y el plan sale igual, el plan ha dejado de describir lo que pasó.

### 4.2 · La marca de la respuesta es **la más vieja** de lo que intervino

Hoy `Respuesta.marca` es la del índice de topología, *«lo único materializado que interviene en
v1»*. Con la caché deja de serlo.

> **La respuesta es tan fresca como su parte más rancia.**

Componerla de otra forma —la más nueva, o la del índice a secas— produciría una respuesta que
declara una frescura que ninguna de sus partes tiene. Es la clase de mentira que este proyecto
existe para acotar.

---

## 5. Los peldaños

Cuatro, y los tres primeros son de `ore-core`, que **compila en esta máquina**. `ore-exec`
enlaza el evaluador de Cedar y solo compila en CI, así que cuanto menos lógica viva allí,
antes se sabe si está mal.

| | Qué | Dónde |
|---|---|---|
| **E4.1** | `Entrada.datasource` | `ore_core::cache` ✅ local |
| **E4.2** | `Origen`, y la reescritura de una `Lectura` | `ore-exec::plan` |
| **E4.3** | `Motor::cargar_cache` y la consulta en ③ | `ore-exec::plan` |
| **E4.4** | `--cache` en la CLI, el origen impreso, y la marca compuesta | `ore-exec::main`, `responder` |

---

## 6. Listo cuando

1. Un plan cuya caché sirve **no produce ninguna lectura contra el origen**, y su forma
   canónica lo dice.
2. Un plan cuya caché se escribió **bajo otro bundle** produce la lectura contra el origen
   **con el motivo dentro** — no en silencio.
3. La marca de la respuesta es la más vieja de las que intervinieron, y hay una prueba con
   dos marcas distintas que falla si se coge la otra.
4. CI en verde, que es el único sitio donde `ore-exec` se compila.

---

## 7. Lo que **no** entra

**Servir media lectura.** Si la caché tiene tres de las cuatro propiedades, se va al origen
entera. Partir una lectura en dos y ensamblarlas es trabajo de ④ y no hay medida que diga que
merece la pena; hoy sería inventarlo.

**Escribir la tabla.** Es E3, y sigue siendo un programa delegado. E4 se prueba con un
manifiesto escrito a mano apuntando a una tabla que existe, que es exactamente como se probó
el índice de topología antes de que hubiera con qué construirlo.

**Decidir por latencia.** La caché se usa si sirve. Un planificador que estime costes y elija
es otro proyecto, y sin medidas sería adivinar con aspecto de optimizar.
