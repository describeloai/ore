# 0074 · Artifacts

**Estado:** **borrador · hoja de ruta** (2026-10-25). Se trabaja sobre este documento mientras se
construye; al cerrar, se rehace limpio con la imagen del producto. **Decide (propuesta):** que los
paquetes de software que un cliente trae de fuera —un jar de su Artifactory, una rueda de Python de
su equipo— son **activos del catálogo** (`kind: Artifact` en `base.schema.nombre`), que los
resolvers de las capas los sirven a los repositorios como si fueran Central o PyPI, y que a la
larga **todo lo que entra en un Build pasa por aquí**. Sigue a [`0037`](0037-language-servers-in-code-repositories.md)
③c (la capa de la JVM), [`0050`](0050-functions-in-code-repositories.md) (las capas de Python y Node),
[`0038`](0038-assets-catalog-namespaces.md) (`base.schema.nombre`), [`0041`](0041-model-registry.md)
(un registro que es el catálogo) y [`0047`](0047-ore-access-control.md) (quién puede).

## 1 · El problema

El cliente que escribe transforms en Java (y buena parte del de Python) lo hace por **su** código:
librerías internas, parsers de su sector, el motor de reglas de hace diez años. Viven en su
servidor privado (Artifactory, Nexus, GitHub Packages, CodeArtifact) y **no están en Central ni en
PyPI**. Hoy, si su `pom.xml` las pide, la capa falla: no las encuentra.

Conectar nuestros Jobs a su servidor no es la respuesta: pide red hacia su casa, credenciales suyas
dentro de nuestros Jobs y que su servidor esté de pie cada vez que corre un Build. Lo profesional
(Foundry lo hace con Artifact Repositories) es **un puente**: el cliente publica en nosotros desde
donde sí tiene acceso, y nuestros Builds sólo leen de nosotros.

## 2 · Cómo está hoy (medido en el código, 2026-10-25)

| | resuelve con | contra | quién sale a internet |
|---|---|---|---|
| **Java** | Maven (`puesto/jvm/capa/resolver.sh`) | Maven Central | el contenedor `resolver` del Job `55-la-capa-jvm`, y sólo él |
| **Python** | uv (`puesto/python/capa/resolver.sh`) | PyPI | el `resolver` del Job `52-la-capa` |
| **Node** | npm (`puesto/node/capa/resolver.sh`) | el registro de npm | el `resolver` del Job `57-la-capa-node` |

- **El puesto (sesión, Build, Preview) no tiene red.** Baja la capa ya resuelta del almacén al
  arrancar.
- **El resolver sí sale, y a propósito sin nada más**: sin el testigo y sin credencial de nube, para
  que lo que baja de internet no pueda publicar. Lo que resuelve queda con sumas y lock en el
  informe de la capa.
- Lo que la imagen ya trae (`provisto.txt`, `jars.txt`) entra como «provisto»; un choque de
  versiones se avisa.

## 3 · La idea

### 3.1 · Un `Artifact` es un activo del catálogo

- **Un activo = un paquete**, con todas sus versiones: `banco.riesgo.motor_riesgo`, de formato
  `maven`, coordenada `com.banco:motor-riesgo`. El nombre en el catálogo es de ORE; la coordenada es
  del ecosistema y va dentro del documento.
- **Lo hereda todo del catálogo**, como `Model` (0041): ruta única, dueño (0052), permisos (0047),
  búsqueda, linaje y página propia en Assets.
- **El documento vive en el árbol; los binarios, en el almacén.** El documento dice qué versiones
  hay, con su suma y estado. Los ficheros (`.jar`, `.pom`, `.whl`) van al almacén de objetos, por
  suma.
- **Diferencia con `Model`:** aquí **sí hay versiones numeradas y son inmutables**. Una `1.0.0`
  publicada no se reescribe nunca (lo exigen Maven y PyPI, y es lo que hace reproducible un Build).
  Se publica la `1.0.1`, o se retira la `1.0.0`.

### 3.2 · Dónde se ve

- **En Code Repositories, en su home: «+ New artifact» a la izquierda de «+ New repository».** Abre
  la creación (formato, ruta, coordenada) y, ya creado, la pestaña Publish con el token y el comando
  `mvn deploy:deploy-file` / `uv publish` listo para copiar.
- **Sin entrada propia en el sidebar de la plataforma.** Quien lo consume es un repositorio de
  código: su sitio natural es al lado. Por ser activo, además aparece en Assets y en la búsqueda.

### 3.3 · Cómo lo consumen los repositorios

- **Una dirección por celda y formato**, la que hablan las herramientas: `…/artifacts/maven/` (layout
  de Maven) y `…/artifacts/pypi/simple/` (PEP 503). La coordenada se traduce al activo; el permiso
  se mira por activo.
- **El resolver de cada capa la usa además de Central/PyPI**, con un token de lectura de quien
  construye. El cliente sólo escribe la coordenada en su `pom.xml` / `pyproject.toml`, como haría con
  Central.
- **⛔ Confusión de dependencias:** si una coordenada existe en Artifacts, **se toma sólo de
  Artifacts**, nunca de Central/PyPI. Si no, alguien publica `com.banco:motor-riesgo:9.9.9` en
  Central y nuestro resolver lo prefiere. Es el ataque clásico, y se cierra en el resolver.

### 3.4 · El espejo y cortar la salida (la meta)

Hoy el resolver sale a Central/PyPI/npm. **El espejo** es que Artifacts haga de proxy: el resolver
pide a Artifacts, y Artifacts baja de Central la primera vez, guarda el fichero con su suma y lo
sirve desde ahí siempre. Con eso:

- **El resolver deja de salir a internet**; sólo Artifacts sale, a una lista cerrada de orígenes.
- **Todo lo que entra en un Build queda registrado** (qué, cuándo, con qué suma) y puede
  bloquearse.
- Si Central cae o retira un paquete, los Builds siguen.

### 3.5 · Retirar y consumidores

- **Retirar (recall)** una versión: se marca en el documento como retirada, con motivo (p. ej. una
  vulnerabilidad). Los resolvers se niegan a usarla y el informe de la capa dice por qué; los Builds
  que la llevan fijada fallan con ese mensaje hasta que suban de versión. **No se borra**: lo
  construido con ella sigue siendo auditable.
- **Consumidores:** qué repositorios (y con ello qué transforms y datasets) usan cada versión. Sale
  del informe de la capa, que ya guarda qué resolvió cada repositorio, y se escribe como linaje del
  catálogo (`usa` / `usado_por`), igual que una `Function` con un `Model`. Responde a «¿a quién
  afecta retirar `motor-riesgo 3.2.0`?» antes de retirarla.

## 4 · Decisiones (propuestas, por cerrar)

| | propuesta | estado |
|---|---|---|
| **D-A1** | un `Artifact` es un activo del catálogo en `base.schema.nombre`; un activo = un paquete | propuesta |
| **D-A2** | versiones numeradas e inmutables; nunca se reescribe ni se borra una publicada, se retira | propuesta |
| **D-A3** | documento en el árbol, binarios en el almacén por suma | propuesta |
| **D-A4** | formatos: `maven` y `pypi` primero; `npm` después; ni Conda ni Docker | propuesta |
| **D-A5** | una dirección por celda y formato; permiso por activo (0047); publicar con token de alcance a un activo | propuesta |
| **D-A6** | una coordenada de Artifacts nunca se resuelve de fuera (confusión de dependencias) | propuesta |
| **D-A7** | en la consola, «+ New artifact» en el home de Code Repositories; sin entrada en el sidebar | propuesta |
| **D-A8** | el espejo: el resolver sólo habla con Artifacts; sólo Artifacts sale, a orígenes cerrados | propuesta |

## 5 · Hoja de ruta

| paso | qué | listo cuando |
|---|---|---|
| **AR0** | previo: el `kind: Artifact` en la spec (versión nueva de OOS), dónde va el binario, el token de publicar en 0047 | la spec compila un `Artifact` y el plan de rutas está escrito |
| **AR1** | Maven: crear, publicar (`deploy:deploy-file`), servir `…/artifacts/maven/`; la capa de la JVM lo usa con D-A6 | un transform Java en victor importa una clase de un jar publicado en Artifacts |
| **AR2** | PyPI: publicar (`uv publish`/`twine`), servir `…/pypi/simple/`; la capa de Python lo usa | un transform Python importa una rueda publicada |
| **AR3** | consola: «+ New artifact», creación, Publish con instrucciones, página del activo con versiones | se crea y publica desde la consola sin terminal salvo el comando de subir |
| **AR4** | espejo de Central y PyPI; el resolver sin salida a internet | los Jobs de las capas resuelven con la salida cerrada |
| **AR5** | retirar y consumidores (linaje desde el informe de la capa) | retirar una versión dice a quién afecta y los Builds lo explican |
| **AR6** | npm y la capa de Node | — |

## 6 · Preguntas abiertas

- **¿Activo = paquete o activo = repositorio** (varios paquetes, como el Artifact Repository de
  Foundry)? Propuesta: paquete, porque permisos, retirada y consumidores son por paquete; el schema
  ya agrupa.
- **¿Publicar desde un repositorio de código de ORE?** (una librería escrita en ORE que otros
  repositorios consumen). Encaja en el mismo activo; no entra en AR1–AR3.
- **¿Cuota de almacén por celda?**
- **¿Escaneo de vulnerabilidades** de lo publicado y de lo espejado? Natural tras AR4.
