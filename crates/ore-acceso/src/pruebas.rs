//! Contra un `ore-iam` de mentira: el mismo servidor de `ore-entrada`, en un hilo,
//! que contesta lo que cada prueba le pide y apunta lo que le llegó.

use super::*;
use ore_entrada::http::{Peticion, Respuesta, servir};
use std::net::TcpListener;
use std::sync::Arc;

/// Lo que el `ore-iam` de mentira vio: camino, `Ore-Sujeto`, cuerpo.
type Visto = Arc<Mutex<Vec<(String, Option<String>, String)>>>;

/// Levanta uno con este manejador. Devuelve su dirección y lo que va viendo.
fn falso(f: impl Fn(&Peticion) -> Respuesta + Send + Sync + 'static) -> (String, Visto) {
    let escucha = TcpListener::bind("127.0.0.1:0").unwrap();
    let dir = escucha.local_addr().unwrap().to_string();
    let visto: Visto = Arc::new(Mutex::new(Vec::new()));
    let v = Arc::clone(&visto);
    std::thread::spawn(move || {
        let _ = servir(escucha, move |p| {
            v.lock().unwrap().push((
                p.ruta.clone(),
                p.cabeceras.get("ore-sujeto").cloned(),
                p.cuerpo.clone(),
            ));
            f(p)
        });
    });
    (dir, visto)
}

fn respuesta(codigo: u16, cuerpo: &str) -> Respuesta {
    Respuesta {
        codigo,
        cuerpo: Json::Crudo(cuerpo.to_string()),
    }
}

fn acceso(dir: &str) -> Acceso {
    Acceso::nuevo(dir, Box::new(Fija("token-de-la-celda".into())))
}

const PERMITE: &str = r#"{"decision":true,"context":{"id":"dec_1","version":"v","vale":30}}"#;

#[test]
fn permite_con_los_dos_tokens_y_la_forma_de_authzen() {
    let (dir, visto) = falso(|p| {
        assert_eq!(
            p.cabeceras.get("authorization").map(String::as_str),
            Some("Bearer token-de-la-celda")
        );
        respuesta(200, PERMITE)
    });
    let d = acceso(&dir).puede(
        "tok-ana",
        "persona:ana",
        "fuente:crear",
        Recurso::ORGANIZACION,
        "POST /fuentes",
    );
    assert_eq!(d, Decision::Permite { id: "dec_1".into() });
    let v = visto.lock().unwrap();
    assert_eq!(v[0].0, "/access/v1/evaluation");
    assert_eq!(v[0].1.as_deref(), Some("tok-ana"));
    for k in [
        r#""subject""#,
        r#""persona:ana""#,
        r#""fuente:crear""#,
        r#""organizacion""#,
        r#""POST /fuentes""#,
    ] {
        assert!(v[0].2.contains(k), "falta {k} en {}", v[0].2);
    }
}

#[test]
fn niega_con_su_motivo_y_su_codigo() {
    let (dir, _) = falso(|_| {
        respuesta(
            200,
            r#"{"decision":false,"context":{"id":"dec_2","vale":30,"motivo":"no tienes `fuente:crear` en esta organización"}}"#,
        )
    });
    let d = acceso(&dir).puede(
        "t",
        "persona:zoe",
        "fuente:crear",
        Recurso::ORGANIZACION,
        "",
    );
    assert!(!d.permite());
    assert_eq!(d.codigo(), 403);
    let Decision::Niega { id, motivo } = d else {
        panic!("no nego")
    };
    assert_eq!(id, "dec_2");
    assert!(motivo.contains("fuente:crear"));
}

/// ⭐ La caché: dentro de `vale`, la segunda pregunta no sale; con `vale` 0, sí.
#[test]
fn la_respuesta_se_guarda_lo_que_dice_vale() {
    let (dir, visto) = falso(|_| respuesta(200, PERMITE));
    let a = acceso(&dir);
    for _ in 0..3 {
        assert!(
            a.puede("t", "persona:ana", "x:y", Recurso::ORGANIZACION, "")
                .permite()
        );
    }
    assert_eq!(
        visto.lock().unwrap().len(),
        1,
        "preguntó más de una vez dentro de `vale`"
    );
    // Otra acción, otra pregunta.
    a.puede("t", "persona:ana", "x:z", Recurso::ORGANIZACION, "");
    assert_eq!(visto.lock().unwrap().len(), 2);

    let (dir, visto) =
        falso(|_| respuesta(200, r#"{"decision":true,"context":{"id":"d","vale":0}}"#));
    let a = acceso(&dir);
    a.puede("t", "persona:ana", "x:y", Recurso::ORGANIZACION, "");
    a.puede("t", "persona:ana", "x:y", Recurso::ORGANIZACION, "");
    assert_eq!(
        visto.lock().unwrap().len(),
        2,
        "con vale 0 guardó la respuesta"
    );
}

/// ⛔ Sin respuesta, NIEGA con 503: nadie escuchando, 5xx, o más lento que el plazo.
#[test]
fn sin_quien_decida_niega_con_503() {
    let muerto = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    };
    let d = acceso(&muerto).puede("t", "persona:ana", "x:y", Recurso::ORGANIZACION, "");
    assert_eq!(d.codigo(), 503, "{d:?}");

    let (dir, _) = falso(|_| respuesta(500, r#"{"error":"la base no contesta"}"#));
    let d = acceso(&dir).puede("t", "persona:ana", "x:y", Recurso::ORGANIZACION, "");
    assert_eq!(d.codigo(), 503);

    let (dir, _) = falso(|_| {
        std::thread::sleep(Duration::from_millis(1500));
        respuesta(200, PERMITE)
    });
    let t = Instant::now();
    let d = acceso(&dir).con_plazo(Duration::from_millis(200)).puede(
        "t",
        "persona:ana",
        "x:y",
        Recurso::ORGANIZACION,
        "",
    );
    assert_eq!(d.codigo(), 503, "esperó a quien tardaba: {d:?}");
    assert!(
        t.elapsed() < Duration::from_millis(1200),
        "no respetó el plazo: {:?}",
        t.elapsed()
    );
}

/// Un 401 del SUJETO es un 401 para la persona; uno de la CELDA es un 503 (la
/// celda mal configurada no es culpa de quien pide).
#[test]
fn un_401_del_sujeto_no_es_uno_de_la_celda() {
    let (dir, _) = falso(|_| {
        respuesta(
            401,
            r#"{"error":"el token de `Ore-Sujeto` no vale: el token caducó"}"#,
        )
    });
    assert_eq!(
        acceso(&dir)
            .puede("t", "p", "x:y", Recurso::ORGANIZACION, "")
            .codigo(),
        401
    );
    let (dir, _) = falso(|_| {
        respuesta(
            401,
            r#"{"error":"esta celda no está registrada en `ore-iam`, o está retirada"}"#,
        )
    });
    assert_eq!(
        acceso(&dir)
            .puede("t", "p", "x:y", Recurso::ORGANIZACION, "")
            .codigo(),
        503
    );
}

#[test]
fn una_celda_que_no_se_presenta_no_deja_pasar() {
    struct Rota;
    impl Credencial for Rota {
        fn token(&self) -> Result<String, String> {
            Err("sin servidor de metadatos".into())
        }
    }
    let (dir, visto) = falso(|_| respuesta(200, PERMITE));
    let d = Acceso::nuevo(&dir, Box::new(Rota)).puede("t", "p", "x:y", Recurso::ORGANIZACION, "");
    assert_eq!(d.codigo(), 503);
    assert!(visto.lock().unwrap().is_empty());
}

fn evento(id: &str, decision: Option<&str>) -> Evento {
    Evento {
        id: id.into(),
        operacion: "fuente:crear".into(),
        sobre: "fuente/ventas".into(),
        resultado: "hecho".into(),
        decision: decision.map(str::to_string),
        commit: Some("abc123".into()),
        abre: None,
        detalle: Some(Json::obj([("tipo", Json::s("postgres"))])),
    }
}

#[test]
fn hizo_anota_y_la_segunda_vez_ya_estaba() {
    let (dir, visto) = falso(|p| {
        if p.cuerpo.contains("ev-repetido") {
            respuesta(200, r#"{"id":"ev-repetido","ya":true}"#)
        } else {
            respuesta(201, r#"{"id":"ev-1"}"#)
        }
    });
    let a = acceso(&dir);
    assert_eq!(
        a.hizo(Some("tok-ana"), &evento("ev-1", Some("dec_1"))),
        Ok(Hecho::Anotado)
    );
    assert_eq!(
        a.hizo(Some("tok-ana"), &evento("ev-repetido", None)),
        Ok(Hecho::YaEstaba)
    );
    let v = visto.lock().unwrap();
    assert_eq!(v[0].0, "/access/v1/eventos");
    assert!(
        v[0].2.contains(r#""commit":"abc123""#) && v[0].2.contains(r#""decision":"dec_1""#),
        "{}",
        v[0].2
    );
}

/// ⭐ Si `ore-iam` no contesta, el evento espera en disco, y el reintento va SIN
///   `Ore-Sujeto`: el sujeto sale de la decisión. Uno sin decisión no se encola.
#[test]
fn hizo_espera_en_disco_y_se_reintenta_sin_el_token_de_la_persona() {
    let dir_p = std::env::temp_dir().join(format!("ore-acceso-pendientes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_p);
    let muerto = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    };
    let a = acceso(&muerto).con_pendientes(dir_p.clone());
    assert!(matches!(
        a.hizo(Some("tok"), &evento("ev-9", Some("dec_9"))),
        Ok(Hecho::Encolado(_))
    ));
    assert!(dir_p.join("ev-9.json").exists());
    assert!(
        a.hizo(Some("tok"), &evento("ev-sin", None)).is_err(),
        "encoló un evento sin decisión"
    );
    // `hizo_antes` no encola: si no se anota, no se actúa.
    assert!(
        a.hizo_antes(Some("tok"), &evento("ev-antes", Some("dec_9")))
            .is_err()
    );

    let (vivo, visto) = falso(|_| respuesta(201, r#"{"id":"ev-9"}"#));
    let a = acceso(&vivo).con_pendientes(dir_p.clone());
    assert_eq!(a.reintentar(), (1, 0));
    assert!(
        !dir_p.join("ev-9.json").exists(),
        "el enviado sigue en la cola"
    );
    let v = visto.lock().unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].1, None, "el reintento mandó Ore-Sujeto");
    assert!(v[0].2.contains(r#""decision":"dec_9""#));

    // Y uno cuya decisión ya no vive: a `muertos/`, no para siempre en la cola.
    std::fs::write(
        dir_p.join("ev-viejo.json"),
        evento("ev-viejo", Some("dec_x")).json().jcs(),
    )
    .unwrap();
    let (rechaza, _) = falso(|_| {
        respuesta(
            400,
            r#"{"error":"sin `Ore-Sujeto`, el evento tiene que nombrar una decisión viva"}"#,
        )
    });
    assert_eq!(
        acceso(&rechaza).con_pendientes(dir_p.clone()).reintentar(),
        (0, 0)
    );
    assert!(dir_p.join("muertos").join("ev-viejo.json").exists());
    let _ = std::fs::remove_dir_all(&dir_p);
}

#[test]
fn los_ids_no_se_repiten() {
    let a: std::collections::BTreeSet<String> = (0..1000).map(|_| nuevo_id()).collect();
    assert_eq!(a.len(), 1000);
}
