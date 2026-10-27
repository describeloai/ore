# 0004 · Distribución del binario

**Estado:** aceptado · **Fecha:** 2026-08-30 · **Decide:** el artefacto se publica como `ore` compila un bundle

---

## Contexto

`README` §Instalación lleva cinco canales anunciados y ninguno existente. Hasta hoy
tampoco había CI: ni un flujo, ni una imagen, ni un instalador. Nada de ORE se puede
obtener de ningún sitio que no sea `cargo build` sobre el repositorio.

La referencia que se puso encima de la mesa fue OpenMetadata, cuyo arranque es
`pip install openmetadata-ingestion[docker]` seguido de `metadata docker run`. La
pregunta era si copiar esa forma.

## La distinción que decide

Ese comando levanta **servidor, base de datos y motor de búsqueda**, y no por
descuido: un catálogo *es* un servicio vivo que almacena metadatos y sirve
búsquedas. No puede haber OpenMetadata sin OpenMetadata corriendo.

El compilador de ORE es lo contrario **por invariante**: hermético, sin red, sin
credenciales, sin reloj, y su salida es un fichero. Y no es aspiración — se midió:

```
ore            4,15 MB
dependencias   anyhow · clap · saphyr-parser · sha2 · unicode-normalization
ficheros que lee en ejecución    ninguno
```

La única referencia a `vendor/oos/schemas` en todo `ore-core` está bajo `#[cfg(test)]`:
las reglas de forma van compiladas ([ADR 0002](0002-sin-validador-de-json-schema.md)).
**El binario no necesita ni la especificación en disco.**

> No hay nada que levantar. El equivalente elegante no es `ore docker run`; es que la
> instalación consista en tener el fichero.

## Decisión

Publicar binarios por plataforma desde un flujo que trata a su propio artefacto como
`ore` trata a un bundle. Cuatro pasos, y cada uno responde a algo que este proyecto ya
afirma en otro sitio:

| Paso | De dónde sale |
|---|---|
| **no existe si la suite no pasa** | `74/74` es la afirmación del producto; un binario que no la cruza no es una versión de esto |
| **se construye dos veces y se comparan los hashes** | **G1**: el mismo commit produce el mismo digest. Un motor no puede prometer para un bundle lo que no cumple para sí mismo |
| **se sella con procedencia SLSA** | `DESIGN` §5.2 compara OOS con SLSA — *«propiedades verificables sobre construcciones y las atestaciones que las prueban»*. El paso hace literal la analogía |
| **se publica su `sha256` al lado** | lo mismo que viaja en cada bundle |

Y `ore --version` deja de decir solo un número:

```
ore 0.1.0 (a1b2c3…)
OOS: oos.dev/v1alpha1 · v1alpha2 · v1alpha3 · v1alpha4
```

El commit entra por **variable de entorno al compilar**, no por un `build.rs` que
invoque a git: así una compilación local dice honestamente `sin sellar` en vez de
estampar el hash de un árbol que puede estar sucio. Y las versiones de OOS **se
derivan** de `ApiVersion::ALL` (P2): una lista escrita a mano habría envejecido en
silencio la primera vez que el motor aprendiera una versión nueva.

## Las cuatro consecuencias que importan

**1 · Binarios crudos, no `.tar.gz`.** Lo que se atesta y lo que se descarga tienen que
ser la misma cadena de bytes. Un archivo comprimido interpone un artefacto cuyos bytes
dependen del `tar` y el `gzip` de cada runner —orden, `mtime`, propietario, nivel de
compresión—, y entonces el hash publicado ya no es el del binario sino el de una
envoltura. Un `chmod +x` en el instalador sale más barato que una fuente de
no-determinismo que nadie va a auditar.

**2 · Sin caché de compilación en el camino de publicación.** Una caché envenenada
produce un binario envenenado **que además sale firmado**, y la atestación lo avalaría.
`ci.yml` sí cachea: allí lo que está en juego es un minuto, no la procedencia.

**3 · Solo acciones de GitHub en `release.yml`.** Una acción de terceros en ese camino
habría que fijarla por SHA y auditarla en cada subida; no tenerla sale más barato. Es la
misma disciplina con la que se descartaron los validadores de JSON Schema y los colores
de `clap`, aplicada a la cadena de construcción en vez de al árbol de dependencias.

**4 · Runners nativos, no compilación cruzada.** Sin FFI, cruzar sería posible; pero el
enlazador cruzado es una variable más, y lo que este proyecto afirma es que **no arrastra
nada nativo**, no que sepa configurar cinco toolchains. Construir en la plataforma de
destino además comprueba que el binario **arranca** ahí, que es el fallo que si no
descubre el primer usuario.

## Lo que se acepta a cambio

- **El determinismo se comprueba dentro de una máquina, no entre máquinas.** Dos
  compilaciones en el mismo runner dan el mismo `sha256`; que un tercero reproduzca el
  binario bit a bit desde otro entorno **no está comprobado**, y por tanto no se afirma.
  Es la parte medible de la propiedad, y decir solo eso es lo que impide que se convierta
  en una promesa sin comprobador.
- **Cinco plataformas, no todas.** Falta `aarch64-pc-windows-msvc` y falta glibc. Un
  binario musl estático cubre cualquier Linux; el día que alguien pida lo otro, la matriz
  crece en una línea.
- **La caché fuera encarece cada release** en unos minutos de compilación. Es exactamente
  lo que se compra.
- **`npm`, `brew`, Docker y `curl | sh` siguen sin existir.** Los cuatro son envoltorios
  sobre una release de GitHub con binarios y checksums, que es lo que este flujo produce.
  Se construyen cuando haya un ciclo completo que instalar: un instalador que lleva al
  desarrollador hasta `ore source add` y ahí choca contra un muro es peor que no tenerlo.
