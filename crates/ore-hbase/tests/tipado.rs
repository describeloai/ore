//! **Los tipos de punta a punta** contra el cluster simulado (ficha 0072, N4): una tabla cuya clave es un `int` de Java
//! (`i32be`) de −200 a 199 y cuyas celdas son `int`, `BigDecimal`, `boolean`, `long` de milisegundos, `double` y
//! texto, todo codificado como lo hace `org.apache.hadoop.hbase.util.Bytes`.
//!
//! El orden de las claves en HBase es el de los **bytes**: los positivos (`0x00…`) van antes que los negativos
//! (`0x80…`). Las cuatro regiones del simulador caen sobre ese orden —la 0 y la 1 son 0…199, la 2 y la 3 son −200…−1—,
//! asi que un rango que cruza el cero toca regiones que no son contiguas en valor. Eso es lo que se mide.

#[path = "../../ore-hbase-rpc/tests/comun/simulado.rs"]
mod simulado;

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Decimal128Type, Float64Type, Int64Type, TimestampMicrosecondType,
};
use arrow_ipc::reader::StreamReader;
use ore_base::tipos::{Fisico, Valor};
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Filtro, Peticion};
use ore_hbase::codificacion::{Codificacion, codificar};
use ore_hbase::lectura::{Hbase, Token};
use simulado::{Accion, Celdas, Llamada, Mundo, Sim};

const BASE_MS: i64 = 1_700_000_000_000; // 2023-11-14 22:13:20 UTC
const DIA_0: i32 = 19_000;

fn clave(n: i32) -> Vec<u8> {
    n.to_be_bytes().to_vec()
}

fn n_de(clave: &[u8]) -> i32 {
    i32::from_be_bytes(clave.try_into().unwrap())
}

fn decimal(n: i32) -> Vec<u8> {
    codificar(
        Codificacion::Decimal,
        &Fisico::Decimal {
            precision: 12,
            escala: 2,
        },
        &Valor::Decimal(i128::from(n) * 100 + 5),
    )
    .unwrap()
}

/// Las celdas de la fila `n`.
fn celdas() -> Celdas {
    Arc::new(|k| {
        let n = n_de(k);
        let c = |q: &str, v: Vec<u8>| (b"f".to_vec(), q.as_bytes().to_vec(), v);
        vec![
            c("edad", (n * 2).to_be_bytes().to_vec()),
            c("total", decimal(n)),
            c("ok", vec![if n % 2 == 0 { 0xFF } else { 0x00 }]),
            c("ts", (BASE_MS + i64::from(n) * 1000).to_be_bytes().to_vec()),
            c("x", (f64::from(n) * 0.5).to_be_bytes().to_vec()),
            c("nombre", format!("n{n}").into_bytes()),
            c(
                "fecha",
                ((i64::from(DIA_0) + i64::from(n)) * 86_400_000)
                    .to_be_bytes()
                    .to_vec(),
            ),
        ]
    })
}

fn nada(_: &Llamada, _: &mut Mundo) -> Accion {
    Accion::Normal
}

fn sim() -> Sim {
    let mut claves: Vec<Vec<u8>> = (-200..200).map(clave).collect();
    claves.sort(); // el orden de HBase: por bytes
    Sim::con_filas(claves, 4, celdas(), nada)
}

fn url(s: &Sim) -> String {
    format!(
        "hbase://{}?simple=acepto&traduccion=rs.sim:16020%3D{}&paralelo=1",
        s.zk, s.rpc
    )
}

const TIPOS: &[(&str, &str, &str)] = &[
    ("_clave", "Integer", "i32be"),
    ("f:edad", "Integer", "i32be"),
    ("f:total", "Decimal<12, 2>", "decimal"),
    ("f:ok", "Boolean", "bool"),
    ("f:ts", "DateTimeTz", "epoch-ms"),
    ("f:x", "Float", "f64be"),
    ("f:fecha", "Date", "epoch-ms"),
    ("f:nombre", "String", "utf8"),
];

fn peticion(s: &Sim, cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
    Peticion {
        url: url(s),
        objeto: "default.t".into(),
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        filtros,
        tipos: TIPOS
            .iter()
            .map(|(c, t, _)| (c.to_string(), t.to_string()))
            .collect(),
        codificaciones: TIPOS
            .iter()
            .map(|(c, _, e)| (c.to_string(), e.to_string()))
            .collect(),
        ..Peticion::default()
    }
}

fn leer(p: &Peticion) -> (Result<u64, Fallo>, Vec<u8>) {
    let mut s = Hbase::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let r = Hbase::leer(&mut s, &Token::default(), p, &mut salida);
    (r, salida)
}

/// Las claves (como enteros) que salieron, ordenadas.
fn claves_leidas(p: &Peticion) -> Vec<i32> {
    let (r, bytes) = leer(p);
    r.unwrap();
    let mut v = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let lote = lote.unwrap();
        let col = lote.column(0).as_primitive::<Int64Type>();
        v.extend((0..lote.num_rows()).map(|i| col.value(i) as i32));
    }
    v.sort_unstable();
    v
}

fn rango(desde: i32, hasta_incl: i32) -> Vec<i32> {
    (desde..=hasta_incl).collect()
}

#[test]
fn cada_tipo_sale_con_su_valor_y_su_tipo_de_arrow() {
    let s = sim();
    let cols = [
        "_clave", "f:edad", "f:total", "f:ok", "f:ts", "f:x", "f:fecha", "f:nombre",
    ];
    let (r, bytes) = leer(&peticion(&s, &cols, vec![]));
    assert_eq!(r.unwrap(), 400);
    let lector = StreamReader::try_new(Cursor::new(bytes), None).unwrap();
    let tipos: Vec<String> = lector
        .schema()
        .fields()
        .iter()
        .map(|f| f.data_type().to_string())
        .collect();
    assert_eq!(
        tipos,
        [
            "Int64",
            "Int64",
            "Decimal128(12, 2)",
            "Boolean",
            "Timestamp(µs, \"+00:00\")",
            "Float64",
            "Date32",
            "Utf8"
        ]
    );
    let mut vistas = 0;
    for lote in lector {
        let l = lote.unwrap();
        for i in 0..l.num_rows() {
            let n = l.column(0).as_primitive::<Int64Type>().value(i) as i32;
            assert_eq!(
                l.column(1).as_primitive::<Int64Type>().value(i),
                i64::from(n) * 2,
                "edad de {n}"
            );
            assert_eq!(
                l.column(2).as_primitive::<Decimal128Type>().value(i),
                i128::from(n) * 100 + 5,
                "total de {n}"
            );
            assert_eq!(l.column(3).as_boolean().value(i), n % 2 == 0, "ok de {n}");
            assert_eq!(
                l.column(4)
                    .as_primitive::<TimestampMicrosecondType>()
                    .value(i),
                (BASE_MS + i64::from(n) * 1000) * 1000,
                "ts de {n}"
            );
            assert_eq!(
                l.column(5).as_primitive::<Float64Type>().value(i),
                f64::from(n) * 0.5,
                "x de {n}"
            );
            assert_eq!(
                l.column(6).as_primitive::<Date32Type>().value(i),
                DIA_0 + n,
                "fecha de {n}"
            );
            assert_eq!(l.column(7).as_string::<i32>().value(i), format!("n{n}"));
            vistas += 1;
        }
    }
    assert_eq!(vistas, 400);
}

#[test]
fn un_rango_sobre_una_clave_entera_cruza_el_cero_y_toca_las_regiones_de_los_dos_lados() {
    let s = sim();
    // id < 5: los no negativos 0..4 (region 0) y todos los negativos (regiones 2 y 3). La region 1 va de 0x64… a
    // 0xFFFFFF38 —abarca el hueco entero—, asi que el tramo de los negativos (0x80000000…) entra en ella aunque no
    // tenga filas alli: un scanner vacio. Sin conocer los datos es lo correcto, y el resultado es exacto.
    let p = peticion(&s, &["_clave"], vec![Filtro::uno("_clave", "lt", "5")]);
    assert_eq!(claves_leidas(&p), rango(-200, 4));
    assert_eq!(s.n_registros("abre"), 4, "{:?}", s.registro());
}

#[test]
fn un_rango_de_tres_a_cada_lado_del_cero_son_dos_tramos_y_dos_regiones() {
    let s = sim();
    let p = peticion(
        &s,
        &["_clave"],
        vec![
            Filtro::uno("_clave", "ge", "-3"),
            Filtro::uno("_clave", "le", "3"),
        ],
    );
    assert_eq!(claves_leidas(&p), rango(-3, 3));
    assert_eq!(s.n_registros("abre"), 2, "{:?}", s.registro());
}

#[test]
fn un_in_sobre_una_clave_entera_lee_solo_esas_filas_aunque_haya_negativas_y_una_que_no_existe() {
    let s = sim();
    let lista = Filtro {
        columna: "_clave".into(),
        operador: "in".into(),
        valor: ore_driver::Valor::Lista(vec![
            "-1".into(),
            "0".into(),
            "199".into(),
            "250".into(),
            "-1".into(),
        ]),
    };
    assert_eq!(
        claves_leidas(&peticion(&s, &["_clave"], vec![lista])),
        vec![-1, 0, 199]
    );
}

#[test]
fn eq_sobre_el_menor_y_el_mayor_de_los_negativos() {
    let s = sim();
    assert_eq!(
        claves_leidas(&peticion(
            &s,
            &["_clave"],
            vec![Filtro::uno("_clave", "eq", "-200")]
        )),
        vec![-200]
    );
    assert_eq!(
        claves_leidas(&peticion(
            &s,
            &["_clave"],
            vec![Filtro::uno("_clave", "eq", "-1")]
        )),
        vec![-1]
    );
}

#[test]
fn los_filtros_sobre_celdas_se_comparan_como_numeros_y_no_como_texto() {
    let s = sim();
    let con = |col: &str, op: &str, v: &str| {
        claves_leidas(&peticion(&s, &["_clave"], vec![Filtro::uno(col, op, v)]))
    };
    // edad = 2n > 100  ->  n > 50: como texto, "98" > "100" y saldrian mal
    assert_eq!(con("f:edad", "gt", "100"), rango(51, 199));
    // total = n*100 + 5 >= 10005  ->  n >= 100
    assert_eq!(con("f:total", "ge", "100.05"), rango(100, 199));
    // x = n / 2 < -90  ->  n < -180
    assert_eq!(con("f:x", "lt", "-90"), rango(-200, -181));
    // ts >= 2023-11-14 22:13:20 UTC  ->  n >= 0
    assert_eq!(con("f:ts", "ge", "2023-11-14 22:13:20+00"), rango(0, 199));
    // fecha = DIA_0 + n; 19000 = 2022-01-08
    assert_eq!(con("f:fecha", "eq", "2022-01-08"), vec![0]);
    assert_eq!(con("f:nombre", "eq", "n7"), vec![7]);
}

#[test]
fn un_booleano_y_un_decimal_en_un_filtro_se_evaluan_sobre_el_valor() {
    let s = sim();
    let pares = claves_leidas(&peticion(
        &s,
        &["_clave"],
        vec![Filtro::uno("f:ok", "eq", "true")],
    ));
    assert_eq!(pares.len(), 200);
    assert!(pares.iter().all(|n| n % 2 == 0));
    let nulos = Filtro {
        columna: "f:no_existe".into(),
        operador: "isNull".into(),
        valor: ore_driver::Valor::Ninguno,
    };
    assert_eq!(
        claves_leidas(&peticion(&s, &["_clave"], vec![nulos])).len(),
        400
    );
}

#[test]
fn un_filtro_cuyo_valor_no_es_del_tipo_se_rechaza_como_operador() {
    let s = sim();
    let (r, bytes) = leer(&peticion(
        &s,
        &["_clave"],
        vec![Filtro::uno("f:edad", "gt", "muchos")],
    ));
    assert_eq!(r.unwrap_err().codigo, Codigo::Operador);
    assert!(bytes.is_empty());
}

#[test]
fn un_tipo_sin_su_codificacion_es_un_error_de_esquema_que_nombra_la_columna_y_no_abre_el_flujo() {
    let s = sim();
    let mut p = peticion(&s, &["_clave", "f:edad"], vec![]);
    p.codificaciones.retain(|(c, _)| c != "f:edad");
    let (r, bytes) = leer(&p);
    let e = r.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(
        e.mensaje.contains("f:edad") && e.mensaje.contains("i32be"),
        "{}",
        e.mensaje
    );
    assert!(bytes.is_empty());
}

#[test]
fn una_celda_cuyos_bytes_no_son_del_tipo_dice_columna_y_clave() {
    // la fila 7 tiene 3 bytes donde un `int` pide 4
    let malo: Celdas = Arc::new(|k| {
        let n = n_de(k);
        let edad = if n == 7 {
            vec![0, 0, 14]
        } else {
            (n * 2).to_be_bytes().to_vec()
        };
        vec![(b"f".to_vec(), b"edad".to_vec(), edad)]
    });
    let mut claves: Vec<Vec<u8>> = (0..20).map(clave).collect();
    claves.sort();
    let s = Sim::con_filas(claves, 1, malo, nada);
    let (r, _) = leer(&peticion(&s, &["_clave", "f:edad"], vec![]));
    let e = r.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(
        e.mensaje.contains("f:edad")
            && e.mensaje.contains("0x00000007")
            && e.mensaje.contains("4 byte"),
        "{}",
        e.mensaje
    );
}

#[test]
fn una_clave_con_bytes_que_no_son_del_ancho_del_tipo_tambien_se_dice() {
    // una clave de 3 bytes en una tabla de claves `i32be`: la fila esta mal, no el lector
    // (una fila sin celdas no existe en HBase: se le da una cualquiera)
    let sin_celdas: Celdas = Arc::new(|_| vec![(b"f".to_vec(), b"q".to_vec(), vec![])]);
    let s = Sim::con_filas(vec![vec![0, 0, 0, 2], vec![0, 0, 1]], 1, sin_celdas, nada);
    let (r, _) = leer(&peticion(&s, &["_clave"], vec![]));
    let e = r.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(
        e.mensaje.contains("_clave") && e.mensaje.contains("i32be"),
        "{}",
        e.mensaje
    );
}

#[test]
fn con_paralelo_la_misma_tabla_da_lo_mismo() {
    let s = sim();
    let mut p = peticion(
        &s,
        &["_clave", "f:edad"],
        vec![Filtro::uno("_clave", "lt", "5")],
    );
    p.url = p.url.replace("paralelo=1", "paralelo=4");
    assert_eq!(claves_leidas(&p), rango(-200, 4));
}
