# 0020 · El plano de control

**Estado:** aceptado · **Fecha:** 2026-09-08 · **Decide:** que atender a un cliente es de un
**programa delegado**, que sólo puede correr los verbos herméticos de `ore`, y que sin proveedor
de identidad **las rutas de datos no se montan**

---

## El problema

`ore serve` lleva desde v1alpha1 declarado y sin hacer nada. Deja de ser un hueco de
implementación en el momento en que hay una consola que pretende que **pulsar un botón sea una
operación en el servidor** — y entonces hay que contestar tres cosas que no tienen respuesta
obvia:

1. **dónde vive el proceso que escucha**, dado que `ore` no puede abrir un socket;
2. **qué puede ejecutar**, si va a mirar a internet;
3. **quién pregunta**, y qué pasa el día que nadie lo haya configurado.

Y una restricción que no es negociable y que ya está hecha cumplir por una prueba:
[`tests/dependencias.rs`](../../crates/ore-cli/tests/dependencias.rs) lee el `Cargo.lock` y falla
si aparece una crate de red, de TLS o de FFI en el cierre de `ore-cli`. Su primer veto lo dice
entero: *«un planificador asíncrono sólo hace falta para hablar con algo»*. ⇒ **`ore serve` no
podía ser un servidor HTTP dentro de `ore-cli`** aunque quisiéramos.

---

## Lo que se miró antes de decidir

`pruebas-de-fuego/medida-la-forma-del-servidor.py`. Cuatro plataformas que ya contestaron esta
pregunta, y contestan lo mismo cinco veces:

| rasgo | de dónde |
|---|---|
| la consola **no** es privilegiada: usa la misma API pública que el CLI | AWS |
| control plane y data plane están **separados** | Databricks |
| la escritura pasa por un **verbo declarado**, no por acceso libre | Foundry |
| la credencial se **presta**, no se entrega | Databricks |
| el botón **declara** y un reconciliador cumple | Kubernetes |

Los cinco son la misma figura: hay **un** sitio donde se decide, y la pantalla no es ese sitio.
Que es `DESIGN` §3.8 —*la política se aplica en un punto único, nunca el consumidor*— dicha desde
fuera.

---

## Decisión

> **Atender a un cliente es de un programa delegado. Es la cuarta vez que este árbol delega, y
> por la misma razón que las tres anteriores.**

| | qué delega | ADR |
|---|---|---|
| `ore-read-<tipo>` | leer filas de un origen | [0008](0008-el-protocolo-del-driver.md) |
| `ore-maintain` | correr el circuito Δ | [0013](0013-el-protocolo-del-mantenedor.md) |
| `ore-store-<tipo>` | sellar y subir el artefacto | [0015](0015-el-protocolo-del-almacen.md) |
| **`ore-serve`** | **atender a un cliente** | **0020** |

En las otras tres, que el compilador se quedara fuera era el efecto secundario. Aquí es el punto:
**el proceso que mira a internet no es el que decide qué significan las cosas.**

### ① La lista es de permitidos, y ése es todo el diseño

`ore-serve` corre verbos de `ore` como subproceso. Lo que puede correr está en una lista de
**permitidos**, no de vetados, y no es una preferencia de estilo:

> Con una lista de vetados, **el verbo que se añada mañana llega permitido**, y nadie tiene que
> decir nada para que eso pase.

Es **P4** —*omitir es cerrar, no abrir*— aplicada al proceso en vez de al dato, y es la misma
denegación por defecto que `ConduitPolicy`: una autorización ausente es ⊥ y no admite nada.

El corte fino de la lista no lo inventa este ADR, lo dice `ore-cli/src/fuente.rs`: **`source add`
está y `source catalog` no**, porque registrar una fuente y sondearla son dos actos con fronteras
de confianza distintas — *«No abre un socket»*.

⇒ **El servidor no necesita la credencial de ningún origen.** Lo que toca el mundo se va a un Job
con la imagen de drivers, su identidad y su cuota, que es el reparto que ya corre en
[`malla/94-flujo-completo.yaml`](../../malla/94-flujo-completo.yaml).

### ② Sin proveedor de identidad, las rutas de datos no se montan

El puerto de identidad **no tiene defecto**. Sin proveedor configurado, `/fuentes`, `/paquetes` y
todo lo que lee el árbol **no existen** — y contestan `404`, no `401`: un `401` insinuaría que la
ruta está y que con la credencial correcta contestaría.

La forma es prestada y se dice de dónde: la plataforma resolvió esto mismo en
`auth/src/identidad.ts`, y su conclusión —*sin proveedor, las rutas de datos no se montan*— es la
que se copia. El modo de prueba, en el que el sujeto llega en una cabecera, exige **dos**
interruptores: pedirlo por su nombre y declarar que esto no es producción. Uno solo se pulsa sin
querer, y el modo que se enciende sin querer es el que se queda encendido.

### ③ Y no autoriza

Este proceso dice **quién pregunta** y nada más. Quién puede qué se decide en otro sitio, y
mezclarlo aquí es exactamente cómo la frontera entre identidad y permiso se dibuja torcida.

---

## Lo que se aceptó a cambio

**Ni una crate ajena, y por tanto HTTP escrito a mano.** No hay `keep-alive`, no se sirven
ficheros y no hay negociación de contenido: cada conexión atiende una petición y se cierra. Es
más lento, y en un plano de control no se nota. Lo que compra es que el proceso que mirará a
internet tenga un cierre de dependencias que cabe en una línea — la misma economía por la que
este árbol no lleva analizador de JSON ([ADR 0002](0002-sin-validador-de-json-schema.md)): **JSON
es un subconjunto de YAML**, así que el analizador de eventos que ya existe lee una petición sin
enterarse de que no es un documento OOS.

**El servidor lanza procesos.** Un verbo hermético es un `fork` y un `exec`, con su coste. La
alternativa era enlazar `ore-core` y llamar a las funciones, y se descartó por una razón concreta:
`ore` decide su código de salida y sus mensajes en `main.rs`, y reimplementar esa traducción aquí
sería tener **dos** definiciones de qué significa que algo falló.

**No hay reconciliador todavía.** El botón «añadir origen» es una llamada síncrona, no un objeto
declarado que algo lleve a su estado. El flujo que la consola describe —*al dar `add` se redirige
al catálogo con sus contenidos descubiertos*— es un reconciliador, y **hoy no está**: `POST
/fuentes` da de alta y responde diciendo, en el cuerpo, que leer el origen no corre aquí. Es
honesto y no es lo que se quiere; queda escrito.

**El árbol vive donde se le diga, y hoy no vive en ningún sitio duradero.** `--repo` es un
directorio. En el clúster eso es un `emptyDir` que muere con el pod. Dónde vive el repositorio es
la decisión que esto deja destapada y no cierra.

---

## Lo que se comprueba, y no se promete

`pruebas-de-fuego/servidor.sh` levanta el servidor de verdad en un puerto y le habla con `curl`.
Cuatro hechos que no se pueden afirmar sin un socket:

```text
1  sin `--identidad`, las rutas de datos NO ESTAN        404, no 401
2  con identidad y sin sujeto 401 · con sujeto, pasa
3  una URL con credencial dentro se NIEGA                y dice por qué
4  alta → inducir → cola → responder                     cierra las decisiones
```

Y 14 pruebas dentro del crate, de las que tres son el diseño y no un detalle: que un verbo que
existe y no se listó **no pasa**, que `package` a secas no pasa y `package new` sí, y que
`Ausente` e `Invalida` no se colapsan.

---

## ✏️ Enmienda del 2026-09-08 — la identidad, y de dónde sale la llave

`--identidad oidc` verifica un token del realm. Lo que este ADR añade es **una
decisión que no era obvia y que da forma al despliegue**:

> **El juego de llaves llega como un fichero. Este proceso no va a buscarlo.**

Es la cuarta vez que el árbol reparte lo mismo: leer un origen es de
`ore-read-<tipo>`, subir un artefacto de `ore-store-<tipo>`, atender a un
cliente de `ore-serve`, y **traer el JWKS de un Job con la imagen que tiene
TLS**. Compra tres cosas concretas: el plano de control no necesita una pila
TLS de salida —su `NetworkPolicy` sigue abriendo sólo la forja y el DNS—, su
arranque no depende de que el IdP esté vivo, y la rotación de llaves es un
despliegue visible en vez de un temporizador que un día falla en silencio.

**El precio, dicho:** si nadie refresca el fichero, una rotación del realm deja
fuera a todo el mundo. Quien lo refresca es `malla/50-jwks.yaml`.

### Lo que se comprueba de un token, y en qué orden

```text
1  la forma        tres partes
2  el algoritmo    contra una LISTA de permitidos — `none` no es un algoritmo
3  la llave        la del `kid`
4  la FIRMA        antes de creerse un solo campo del cuerpo
5  el emisor       `iss` exacto
6  la audiencia    la NUESTRA — un token del mismo realm para otro servicio no vale
7  el reloj        `exp` y `nbf`, con 60s de holgura
```

El orden no es de estilo: **leer `iss` de un token sin verificar es leerle un
dato a quien lo escribió**. Y el paso 6 es el que la gente olvida — el realm es
compartido, así que un token perfectamente firmado de `rubix-consola` llegaría
aquí sin nada malo salvo que no es para nosotros.

### Y la dependencia que entra

`RS256`, medido contra el generador de realms de la plataforma. `ed25519-compact`
—que ya estaba— no sirve: es otra curva. Entra `rsa`, Rust puro y sin FFI, y
**no se escribe a mano**: equivocarse en el relleno de PKCS#1 no hace que las
firmas dejen de verificar, hace que verifiquen firmas inválidas. Es la misma
frase que `ore-core` ya tiene escrita para Ed25519.

### Lo que NO está encendido

El IdP vive en el otro clúster y está suspendido: `login.paladio.io` contesta
`503`. El código está construido y probado —34 pruebas en el crate y
`pruebas-de-fuego/servidor-oidc.sh` acuñando tokens de verdad contra un socket
de verdad—, y encenderlo necesita dos cosas que no son código: **levantar el
IdP** y **crear el cliente `ore-serve` en el realm**, que es una audiencia con
todos los flujos apagados, igual que `rubix-api`.
