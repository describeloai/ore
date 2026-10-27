//! **El lote dorado de Postgres** (ORE 0075, fase 4): lo que `Col` produce hoy, fijado antes de tocar cómo se arma.
//!
//! Cada columna recibe los textos tal como los manda el protocolo simple de Postgres (`t`/`f`, `infinity`, `\x…`,
//! `{1,NULL}`) por el mismo camino que `por_lotes` (`Col::poner`, y los bytes tal cual en una geometría). Se vuelcan el
//! tipo, lo que dijo cada celda (valor, aviso o error), lo que quedó en la columna y su largo tras un error. Se compara
//! con `tests/dorado/lote.txt`; si un cambio mueve una línea, falla.
//!
//! Regenerar (solo con una razón dicha en el commit): `ORE_ACTUALIZAR=1 cargo test -p ore-read-postgres dorado`.

use super::*;
use std::fmt::Write as _;

/// Una columna de Postgres en construcción: lo que `por_lotes` hace con una celda, en un lote de una columna.
pub(super) struct Una {
    forma: Forma,
    lote: RecordBatchBuilder,
}

impl Una {
    pub(super) fn de(forma: &Forma) -> Una {
        let c = Columna::de_forma("c", ore(forma), "");
        Una {
            forma: forma.clone(),
            lote: RecordBatchBuilder::nuevo(&[c]).unwrap(),
        }
    }

    pub(super) fn poner(&mut self, v: Option<&str>) -> Result<Option<Aviso>, String> {
        let r = poner(&mut self.lote.col(0), &self.forma, v);
        if r.is_ok() {
            self.lote.fila_escrita();
        }
        r
    }

    fn bytes(&mut self, v: &[u8]) {
        self.lote.col(0).bytes(v).unwrap();
        self.lote.fila_escrita();
    }

    pub(super) fn largo(&self) -> usize {
        self.lote.celdas(0)
    }

    pub(super) fn fin(&mut self) -> arrow_array::ArrayRef {
        self.lote.finish().unwrap().column(0).clone()
    }
}

/// Una celda: su texto (`None` es nulo).
type Celda = Option<&'static str>;

/// El tipo con lo que su `Debug` calla: cómo se llama el elemento de una lista (`item` aquí, `element` en el lago).
fn tipo(t: &DataType) -> String {
    match t {
        DataType::List(f) => format!("{t:?} elemento `{}`", f.name()),
        t => format!("{t:?}"),
    }
}

/// Vuelca una columna: su forma, su tipo, y por celda el resultado de `poner`.
fn columna(nombre: &str, forma: Forma, celdas: &[Celda]) -> String {
    let mut s = String::new();
    writeln!(s, "== {nombre}: {forma:?} -> {:?}", tipo_arrow(&forma)).unwrap();
    let mut col = Una::de(&forma);
    for (i, c) in celdas.iter().enumerate() {
        let r = col.poner(*c);
        writeln!(
            s,
            "  {i} {:<28} -> {r:?} (lleva {})",
            format!("{c:?}"),
            col.largo()
        )
        .unwrap();
    }
    let a = col.fin();
    writeln!(s, "  tipo {}", tipo(a.data_type())).unwrap();
    writeln!(s, "  nulos {} de {}", a.null_count(), a.len()).unwrap();
    for l in format!("{a:?}").lines() {
        writeln!(s, "  | {l}").unwrap();
    }
    s
}

/// La geometría entra con sus bytes, sin pasar por `poner`.
fn geo(celdas: &[Option<&'static [u8]>]) -> String {
    let forma = Forma::Geo;
    let mut s = String::new();
    writeln!(s, "== geo: {forma:?} -> {:?}", tipo_arrow(&forma)).unwrap();
    let mut col = Una::de(&forma);
    for c in celdas {
        match c {
            Some(v) => col.bytes(v),
            None => {
                col.poner(None).unwrap();
            }
        }
    }
    let a = col.fin();
    writeln!(s, "  tipo {:?}", a.data_type()).unwrap();
    for l in format!("{a:?}").lines() {
        writeln!(s, "  | {l}").unwrap();
    }
    s
}

fn fijado(caso: &str, actual: &str) {
    let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/dorado")
        .join(format!("{caso}.txt"));
    if std::env::var_os("ORE_ACTUALIZAR").is_some() {
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(&ruta, actual).unwrap();
        return;
    }
    let esperado = std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("falta {}: {e}", ruta.display()))
        .replace("\r\n", "\n");
    assert_eq!(esperado, actual, "el caso `{caso}` ya no es el de antes");
}

#[test]
fn el_lote_de_postgres_es_el_de_siempre() {
    let esc = |f: Fisico| Forma::Escalar(Some(f));
    let lis = Forma::Lista;
    let dec = Fisico::Decimal {
        precision: 12,
        escala: 2,
    };
    let mut s = String::new();
    let mut c = |n: &str, f: Forma, celdas: &[Celda]| s.push_str(&columna(n, f, celdas));
    c(
        "entero",
        esc(Fisico::Entero),
        &[
            Some("42"),
            Some("-7"),
            Some("9223372036854775807"),
            None,
            Some("x"),
            Some(""),
        ],
    );
    c(
        "real",
        esc(Fisico::Real),
        &[
            Some("1.5"),
            Some("-0"),
            Some("NaN"),
            Some("Infinity"),
            Some("1e300"),
            Some("nada"),
        ],
    );
    c(
        "logico",
        esc(Fisico::Logico),
        &[Some("t"), Some("f"), Some("true"), None, Some("x")],
    );
    c(
        "decimal",
        esc(dec),
        &[
            Some("1234.56"),
            Some("-0.05"),
            Some("NaN"),
            Some("Infinity"),
            Some("-Infinity"),
            Some("1.239"),
            Some("doce"),
            None,
        ],
    );
    c(
        "fecha",
        esc(Fisico::Fecha),
        &[
            Some("2024-05-06"),
            Some("infinity"),
            Some("-infinity"),
            None,
            Some("2024-13-40"),
        ],
    );
    c(
        "hora",
        esc(Fisico::Hora),
        &[Some("10:11:12.5"), Some("00:00:00"), Some("25:00:00"), None],
    );
    c(
        "fecha_hora",
        esc(Fisico::FechaHora),
        &[
            Some("2026-10-26 10:11:12.123456"),
            Some("infinity"),
            Some("-infinity"),
            Some("2026-10-26 10:11:12"),
            None,
            Some("ayer"),
        ],
    );
    c(
        "instante",
        esc(Fisico::Instante),
        &[
            Some("2026-10-26 10:11:12.123456+00"),
            Some("2026-10-26 12:11:12+02"),
            Some("infinity"),
            Some("-infinity"),
            None,
            Some("ayer"),
        ],
    );
    c(
        "binario",
        esc(Fisico::Binario),
        &[
            Some("\\xdeadbeef"),
            Some("\\x"),
            Some("\\xzz"),
            None,
            Some("3q2+7w=="),
        ],
    );
    c(
        "texto",
        esc(Fisico::Texto),
        &[
            Some("ñandú"),
            Some(""),
            Some("NULL"),
            None,
            Some("t"),
            Some("infinity"),
        ],
    );
    c(
        "sin_tipo",
        Forma::Escalar(None),
        &[Some("{\"a\":1}"), None, Some("t")],
    );
    c(
        "lista_entero",
        lis(Fisico::Entero),
        &[
            Some("{1,2,NULL}"),
            None,
            Some("{}"),
            Some("[2:3]={7,8}"),
            Some("{x}"),
            Some("{{1,2},{3,4}}"),
            Some("nada"),
            Some("{4}"),
        ],
    );
    c(
        "lista_texto",
        lis(Fisico::Texto),
        &[
            Some(r#"{"a,b","x\"y",NULL,"NULL"}"#),
            Some("{t,infinity}"),
            None,
        ],
    );
    c(
        "lista_logico",
        lis(Fisico::Logico),
        &[Some("{t,f,NULL}"), Some("{x}")],
    );
    c(
        "lista_fecha",
        lis(Fisico::Fecha),
        &[Some("{2024-05-06,infinity}"), Some("{}")],
    );
    c(
        "lista_decimal",
        lis(dec),
        &[Some("{1.5,NaN,NULL}"), Some("{doce}")],
    );
    c(
        "lista_instante",
        lis(Fisico::Instante),
        &[Some("{\"2026-10-26 10:11:12+00\",-infinity}")],
    );
    s.push_str(&geo(&[Some(&[1, 1, 0, 0, 0]), None, Some(&[])]));
    fijado("lote", &s);
}

/// El esquema del lote que arma `por_lotes` es el del `Plan`: aquí, el de cada forma.
#[test]
fn el_tipo_de_cada_forma_es_el_de_siempre() {
    let mut s = String::new();
    let fisicos = [
        Fisico::Entero,
        Fisico::Real,
        Fisico::Logico,
        Fisico::Decimal {
            precision: 38,
            escala: 10,
        },
        Fisico::Fecha,
        Fisico::Hora,
        Fisico::FechaHora,
        Fisico::Instante,
        Fisico::Texto,
        Fisico::Binario,
    ];
    for f in fisicos {
        writeln!(
            s,
            "{f:?}: {} | lista {}",
            tipo(&tipo_arrow(&Forma::Escalar(Some(f)))),
            tipo(&tipo_arrow(&Forma::Lista(f)))
        )
        .unwrap();
    }
    writeln!(s, "sin tipo: {:?}", tipo_arrow(&Forma::Escalar(None))).unwrap();
    writeln!(s, "geo: {:?}", tipo_arrow(&Forma::Geo)).unwrap();
    fijado("tipos", &s);
}
