## Verificar antes de ejecutar

```bash
gh attestation verify ore-<versión>-<plataforma> --repo describeloai/ore
sha256sum -c SHA256SUMS --ignore-missing
```

La atestación dice **de qué commit y de qué flujo** salió este binario, y está
firmada por GitHub. `ore --version` lo repite desde dentro: lleva su commit
sellado, porque un motor que promete que *el mismo commit produce el mismo
digest* tiene que poder decir de qué commit viene.

Antes de publicarse, ese commit pasó los 90 casos de la suite de conformidad de
OOS, y su binario se construyó **dos veces** para comprobar que las dos
compilaciones dan el mismo `sha256`.

## Qué hay dentro

Un binario estático de unos 4 MB, sin dependencias nativas y **sin ficheros que
leer en ejecución**: los esquemas de OOS van compilados. No hay servidor, ni
base de datos, ni índice que levantar.

```bash
chmod +x ore-<versión>-<plataforma>
mv ore-<versión>-<plataforma> /usr/local/bin/ore
ore --version
```

## Qué existe y qué no

`ore --help` anuncia más comandos de los que implementa. Los que existen lo
dicen al ejecutarse;
el resto falla explicando en qué fase está. Consulta el `README` del repositorio
para el estado por fases: es la única copia de ese marcador, a propósito.

**Y esto publica `ore`, no el ejecutor.** El nivel L2 —autorizar, planificar,
federar— vive en `ore-exec` y en los lectores, que son binarios aparte porque
enlazan cosas que el compilador no puede enlazar: un evaluador de políticas trae
un reloj, y un driver trae una pila TLS. Se construyen desde el repositorio y
todavía no se distribuyen.

Que estén fuera no es un reparto de directorios: `ore` sigue sin arrastrar nada
nativo, y hay una prueba que lee `Cargo.lock` y falla si alguna de esas crates
aparece en su cierre.
