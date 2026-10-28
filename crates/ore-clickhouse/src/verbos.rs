//! **Los verbos que son de ClickHouse** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y `cambio`.
//! Los cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o el motivo del fallo (que va a stderr, literal,
//! sin la credencial).
//!
//! **Lo que ve el usuario es lo que el catálogo dice.** `system.tables` y `system.columns` sólo enseñan lo que el usuario tiene permiso
//! de ver, así que no se descubre lo que no se puede leer. Las bases del propio servidor (`system`, `INFORMATION_SCHEMA`) no se catalogan.

use crate::lectura::ClickHouse;
use crate::rowbinary::{Tipo, analizar};
use crate::traductor::{identificador, partes_del_objeto};
use crate::transporte::{Cliente, Param};
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};
use std::collections::BTreeMap;
use std::process::ExitCode;

/// Las bases que mantiene el servidor: no son datos de nadie.
const BASES_DEL_SERVIDOR: &str = "'system', 'INFORMATION_SCHEMA', 'information_schema'";

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-clickhouse: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("clickhouse", String::as_str),
        ),
        "check" => check(&entrada),
        "explorar" => explorar(&entrada),
        "testigo" => testigo(&entrada),
        "cambio" => cambio(&entrada),
        otro => Err(format!(
            "verbo desconocido `{otro}` (capacidades, leer, estimar, servir, catalogo, check, explorar, testigo, cambio)"
        )),
    };
    match hecho {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-clickhouse: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

/// Conecta (versión, suelo y perfil incluidos) y devuelve el cliente.
fn abrir(url: &str) -> Result<Cliente, String> {
    ClickHouse::conectar(url)
        .map(|s| s.cliente)
        .map_err(|f| texto(f, url))
}

/// Un objeto que el usuario ve: `(base, nombre, motor, filas, comentario)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Objeto {
    pub base: String,
    pub nombre: String,
    pub motor: String,
    pub filas: Option<u64>,
    pub comentario: String,
}

impl Objeto {
    pub fn vista(&self) -> bool {
        matches!(
            self.motor.as_str(),
            "View" | "MaterializedView" | "LiveView" | "WindowView"
        )
    }

    pub fn completo(&self) -> String {
        format!("{}.{}", self.base, self.nombre)
    }
}

/// `\N` es el nulo de `TabSeparated`.
fn campo(c: &str) -> Option<String> {
    (c != "\\N").then(|| desescapar(c))
}

fn desescapar(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    let mut c = t.chars();
    while let Some(x) = c.next() {
        if x != '\\' {
            s.push(x);
            continue;
        }
        match c.next() {
            Some('t') => s.push('\t'),
            Some('n') => s.push('\n'),
            Some('r') => s.push('\r'),
            Some('0') => s.push('\0'),
            Some(otro) => s.push(otro),
            None => s.push('\\'),
        }
    }
    s
}

/// Las tablas y vistas que el usuario ve, de la base de la URL o de todas las que no son del servidor.
pub fn objetos(c: &Cliente) -> Result<Vec<Objeto>, Fallo> {
    let (filtro, params): (String, Vec<(&str, Param)>) = match &c.destino().base {
        Some(b) => (
            "database = {d:String}".into(),
            vec![("d", Param::Texto(b.as_str()))],
        ),
        None => (format!("database NOT IN ({BASES_DEL_SERVIDOR})"), vec![]),
    };
    // Las tablas internas de una vista materializada (`.inner_id.…`) no son de nadie.
    let hoja = c.texto_con(
        &format!(
            "SELECT database, name, engine, total_rows, comment FROM system.tables \
             WHERE {filtro} AND NOT startsWith(name, '.inner') AND NOT is_temporary ORDER BY database, name"
        ),
        &params,
    )?;
    Ok(hoja
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            Some(Objeto {
                base: campo(p.next()?)?,
                nombre: campo(p.next()?)?,
                motor: campo(p.next()?)?,
                filas: p.next().and_then(campo).and_then(|n| n.parse().ok()),
                comentario: p.next().and_then(campo).unwrap_or_default(),
            })
        })
        .collect())
}

/// Una columna como la dice `system.columns`.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnaCH {
    pub base: String,
    pub tabla: String,
    pub nombre: String,
    pub tipo: String,
    pub comentario: String,
    pub en_la_clave_de_orden: bool,
}

fn columnas(c: &Cliente) -> Result<Vec<ColumnaCH>, Fallo> {
    let (filtro, params): (String, Vec<(&str, Param)>) = match &c.destino().base {
        Some(b) => (
            "database = {d:String}".into(),
            vec![("d", Param::Texto(b.as_str()))],
        ),
        None => (format!("database NOT IN ({BASES_DEL_SERVIDOR})"), vec![]),
    };
    let hoja = c.texto_con(
        &format!(
            "SELECT database, table, name, type, comment, is_in_sorting_key FROM system.columns \
             WHERE {filtro} ORDER BY database, table, position"
        ),
        &params,
    )?;
    Ok(hoja
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            Some(ColumnaCH {
                base: campo(p.next()?)?,
                tabla: campo(p.next()?)?,
                nombre: campo(p.next()?)?,
                tipo: campo(p.next()?)?,
                comentario: p.next().and_then(campo).unwrap_or_default(),
                en_la_clave_de_orden: p.next() == Some("1"),
            })
        })
        .collect())
}

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va **literal** (es lo único accionable): abre (red, TLS,
/// credencial, versión, perfil), lista lo que la URL deja ver y, si hay algo, **lee una fila de una tabla** (el permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let c = ClickHouse::conectar(&url)?.cliente;
        let todos = objetos(&c)?;
        if let Some(b) = &c.destino().base
            && todos.is_empty()
        {
            return Err(Fallo::new(
                Codigo::Objeto,
                format!("la base `{b}` no existe o no tiene tablas que el usuario vea"),
            ));
        }
        if let Some(o) = todos.iter().find(|o| !o.vista()) {
            c.texto(&format!(
                "SELECT 1 FROM {}.{} LIMIT 1",
                identificador(&o.base)?,
                identificador(&o.nombre)?
            ))?;
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`catalogo`**: cada tabla y vista que la URL deja ver, con los tipos de `system.columns` (no se infiere nada), sus comentarios y
/// su clave de ordenación como aviso. **ClickHouse no tiene claves únicas** (su `PRIMARY KEY` es un índice disperso, no restringe
/// nada), así que `clave` y `unicas` van vacías: la ausencia es una respuesta cierta y no se inventa una clave que no garantiza nada.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err(
            "no llegó nada por stdin. `catalogo` espera la URL de ClickHouse por ahí".into(),
        );
    }
    let c = abrir(url)?;
    let objs = objetos(&c).map_err(|f| texto(f, url))?;
    let mut cols = columnas(&c).map_err(|f| texto(f, url))?;
    let mut por_tabla: BTreeMap<(String, String), Vec<ColumnaCH>> = BTreeMap::new();
    for k in cols.drain(..) {
        por_tabla
            .entry((k.base.clone(), k.tabla.clone()))
            .or_default()
            .push(k);
    }
    let mut tablas = Vec::new();
    for o in &objs {
        let cs = por_tabla
            .remove(&(o.base.clone(), o.nombre.clone()))
            .unwrap_or_default();
        for a in avisos(o, &cs) {
            eprintln!("ore-read-clickhouse: aviso · `{}`: {a}", o.completo());
        }
        tablas.push(tabla_de(o, &cs));
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// ¿Se lee con `FINAL`? Las tablas que fusionan por clave dejan filas repetidas hasta que se fusionan (ADR 0077 §3.3).
pub fn pide_final(motor: &str) -> bool {
    motor.contains("ReplacingMergeTree") || motor.contains("CollapsingMergeTree")
}

/// Lo que quien cataloga debe saber de esta tabla y el catálogo no puede llevar.
fn avisos(o: &Objeto, cs: &[ColumnaCH]) -> Vec<String> {
    let mut a = Vec::new();
    for k in cs {
        if let Err(m) = analizar(&k.tipo) {
            a.push(format!(
                "la columna `{}` es `{}`: se cataloga y no se lee ({m})",
                k.nombre, k.tipo
            ));
        }
    }
    if pide_final(&o.motor) {
        a.push(format!(
            "es {}: se lee con `FINAL`, así que cada clave sale una vez (más lento que sin él)",
            o.motor
        ));
    }
    let orden: Vec<&str> = cs
        .iter()
        .filter(|k| k.en_la_clave_de_orden)
        .map(|k| k.nombre.as_str())
        .collect();
    if !orden.is_empty() && !o.vista() {
        a.push(format!(
            "ClickHouse no tiene claves únicas: `clave` va vacía. Su clave de ordenación es ({}), que no impide repetidos",
            orden.join(", ")
        ));
    }
    a
}

/// Una tabla o vista de ClickHouse, como la tabla del catálogo.
fn tabla_de(o: &Objeto, cs: &[ColumnaCH]) -> Tabla {
    let columnas = cs
        .iter()
        .map(|k| {
            let t = analizar(&k.tipo).ok();
            Columna {
                nombre: k.nombre.clone(),
                // `None`: este conector no supo traducirlo (no es un hueco a rellenar).
                tipo: t.as_ref().map(|t| t.fisico().1),
                // El tipo de ClickHouse tal cual (`Nullable(Decimal(12, 2))`): con él, el motor sabe lo que el escalar no lleva.
                origen: Some(k.tipo.clone()),
                obligatoria: !matches!(t, Some(Tipo::Nulable(_))),
                valores: Vec::new(),
                descripcion: (!k.comentario.is_empty()).then(|| k.comentario.clone()),
            }
        })
        .collect();
    Tabla {
        nombre: o.completo(),
        columnas,
        filas_estimadas: o.filas,
        clase: if o.vista() { "view" } else { "table" }.to_string(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// La cara `I`: lo que la inducción declara que se puede empujar (lo que el conector v2 pone, menos `like`), y el recorrido completo.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                ore_driver::EMPUJE_INDUCIDO
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("cheap")),
    ])
}

/// La cara `D`. ClickHouse no tiene un registro de cambios que se lea de fuera: **no se declara lo que no se hace**. El incremental
/// por columna y la huella por *parts* (el verbo `cambio`) son otra cosa.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// **`explorar`**: las bases con tablas o vistas que la URL deja ver. Lo que `catalogo` recorre si la URL no nombra una.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = abrir(&url)?;
    let objs = objetos(&c).map_err(|f| texto(f, &url))?;
    let mut por_base: BTreeMap<&str, usize> = BTreeMap::new();
    for o in &objs {
        *por_base.entry(o.base.as_str()).or_default() += 1;
    }
    let contiene: Vec<Json> = por_base
        .iter()
        .map(|(b, n)| Json::obj([("nombre", Json::s(*b)), ("objetos", Json::Int(*n as i64))]))
        .collect();
    let nota = match &c.destino().base {
        Some(b) => format!("la URL nombra la base `{b}`: el catálogo recorre solo esa"),
        None => "la URL no nombra una base: el catálogo recorre todas las que ve el usuario, menos las del servidor".to_string(),
    };
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: hasta dónde está el origen. ClickHouse no tiene un orden de cambios que se lea de fuera, así que `none`, que es una
/// respuesta cierta (ADR 0016): devolver «ahora» inventaría una marca que la lectura no respalda.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    abrir(&url)?;
    Ok(ore_driver::testigo("none", None))
}

/// La huella de una tabla `MergeTree`: los nombres de sus *parts* activas, ordenados. Un `INSERT` crea una *part*; una mutación
/// (`ALTER … UPDATE/DELETE`, un `DELETE` ligero) y una fusión renombran las suyas; `DROP` y `TRUNCATE` las quitan. **Igual quiere
/// decir que no cambió**; distinta puede ser una fusión que no cambió un dato (un falso «cambió», que sólo cuesta una copia de más).
fn huella_de_partes(c: &Cliente, base: &str, tabla: &str) -> Result<String, Fallo> {
    let hoja = c.texto_con(
        "SELECT toString(cityHash64(arraySort(groupArray(name)))), count() FROM system.parts \
         WHERE database = {d:String} AND table = {t:String} AND active",
        &[("d", Param::Texto(base)), ("t", Param::Texto(tabla))],
    )?;
    let (hash, partes) = hoja.trim_end().split_once('\t').ok_or_else(|| {
        Fallo::origen("ClickHouse contestó a las partes con algo que no se entiende")
    })?;
    Ok(format!("{hash}:{partes}"))
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA tabla desde la última copia? Con una `MergeTree` **no replicada**, la huella son sus *parts*
/// activas y `respalda: true`. Con cualquier otra cosa (una réplica, una vista, `Memory`, una tabla distribuida) no hay huella que
/// respalde: `respalda: false` y la copia no se fía. Si la tabla no está es un error, no «no respalda».
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let c = abrir(&url)?;
    let (b, t) =
        partes_del_objeto(&objeto, c.destino().base.as_deref()).map_err(|f| texto(f, &url))?;
    let todos = objetos(&c).map_err(|f| texto(f, &url))?;
    let Some(o) = todos.iter().find(|o| o.base == b && o.nombre == t) else {
        return Err(format!("`{b}.{t}` no existe, o el usuario no puede verla"));
    };
    let propia = o.motor.ends_with("MergeTree") && !o.motor.starts_with("Replicated");
    if !propia {
        return Ok(ore_driver::huella("sin-contadores", None, false));
    }
    huella_o_nada(huella_de_partes(&c, &b, &t), &url)
}

/// La huella de las *parts*, o `respalda: false` si el usuario no puede leer `system.parts` (hace falta `GRANT SELECT ON
/// system.parts`): sin permiso no hay huella que respalde, y la copia no se fía; se dice por stderr para que se pueda arreglar.
/// Cualquier otro fallo es un error.
fn huella_o_nada(h: Result<String, Fallo>, url: &str) -> Result<String, String> {
    match h {
        Ok(h) => Ok(ore_driver::huella("partes", Some(&h), true)),
        Err(f) if f.codigo == Codigo::Credencial => {
            eprintln!(
                "ore-read-clickhouse: aviso · el usuario no puede leer system.parts ({}): sin huella que respalde;                  `GRANT SELECT ON system.parts` lo arregla",
                texto(f, url)
            );
            Ok(ore_driver::huella("sin-contadores", None, false))
        }
        Err(f) => Err(texto(f, url)),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn objeto(motor: &str) -> Objeto {
        Objeto {
            base: "kit".into(),
            nombre: "t".into(),
            motor: motor.into(),
            filas: Some(10),
            comentario: String::new(),
        }
    }

    fn col(n: &str, t: &str, orden: bool, comentario: &str) -> ColumnaCH {
        ColumnaCH {
            base: "kit".into(),
            tabla: "t".into(),
            nombre: n.into(),
            tipo: t.into(),
            comentario: comentario.into(),
            en_la_clave_de_orden: orden,
        }
    }

    fn columnas_de_prueba() -> Vec<ColumnaCH> {
        vec![
            col("id", "Int64", true, ""),
            col(
                "total",
                "Nullable(Decimal(12, 2))",
                false,
                "Importe con IVA",
            ),
            col("n", "UInt64", false, ""),
            col("m", "Map(String, Int64)", false, ""),
        ]
    }

    #[test]
    fn la_tabla_del_catalogo_lleva_tipos_obligatoriedad_y_comentarios_pero_no_clave() {
        let t = tabla_de(&objeto("MergeTree"), &columnas_de_prueba());
        assert_eq!(t.nombre, "kit.t");
        assert_eq!(t.clase, "table");
        // ClickHouse no tiene claves únicas: la clave de ordenación no se hace pasar por una.
        assert!(t.clave.is_empty() && t.unicas.is_empty());
        assert_eq!(t.filas_estimadas, Some(10));
        let tipos: Vec<_> = t.columnas.iter().map(|c| c.tipo.clone()).collect();
        assert_eq!(
            tipos,
            vec![
                Some("Integer".into()),
                Some("Decimal<12, 2>".into()),
                Some("Decimal<20, 0>".into()),
                // Lo que no se sabe leer no inventa un tipo.
                None,
            ]
        );
        assert_eq!(
            t.columnas[1].origen.as_deref(),
            Some("Nullable(Decimal(12, 2))")
        );
        assert_eq!(
            t.columnas[1].descripcion.as_deref(),
            Some("Importe con IVA")
        );
        assert!(t.columnas[0].obligatoria && !t.columnas[1].obligatoria);
    }

    #[test]
    fn las_vistas_se_distinguen_de_las_tablas() {
        assert_eq!(tabla_de(&objeto("View"), &[]).clase, "view");
        assert_eq!(tabla_de(&objeto("MaterializedView"), &[]).clase, "view");
        assert_eq!(tabla_de(&objeto("ReplacingMergeTree"), &[]).clase, "table");
    }

    #[test]
    fn los_avisos_dicen_lo_que_el_catalogo_no_puede_llevar() {
        let a = avisos(&objeto("ReplacingMergeTree"), &columnas_de_prueba()).join("\n");
        assert!(
            a.contains("`m` es `Map(String, Int64)`: se cataloga y no se lee"),
            "{a}"
        );
        assert!(a.contains("se lee con `FINAL`"), "{a}");
        assert!(a.contains("clave de ordenación es (id)"), "{a}");
        assert!(!a.contains("`id` es") && !a.contains("`n` es"), "{a}");
        let a = avisos(&objeto("MergeTree"), &[col("id", "Int64", true, "")]).join("\n");
        assert!(!a.contains("FINAL"), "{a}");
    }

    #[test]
    fn final_solo_en_los_motores_que_fusionan_por_clave() {
        for m in [
            "ReplacingMergeTree",
            "ReplicatedReplacingMergeTree",
            "CollapsingMergeTree",
            "VersionedCollapsingMergeTree",
            "SharedReplacingMergeTree",
        ] {
            assert!(pide_final(m), "{m}");
        }
        for m in [
            "MergeTree",
            "SummingMergeTree",
            "AggregatingMergeTree",
            "Memory",
            "View",
        ] {
            assert!(!pide_final(m), "{m}");
        }
    }

    #[test]
    fn el_origen_no_declara_cambios_que_no_lee() {
        assert_eq!(cambios().jcs(), r#"{"mode":"none","witness":"none"}"#);
        assert!(lee().jcs().contains("\"fullScan\":\"cheap\""));
    }

    #[test]
    fn sin_permiso_sobre_las_partes_no_hay_huella_que_respalde_pero_otro_fallo_es_un_error() {
        let sin_permiso = Fallo::new(Codigo::Credencial, "ACCESS_DENIED".to_string());
        let h = huella_o_nada(Err(sin_permiso), "clickhouse://x").unwrap();
        assert!(h.contains(r#""respalda":false"#), "{h}");
        let roto = Fallo::new(Codigo::Conexion, "se cortó".to_string());
        assert!(huella_o_nada(Err(roto), "clickhouse://x").is_err());
        let h = huella_o_nada(Ok("123:4".into()), "clickhouse://x").unwrap();
        assert!(
            h.contains(r#""respalda":true"#) && h.contains("123:4"),
            "{h}"
        );
    }
}
