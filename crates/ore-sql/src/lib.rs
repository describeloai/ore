//! **La forma de una consulta**, y el dialecto como dato.
//!
//! ```text
//! SELECT <proyeccion> FROM <objeto>
//! WHERE  <recorte por clave>  AND  <filtros>  AND  <rango del cursor>
//! ```
//!
//! Eso lo construían dos ficheros escritos por separado —`ore-read-postgres` y
//! `ore-read-bigquery`— y lo construían **igual**: los cinco hitos de la forma
//! coincidían en los dos, y quince líneas eran idénticas descontando la
//! puntuación. Lo que de verdad difería eran **cuatro ejes**, y los cuatro son
//! datos: cómo se cita un identificador, cómo se marca un parámetro, cómo se
//! expresa el recorte por clave, y de quién es la consulta del catálogo — que
//! no entra aquí, y el porqué está abajo.
//!
//! # Por qué una biblioteca y no un `ore-read-sql`
//!
//! Porque **el transporte no es dialecto**. Se midió sobre `Cargo.lock`: el
//! lector de PostgreSQL arrastra 114 crates y el de BigQuery 25, y de los 114
//! hay **89 que solo son suyos** —`tokio`, `native-tls`, `openssl`, FFI de
//! plataforma—. Un binario único se los llevaría a un lector que delega en `bq`
//! y no abre un socket, y `ore-cli/tests/dependencias.rs` existe justo para que
//! eso no ocurra sin que nadie lo note.
//!
//! Así que se comparte la forma, y cada driver se queda con dos cosas: su
//! transporte, y una **constante** que describe su dialecto. Una constante y no
//! un fichero: sigue siendo dato —se lee de un vistazo y se compara con el de
//! al lado— y no añade un analizador ni un modo de fallo por fichero ausente.
//!
//! # Lo que NO está aquí, a propósito
//!
//! **La consulta del catálogo.** La de cada familia es larga, específica y ya
//! vive donde tiene que vivir. Meterla aquí sería juntar dos cosas que solo
//! comparten el nombre «SQL».
//!
//! **El juicio sobre las caras**, y no por descuido: se midió y **no son del
//! dialecto**.
//!
//! El espectro de la migración las llamaba «banderas del dialecto», y al ir a
//! moverlas se ve que no lo son:
//!
//! - `fullScan: expensive` frente a `cheap` sale de saber que BigQuery **factura
//!   por bytes leídos**. Es un hecho del producto, no de su gramática: dos
//!   sistemas que hablan el mismo dialecto pueden cobrar distinto, y uno
//!   compatible con el protocolo de PostgreSQL que facturara por escaneo diría
//!   `expensive` con la cita y la marca de PostgreSQL.
//! - **De dónde sale la cara `D`** es un sondeo, y sondea sitios distintos:
//!   `wal_level` es del **clúster** y `enable_change_history` es de la **tabla**.
//!   No es la misma consulta con otro nombre; el hecho no vive en el mismo
//!   sitio.
//!
//! Las dos son de la **familia** —del driver— y ya viven ahí. Meterlas aquí
//! habría hecho del dialecto un cajón: un sitio donde cabe todo lo que varía,
//! que es otra forma de no decir nada.
//!
//! La tercera bandera **sí** era del dialecto, y está: [`Dialecto::exige_tipos`].

use std::collections::BTreeMap;

use ore_driver::Peticion;

/// **Un dialecto**: los tres ejes que varían de una familia SQL a otra.
///
/// Ninguno es una decisión que haya que tomar mirando el plan; los tres se leen
/// de un vistazo y se comparan con el de al lado. Eso es lo que los hace datos.
#[derive(Debug, Clone, Copy)]
pub struct Dialecto {
    pub cita: Cita,
    pub marca: Marca,
    pub recorte: Recorte,
}

/// Cómo se cita un identificador.
#[derive(Debug, Clone, Copy)]
pub enum Cita {
    /// `"a"."b"` — se cita **por partes**, y el delimitador de dentro se
    /// **dobla**. Es lo de PostgreSQL y lo del SQL estándar.
    PorPartes(char),
    /// `` `a.b` `` — se cita **entero**, y el dialecto **no tiene escape** para
    /// el delimitador dentro de un identificador citado. Es lo de BigQuery.
    ///
    /// Por eso un identificador que lo contenga se **rechaza**: inventar un
    /// escape que el dialecto no tiene produciría una consulta que compila y
    /// pregunta otra cosa.
    Entero(char),
}

/// Cómo se marca un parámetro, y si además lleva su tipo.
#[derive(Debug, Clone, Copy)]
pub enum Marca {
    /// `$1`, `$2`… — **posicional**. El servidor coacciona el texto al tipo de
    /// la columna, así que el parámetro no necesita decir de qué tipo es.
    Posicional(char),
    /// `@p0`, `@p1`… — **con nombre**, y el parámetro sale como
    /// `nombre:TIPO:valor`.
    ///
    /// Existe porque GoogleSQL **no coacciona**: comparar un `STRING` con un
    /// `INT64` es un error de tipos. Y la salida falsa —`CAST(col AS STRING)`—
    /// compila, rompe el orden (`"10" < "9"`, así que un rango sobre un cursor
    /// entero devuelve otras filas **sin fallar**) y además mata el recorte por
    /// partición.
    ConNombre(char),
}

/// Cómo se expresa el recorte por clave.
#[derive(Debug, Clone, Copy)]
pub enum Recorte {
    /// `(a, b) IN ((@1, @2), (@3, @4))`.
    TuplaEnLista,
    /// `((a = @1 AND b = @2) OR (a = @3 AND b = @4))`.
    ///
    /// Más larga y se lee igual en cualquier dialecto: no depende de cómo se
    /// comparan dos tuplas, que es lo que cambia entre motores.
    DisyuncionDeConjunciones,
}

/// Lo que sale: el texto y sus parámetros, ya en la forma que espera el
/// transporte de cada familia.
#[derive(Debug, PartialEq, Eq)]
pub struct Consulta {
    pub texto: String,
    pub parametros: Vec<String>,
    /// Las columnas del `SELECT`, **sin citar y en su orden**.
    ///
    /// Hace falta porque la proyección se **de-duplica**: dos propiedades de la
    /// misma columna la piden una vez, así que la posición *i* del resultado ya
    /// no es la propiedad *i* de la petición. Un driver que leyera por índice
    /// sin mirar esto devolvería las columnas corridas —y solo cuando dos
    /// propiedades compartieran columna, que es el caso que nadie prueba.
    pub columnas: Vec<String>,
}

impl Cita {
    /// Cita un nombre, o dice por qué no se puede.
    pub fn ident(self, s: &str) -> Result<String, String> {
        match self {
            Cita::PorPartes(d) => Ok(s
                .split('.')
                .map(|p| format!("{d}{}{d}", p.replace(d, &format!("{d}{d}"))))
                .collect::<Vec<_>>()
                .join(".")),
            Cita::Entero(d) => {
                if s.contains(d) {
                    return Err(format!(
                        "`{s}` lleva un `{d}`, y este dialecto no tiene forma de escaparlo dentro \
                         de un identificador citado. No se traduce"
                    ));
                }
                Ok(format!("{d}{s}{d}"))
            }
        }
    }
}

impl Dialecto {
    /// **Si este dialecto necesita saber el tipo de cada columna** antes de
    /// traducir.
    ///
    /// No es una preferencia: GoogleSQL no coacciona un `STRING` a un `INT64`,
    /// así que su parámetro tiene que decir de qué tipo es; el de PostgreSQL
    /// llega como texto y lo coacciona el servidor.
    ///
    /// Se pregunta en vez de saberse de memoria. Los dos drivers lo sabían cada
    /// uno por su cuenta —uno pasaba un mapa vacío y el otro hacía una consulta
    /// de más— y el tercero tendría que acordarse.
    pub const fn exige_tipos(&self) -> bool {
        matches!(self.marca, Marca::ConNombre(_))
    }
}

/// **La preparación entera**, para que ningún driver tenga que acordarse.
///
/// `traer_tipos` se invoca **solo si el dialecto lo exige**, así que un driver
/// posicional puede pasar una consulta cara sin pagarla, y uno tipado no puede
/// olvidarse de ella.
pub fn preparar<F>(
    p: &Peticion,
    d: &Dialecto,
    objeto: &str,
    traer_tipos: F,
) -> Result<Consulta, String>
where
    F: FnOnce() -> Result<BTreeMap<String, String>, String>,
{
    let tipos = if d.exige_tipos() {
        traer_tipos()?
    } else {
        BTreeMap::new()
    };
    consulta(p, d, objeto, &tipos)
}

/// El nombre base de un tipo: `NUMERIC(10, 2)` es `NUMERIC`.
///
/// La parametrización del tipo no viaja con el parámetro, y quitarla aquí evita
/// que cada dialecto tipado la quite por su cuenta.
fn tipo_base(t: &str) -> &str {
    t.split_once('(').map_or(t, |(b, _)| b).trim()
}

/// El estado de la traducción: acumula parámetros y sabe marcarlos.
struct Marcador<'a> {
    marca: Marca,
    tipos: &'a BTreeMap<String, String>,
    parametros: Vec<String>,
}

impl Marcador<'_> {
    /// Añade un valor y devuelve su marca dentro del texto.
    ///
    /// **Un tipo que no llega no se sustituye por texto: se rechaza la petición
    /// entera.** Emitirlo como `STRING` compilaría y rompería el orden de un
    /// cursor entero sin fallar, que es la dirección insegura.
    fn marcar(&mut self, columna: &str, valor: &str) -> Result<String, String> {
        match self.marca {
            Marca::Posicional(p) => {
                self.parametros.push(valor.to_string());
                Ok(format!("{p}{}", self.parametros.len()))
            }
            Marca::ConNombre(p) => {
                let t = self.tipos.get(columna).ok_or_else(|| {
                    format!(
                        "no se sabe el tipo de `{columna}`, y este dialecto exige tiparlo. Sin él \
                         el parámetro saldría como texto: comparar texto con un entero es un error \
                         de tipos, y compararlo COMO texto rompería el orden. No se adivina"
                    )
                })?;
                let n = format!("p{}", self.parametros.len());
                self.parametros
                    .push(format!("{n}:{}:{valor}", tipo_base(t)));
                Ok(format!("{p}{n}"))
            }
        }
    }
}

/// **La forma.** Determinista, pura y sin servidor — que es lo que hace que
/// *«el SQL emitido contiene solo las columnas proyectadas»* sea un aserto y no
/// una promesa. Un aserto que exigiera un servidor no se ejecutaría nunca en la
/// suite.
///
/// `objeto` llega **ya cualificado por el driver**: BigQuery le antepone su
/// proyecto y PostgreSQL no tiene nada que anteponer. Cualificar aquí obligaría
/// a esta crate a saber cómo se nombra una tabla en cada producto, que es
/// exactamente lo que se está sacando de ella.
///
/// `tipos` es propiedad → tipo de la fuente, y solo lo mira un dialecto con
/// [`Marca::ConNombre`]. Los demás pueden pasar un mapa vacío.
pub fn consulta(
    p: &Peticion,
    d: &Dialecto,
    objeto: &str,
    tipos: &BTreeMap<String, String>,
) -> Result<Consulta, String> {
    // **Sin repetir.** Dos propiedades pueden salir de la misma columna, y un
    // motor nombra cada campo del resultado por su columna: pedirla dos veces
    // devolvería un resultado con el nombre repetido y la fila se leería mal.
    // Se piden las distintas y la fila se arma después por nombre, así que las
    // dos propiedades reciben el mismo valor, que es lo correcto.
    let mut columnas: Vec<String> = Vec::new();
    let mut citadas: Vec<String> = Vec::new();
    for (_, c) in &p.proyeccion {
        if columnas.iter().any(|x| x == c) {
            continue;
        }
        citadas.push(d.cita.ident(c)?);
        columnas.push(c.clone());
    }
    let mut texto = format!(
        "SELECT {} FROM {}",
        citadas.join(", "),
        d.cita.ident(objeto)?
    );

    let mut m = Marcador {
        marca: d.marca,
        tipos,
        parametros: Vec::new(),
    };
    let mut condiciones: Vec<String> = Vec::new();

    // El recorte por clave. Una clave compuesta es una tupla: se comparan sus
    // columnas, no una concatenación —que exigiría un separador que nadie
    // declaró.
    if !p.claves.is_empty() && !p.clave_columnas.is_empty() {
        condiciones.push(match d.recorte {
            Recorte::TuplaEnLista => {
                let cols: Vec<String> = p
                    .clave_columnas
                    .iter()
                    .map(|c| d.cita.ident(c))
                    .collect::<Result<_, _>>()?;
                let mut tuplas: Vec<String> = Vec::new();
                for t in &p.claves {
                    let marcas: Vec<String> = p
                        .clave_columnas
                        .iter()
                        .zip(t)
                        .map(|(col, v)| m.marcar(col, v))
                        .collect::<Result<_, _>>()?;
                    tuplas.push(format!("({})", marcas.join(", ")));
                }
                format!("({}) IN ({})", cols.join(", "), tuplas.join(", "))
            }
            Recorte::DisyuncionDeConjunciones => {
                let mut tuplas: Vec<String> = Vec::new();
                for t in &p.claves {
                    let mut iguales: Vec<String> = Vec::new();
                    for (col, v) in p.clave_columnas.iter().zip(t) {
                        iguales.push(format!("{} = {}", d.cita.ident(col)?, m.marcar(col, v)?));
                    }
                    tuplas.push(format!("({})", iguales.join(" AND ")));
                }
                format!("({})", tuplas.join(" OR "))
            }
        });
    }

    for (col, op, valor) in &p.filtros {
        let simbolo = match op.as_str() {
            "gt" => ">",
            _ => "=",
        };
        condiciones.push(format!(
            "{} {simbolo} {}",
            d.cita.ident(col)?,
            m.marcar(col, valor)?
        ));
    }

    // **El rango, cuando va sobre una columna.** `start` exclusivo y `end`
    // inclusivo, la convención de Iceberg: dos refrescos encadenados no repiten
    // ni se saltan el borde. Sale como dos condiciones más, y eso es lo que
    // dice que la petición estaba cortada por el sitio correcto — para el SQL,
    // un rango es un `WHERE`.
    if let Some(cursor) = p.cursor.as_deref() {
        if let Some(s) = &p.start {
            condiciones.push(format!(
                "{} > {}",
                d.cita.ident(cursor)?,
                m.marcar(cursor, s)?
            ));
        }
        if let Some(e) = &p.end {
            condiciones.push(format!(
                "{} <= {}",
                d.cita.ident(cursor)?,
                m.marcar(cursor, e)?
            ));
        }
    }

    if !condiciones.is_empty() {
        texto.push_str(" WHERE ");
        texto.push_str(&condiciones.join(" AND "));
    }
    Ok(Consulta {
        texto,
        parametros: m.parametros,
        columnas,
    })
}

/// Los dos dialectos que hay, como constantes.
///
/// Nacen **dos y no uno**: una forma extraída de un solo caso es el caso con
/// otro nombre. Estos dos se escribieron por separado, y que la forma los
/// cubriera a los dos es la única evidencia de que es forma y no PostgreSQL
/// disfrazado.
pub mod dialectos {
    use super::{Cita, Dialecto, Marca, Recorte};

    pub const POSTGRES: Dialecto = Dialecto {
        cita: Cita::PorPartes('"'),
        marca: Marca::Posicional('$'),
        recorte: Recorte::TuplaEnLista,
    };

    pub const BIGQUERY: Dialecto = Dialecto {
        cita: Cita::Entero('`'),
        marca: Marca::ConNombre('@'),
        recorte: Recorte::DisyuncionDeConjunciones,
    };
}

#[cfg(test)]
mod tests {
    use super::dialectos::{BIGQUERY, POSTGRES};
    use super::*;

    fn tipos() -> BTreeMap<String, String> {
        [
            ("employee_id", "INT64"),
            ("base_pay", "NUMERIC(10, 2)"),
            ("cost_center", "STRING"),
            ("actualizado", "TIMESTAMP"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    fn peticion() -> Peticion {
        Peticion {
            url: "x://y".into(),
            objeto: "public.employees".into(),
            proyeccion: vec![
                ("baseSalary".into(), "base_pay".into()),
                ("employeeId".into(), "employee_id".into()),
            ],
            clave_columnas: vec!["employee_id".into()],
            claves: vec![vec!["7".into()], vec!["9".into()]],
            filtros: vec![("cost_center".into(), "eq".into(), "finanzas".into())],
            ..Default::default()
        }
    }

    /// **El aserto que sostiene la máscara, y ahora se escribe una vez.**
    ///
    /// Una propiedad `redact` no está en el plan, luego no está en la petición,
    /// luego no puede estar en el SQL. Se comprueba **en los dos dialectos**,
    /// porque una garantía que solo se prueba en uno es la que falta en el
    /// otro.
    #[test]
    fn ningun_dialecto_pide_una_columna_fuera_de_la_proyeccion() {
        for (d, esperado) in [
            (POSTGRES, "SELECT \"base_pay\", \"employee_id\" FROM \"public\".\"employees\""),
            (BIGQUERY, "SELECT `base_pay`, `employee_id` FROM `public.employees`"),
        ] {
            let c = consulta(&peticion(), &d, "public.employees", &tipos()).expect("traduce");
            assert!(c.texto.starts_with(esperado), "{}", c.texto);
            assert!(!c.texto.contains('*'), "{}", c.texto);
            assert!(!c.texto.contains("national_id"), "{}", c.texto);
        }
    }

    /// Y ningún valor se interpola, en ninguno de los dos.
    #[test]
    fn ningun_dialecto_interpola_un_valor_en_el_texto() {
        for d in [POSTGRES, BIGQUERY] {
            let c = consulta(&peticion(), &d, "public.employees", &tipos()).expect("traduce");
            assert!(!c.texto.contains("finanzas"), "{}", c.texto);
            assert!(!c.texto.contains('7'), "{}", c.texto);
            assert_eq!(c.parametros.len(), 3, "{:?}", c.parametros);
        }
    }

    /// La marca posicional no tipa; la nombrada sí, y con el tipo de su columna.
    #[test]
    fn cada_marca_emite_sus_parametros_como_los_espera_su_transporte() {
        let pg = consulta(&peticion(), &POSTGRES, "public.employees", &tipos()).expect("pg");
        assert_eq!(pg.parametros, vec!["7", "9", "finanzas"]);
        assert!(pg.texto.contains("($1), ($2)"), "{}", pg.texto);

        let bq = consulta(&peticion(), &BIGQUERY, "acme.hr.employees", &tipos()).expect("bq");
        assert_eq!(
            bq.parametros,
            vec!["p0:INT64:7", "p1:INT64:9", "p2:STRING:finanzas"]
        );
        assert!(bq.texto.contains("@p0"), "{}", bq.texto);
    }

    /// Cada recorte por clave se expresa como el suyo, y una clave compuesta se
    /// compara **columna a columna** en los dos: la tupla sigue siendo tupla.
    #[test]
    fn una_clave_compuesta_se_compara_columna_a_columna_en_los_dos() {
        let mut p = peticion();
        p.clave_columnas = vec!["employee_id".into(), "cost_center".into()];
        p.claves = vec![vec!["7".into(), "finanzas".into()]];
        p.filtros.clear();

        let pg = consulta(&p, &POSTGRES, "t", &tipos()).expect("pg");
        assert!(
            pg.texto
                .contains("(\"employee_id\", \"cost_center\") IN (($1, $2))"),
            "{}",
            pg.texto
        );
        let bq = consulta(&p, &BIGQUERY, "t", &tipos()).expect("bq");
        assert!(
            bq.texto
                .contains("((`employee_id` = @p0 AND `cost_center` = @p1))"),
            "{}",
            bq.texto
        );
    }

    /// El rango: `start` exclusivo, `end` inclusivo, dos condiciones más, en
    /// los dos dialectos.
    #[test]
    fn el_rango_sale_como_dos_condiciones_en_los_dos() {
        let mut p = peticion();
        p.claves.clear();
        p.filtros.clear();
        p.cursor = Some("actualizado".into());
        p.start = Some("2026-01-01".into());
        p.end = Some("2026-02-01".into());

        let pg = consulta(&p, &POSTGRES, "t", &tipos()).expect("pg");
        assert!(
            pg.texto
                .ends_with("WHERE \"actualizado\" > $1 AND \"actualizado\" <= $2"),
            "{}",
            pg.texto
        );
        let bq = consulta(&p, &BIGQUERY, "t", &tipos()).expect("bq");
        assert!(
            bq.texto
                .ends_with("WHERE `actualizado` > @p0 AND `actualizado` <= @p1"),
            "{}",
            bq.texto
        );
        assert!(bq.parametros[0].starts_with("p0:TIMESTAMP:"), "{bq:?}");
    }

    /// Un tipo que no llega **rechaza la petición** en el dialecto que tipa, y
    /// no afecta al que no.
    #[test]
    fn una_columna_sin_tipo_solo_rompe_al_dialecto_que_tipa() {
        let mut t = tipos();
        t.remove("cost_center");
        assert!(consulta(&peticion(), &POSTGRES, "t", &t).is_ok());
        let e = consulta(&peticion(), &BIGQUERY, "t", &t).expect_err("se niega");
        assert!(e.contains("cost_center"), "{e}");
        assert!(e.contains("No se adivina"), "{e}");
    }

    /// El delimitador dentro de un identificador: uno lo **dobla** y el otro
    /// **rechaza**, porque su dialecto no tiene escape.
    #[test]
    fn el_delimitador_dentro_se_dobla_o_se_rechaza_segun_el_dialecto() {
        assert_eq!(
            Cita::PorPartes('"').ident("ma\"la").as_deref(),
            Ok("\"ma\"\"la\"")
        );
        let e = Cita::Entero('`').ident("ma`la").expect_err("se niega");
        assert!(e.contains("no tiene forma de escaparlo"), "{e}");
    }

    /// **La forma traduce exactamente los operadores que una petición admite.**
    ///
    /// Son dos listas en dos crates —`ore-driver` no puede depender de este,
    /// que depende de él— y esta prueba es la que impide que se separen. Si
    /// divergieran, la que sobra sería la peligrosa: un operador que la
    /// petición admite y la forma no traduce se caería del `WHERE` y la
    /// consulta devolvería más filas de las pedidas.
    #[test]
    fn la_forma_traduce_los_mismos_operadores_que_la_peticion_admite() {
        let mut p = peticion();
        p.claves.clear();
        for op in ore_driver::OPERADORES {
            p.filtros = vec![("cost_center".into(), (*op).into(), "x".into())];
            let c = consulta(&p, &POSTGRES, "t", &tipos()).expect("traduce");
            let simbolo = if *op == "gt" { " > " } else { " = " };
            assert!(
                c.texto.contains(&format!("\"cost_center\"{simbolo}$1")),
                "`{op}` no se traduce: {}",
                c.texto
            );
        }
    }

    /// Y quién necesita los tipos se **pregunta**, no se sabe de memoria.
    #[test]
    fn solo_el_dialecto_que_marca_con_nombre_exige_tipos() {
        assert!(!POSTGRES.exige_tipos());
        assert!(BIGQUERY.exige_tipos());

        // `preparar` no invoca la consulta cara si el dialecto no la necesita.
        let mut llamado = false;
        let c = preparar(&peticion(), &POSTGRES, "t", || {
            llamado = true;
            Ok(BTreeMap::new())
        });
        assert!(c.is_ok() && !llamado, "el dialecto posicional no pide tipos");
    }

    /// Sin condiciones no hay `WHERE`. Es la mitad de la forma que ninguno de
    /// los dos ficheros originales probaba, y salió al juntarlos.
    #[test]
    fn sin_condiciones_no_hay_where() {
        let mut p = peticion();
        p.claves.clear();
        p.filtros.clear();
        for d in [POSTGRES, BIGQUERY] {
            let c = consulta(&p, &d, "t", &tipos()).expect("traduce");
            assert!(!c.texto.contains("WHERE"), "{}", c.texto);
            assert!(c.parametros.is_empty(), "{:?}", c.parametros);
        }
    }

    /// Dos propiedades de la misma columna se piden **una vez**: un motor
    /// nombra cada campo del resultado por su columna, y pedirla dos veces
    /// devolvería un nombre repetido.
    ///
    /// Y por eso `columnas` sale también: con la de-duplicación, la posición
    /// *i* del resultado ya no es la propiedad *i* de la petición, y un driver
    /// que leyera por índice devolvería las columnas corridas. `ore-read-postgres`
    /// leía por índice y funcionaba porque su traductor **no** de-duplicaba —
    /// migrarlo sin esto habría cambiado su resultado en silencio.
    #[test]
    fn una_columna_pedida_por_dos_propiedades_sale_una_vez_y_se_dice_en_que_orden() {
        let mut p = peticion();
        p.proyeccion = vec![
            ("a".into(), "base_pay".into()),
            ("b".into(), "base_pay".into()),
            ("c".into(), "cost_center".into()),
        ];
        p.claves.clear();
        p.filtros.clear();
        let c = consulta(&p, &POSTGRES, "t", &tipos()).expect("traduce");
        assert!(
            c.texto.starts_with("SELECT \"base_pay\", \"cost_center\" FROM"),
            "{}",
            c.texto
        );
        assert_eq!(c.columnas, vec!["base_pay", "cost_center"]);
    }
}
