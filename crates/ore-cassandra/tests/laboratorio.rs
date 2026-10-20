//! **El crate contra un Cassandra de verdad** (ficha 0070, M1).
//!
//! Se salta solo si no hay laboratorio. Con él (`cassandra:5` con autenticación y un
//! rol de **solo lectura**, sembrado como en X0 §3):
//!
//! ```text
//! CASSANDRA_LAB_URL='cassandra://ore_lector:lector@127.0.0.1:9043/ore_prueba?contacto=solo' \
//!     cargo test -p ore-cassandra --test laboratorio
//! ```
//!
//! Lo que comprueba es lo que X0 midió: el catálogo de `system_schema`, que el
//! traductor no pide nada que el servidor rechace (nunca `ALLOW FILTERING`), que lo
//! que el servidor no sabe filtrar lo filtra el conector y devuelve **exactamente**
//! lo pedido, y que los tipos llegan a Arrow sin perder cifras.

use arrow_array::cast::AsArray;
use ore_cassandra::lote::Constructor;
use ore_cassandra::traductor::{Modo, Opciones, Plan, traducir};
use ore_cassandra::transporte::{Conexion, Fila};
use ore_cassandra::{Rol, Tabla, Tipo};
use ore_driver::{Codigo, Filtro, Peticion, Valor};
use scylla::value::CqlValue;

fn url() -> Option<String> {
    let u = std::env::var("CASSANDRA_LAB_URL").ok();
    if u.is_none() {
        eprintln!("sin CASSANDRA_LAB_URL: se salta (no hay laboratorio)");
    }
    u
}

fn pet(tabla: &str, cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
    Peticion {
        objeto: format!("ore_prueba.{tabla}"),
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        filtros,
        ..Default::default()
    }
}

fn leer(c: &Conexion, p: &Peticion, o: &Opciones) -> (Plan, Tabla, Vec<Fila>) {
    let t = c
        .tabla("ore_prueba", p.objeto.rsplit('.').next().unwrap())
        .unwrap();
    let plan = traducir(p, &t, o).unwrap().plan;
    let filas = c.leer(&plan, &t, 4).unwrap();
    (plan, t, filas)
}

fn entero(f: &Fila, i: usize) -> i64 {
    match f[i].as_ref().unwrap() {
        CqlValue::Int(n) => i64::from(*n),
        CqlValue::BigInt(n) => *n,
        otro => panic!("no es un entero: {otro:?}"),
    }
}

fn ids(filas: &[Fila]) -> Vec<i64> {
    let mut v: Vec<i64> = filas.iter().map(|f| entero(f, 0)).collect();
    v.sort_unstable();
    v
}

fn nulo(op: &str, col: &str) -> Filtro {
    Filtro {
        columna: col.into(),
        operador: op.into(),
        valor: Valor::Ninguno,
    }
}

#[test]
fn el_catalogo_de_system_schema() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let (version, partidor) = c.servidor().unwrap();
    assert!(
        version.starts_with('5') || version.starts_with('4'),
        "{version}"
    );
    assert!(partidor.ends_with("Murmur3Partitioner"));

    let tablas = c.tablas("ore_prueba").unwrap();
    for t in ["tipos", "pedidos", "eventos", "estaticas", "contadores"] {
        assert!(tablas.contains(&t.to_string()), "{t} en {tablas:?}");
    }

    let t = c.tabla("ore_prueba", "pedidos").unwrap();
    assert_eq!(t.clave_primaria(), ["cliente", "dia", "hora", "id"]);
    assert_eq!(
        t.columna("hora").unwrap().rol,
        Rol::Clustering {
            posicion: 0,
            descendente: true
        }
    );
    assert_eq!(t.columna("total").unwrap().tipo, Tipo::DecimalLibre);

    let e = c.tabla("ore_prueba", "eventos").unwrap();
    assert_eq!(
        e.columna("actualizado").unwrap().indice,
        ore_cassandra::Indice::Sai
    );
    assert_eq!(
        e.columna("n").unwrap().indice,
        ore_cassandra::Indice::Clasico
    );

    let tipos = c.tabla("ore_prueba", "tipos").unwrap();
    assert_eq!(tipos.columna("t_varint").unwrap().tipo, Tipo::Varint);
    assert_eq!(tipos.columna("t_udt").unwrap().tipo, Tipo::Json);
    assert_eq!(tipos.columna("t_fl").unwrap().tipo, Tipo::Json);
    assert_eq!(tipos.columna("t_time").unwrap().tipo, Tipo::Hora);
    assert!(tipos.avisos.is_empty(), "{:?}", tipos.avisos);

    let e = c.tabla("ore_prueba", "no_existe").unwrap_err();
    assert_eq!(e.codigo, Codigo::Objeto, "{}", e.mensaje);
}

#[test]
fn un_recorrido_por_rangos_de_token_lee_toda_la_tabla() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let (plan, _, filas) = leer(
        &c,
        &pet("eventos", &["id", "cuerpo"], vec![]),
        &Opciones::default(),
    );
    assert_eq!(plan.modo, Modo::Recorrido);
    assert!(
        !plan.cql.to_ascii_uppercase().contains("ALLOW"),
        "{}",
        plan.cql
    );
    assert_eq!(ids(&filas), [1, 2, 3, 4]);
    assert!(c.rangos().unwrap().len() >= 16, "los vnodes del nodo");
}

#[test]
fn la_particion_fijada_es_una_consulta_por_particion() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let (plan, _, filas) = leer(
        &c,
        &pet("eventos", &["id"], vec![Filtro::uno("id", "eq", "2")]),
        &Opciones::default(),
    );
    assert_eq!(plan.modo, Modo::Particion);
    assert_eq!(ids(&filas), [2]);

    let en = Filtro {
        columna: "id".into(),
        operador: "in".into(),
        valor: Valor::Lista(vec!["1".into(), "3".into(), "99".into()]),
    };
    let (plan, _, filas) = leer(&c, &pet("eventos", &["id"], vec![en]), &Opciones::default());
    assert_eq!(plan.modo, Modo::Particion);
    assert_eq!(plan.juegos.len(), 3);
    assert_eq!(ids(&filas), [1, 3], "el 99 no existe");

    // Un `IN ()` no deja pasar nada, y no se le pregunta al servidor.
    let vacio = Filtro {
        columna: "id".into(),
        operador: "in".into(),
        valor: Valor::Lista(vec![]),
    };
    let (_, _, filas) = leer(
        &c,
        &pet("eventos", &["id"], vec![vacio]),
        &Opciones::default(),
    );
    assert!(filas.is_empty());

    // Un valor que no cabe en un `int` no devuelve nada, ni falla.
    let (_, _, filas) = leer(
        &c,
        &pet(
            "eventos",
            &["id"],
            vec![Filtro::uno("id", "eq", "99999999999")],
        ),
        &Opciones::default(),
    );
    assert!(filas.is_empty());
}

#[test]
fn el_cursor_sobre_un_sai_y_el_mismo_por_recorrido_dan_lo_mismo() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let mut p = pet("eventos", &["id"], vec![]);
    p.cursor = Some("actualizado".into());
    p.start = Some("2026-10-20T10:30:00Z".into());
    p.end = Some("2026-10-20T11:30:00Z".into());
    let (plan, _, por_indice) = leer(&c, &p, &Opciones::default());
    assert_eq!(plan.modo, Modo::Indice);
    assert_eq!(ids(&por_indice), [2]);
    let (plan, _, por_recorrido) = leer(
        &c,
        &p,
        &Opciones {
            preferir_recorrido: true,
        },
    );
    assert_eq!(plan.modo, Modo::Recorrido);
    assert_eq!(ids(&por_recorrido), [2]);

    // Un índice clásico: solo `=`.
    let (plan, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![Filtro::uno("n", "eq", "3")]),
        &Opciones::default(),
    );
    assert_eq!(plan.modo, Modo::Indice);
    assert_eq!(ids(&f), [3]);
}

#[test]
fn lo_que_el_servidor_no_sabe_lo_filtra_el_conector() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let o = Opciones::default();
    // `!=`: el servidor ni con ALLOW FILTERING. Un nulo no casa.
    let (plan, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![Filtro::uno("cuerpo", "neq", "a")]),
        &o,
    );
    assert_eq!(plan.modo, Modo::Recorrido);
    assert_eq!(ids(&f), [2, 3, 4]);
    // `IS NULL`: ni existe en CQL.
    let (_, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![nulo("isNull", "actualizado")]),
        &o,
    );
    assert_eq!(ids(&f), [4]);
    let (_, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![nulo("isNotNull", "actualizado")]),
        &o,
    );
    assert_eq!(ids(&f), [1, 2, 3]);
    // `LIKE` sin índice.
    let (_, _, f) = leer(
        &c,
        &pet(
            "eventos",
            &["id"],
            vec![Filtro::uno("cuerpo", "like", "sin%")],
        ),
        &o,
    );
    assert_eq!(ids(&f), [4]);
    // Una columna sin índice: igualdad y rango.
    let (_, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![Filtro::uno("cuerpo", "eq", "b")]),
        &o,
    );
    assert_eq!(ids(&f), [2]);
    let (_, _, f) = leer(
        &c,
        &pet("eventos", &["id"], vec![Filtro::uno("cuerpo", "gt", "a")]),
        &o,
    );
    assert_eq!(ids(&f), [2, 3, 4]);
    // `limit` lo pone el conector cuando el servidor no puede.
    let mut p = pet("eventos", &["id"], vec![Filtro::uno("cuerpo", "neq", "a")]);
    p.limit = Some(2);
    let (plan, _, f) = leer(&c, &p, &o);
    assert_eq!((plan.limit_servidor, f.len()), (None, 2));
}

#[test]
fn una_particion_solo_estatica_no_es_una_fila() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let (plan, _, filas) = leer(
        &c,
        &pet("estaticas", &["p", "s", "v"], vec![]),
        &Opciones::default(),
    );
    // 3 filas en X0 §3.1 (una con `c` a null): aquí son las 2 de la tabla.
    assert_eq!(plan.sin_clustering.as_deref(), Some("c"));
    assert_eq!(filas.len(), 2, "{filas:?}");
    assert!(filas.iter().all(|f| matches!(f[0], Some(CqlValue::Int(1)))));
}

#[test]
fn una_tabla_grande_se_lee_entera_y_en_paralelo() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let t = c.tabla("ore_prueba", "grande");
    let Ok(t) = t else {
        eprintln!("sin la tabla `grande`: se salta");
        return;
    };
    let p = pet("grande", &["id", "n"], vec![]);
    let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
    let t0 = std::time::Instant::now();
    let filas = c.leer(&plan, &t, 4).unwrap();
    eprintln!("{} filas en {:?}", filas.len(), t0.elapsed());
    assert_eq!(filas.len(), 300_000);
    let mut vistos: Vec<i64> = filas.iter().map(|f| entero(f, 0)).collect();
    vistos.sort_unstable();
    vistos.dedup();
    assert_eq!(
        vistos.len(),
        300_000,
        "ni una repetida ni una perdida entre rangos"
    );

    // Una igualdad sobre una columna sin índice: el conector filtra los 300 000.
    let p = pet("grande", &["id"], vec![Filtro::uno("n", "eq", "700")]);
    let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
    let filas = c.leer(&plan, &t, 4).unwrap();
    assert_eq!(ids(&filas), [100]);

    // El tiempo es del cliente.
    let mut p = pet("grande", &["id"], vec![]);
    p.timeout_ms = Some(1);
    let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
    let e = c.leer(&plan, &t, 4).unwrap_err();
    assert_eq!(e.codigo, Codigo::Tiempo, "{}", e.mensaje);
}

#[test]
fn todos_los_tipos_llegan_a_arrow_sin_perder_cifras() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let t = c.tabla("ore_prueba", "tipos").unwrap();
    // Todas las columnas, salvo las que no hacen falta para este caso.
    let cols: Vec<&str> = t.columnas.iter().map(|c| c.nombre.as_str()).collect();
    let p = pet(
        "tipos",
        &cols,
        vec![Filtro::uno(
            "id",
            "eq",
            "11111111-1111-1111-1111-111111111111",
        )],
    );
    let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
    assert_eq!(plan.modo, Modo::Particion);
    let filas = c.leer(&plan, &t, 1).unwrap();
    assert_eq!(filas.len(), 1);

    let tipo_de = |n: &String| (n.clone(), t.columna(n).unwrap().tipo.clone());
    // El `time` de la semilla (`12:34:56.123456789`) trae nanosegundos sueltos: no
    // cabe en microsegundos, y el lote lo dice con la columna y la fila.
    let con_time: Vec<(String, Tipo)> = plan.leidas.iter().map(tipo_de).collect();
    let mut b = Constructor::nuevo(&con_time).unwrap();
    let e = b.push(&filas[0]).unwrap_err();
    assert!(
        e.to_string().contains("columna `t_time`, fila 0")
            && e.to_string().contains("nanosegundos"),
        "{e}"
    );

    // Sin ella, el resto entra.
    let leidas: Vec<String> = plan
        .leidas
        .iter()
        .filter(|n| *n != "t_time")
        .cloned()
        .collect();
    let posiciones: Vec<usize> = leidas
        .iter()
        .map(|n| plan.leidas.iter().position(|x| x == n).unwrap())
        .collect();
    let fila: Fila = posiciones.iter().map(|i| filas[0][*i].clone()).collect();
    let columnas: Vec<(String, Tipo)> = leidas.iter().map(tipo_de).collect();
    let mut b = Constructor::nuevo(&columnas).unwrap();
    b.push(&fila).unwrap();
    let lote = b.finish().unwrap();
    let col = |n: &str| {
        lote.column(leidas.iter().position(|x| x == n).unwrap())
            .clone()
    };
    let texto = |n: &str| col(n).as_string::<i32>().value(0).to_string();

    assert_eq!(texto("t_varint"), "123456789012345678901234567890");
    assert_eq!(texto("t_dec"), "12345678901234567890.123456789");
    assert_eq!(texto("t_dur"), "14mo3d14400000000000ns");
    assert_eq!(texto("t_blob"), "yv4=");
    assert_eq!(texto("t_inet"), "10.0.0.1");
    assert_eq!(texto("t_text"), "ñandú 🦆");
    assert_eq!(
        col("t_big")
            .as_primitive::<arrow_array::types::Int64Type>()
            .value(0),
        i64::MAX
    );
    assert_eq!(
        col("t_int")
            .as_primitive::<arrow_array::types::Int64Type>()
            .value(0),
        2147483647
    );
    assert_eq!(
        col("t_tiny")
            .as_primitive::<arrow_array::types::Int64Type>()
            .value(0),
        127
    );
    assert_eq!(
        col("t_ts")
            .as_primitive::<arrow_array::types::TimestampMicrosecondType>()
            .value(0),
        1_792_499_696_789_000
    );
    assert_eq!(
        col("t_date")
            .as_primitive::<arrow_array::types::Date32Type>()
            .value(0),
        20_746
    );
    let lista: serde_json::Value = serde_json::from_str(&texto("t_list")).unwrap();
    assert_eq!(lista, serde_json::json!([1, 2, 3]));
    let mapa: serde_json::Value = serde_json::from_str(&texto("t_map")).unwrap();
    assert_eq!(mapa, serde_json::json!({"k": 1}));
    let udt: serde_json::Value = serde_json::from_str(&texto("t_udt")).unwrap();
    assert_eq!(udt["calle"], "Gran Via");
    assert_eq!(udt["geo"], serde_json::json!([40.4, -3.7]));
    let tupla: serde_json::Value = serde_json::from_str(&texto("t_tup")).unwrap();
    assert_eq!(tupla, serde_json::json!([1, "z"]));
}

#[test]
fn los_errores_son_los_siete_codigos() {
    let Some(u) = url() else { return };
    // Contraseña mala.
    let mala = u.replacen(":lector@", ":MALA@", 1);
    if mala != u {
        let Err(e) = Conexion::abrir(&mala) else {
            panic!("abrió con una contraseña mala")
        };
        assert_eq!(e.codigo, Codigo::Credencial, "{}", e.mensaje);
        assert!(
            !e.mensaje.contains("MALA"),
            "la credencial no sale: {}",
            e.mensaje
        );
    }
    // Nadie escucha.
    let Err(e) = Conexion::abrir("cassandra://u:p@127.0.0.1:1/ore_prueba") else {
        panic!("abrió donde nadie escucha")
    };
    assert_eq!(e.codigo, Codigo::Conexion, "{}", e.mensaje);
    assert!(e.reintentable);
    // Un keyspace al que el rol no llega.
    let c = Conexion::abrir(&u).unwrap();
    let e = c.tabla("ore_prueba", "no_existe").unwrap_err();
    assert_eq!(e.codigo, Codigo::Objeto);
}

#[test]
fn el_rol_solo_ve_lo_que_puede_leer() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let p = c.permisos().unwrap();
    assert!(p.puede_leer("ore_prueba", "eventos"));
    // `system_schema` enseña `otra.secreta` (X0 §3.5); el rol no la lee, y no se cataloga.
    assert!(c.tablas("otra").unwrap().contains(&"secreta".to_string()));
    assert!(!p.puede_leer("otra", "secreta"));
    assert!(!p.ve_keyspace("otra"));

    // Sin keyspace en la URL, el catálogo recorre los que el rol ve: ni `otra` ni el sistema.
    let sin_ks = u.replacen("/ore_prueba", "/", 1);
    let cat = ore_cassandra::verbos::catalogo(&sin_ks, "cass").unwrap();
    assert!(cat.contains("ore_prueba.eventos"), "{cat}");
    assert!(
        !cat.contains("otra.") && !cat.contains("secreta"),
        "no se enseña lo ajeno"
    );
    assert!(
        !cat.contains("system_schema") && !cat.contains("system."),
        "ni el sistema"
    );
    let ex = ore_cassandra::verbos::explorar(&format!(r#"{{"url":"{sin_ks}"}}"#)).unwrap();
    assert!(ex.contains("ore_prueba") && !ex.contains("otra"), "{ex}");
}

#[test]
fn check_dice_la_causa_literal() {
    let Some(u) = url() else { return };
    let ok = ore_cassandra::verbos::check(&format!(r#"{{"url":"{u}"}}"#)).unwrap();
    assert_eq!(ok, r#"{"ok":true}"#);
    let mala = u.replacen(":lector@", ":MALA@", 1);
    if mala != u {
        let r = ore_cassandra::verbos::check(&format!(r#"{{"url":"{mala}"}}"#)).unwrap();
        assert!(
            r.contains(r#""ok":false"#) && r.contains("incorrect"),
            "{r}"
        );
        assert!(!r.contains("MALA"), "la contraseña no sale: {r}");
    }
    let r = ore_cassandra::verbos::check(r#"{"url":"cassandra://u:p@127.0.0.1:1/x"}"#).unwrap();
    assert!(r.contains(r#""ok":false"#), "{r}");
}

#[test]
fn la_misma_peticion_da_la_misma_salida_en_el_mismo_orden() {
    let Some(u) = url() else { return };
    let c = Conexion::abrir(&u).unwrap();
    let Ok(t) = c.tabla("ore_prueba", "grande") else {
        eprintln!("sin la tabla `grande`: se salta");
        return;
    };
    let plan = traducir(&pet("grande", &["id"], vec![]), &t, &Opciones::default())
        .unwrap()
        .plan;
    // Cuatro rangos leen a la vez y contestan cuando pueden; la salida no depende de
    // quién contestó antes.
    let a: Vec<i64> = c
        .leer(&plan, &t, 4)
        .unwrap()
        .iter()
        .map(|f| entero(f, 0))
        .collect();
    let b: Vec<i64> = c
        .leer(&plan, &t, 4)
        .unwrap()
        .iter()
        .map(|f| entero(f, 0))
        .collect();
    let uno: Vec<i64> = c
        .leer(&plan, &t, 1)
        .unwrap()
        .iter()
        .map(|f| entero(f, 0))
        .collect();
    assert_eq!(a.len(), 300_000);
    assert_eq!(a, b, "dos lecturas, dos órdenes");
    assert_eq!(a, uno, "el paralelo no cambia el orden");
}
