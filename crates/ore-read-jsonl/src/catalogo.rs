//! **El catálogo de un directorio de ficheros.**
//!
//! Era el hueco que dejaba a esta familia saliendo y no entrando: `leer` estaba
//! y `catalogo` contestaba *«no lo sé hacer todavía»*. Lo que faltaba no era
//! código: era **una decisión**, porque un NDJSON no tiene esquema y hay dos
//! formas de sacarle uno y una es mentir.
//!
//! # La decisión: se emite lo que es un hecho, se avisa de lo que no
//!
//! Es la regla que el inductor ya usa —*«se emite lo que es un hecho, se reporta
//! lo que es una conjetura»*— aplicada un piso más abajo, y aquí decide tres
//! cosas:
//!
//! **Se lee el fichero ENTERO, no una muestra.** Una muestra de mil líneas es
//! una conjetura sobre el resto; leerlo entero es un hecho sobre este fichero.
//! Y no cuesta nada que no se pague ya: `leer` lo recorre igual.
//!
//! **El tipo sale de cómo estaba escrito el valor, y solo si no hay dudas.**
//! `"7"` es una cadena y `7` es un número, y eso lo sabe el analizador porque
//! [`Style`] lo conserva. Si una columna trae números en mil líneas y una cadena
//! en una, el tipo **no es `Integer`**: son dos cosas, y lo honesto es decir las
//! dos y no elegir. Sale como `sourceType`, que aguas abajo **se cita y no se
//! interpreta** — el mismo trato que la receta de BigQuery le da a un `STRUCT`.
//!
//! **No se propone clave primaria.** Que `id` sea único en este fichero es un
//! hecho de este fichero y una conjetura sobre la tabla. El inductor ya sabe qué
//! hacer con una entidad sin clave: fallar con `OOS2010`, que es lo correcto —
//! *«inventar la clave sería lo único peor»*.
//!
//! # Las dos caras de un fichero
//!
//! `reads: { predicatePushdown: [eq], fullScan: cheap }` — es lo que este driver
//! sabe empujar sobre un fichero y lo que cuesta recorrerlo, que es poco.
//!
//! `changes: { mode: none, witness: snapshot }`. Un fichero **no tiene
//! changelog**, así que no emite cambios; y sin embargo **sabe fecharse**: el
//! digest de su contenido nombra exactamente esta versión de él, que es lo que
//! `snapshot` significa. Declarar `witness: none` tiraría eso, y además haría
//! que `materialize` avisara en cada copia de que el origen contesta más de lo
//! que la tabla declara.

use std::collections::BTreeMap;

use ore_core::json::Json;
use ore_core::parse::{Node, Style};
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};

/// Las extensiones que se toman por NDJSON. Se dice cuál se mira en vez de
/// abrir todo y adivinar: un `.json` suele ser **un** documento y no una línea
/// por fila, y tratarlo como NDJSON daría una tabla de una fila con las claves
/// del documento entero.
pub const EXTENSIONES: &[&str] = &["ndjson", "jsonl"];

/// **El fichero de un objeto**, y por qué el objeto no lleva la extensión.
///
/// El catálogo nombra `pedidos`, no `pedidos.ndjson`, y eso salió de mirar lo
/// que pasaba al inducir: el nombre del catálogo es `<contenedor>.<objeto>` en
/// las otras dos familias, así que **el punto es un separador**. Con la
/// extensión dentro, `clientes.ndjson` y `pedidos.ndjson` daban los dos la
/// entidad `Ndjson` y el inductor paraba con una colisión — correctamente, y
/// por un nombre que no era el del objeto sino el del formato.
///
/// Así que la extensión es de la familia, no del objeto. Aquí se resuelve, y
/// **sin adivinar**: se prueba el nombre tal cual —para que alguien pueda
/// nombrar un fichero exacto— y luego con cada extensión conocida. Si encajaran
/// dos, se dice cuáles en vez de elegir.
pub fn fichero(url: &str, objeto: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::path::Path::new(url);
    let tal_cual = dir.join(objeto);
    if tal_cual.is_file() {
        return Ok(tal_cual);
    }
    let encajan: Vec<std::path::PathBuf> = EXTENSIONES
        .iter()
        .map(|e| dir.join(format!("{objeto}.{e}")))
        .filter(|p| p.is_file())
        .collect();
    match encajan.as_slice() {
        [uno] => Ok(uno.clone()),
        [] => Err(format!(
            "`{objeto}` no está en `{url}`: no hay ni un fichero con ese nombre ni uno con {}",
            EXTENSIONES
                .iter()
                .map(|e| format!("`.{e}`"))
                .collect::<Vec<_>>()
                .join(" o ")
        )),
        varios => Err(format!(
            "`{objeto}` encaja con {} ficheros a la vez: {}. Elegir uno decidiría cuál de los dos \
             es la tabla, así que no se elige",
            varios.len(),
            varios
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// **Qué contiene esta fuente.**
///
/// Los ficheros del directorio, y una nota que es la mitad de la respuesta:
/// aquí **no hay nada que seleccionar**. Una fuente de esta familia es el
/// directorio entero y su catálogo trae todos sus ficheros, así que la
/// pregunta que `explorar` contesta —*¿cuál de los contenedores declaro?*—
/// solo existe donde una URL nombra uno, que es el caso de BigQuery.
///
/// Se contesta igualmente en vez de negarse: *«no hay nada que elegir»* es una
/// respuesta, y un verbo que se niega en dos familias de tres deja de poder
/// usarse sin saber de antemano cuál es cuál.
pub fn explorar(url: &str) -> Result<String, String> {
    let mut avisos = Vec::new();
    let catalogo = de_directorio("explorar", url, &mut avisos)?;
    let arbol =
        ore_core::parse::parse(&catalogo).map_err(|e| format!("el catálogo no analiza: {e:?}"))?;
    let contiene: Vec<Json> = arbol
        .get("tables")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| t.get("name").and_then(|(_, n)| n.as_str()))
        .map(|n| Json::obj([("nombre", Json::s(n))]))
        .collect();
    Ok(Json::obj([
        ("contiene", Json::Arr(contiene)),
        (
            "nota",
            Json::s(concat!(
                "una fuente de esta familia es el directorio entero y su catálogo ",
                "trae todos estos ficheros: no hay que elegir ninguno"
            )),
        ),
    ])
    .pretty())
}

/// Lo que se observó de una columna a lo largo del fichero.
#[derive(Default)]
struct Observado {
    /// Las clases de valor vistas, sin contar los nulos. Ordenado y sin
    /// repetir: es lo que se emite cuando hay más de una.
    clases: Vec<&'static str>,
    /// En cuántas líneas apareció con un valor que no era nulo.
    con_valor: usize,
}

/// La clase de un valor, **de cómo estaba escrito**.
///
/// `None` es un nulo, que no es una clase: es la ausencia de una.
fn clase(n: &Node) -> Option<&'static str> {
    match n {
        Node::Mapping { .. } => Some("object"),
        Node::Sequence { .. } => Some("array"),
        // Entrecomillado es una cadena sin ambigüedad, siempre.
        Node::Scalar {
            style: Style::Quoted | Style::Block,
            ..
        } => Some("String"),
        Node::Scalar { raw, .. } => match raw.as_str() {
            "" | "~" | "null" | "Null" | "NULL" => None,
            "true" | "false" => Some("Boolean"),
            v if entero(v) => Some("Integer"),
            v if decimal(v) => Some("Decimal"),
            // Un escalar sin comillas que no es número ni booleano no puede
            // salir de JSON válido. Si llega, es texto.
            _ => Some("String"),
        },
    }
}

fn entero(v: &str) -> bool {
    let s = v.strip_prefix('-').unwrap_or(v);
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// **`Decimal` y no `Float`.** Un número con parte fraccionaria llega como
/// texto y así se queda: el árbol no tiene coma flotante en ningún sitio, y
/// `68400.50` no tiene representación exacta en binario.
fn decimal(v: &str) -> bool {
    let s = v.strip_prefix('-').unwrap_or(v);
    match s.split_once('.') {
        Some((a, b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes().all(|c| c.is_ascii_digit())
                && b.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Los escalares que OOS sabe nombrar. `object` y `array` no están, y por eso
/// salen como `sourceType`: tienen estructura que el origen acaba de enumerar,
/// y traducirlos a `Opaque` tiraría ese hecho.
fn es_escalar_oos(c: &str) -> bool {
    matches!(c, "String" | "Integer" | "Decimal" | "Boolean")
}

/// El catálogo de un directorio. `avisos` recoge lo que hay que decir por
/// stderr, que es donde va lo que no es el catálogo.
pub fn de_directorio(fuente: &str, ruta: &str, avisos: &mut Vec<String>) -> Result<String, String> {
    let dir = std::path::Path::new(ruta);
    let mut entradas: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("no se pudo leer el directorio `{ruta}`: {e}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| EXTENSIONES.contains(&x))
        })
        .collect();
    // El orden del sistema de ficheros no es estable entre máquinas, y el
    // catálogo alimenta un digest. Se ordena aquí: es lo único de este fichero
    // que no es un hecho del origen, y por eso se dice.
    entradas.sort();

    let mut tablas: Vec<Tabla> = Vec::new();
    let mut troncos: std::collections::BTreeSet<String> = Default::default();
    for f in &entradas {
        // El nombre del OBJETO es el tronco, sin extension: el punto es un
        // separador en el contrato del catalogo, y con la extension dentro dos
        // ficheros distintos daban la misma entidad. Ver `fichero`.
        let nombre = f
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or_default()
            .to_string();
        if !troncos.insert(nombre.clone()) {
            return Err(format!(
                "hay dos ficheros que se llaman `{nombre}` con extensiones distintas en `{ruta}`.                  Serian la misma tabla y no lo son, asi que no se elige"
            ));
        }
        let texto = std::fs::read_to_string(f)
            .map_err(|e| format!("no se pudo leer `{}`: {e}", f.display()))?;

        let mut orden: Vec<String> = Vec::new();
        let mut visto: BTreeMap<String, Observado> = BTreeMap::new();
        let mut lineas = 0usize;
        for linea in texto.lines().filter(|l| !l.trim().is_empty()) {
            let n = ore_core::parse::parse(linea)
                .map_err(|e| format!("una línea de `{nombre}` no analiza: {e:?}"))?;
            lineas += 1;
            for (k, v) in n.entries() {
                let Some(col) = k.as_str() else { continue };
                let o = visto.entry(col.to_string()).or_insert_with(|| {
                    orden.push(col.to_string());
                    Observado::default()
                });
                if let Some(c) = clase(v) {
                    o.con_valor += 1;
                    if !o.clases.contains(&c) {
                        o.clases.push(c);
                        o.clases.sort_unstable();
                    }
                }
            }
        }

        if lineas == 0 || orden.is_empty() {
            // Decirlo es mejor que emitir una tabla sin columnas, que tendría
            // el mismo aspecto que un objeto que de verdad no tiene ninguna.
            avisos.push(format!(
                "`{nombre}` no tiene ninguna línea con datos, así que no se puede decir qué \
                 columnas tiene. No entra en el catálogo"
            ));
            continue;
        }

        let mut columnas: Vec<Columna> = Vec::new();
        for col in &orden {
            let o = &visto[col];
            // El tipo o su cita, nunca los dos.
            let (tipo, origen) = match o.clases.as_slice() {
                [uno] if es_escalar_oos(uno) => (Some(uno.to_string()), None),
                [uno] => (None, Some(uno.to_string())),
                varias => {
                    let union = varias.join("|");
                    avisos.push(format!(
                        "`{nombre}.{col}` trae {} clases de valor distintas ({union}): no es un \
                         tipo, son varios, y se cita sin traducir",
                        varias.len()
                    ));
                    (None, Some(union))
                }
            };
            columnas.push(Columna {
                nombre: col.clone(),
                tipo,
                origen,
                // Obligatoria si apareció con valor en TODAS las líneas. Es un
                // hecho de este fichero y se emite como tal.
                obligatoria: o.con_valor == lineas,
                ..Columna::default()
            });
        }

        tablas.push(Tabla {
            nombre: nombre.clone(),
            columnas,
            clase: "table".to_string(),
            lee: Some(Json::obj([
                ("predicatePushdown", Json::Arr(vec![Json::s("eq")])),
                ("fullScan", Json::s("cheap")),
            ])),
            cambia: Some(Json::obj([
                ("mode", Json::s("none")),
                ("witness", Json::s("snapshot")),
            ])),
            ..Tabla::default()
        });
    }

    if tablas.is_empty() {
        return Err(format!(
            "no hay ningún fichero {} con datos en `{ruta}`. Un catálogo vacío tendría el mismo \
             aspecto que un esquema sin tablas, así que se dice en vez de devolverlo",
            EXTENSIONES
                .iter()
                .map(|e| format!("`.{e}`"))
                .collect::<Vec<_>>()
                .join(" o ")
        ));
    }
    avisos.push(
        "las columnas son las OBSERVADAS: una clave que no aparezca en ninguna línea no existe \
         para este catálogo, y no se propone clave primaria porque ser única aquí es un hecho de \
         este fichero y una conjetura sobre la tabla"
            .to_string(),
    );
    // **El emisor es el del protocolo**, no uno de aqui.
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en_disco(nombre: &str, ficheros: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jsonl-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (n, c) in ficheros {
            std::fs::write(d.join(n), c).unwrap();
        }
        d
    }

    fn catalogo(nombre: &str, ficheros: &[(&str, &str)]) -> (String, Vec<String>) {
        let d = en_disco(nombre, ficheros);
        let mut avisos = Vec::new();
        let c = de_directorio("lago", d.to_str().unwrap(), &mut avisos).expect("catálogo");
        (c, avisos)
    }

    /// **Cómo estaba escrito el valor decide el tipo.** `"7"` es una cadena y
    /// `7` un número, y el analizador conserva la diferencia.
    #[test]
    fn el_tipo_sale_de_como_estaba_escrito_el_valor() {
        let (c, _) = catalogo(
            "tipos",
            &[(
                "pedidos.ndjson",
                "{\"id\": 7, \"ref\": \"7\", \"total\": 12.50, \"pagado\": true}\n",
            )],
        );
        assert!(c.contains("\"name\": \"id\""), "{c}");
        assert!(c.contains("\"type\": \"Integer\""), "{c}");
        assert!(c.contains("\"type\": \"String\""), "{c}");
        assert!(c.contains("\"type\": \"Decimal\""), "{c}");
        assert!(c.contains("\"type\": \"Boolean\""), "{c}");
    }

    /// Y si una columna trae dos clases, **no se elige una**: se citan las dos
    /// sin traducir, y se avisa.
    #[test]
    fn una_columna_con_dos_clases_no_se_traduce_a_una() {
        let (c, avisos) = catalogo("mezcla", &[("x.ndjson", "{\"v\": 1}\n{\"v\": \"uno\"}\n")]);
        assert!(c.contains("\"sourceType\": \"Integer|String\""), "{c}");
        assert!(!c.contains("\"type\":"), "{c}");
        assert!(
            avisos
                .iter()
                .any(|a| a.contains("no es un tipo, son varios")),
            "{avisos:?}"
        );
    }

    /// Un objeto o una lista **no** se aplanan a `Opaque`: tienen estructura
    /// que el origen acaba de enumerar.
    #[test]
    fn un_objeto_anidado_se_cita_y_no_se_traduce() {
        let (c, _) = catalogo(
            "anidado",
            &[(
                "x.ndjson",
                "{\"dir\": {\"cp\": \"08001\"}, \"tags\": [1, 2]}\n",
            )],
        );
        assert!(c.contains("\"sourceType\": \"object\""), "{c}");
        assert!(c.contains("\"sourceType\": \"array\""), "{c}");
    }

    /// `required` es un hecho de ESTE fichero: apareció con valor en todas las
    /// líneas. Un nulo o una ausencia lo quitan.
    #[test]
    fn obligatoria_es_haber_aparecido_con_valor_en_todas_las_lineas() {
        let (c, _) = catalogo(
            "obligatoria",
            &[(
                "x.ndjson",
                "{\"a\": 1, \"b\": 2, \"c\": null}\n{\"a\": 3}\n",
            )],
        );
        // `a` está en las dos con valor; `b` falta en una; `c` es nula.
        let a = c.split("\"name\": \"a\"").nth(1).unwrap_or_default();
        assert!(a.starts_with(",\n") || a.contains("required"), "{c}");
        assert_eq!(c.matches("\"required\": true").count(), 1, "{c}");
    }

    /// **No se propone clave primaria**, aunque una columna sea única aquí.
    #[test]
    fn no_se_propone_clave_primaria_aunque_lo_parezca() {
        let (c, avisos) = catalogo("clave", &[("x.ndjson", "{\"id\": 1}\n{\"id\": 2}\n")]);
        assert!(!c.contains("primaryKey"), "{c}");
        assert!(
            avisos
                .iter()
                .any(|a| a.contains("conjetura sobre la tabla")),
            "{avisos:?}"
        );
    }

    /// Las dos caras de un fichero: se deja recorrer barato y **no emite
    /// cambios**, pero sabe fecharse.
    #[test]
    fn un_fichero_no_emite_cambios_y_aun_asi_sabe_fecharse() {
        let (c, _) = catalogo("caras", &[("x.ndjson", "{\"a\": 1}\n")]);
        assert!(c.contains("\"fullScan\": \"cheap\""), "{c}");
        assert!(c.contains("\"mode\": \"none\""), "{c}");
        assert!(c.contains("\"witness\": \"snapshot\""), "{c}");
    }

    /// Un fichero vacío no entra, y se dice. Emitirlo con cero columnas tendría
    /// el mismo aspecto que un objeto que de verdad no tiene ninguna.
    #[test]
    fn un_fichero_sin_datos_no_entra_y_se_avisa() {
        let (c, avisos) = catalogo(
            "vacio",
            &[("lleno.ndjson", "{\"a\": 1}\n"), ("vacio.ndjson", "\n\n")],
        );
        assert!(c.contains("\"name\": \"lleno\""), "{c}");
        assert!(!c.contains("\"name\": \"vacio\""), "{c}");
        assert!(
            avisos.iter().any(|a| a.contains("no tiene ninguna línea")),
            "{avisos:?}"
        );
    }

    /// Y un directorio sin nada que leer **se niega** en vez de devolver un
    /// catálogo vacío.
    #[test]
    fn un_directorio_sin_ndjson_se_niega() {
        let d = en_disco("nada", &[("notas.txt", "hola")]);
        let mut avisos = Vec::new();
        let e = de_directorio("lago", d.to_str().unwrap(), &mut avisos).expect_err("se niega");
        assert!(e.contains("mismo aspecto que un esquema sin tablas"), "{e}");
    }

    /// El orden de los ficheros no es el del sistema de ficheros: el catálogo
    /// alimenta un digest y tiene que ser el mismo en dos máquinas.
    #[test]
    fn el_orden_de_las_tablas_es_estable() {
        let (c, _) = catalogo(
            "orden",
            &[
                ("z.ndjson", "{\"a\": 1}\n"),
                ("a.ndjson", "{\"a\": 1}\n"),
                ("m.ndjson", "{\"a\": 1}\n"),
            ],
        );
        let pos = |n: &str| c.find(&format!("\"name\": \"{n}\"")).unwrap_or(usize::MAX);
        assert!(pos("a") < pos("m"), "{c}");
        assert!(pos("m") < pos("z"), "{c}");
    }
}
