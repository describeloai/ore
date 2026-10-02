//! `POST /funciones/firma` (ADR 0050 G5b · Dry Run): la firma de cada
//! `@function` de un texto **sin guardar**, mientras se escribe.
//!
//! Es la derivación del commit (`ore_code::python::derivar`, la que escribe el
//! documento `Function`), sobre el texto del editor y no sobre el árbol: no lee
//! git, no escribe nada y no ejecuta el código. Por eso puede ir a cada pausa
//! del teclado (microsegundos; el límite de tamaño y la pila los pone
//! `ore-code`). Lo que el commit comprueba **además** —que la colección de un
//! `Media<…>` exista, que dos `def` no choquen en el paquete— lo sigue
//! comprobando el commit: esto dice lo que el código dice de sí mismo.
//!
//! Cuerpo: `{"ruta": "<desde la carpeta del paquete>", "texto": "…",
//! "paquete": "<ns>"}`. La consola sabe las tres cosas del repositorio
//! abierto. Respuesta: las funciones **en el orden del fichero**, cada una con
//! su firma o sus fallos, y los fallos del fichero (sintaxis, versión, avisos),
//! todos con línea y columna para señalarlos en el editor.

use ore_code::lineas::Lineas;
use ore_code::{Campo, Fallo, Firma, Salida};
use ore_core::json::Json;
use ore_core::parse;
use ore_entrada::http::Respuesta;

pub(crate) fn firma(cuerpo: &str) -> Respuesta {
    let n = match parse::parse(cuerpo) {
        Ok(n @ parse::Node::Mapping { .. }) => n,
        _ => return Respuesta::error(400, "el cuerpo no es un objeto JSON"),
    };
    let texto_de = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let Some(texto) = texto_de("texto") else {
        return Respuesta::error(422, "falta `texto`: el fuente, tal como está en el editor");
    };
    let Some(ruta) = texto_de("ruta").filter(|r| !r.is_empty() && !r.starts_with('/')) else {
        return Respuesta::error(
            422,
            "falta `ruta`: la del fichero desde la carpeta del paquete",
        );
    };
    let paquete = texto_de("paquete").unwrap_or_default();
    Respuesta::ok(derivada(&texto, &ruta, &paquete))
}

/// Lo que la respuesta lleva. Aparte, para probarlo sin HTTP.
fn derivada(texto: &str, ruta: &str, paquete: &str) -> Json {
    let d = ore_code::python::derivar(texto, ruta);
    let l = Lineas::new(texto);
    let fallos = |fs: &[Fallo]| Json::Arr(fs.iter().map(|f| fallo(&l, f)).collect());
    let funciones = d
        .funciones
        .iter()
        .map(|f| {
            let (linea, columna) = l.posicion(f.rango.inicio);
            let mut o = Json::obj([
                ("nombre", Json::s(&f.nombre)),
                ("linea", Json::Int(linea.into())),
                ("columna", Json::Int(columna.into())),
            ]);
            if let Json::Obj(m) = &mut o {
                match &f.resultado {
                    Ok(firma) => {
                        m.insert("firma".into(), firma_json(firma, paquete));
                    }
                    Err(fs) => {
                        m.insert("fallos".into(), fallos(fs));
                    }
                }
            }
            o
        })
        .collect();
    Json::obj([
        ("funciones", Json::Arr(funciones)),
        ("sintaxis", fallos(&d.sintaxis)),
        ("version", fallos(&d.version)),
        ("avisos", fallos(&d.avisos)),
    ])
}

fn fallo(l: &Lineas, f: &Fallo) -> Json {
    let (linea, columna) = l.posicion(f.rango.inicio);
    let (hasta_linea, hasta_columna) = l.posicion(f.rango.fin);
    let mut o = Json::obj([
        ("linea", Json::Int(linea.into())),
        ("columna", Json::Int(columna.into())),
        ("hastaLinea", Json::Int(hasta_linea.into())),
        ("hastaColumna", Json::Int(hasta_columna.into())),
        ("mensaje", Json::s(&f.mensaje)),
    ]);
    if let (Json::Obj(m), Some(a)) = (&mut o, &f.ayuda) {
        m.insert("ayuda".into(), Json::s(a));
    }
    o
}

/// Los campos van en **lista**, no en mapa: el orden de los parámetros es el
/// del `def`, y es el del formulario.
fn campos(cs: &[Campo]) -> Json {
    Json::Arr(
        cs.iter()
            .map(|c| {
                Json::obj([
                    ("nombre", Json::s(&c.nombre)),
                    ("type", Json::s(c.tipo.to_string())),
                    ("required", Json::Bool(c.requerido)),
                ])
            })
            .collect(),
    )
}

fn firma_json(f: &Firma, paquete: &str) -> Json {
    let mut o = Json::obj([
        ("entrypoint", Json::s(&f.entrypoint)),
        ("apiVersion", Json::s(f.api_version())),
        ("input", campos(&f.entrada)),
        (
            "output",
            match &f.salida {
                Salida::Valor(t) => Json::obj([("type", Json::s(t.to_string()))]),
                Salida::Campos(cs) => Json::obj([("campos", campos(cs))]),
            },
        ),
    ]);
    if let Json::Obj(m) = &mut o {
        if !paquete.is_empty() {
            m.insert(
                "referencia".into(),
                Json::s(format!("{paquete}.{}", f.nombre)),
            );
        }
        let mut opcional = |k: &str, v: &Option<String>| {
            if let Some(v) = v {
                m.insert(k.into(), Json::s(v));
            }
        };
        opcional("description", &f.descripcion);
        opcional("over", &f.over);
        opcional("timeout", &f.timeout);
        if let Some(rs) = &f.reads {
            m.insert("reads".into(), Json::Arr(rs.iter().map(Json::s).collect()));
        }
        if let Some(ms) = &f.models {
            m.insert("models".into(), Json::Arr(ms.iter().map(Json::s).collect()));
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUENTE: &str = r#"from dataclasses import dataclass
from decimal import Decimal

from ore import function
from ore.tipos import DateTimeTz, Money


@dataclass
class Linea:
    producto: str
    precio: Money["EUR", 2]


@function(timeout="30s")
def zeta(b: int, a: list[Linea], corte: DateTimeTz | None = None) -> Decimal:
    """La primera, aunque se llame zeta."""
    return Decimal("0")


@function
def rota(x: Money[MONEDA, 2]) -> int:
    ...
"#;

    fn obj(j: &Json) -> &std::collections::BTreeMap<String, Json> {
        match j {
            Json::Obj(m) => m,
            otro => panic!("no es un objeto: {otro:?}"),
        }
    }
    fn arr(j: &Json) -> &[Json] {
        match j {
            Json::Arr(a) => a,
            otro => panic!("no es una lista: {otro:?}"),
        }
    }
    fn s(j: &Json) -> &str {
        match j {
            Json::Str(s) => s,
            otro => panic!("no es una cadena: {otro:?}"),
        }
    }

    #[test]
    fn la_firma_de_un_texto_sin_guardar_en_el_orden_del_fichero() {
        let r = derivada(FUENTE, "riesgo/funciones/riesgo.py", "ventas");
        let fs = arr(&obj(&r)["funciones"]);
        assert_eq!(fs.len(), 2);

        let zeta = obj(&fs[0]);
        assert_eq!(s(&zeta["nombre"]), "zeta");
        assert_eq!(zeta["linea"], Json::Int(15));
        let f = obj(&zeta["firma"]);
        assert_eq!(s(&f["referencia"]), "ventas.zeta");
        assert_eq!(s(&f["entrypoint"]), "riesgo/funciones/riesgo.py:zeta");
        assert_eq!(s(&f["apiVersion"]), "oos.dev/v1alpha20");
        assert_eq!(s(&f["timeout"]), "30s");
        assert_eq!(s(&f["description"]), "La primera, aunque se llame zeta.");
        // El orden es el del `def`, no el alfabético.
        let input: Vec<(&str, &str, bool)> = arr(&f["input"])
            .iter()
            .map(|c| {
                let c = obj(c);
                (
                    s(&c["nombre"]),
                    s(&c["type"]),
                    c["required"] == Json::Bool(true),
                )
            })
            .collect();
        assert_eq!(
            input,
            [
                ("b", "Integer", true),
                (
                    "a",
                    "list<Struct<producto: String, precio: Money<EUR, 2>>>",
                    true
                ),
                ("corte", "DateTimeTz", false),
            ]
        );
        assert_eq!(s(&obj(&f["output"])["type"]), "Decimal");

        // La que no se deriva dice dónde y cómo, sin tumbar a la otra.
        let rota = obj(&fs[1]);
        assert!(!rota.contains_key("firma"));
        let fallo = obj(&arr(&rota["fallos"])[0]);
        assert_eq!(fallo["linea"], Json::Int(21));
        assert!(s(&fallo["mensaje"]).contains("literal"), "{fallo:?}");
    }

    #[test]
    fn un_error_de_sintaxis_es_un_dato_con_su_linea() {
        let r = derivada(
            "from ore import function\n\n@function\ndef f(x: int -> int:\n    ...\n",
            "f.py",
            "p",
        );
        let sintaxis = arr(&obj(&r)["sintaxis"]);
        assert!(!sintaxis.is_empty());
        assert_eq!(obj(&sintaxis[0])["linea"], Json::Int(4));
    }

    #[test]
    fn el_cuerpo_dice_lo_que_falta() {
        assert_eq!(firma("no es json {").codigo, 400);
        assert_eq!(firma(r#"{"ruta": "f.py"}"#).codigo, 422);
        assert_eq!(firma(r#"{"texto": "x = 1"}"#).codigo, 422);
        let r = firma(r#"{"ruta": "f.py", "texto": "x = 1\n"}"#);
        assert_eq!(r.codigo, 200);
        assert_eq!(arr(&obj(&r.cuerpo)["funciones"]).len(), 0);
    }
}
