# `iam` — el plano de identidad y acceso

> **Lo que aquí se guarda no tiene otra casa.** Todo lo demás la tiene.

## Qué hay, y qué NO

```
el árbol ontológico          git, en la forja
quién eres y tu contraseña   Keycloak
el artefacto sellado         Artifact Registry
quién cambió la ontología    el commit de la forja
─────────────────────────────────────────────────
lo que no tiene casa:        esto
```

Y de ahí la regla, que es comprobable y por eso se escribe antes que el DDL:

> ### ⛔ Aquí no entra ni un dato del cliente ni un documento de la ontología. Personas y permisos. Nada más.

El día que alguien quiera meter un catálogo aquí, esa frase es la que lo para.

## Por qué se llama `iam`

Porque es como lo llaman los cuatro, y como ya lo llamaba la plataforma. De su
propia investigación —`docs/iam/01-las-plataformas.md`, comprobada el
2026-08-25— sale la frase que ordena todo esto:

> *«La identidad no es una funcionalidad de un producto: es **un plano con su
> propia consola, su propio ciclo de vida y sus propios estándares**, y en las
> cuatro plataformas vive **por encima** del recurso que protege.»*

| | cómo lo llaman |
|---|---|
| Databricks | la **cuenta** — y los grupos son de cuenta, no del espacio de trabajo |
| AWS | **IAM Identity Center**, un servicio aparte |
| Snowflake | la **cuenta** → roles (`ORGADMIN`, `SECURITYADMIN`) |
| Palantir | **Control Panel** / Multipass, sobre *Organizations* |

Y la plataforma ya tenía `docs/iam/` con cinco documentos y un `rubixiam.md`
vigente. El esquema y la documentación se llaman igual a propósito.

## Las cinco ideas que se toman de `modelo/`, con su procedencia

De las 22 migraciones de la plataforma se toman **ideas, no DDL** — copiar el
esquema renombrando se lleva sus decisiones sin sus motivos, y sus motivos
contestan *sus* preguntas (celdas, outbox, facetas).

| idea | de dónde |
|---|---|
| la correspondencia `(emisor, sub) → sujeto opaco`: el sujeto **no es** el `sub` del IdP, está atado a él | `014-sujeto.sql` |
| la pertenencia sale de la **invitación**, no del IdP: así el plano de control no necesita una credencial de administración del emisor | `021-la-invitacion.sql` |
| cada acto privilegiado deja **huella**, incluido *mirar* — *«un acto privilegiado sin rastro no es un control»* | `020`, `022` |
| el rol es sobre un **recurso**, no un rol del realm | `005`, `006`, `019` |
| **una migración aplicada es inmutable** — *«lo que corrió y lo que dice el fichero dejan de ser lo mismo»* | `006-dueno.sql` |

Y una que **no** se toma: su `celda` / `ambito_de`. Su ámbito es
`origen \| contenedor \| dataset`, que es la forma de una AMP. El nuestro es el
paquete y la vista, y ésos ya viven en la ontología.

## ⭐ Los dos planos, y sus dos vocabularios

`011` y `012` parten en dos lo que era una sola tabla de roles:

| | `iam.rol` · la organización | `iam.rol_de_recurso` · el árbol |
|---|---|---|
| sobre qué | personas y permisos | un ámbito de la ontología |
| quién manda | `ORGADMIN` — **UNO**, se traspasa | `owner` — **muchos**, se nombra |
| qué da | invitar, conceder, traspasar | la **firma** de la certificación |
| ¿ordenado? | ⛔ **no** desde la `014`: son CONJUNTOS de potestades | ⛔ **no**, y a propósito |

⭐ Y arriba tampoco hay escalera. `014` cambió los roles de peldaños a
**conjuntos de potestades**, y con eso la guarda del rodeo dejó de ser una resta:

> **Puedes otorgar un rol si sus potestades están contenidas en las tuyas.**

Un ordinal sólo sabe decir «más» y «menos», y hay una separación que importa y no
es de altura — *¿es el que corta más o menos que el que da de alta?*. La pregunta
no tiene respuesta y un ordinal obliga a inventarla.

Los cuatro: `ORGADMIN` (uno, traspasa), `ACCOUNTADMIN` (todo, y el único que
concede roles), `USERADMIN` (quién está) y `SECURITYADMIN` — que hoy **es una
carcasa y lo dice en su propia fila**: de sus potestades, dos son de Keycloak,
una necesita agentes que no existen y `actividad:leer-toda` no tiene ruta. Entra
igual porque la separación que representa —cortar sin poder nombrar— es una
decisión que no queremos redescubrir a las 3 de la mañana.

⚠️ Y **pertenecer no es un rol**: `pertenencia.rol` es nulable y `null` significa
*pertenece y nada más*. Es su frase — *«pertenecer ya da lectura; leer no es un
rol»*— y con ella se fueron `lector` y `miembro`, que no aportaban ni una
potestad de este plano.

⛔ La tabla de abajo **no tiene `ordinal`**. `owner` no implica `lector`, y
escribirlo sería herencia de roles: *«la travesía deja de ser un `JOIN` sobre un
árbol y empieza a ser un motor de políticas»*. Un ordinal ahí sería esa regla
implícita escrita en una columna.

⚠️ Y falta la guarda de verdad de `conceder`: *para nombrar owner de un ámbito
hay que ser owner de ese ámbito, o de uno que lo contenga*. Es una travesía de
`paquete → vista`, que vive en la forja. Hasta que exista, **`conceder` niega
`owner`**.

## La frontera con el gobierno del flujo

ORE ya gobierna **qué puede fluir hasta dónde** — retículo, conductos,
`OOS4xxx`. Esto gobierna **quién alcanza qué superficie**. No son lo mismo, y el
orden entre ellos está decidido:

> ### La concesión puede NEGAR. No puede conceder por encima del conducto.

Un `grant` que ensanchara lo que el retículo cerró convertiría el gobierno del
flujo en una sugerencia.

## Correr las migraciones

```bash
bash iam/migrar.sh                      # contra $PGHOST/$PGDATABASE
kubectl apply -f malla/65-iam.yaml      # y en el clúster, como Job
```
