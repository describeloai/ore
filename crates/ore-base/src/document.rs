//! **Las versiones de la especificación y su vocabulario de medios** (CI C4: vocabulario):
//! `ApiVersion`, qué medios admite un `ObjectTable` o una `MediaCollection` en cada versión y la
//! columna rescatada. ⚠️ Una versión nueva de OOS es un cambio de vocabulario: recompila todo lo
//! que usa `ore-base`. La forma de los documentos vive en `ore_core::document`, que re-exporta esto.

pub const API_VERSION: &str = "oos.dev/v1alpha1";

/// Las versiones de la especificación que esta implementación entiende.
///
/// Deja de ser una constante en el momento en que hay dos, y el cambio no es
/// cosmético: **el documento elige su conjunto de reglas**. Un `kind` que no
/// existía en v1alpha1 no es un error de escritura, es un documento de otra
/// versión, y decirlo así es la diferencia entre «no reconozco esto» y «esto
/// existe, en otra versión».
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApiVersion {
    V1Alpha1,
    V1Alpha2,
    V1Alpha3,
    V1Alpha4,
    /// v1alpha5 y v1alpha6 no estan, y no faltan: no anadieron gramatica, asi
    /// que ningun documento puede declararlas. **Un `apiVersion` es
    /// consecuencia de haber anadido un `kind`**, igual que un directorio de
    /// esquemas. v1alpha7 anade uno —la vista— y por eso existe aqui.
    V1Alpha7,
    /// v1alpha8. Anade `Table`, adelgaza `View` y **retira** `Binding`.
    ///
    /// Es la primera version que quita algo, y por eso `Kind` necesita `hasta`
    /// al lado de `since`: un `Binding` que declare esta version no es un kind
    /// desconocido ni uno del futuro — es uno del pasado, y el error tiene que
    /// decir eso y decir por que dos documentos.
    V1Alpha8,
    /// v1alpha9. Anade `Model` —el modelo como documento del arbol, que nombra
    /// un perfil certificado y un tier (ORE 0027)— y le da a `Function` un
    /// segundo `runtime`: `model`, con `model` y `prompt` como claves propias.
    /// Lo midio la E0 de 0027: la gramatica de v1alpha8 admitia
    /// `runtime: model` sin comprobarlo y el prompt solo cabia en `x-ore-…`.
    V1Alpha9,
    /// v1alpha10. **Actuar.** Cambia la naturaleza de `Function` —logica
    /// encapsulada que lee, edita o infiere sobre la copia, con su superficie
    /// en los dos sentidos: `over` y `reads` son vistas, `effects` es
    /// opcional— y anade `Action`, la invocacion como documento, sin codigo.
    /// Lo midio `pruebas-de-fuego/medida-forge-function.py` (2026-09-17): una
    /// funcion no podia entrar sola, la copia ya no se decide vista a vista, y
    /// lo que una funcion leia no se declaraba.
    V1Alpha10,
    /// v1alpha11. **Publicar.** Anade `TrainedModel`: el modelo entrenado
    /// como asset del registro —un documento de un paquete que nombra sus
    /// ficheros del lago por prefijo y digest, con que se cargan, que version
    /// son y de que vistas salieron—. Distinto del `Model` de v1alpha9 (un
    /// perfil servido, sin namespace, en la raiz): son dos cosas que se usan
    /// de dos maneras. Lo midio `pruebas-de-fuego/medida-w3-declarar.py`
    /// (2026-09-21, ORE 0031 W3.7): un modelo con pesos era `OOS1005`.
    V1Alpha11,
    /// v1alpha12. **Tener.** Anade `Dataset`: lo que un inquilino tiene
    /// —bytes en su lago, con historia— como UN documento de su paquete, se
    /// llene como se llene: el sistema cumpliendo un plan (`from`) o codigo
    /// escribiendolo (`columns` + `changes`). Retira `View.materialized` y
    /// `View.freshness` (eran de la copia, y la copia es el dataset) y abre
    /// `View.from: {dataset}` y `Entity.backedBy` sobre un dataset. Lo
    /// decidio ORE 0033 (2026-09-21): la copia era una `View` con una nota al
    /// pie y lo escrito una `Table` que apuntaba a lo nuestro; la costura del
    /// gobierno cuelga del plan, no del nombre del documento.
    V1Alpha12,
    /// v1alpha13. **Ordenar.** Anade `Schema` —el segundo nivel del nombre de
    /// lo que un paquete tiene— y `metadata.schema` en el contenido del
    /// catalogo: su nombre pasa a `<paquete>.<schema>.<nombre>`, unico por
    /// schema, como en Unity Catalog. Lo decidio ORE 0038 (2026-09-24): dos
    /// `pedidos` en dos schemas del mismo paquete eran `OOS2035`, y un
    /// catalogo que los pintaba en dos schemas pintaba algo que el lenguaje no
    /// tenia. El schema se DECLARA y la carpeta se ata a el (`OOS2036`,
    /// `OOS2037`): la identidad nunca es la ruta.
    V1Alpha13,
    /// v1alpha14. **Escribir.** La `View` es SQL: `spec.sql` en su `dialect`,
    /// con el contrato que se deriva de la consulta en `spec.columns`, y lo
    /// que se gobierna de ella —lo que lee, el linaje, el canal lateral— se
    /// deriva de lo escrito. Lo decidio ORE 0040 (2026-09-25): de 24 consultas
    /// de un `CREATE VIEW`, 6 cabian en la forma estructurada, y lo que la forma
    /// aseguraba se deriva de la consulta.
    V1Alpha14,
    /// v1alpha15. **Situar.** El `Model` deja de ser vocabulario del arbol
    /// —sin `namespace`, en `modelos/` en la raiz— y pasa a contenido
    /// gobernado: `metadata.namespace` y `metadata.schema`, unico por schema
    /// (ORE 0041). Lo midio el catalogo: el indice daba el modelo con
    /// `paquete: null` y la consola no tenia donde pintarlo, ni el modelo
    /// dueño.
    V1Alpha15,
    /// v1alpha16. **Guardar.** Un fichero es un objeto, no una fila: se apunta
    /// como objeto (`ObjectTable`, en la fuente), se tiene en una coleccion de
    /// su tipo (`MediaCollection`, en una base) y una entidad lo referencia sin
    /// copiarlo (`Media<coleccion>`). Lo que si son filas —Parquet, CSV,
    /// JSONL— es una `Table` con `format`. Lo decidio ORE 0046 (2026-09-28),
    /// medido contra un bucket real: en el mismo prefijo conviven CSV, PDF,
    /// fotos y un zip, y lo que se hace con cada uno no se parece.
    V1Alpha16,
    /// v1alpha17. **Anclar.** Lo que se saca de un fichero es una fila anclada
    /// a una parte de el: los tipos compuestos (`Struct`, `Vector`), el ancla
    /// (`Anchor`), la referencia entera como valor de `Media<c>`, la tabla
    /// anclada (`anchoredTo`) y la coleccion leida como su listado. Lo decidio
    /// ORE 0049 (2026-09-30): sin tipos, todo resultado sobre un medio acaba
    /// como JSON en una cadena.
    V1Alpha17,
    /// v1alpha18. **Promover.** El codigo de un repositorio pasa a ser una
    /// funcion cuando un documento lo nombra: `runtime: python`, un
    /// `entrypoint` que nombra un `def` del paquete cuya cabecera es la firma,
    /// y `models`, los modelos que el codigo puede llamar. Lo decidio ORE 0050
    /// (2026-10-01), sobre la promocion que 0031 W3.8 dejo escrita.
    V1Alpha18,
    /// v1alpha19. **Derivar ficheros.** Una `MediaCollection` escrita dice lo
    /// que el código leyó para escribirla —`derivedFrom`, como el dataset
    /// escrito— y por ahí le baja la clasificación. Lo decidió ORE 0049 B4·1
    /// (2026-10-01): sin esto, una colección generada a partir de algo `high`
    /// llevaba sólo las etiquetas que alguien se acordara de ponerle.
    V1Alpha19,
    /// v1alpha20. **La firma habla OOS.** Un `@function` deriva también a
    /// `Time`, `Opaque`, `DateTimeTz`, `Decimal<p, s>`, `Money`, `Quantity`,
    /// `Struct`, `list<Struct>` y `Media`: la tabla de v1alpha18 §4.6, ampliada.
    /// No cambia ningún `kind`. Lo decidió ORE 0050 G5a (2026-10-02): lo que un
    /// consumidor ve de una función —un pipeline, el formulario de Dry Run—
    /// habla OOS, y la moneda de una columna se perdía en la frontera.
    V1Alpha20,
    /// v1alpha21. **El dueño.** `Entity`, `Function`, `ObjectTable` y `Model`
    /// dicen quién responde de ellos —`spec.owner`, opcional, el handle de
    /// siempre (`OOS2009`)—, como ya lo decían el paquete, el schema, la vista,
    /// el dataset, la colección y el modelo entrenado. No da acceso y no se
    /// hereda; en una función de código no es parte de la firma. Lo decidió ORE
    /// 0052 · Ownership (2026-10-02): lo que se crea es de quien lo crea.
    V1Alpha21,
    /// v1alpha22. **Nunca nula.** La columna de una `Table` dice que el origen
    /// garantiza que nunca es nula —`columns.<c>.required`, opcional, un
    /// booleano—. Sólo una garantía del origen, nunca lo visto en una muestra;
    /// una `View` y un `Dataset` no lo declaran: se deriva. Lo pidió ORE 0051
    /// «ORE Null Contract» (2026-10-02).
    V1Alpha22,
    /// v1alpha23. **La función de TypeScript.** `runtime: node`: la
    /// exportación por defecto de un `.ts` con `functions` en su ruta, que se
    /// llama como el fichero, con su `export const config` al lado; su
    /// documento se deriva leyendo, como el de un `@function` de Python. Lo
    /// pidió ORE 0050 R3 (2026-10-03).
    V1Alpha23,
    /// v1alpha24. **Leer el origen.** El conducto `federation.read`: una
    /// lectura en vivo de una `Table` —y una vista de esta versión que la lee—
    /// lo atraviesa; `fullScan: forbidden` y `requiredFilters` se aplican al
    /// planificar (`OOS2044`, `OOS2045`) y `expensive` se lee con presupuesto.
    /// Sin claves nuevas. Lo pidió ORE 0053 «ORE Federation Engine» (2026-10-04).
    V1Alpha24,
    /// v1alpha25. **El transform.** `kind: Transform`, el productor declarado
    /// de un dataset escrito: se deriva de un `@transform` de Python o de una
    /// sentencia SQL que escribe, y se coteja con el código (`OOS2013`,
    /// `OOS2042`, `OOS2043`). Su identidad es su salida —un productor por
    /// salida (`OOS2047`), que es un `Dataset` o una `MediaCollection` escritos
    /// (`OOS2046`) o un nombre por nacer— y la etiqueta baja por él antes de la
    /// primera ejecución. Lo pidió ORE 0055 «transforms in code repositories»
    /// (2026-10-05).
    V1Alpha25,
    /// v1alpha26. **La función propia.** Una `Function` vive fuera de los
    /// paquetes —`functions/<nombre>.yaml` en la raíz—, se llama
    /// `functions.<nombre>`, lleva su versión (`metadata.version`, calculada) y
    /// la huella de lo que corre (`spec.codeDigest`). Lo pidió ORE 0056
    /// «Versioning» (2026-10-05). v1alpha25 (`Transform`) es de otra entrega.
    V1Alpha26,
    /// v1alpha27. **La base foránea.** Un `Package` con `spec.foreign`
    /// expone, con su nombre, las `Table` y los `ObjectTable` de una fuente
    /// —el mismo documento, dos nombres—; sólo tiene `Schema` y vistas sin
    /// copia (`OOS2049`). Y `datasources[].federation` del `OntologyConfig`:
    /// sin él, leer en vivo es `OOS2051`. Lo pidió ORE 0057 (2026-10-06).
    V1Alpha27,
    /// v1alpha28. **La visibilidad dentro de un árbol.** Todo árbol es un
    /// catálogo —sea cual sea la versión de su config: la regla sólo quita
    /// errores—: sus bases se leen por su nombre (`OOS2028` no se aplica entre
    /// miembros) y `exports` queda como la frontera del artefacto; la base
    /// foránea expone sin pedir `exports` a su fuente. Quién lee qué es del
    /// acceso, al servir. Lo pidió ORE 0059 (2026-10-07), como Unity Catalog.
    V1Alpha28,
    /// v1alpha29. **Las familias de medios.** `media` pasa de 8 palabras a 15
    /// —`structured`, `text`, `geospatial`, `model3d`, `medical`,
    /// `scientific`, `design`—; un `archive` puede ser una colección, y una
    /// colección puede ser `multimodal` (cualquier familia, `formats`
    /// opcional). La puerta es del documento. Lo pidió ORE 0061 AZ5
    /// (2026-10-12): una base estándar sobre los `.gz` de `mnist` nacía vacía.
    V1Alpha29,
    /// v1alpha30. **La salud de un activo.** Obligaciones de quien sirve, sin
    /// clave ni código de documento: un documento que la declara compila
    /// como uno de v1alpha29. Lo pidió ORE 0063 (2026-10-13).
    V1Alpha30,
    /// v1alpha31. **Las promesas de un activo.** `spec.promises` en un
    /// `Dataset` y en una `MediaCollection`: la aserción `library` del
    /// `Ruleset` con columna, argumentos y una biblioteca por familia
    /// (`promesas.rs`). La puerta es del documento. Lo pidió ORE 0063 S7
    /// (2026-10-14).
    V1Alpha31,
    /// v1alpha32. **Los tipos geo.** `Geometry<subtipo, crs>` y
    /// `Geography<subtipo>` como tipos de columna y de propiedad: WKB en orden
    /// x/y, el CRS fiel al origen, y en Parquet el tipo lógico y la clave
    /// GeoParquet a la vez (`01-la-geometria`). Lo pidió ORE 0067 G1
    /// (2026-10-16).
    V1Alpha32,
    /// v1alpha33. **El enriquecimiento.** `runtime: enrich` en un `Transform`:
    /// su código es un `.enrich.yaml` —el objetivo, las capas, la relación y lo
    /// que se toma, con su reparto— (`01-enrich`, `enriquecer.rs`). La puerta
    /// es del documento. Lo pidió ORE 0068 C4 (2026-10-20).
    V1Alpha33,
}

impl ApiVersion {
    pub const ALL: &'static [ApiVersion] = &[
        ApiVersion::V1Alpha1,
        ApiVersion::V1Alpha2,
        ApiVersion::V1Alpha3,
        ApiVersion::V1Alpha4,
        ApiVersion::V1Alpha7,
        ApiVersion::V1Alpha8,
        ApiVersion::V1Alpha9,
        ApiVersion::V1Alpha10,
        ApiVersion::V1Alpha11,
        ApiVersion::V1Alpha12,
        ApiVersion::V1Alpha13,
        ApiVersion::V1Alpha14,
        ApiVersion::V1Alpha15,
        ApiVersion::V1Alpha16,
        ApiVersion::V1Alpha17,
        ApiVersion::V1Alpha18,
        ApiVersion::V1Alpha19,
        ApiVersion::V1Alpha20,
        ApiVersion::V1Alpha21,
        ApiVersion::V1Alpha22,
        ApiVersion::V1Alpha23,
        ApiVersion::V1Alpha24,
        ApiVersion::V1Alpha25,
        ApiVersion::V1Alpha26,
        ApiVersion::V1Alpha27,
        ApiVersion::V1Alpha28,
        ApiVersion::V1Alpha29,
        ApiVersion::V1Alpha30,
        ApiVersion::V1Alpha31,
        ApiVersion::V1Alpha32,
        ApiVersion::V1Alpha33,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ApiVersion::V1Alpha1 => "oos.dev/v1alpha1",
            ApiVersion::V1Alpha2 => "oos.dev/v1alpha2",
            ApiVersion::V1Alpha3 => "oos.dev/v1alpha3",
            ApiVersion::V1Alpha4 => "oos.dev/v1alpha4",
            ApiVersion::V1Alpha7 => "oos.dev/v1alpha7",
            ApiVersion::V1Alpha8 => "oos.dev/v1alpha8",
            ApiVersion::V1Alpha9 => "oos.dev/v1alpha9",
            ApiVersion::V1Alpha10 => "oos.dev/v1alpha10",
            ApiVersion::V1Alpha11 => "oos.dev/v1alpha11",
            ApiVersion::V1Alpha12 => "oos.dev/v1alpha12",
            ApiVersion::V1Alpha13 => "oos.dev/v1alpha13",
            ApiVersion::V1Alpha14 => "oos.dev/v1alpha14",
            ApiVersion::V1Alpha15 => "oos.dev/v1alpha15",
            ApiVersion::V1Alpha16 => "oos.dev/v1alpha16",
            ApiVersion::V1Alpha17 => "oos.dev/v1alpha17",
            ApiVersion::V1Alpha18 => "oos.dev/v1alpha18",
            ApiVersion::V1Alpha19 => "oos.dev/v1alpha19",
            ApiVersion::V1Alpha20 => "oos.dev/v1alpha20",
            ApiVersion::V1Alpha21 => "oos.dev/v1alpha21",
            ApiVersion::V1Alpha22 => "oos.dev/v1alpha22",
            ApiVersion::V1Alpha23 => "oos.dev/v1alpha23",
            ApiVersion::V1Alpha24 => "oos.dev/v1alpha24",
            ApiVersion::V1Alpha25 => "oos.dev/v1alpha25",
            ApiVersion::V1Alpha26 => "oos.dev/v1alpha26",
            ApiVersion::V1Alpha27 => "oos.dev/v1alpha27",
            ApiVersion::V1Alpha28 => "oos.dev/v1alpha28",
            ApiVersion::V1Alpha29 => "oos.dev/v1alpha29",
            ApiVersion::V1Alpha30 => "oos.dev/v1alpha30",
            ApiVersion::V1Alpha31 => "oos.dev/v1alpha31",
            ApiVersion::V1Alpha32 => "oos.dev/v1alpha32",
            ApiVersion::V1Alpha33 => "oos.dev/v1alpha33",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == s)
    }
}

/// v1alpha16. El tipo de medio de un `ObjectTable` (`01-object-table` §4). Una
/// coleccion admite todos menos `archive` y `binary`: sabe lo que guarda.
pub const MEDIOS: &[&str] = &[
    "document",
    "image",
    "audio",
    "video",
    "spreadsheet",
    "email",
    "archive",
    "binary",
];

/// v1alpha29. Las familias que se suman (`01-las-familias` §1).
pub const FAMILIAS_V1ALPHA29: &[&str] = &[
    "structured",
    "text",
    "geospatial",
    "model3d",
    "medical",
    "scientific",
    "design",
];

/// v1alpha29. La colección de lo que no es de una sola familia (`01` §3):
/// sólo en una `MediaCollection`, y sale de un `ObjectTable` de cualquiera.
pub const MULTIMODAL: &str = "multimodal";

/// El vocabulario de `media` de un `ObjectTable` en su versión.
pub fn medios_de_objetos(v: ApiVersion) -> Vec<&'static str> {
    let mut m = MEDIOS.to_vec();
    if v >= ApiVersion::V1Alpha29 {
        m.extend_from_slice(FAMILIAS_V1ALPHA29);
    }
    m
}

/// El vocabulario de `media` de una `MediaCollection` en su versión: nunca
/// `binary`; `archive` y `multimodal` desde v1alpha29.
pub fn medios_de_coleccion(v: ApiVersion) -> Vec<&'static str> {
    let mut m: Vec<&'static str> = medios_de_objetos(v)
        .into_iter()
        .filter(|x| *x != "binary" && (*x != "archive" || v >= ApiVersion::V1Alpha29))
        .collect();
    if v >= ApiVersion::V1Alpha29 {
        m.push(MULTIMODAL);
    }
    m
}

/// v1alpha16 · `03` §1.1. **La columna rescatada**: donde una tabla de CSV o
/// JSONL que la declara recibe lo que no encaja con sus tipos congelados.
pub const COLUMNA_RESCATADA: &str = "_rescued_data";
