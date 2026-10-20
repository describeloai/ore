//! **La tabla de Phoenix**, leída del catálogo y no inferida: las columnas, sus tipos, su orden y la clave
//! primaria (`KEY_SEQ`). Sale de `getColumns` de Avatica (ficha 0071 §3.5).
//!
//! Los nombres de Phoenix son **exactos**: un identificador sin comillas se pasa a mayúsculas
//! (`ore_x0.pedidos` es `ORE_X0.PEDIDOS`), y el conector siempre cita, así que lleva los del catálogo.
//! Los patrones de `getColumns` tratan `_` como un comodín: el resultado se filtra por igualdad exacta, o
//! `ORE_X0` leería también las columnas de `OREAX0`.

use crate::tipos::Tipo;
use ore_avatica::Valor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    pub nombre: String,
    /// El tipo tal como lo dice Phoenix (`UNSIGNED_INT`, `INTEGER ARRAY`): el `physicalType` del catálogo.
    pub tipo_phoenix: String,
    pub tipo: Tipo,
    /// Es un `DATE` de Phoenix (que lleva fecha **y hora**): se lee con `CAST(… AS TIMESTAMP)`, salvo que la
    /// tabla lo declare `Date`.
    pub es_date: bool,
    pub precision: u32,
    pub escala: u32,
    pub anulable: bool,
    /// La posición en la clave primaria (desde 1), o `None`.
    pub clave: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabla {
    pub esquema: Option<String>,
    pub nombre: String,
    /// En el orden de la tabla (`ORDINAL_POSITION`).
    pub columnas: Vec<Columna>,
}

/// Cita un identificador de Phoenix: entre comillas dobles, con las comillas dobladas.
pub fn citar(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

impl Tabla {
    pub fn columna(&self, nombre: &str) -> Option<&Columna> {
        self.columnas.iter().find(|c| c.nombre == nombre)
    }

    /// `"ESQUEMA"."TABLA"` o `"TABLA"` si no tiene esquema.
    pub fn objeto_citado(&self) -> String {
        match &self.esquema {
            Some(e) => format!("{}.{}", citar(e), citar(&self.nombre)),
            None => citar(&self.nombre),
        }
    }

    /// `ESQUEMA.TABLA`, como el objeto de una petición.
    pub fn objeto(&self) -> String {
        match &self.esquema {
            Some(e) => format!("{e}.{}", self.nombre),
            None => self.nombre.clone(),
        }
    }

    /// La clave primaria, en su orden.
    pub fn clave_primaria(&self) -> Vec<String> {
        let mut k: Vec<(&u32, &str)> = self
            .columnas
            .iter()
            .filter_map(|c| c.clave.as_ref().map(|s| (s, c.nombre.as_str())))
            .collect();
        k.sort();
        k.into_iter().map(|(_, n)| n.to_string()).collect()
    }

    /// Lo que va en el `SELECT` para leer esta columna. Un `DATE` de Phoenix llega como **días** y pierde la
    /// hora (§3.4): se pide como `TIMESTAMP`, que llega en milisegundos y la lleva. Si la tabla lo declara
    /// `Date` (`Tipo::Fecha`), se lee tal cual: los días son lo que se pidió.
    pub fn expresion(&self, c: &Columna) -> String {
        let q = citar(&c.nombre);
        if c.es_date && c.tipo != Tipo::Fecha {
            format!("CAST({q} AS TIMESTAMP) AS {q}")
        } else {
            q
        }
    }
}

/// La tabla `nombre` de `esquema`, de lo que devolvió `getColumns`. `None` si no tiene columnas (no existe).
pub fn desde_metadatos(
    esquema: Option<&str>,
    nombre: &str,
    m: &ore_avatica::Tabla,
) -> Result<Option<Tabla>, String> {
    let idx = |n: &str| {
        m.indice(n).ok_or_else(|| {
            format!("`getColumns` no trae la columna `{n}`: no es un Phoenix que se entienda")
        })
    };
    let (i_esq, i_tab, i_col, i_tipo) = (
        idx("TABLE_SCHEM")?,
        idx("TABLE_NAME")?,
        idx("COLUMN_NAME")?,
        idx("TYPE_NAME")?,
    );
    let (i_tam, i_dig, i_nul, i_ord, i_key) = (
        idx("COLUMN_SIZE")?,
        idx("DECIMAL_DIGITS")?,
        idx("NULLABLE")?,
        idx("ORDINAL_POSITION")?,
        idx("KEY_SEQ")?,
    );
    let texto = |v: &Valor| match v {
        Valor::Texto(s) => Some(s.clone()),
        _ => None,
    };
    let entero = |v: &Valor| match v {
        Valor::Entero(n) => Some(*n),
        _ => None,
    };
    let mut cols: Vec<(i64, Columna)> = Vec::new();
    for f in &m.filas {
        // El patrón de `getColumns` trata `_` como comodín: solo vale la igualdad exacta.
        let e = texto(&f[i_esq]).filter(|s| !s.is_empty());
        if e.as_deref() != esquema || texto(&f[i_tab]).as_deref() != Some(nombre) {
            continue;
        }
        let Some(col) = texto(&f[i_col]) else {
            continue;
        };
        let tipo_phoenix = texto(&f[i_tipo]).unwrap_or_default();
        let (precision, escala) = (
            entero(&f[i_tam]).unwrap_or(0).max(0) as u32,
            entero(&f[i_dig]).unwrap_or(0).max(0) as u32,
        );
        let t = tipo_phoenix.to_ascii_uppercase();
        cols.push((
            entero(&f[i_ord]).unwrap_or(0),
            Columna {
                nombre: col,
                es_date: t == "DATE" || t == "UNSIGNED_DATE",
                tipo: Tipo::de(&tipo_phoenix, precision, escala),
                tipo_phoenix,
                precision,
                escala,
                anulable: entero(&f[i_nul]) != Some(0),
                clave: entero(&f[i_key]).filter(|n| *n > 0).map(|n| n as u32),
            },
        ));
    }
    if cols.is_empty() {
        return Ok(None);
    }
    cols.sort_by_key(|(o, _)| *o);
    Ok(Some(Tabla {
        esquema: esquema.map(str::to_string),
        nombre: nombre.to_string(),
        columnas: cols.into_iter().map(|(_, c)| c).collect(),
    }))
}

/// Lo que `getTables` dice de una tabla o vista, antes de pedirle sus columnas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objeto {
    pub esquema: Option<String>,
    pub nombre: String,
    pub vista: bool,
    /// Los cubos de la sal (`SALT_BUCKETS`), si la tabla la lleva.
    pub sal: Option<u32>,
    pub multi_tenant: bool,
}

/// Las tablas y vistas de un `getTables`, sin el esquema `SYSTEM` (el catálogo de Phoenix), ordenadas por
/// esquema y nombre. `TABLE_TYPE` distinto de `TABLE`/`VIEW` (un índice, una tabla del sistema) no entra.
pub fn objetos_desde(m: &ore_avatica::Tabla) -> Result<Vec<Objeto>, String> {
    let idx = |n: &str| {
        m.indice(n).ok_or_else(|| {
            format!("`getTables` no trae la columna `{n}`: no es un Phoenix que se entienda")
        })
    };
    let (i_esq, i_tab, i_tipo) = (idx("TABLE_SCHEM")?, idx("TABLE_NAME")?, idx("TABLE_TYPE")?);
    let (i_sal, i_mt) = (m.indice("SALT_BUCKETS"), m.indice("MULTI_TENANT"));
    let mut v: Vec<Objeto> = Vec::new();
    for f in &m.filas {
        let Valor::Texto(nombre) = &f[i_tab] else {
            continue;
        };
        let esquema = match &f[i_esq] {
            Valor::Texto(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        if esquema.as_deref() == Some("SYSTEM") {
            continue;
        }
        let vista = match &f[i_tipo] {
            Valor::Texto(t) if t == "TABLE" => false,
            Valor::Texto(t) if t == "VIEW" => true,
            _ => continue,
        };
        let sal = i_sal.and_then(|i| match &f[i] {
            Valor::Entero(n) if *n > 0 => Some(*n as u32),
            _ => None,
        });
        let multi_tenant = i_mt.is_some_and(|i| matches!(&f[i], Valor::Bool(true)));
        v.push(Objeto {
            esquema,
            nombre: nombre.clone(),
            vista,
            sal,
            multi_tenant,
        });
    }
    v.sort_by(|a, b| (&a.esquema, &a.nombre).cmp(&(&b.esquema, &b.nombre)));
    Ok(v)
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;
    use ore_avatica::{Anulable, Columna as ColAvatica};

    fn col(n: &str) -> ColAvatica {
        ColAvatica {
            nombre: n.into(),
            etiqueta: n.into(),
            esquema: String::new(),
            tabla: String::new(),
            tipo: "VARCHAR".into(),
            codigo_sql: 12,
            rep: None,
            precision: 0,
            escala: 0,
            anulable: Anulable::Si,
            con_signo: false,
            componente: None,
        }
    }

    pub fn t(s: &str) -> Valor {
        Valor::Texto(s.into())
    }

    pub fn n(x: i64) -> Valor {
        Valor::Entero(x)
    }

    /// Una fila de `getColumns`, con las columnas que el conector lee y en el orden de Phoenix.
    #[allow(clippy::too_many_arguments)]
    pub fn fila(
        esq: Option<&str>,
        tab: &str,
        nombre: &str,
        tipo: &str,
        tam: i64,
        dig: i64,
        nul: i64,
        ord: i64,
        key: Option<i64>,
    ) -> Vec<Valor> {
        vec![
            esq.map_or(Valor::Nulo, t),
            t(tab),
            t(nombre),
            t(tipo),
            n(tam),
            n(dig),
            n(nul),
            n(ord),
            key.map_or(Valor::Nulo, n),
        ]
    }

    pub fn metadatos(filas: Vec<Vec<Valor>>) -> ore_avatica::Tabla {
        ore_avatica::Tabla {
            columnas: [
                "TABLE_SCHEM",
                "TABLE_NAME",
                "COLUMN_NAME",
                "TYPE_NAME",
                "COLUMN_SIZE",
                "DECIMAL_DIGITS",
                "NULLABLE",
                "ORDINAL_POSITION",
                "KEY_SEQ",
            ]
            .into_iter()
            .map(col)
            .collect(),
            filas,
        }
    }

    /// `ORE_X0.PEDIDOS`: la clave es `ID`; hay un `DECIMAL(12,2)` y un `TIMESTAMP`.
    pub fn pedidos() -> Tabla {
        let m = metadatos(vec![
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "ESTADO",
                "VARCHAR",
                0,
                0,
                1,
                5,
                None,
            ),
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "ID",
                "BIGINT",
                0,
                0,
                0,
                1,
                Some(1),
            ),
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "CLIENTE",
                "VARCHAR",
                0,
                0,
                1,
                2,
                None,
            ),
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "IMPORTE",
                "DECIMAL",
                12,
                2,
                1,
                3,
                None,
            ),
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "CREADO",
                "TIMESTAMP",
                0,
                0,
                1,
                4,
                None,
            ),
        ]);
        desde_metadatos(Some("ORE_X0"), "PEDIDOS", &m)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn la_tabla_sale_en_el_orden_de_phoenix_con_su_clave() {
        let t = pedidos();
        let nombres: Vec<&str> = t.columnas.iter().map(|c| c.nombre.as_str()).collect();
        assert_eq!(nombres, ["ID", "CLIENTE", "IMPORTE", "CREADO", "ESTADO"]);
        assert_eq!(t.clave_primaria(), ["ID"]);
        assert_eq!(
            t.columna("IMPORTE").unwrap().tipo,
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
        );
        assert!(!t.columna("ID").unwrap().anulable && t.columna("CLIENTE").unwrap().anulable);
        assert_eq!(t.objeto_citado(), "\"ORE_X0\".\"PEDIDOS\"");
        assert_eq!(t.objeto(), "ORE_X0.PEDIDOS");
    }

    #[test]
    fn un_comodin_no_cuela_otra_tabla() {
        // `getColumns("ORE_X0", "PEDIDOS")` con `_` como comodín trae también `OREAX0.PEDIDOS`.
        let m = metadatos(vec![
            fila(
                Some("ORE_X0"),
                "PEDIDOS",
                "ID",
                "BIGINT",
                0,
                0,
                0,
                1,
                Some(1),
            ),
            fila(
                Some("OREAX0"),
                "PEDIDOS",
                "AJENA",
                "VARCHAR",
                0,
                0,
                1,
                1,
                None,
            ),
            fila(
                Some("ORE_X0"),
                "PEDIDOSX",
                "OTRA",
                "VARCHAR",
                0,
                0,
                1,
                1,
                None,
            ),
        ]);
        let t = desde_metadatos(Some("ORE_X0"), "PEDIDOS", &m)
            .unwrap()
            .unwrap();
        assert_eq!(t.columnas.len(), 1);
        assert_eq!(t.columnas[0].nombre, "ID");
    }

    #[test]
    fn sin_esquema_es_la_tabla_sin_esquema_y_una_tabla_vacia_no_existe() {
        let m = metadatos(vec![
            fila(None, "T", "A", "INTEGER", 0, 0, 1, 1, None),
            fila(Some("X"), "T", "B", "INTEGER", 0, 0, 1, 1, None),
        ]);
        let tb = desde_metadatos(None, "T", &m).unwrap().unwrap();
        assert_eq!(tb.columnas.len(), 1);
        assert_eq!(tb.objeto_citado(), "\"T\"");
        assert_eq!(desde_metadatos(None, "NO", &m).unwrap(), None);
        // Un esquema vacío (`''`) es «sin esquema».
        let m = metadatos(vec![vec![
            t(""),
            t("T"),
            t("A"),
            t("INTEGER"),
            n(0),
            n(0),
            n(1),
            n(1),
            Valor::Nulo,
        ]]);
        assert!(desde_metadatos(None, "T", &m).unwrap().is_some());
    }

    #[test]
    fn un_date_se_lee_como_timestamp_salvo_que_se_declare_date() {
        let m = metadatos(vec![fila(None, "T", "D", "DATE", 0, 0, 1, 1, None)]);
        let mut t = desde_metadatos(None, "T", &m).unwrap().unwrap();
        let c = t.columnas[0].clone();
        assert!(c.es_date);
        assert_eq!(t.expresion(&c), "CAST(\"D\" AS TIMESTAMP) AS \"D\"");
        t.columnas[0].tipo = Tipo::Fecha;
        assert_eq!(t.expresion(&t.columnas[0]), "\"D\"");
        // Una columna normal va citada, y la comilla se dobla.
        assert_eq!(citar("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn un_getcolumns_que_no_es_el_de_phoenix_se_dice() {
        let m = ore_avatica::Tabla {
            columnas: vec![],
            filas: vec![],
        };
        let e = desde_metadatos(None, "T", &m).unwrap_err();
        assert!(e.contains("TABLE_SCHEM"), "{e}");
    }

    #[test]
    fn los_objetos_de_get_tables_sin_el_sistema_ni_los_indices() {
        let col = |n: &str| ColAvatica {
            nombre: n.into(),
            ..col("x")
        };
        let fila =
            |e: Valor, n: &str, ty: &str, sal: Valor, mt: Valor| vec![e, t(n), t(ty), sal, mt];
        let m = ore_avatica::Tabla {
            columnas: [
                "TABLE_SCHEM",
                "TABLE_NAME",
                "TABLE_TYPE",
                "SALT_BUCKETS",
                "MULTI_TENANT",
            ]
            .into_iter()
            .map(col)
            .collect(),
            filas: vec![
                fila(
                    t("SYSTEM"),
                    "CATALOG",
                    "TABLE",
                    Valor::Nulo,
                    Valor::Bool(false),
                ),
                fila(t("ORE_X0"), "SALADA", "TABLE", n(4), Valor::Bool(false)),
                fila(t("ORE_X0"), "IDX", "INDEX", Valor::Nulo, Valor::Nulo),
                fila(t("ORE_X0"), "INQ", "TABLE", Valor::Nulo, Valor::Bool(true)),
                fila(t("ORE_X0"), "CERRADOS", "VIEW", Valor::Nulo, Valor::Nulo),
                fila(
                    Valor::Nulo,
                    "SUELTA",
                    "TABLE",
                    Valor::Nulo,
                    Valor::Bool(false),
                ),
            ],
        };
        let o = objetos_desde(&m).unwrap();
        let nombres: Vec<_> = o.iter().map(|x| x.nombre.as_str()).collect();
        assert_eq!(nombres, ["SUELTA", "CERRADOS", "INQ", "SALADA"]);
        assert_eq!(o[0].esquema, None);
        assert!(o[1].vista && !o[2].vista);
        assert!(o[2].multi_tenant && !o[3].multi_tenant);
        assert_eq!((o[3].sal, o[2].sal), (Some(4), None));
        let sin = ore_avatica::Tabla {
            columnas: vec![],
            filas: vec![],
        };
        assert!(objetos_desde(&sin).unwrap_err().contains("TABLE_SCHEM"));
    }
}
