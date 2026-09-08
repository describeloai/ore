//! Lo que `ore-iam` sirve por HTTP.
//!
//! # De momento, sólo mirar — y mirar DEJA HUELLA
//!
//! Es la idea de su `022`, y su frase se toma entera: *«leer lo que hicieron
//! los demás es la potestad más barata del catálogo y también la más íntima»*.
//! Un verbo de lectura que no deja rastro es un agujero con forma de
//! optimización.
//!
//! ⚠️ Y por eso incluso `GET` abre transacción: lo que se lee y quién lo leyó
//! se confirman juntos.
//!
//! # Lo que todavía NO hay
//!
//! `invitar`, `admitir`, `conceder` y `revocar`. Se dice en vez de dejar el
//! hueco callado: hoy la única forma de que exista una organización es
//! `ore-iam fundar`, que es un acto de operador.

use crate::base::Tx;
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::{Identidad, Proveedor, SinIdentidad};
use postgres::Client;
use std::sync::Mutex;

pub struct Servidor {
    pub base: Mutex<Client>,
    /// Sin proveedor, las rutas de datos **no se montan**. La misma regla que
    /// `ore-serve`, y aquí pesa más: esta superficie administra.
    pub identidad: Option<Proveedor>,
}

impl Servidor {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        match (p.metodo.as_str(), seg.as_slice()) {
            ("GET", ["salud"]) => Respuesta::ok(Json::obj([("ok", Json::Bool(true))])),
            _ => match self.quien(p) {
                Err(r) => r,
                Ok(sujeto) => self.con_sujeto(&sujeto, &seg),
            },
        }
    }

    fn quien(&self, p: &Peticion) -> Result<Identidad, Respuesta> {
        let Some(proveedor) = self.identidad.as_ref() else {
            return Err(Respuesta::error(
                404,
                "sin proveedor de identidad configurado, las rutas de datos no se montan",
            ));
        };
        proveedor(&p.cabeceras).map_err(|e| match e {
            SinIdentidad::Ausente => Respuesta::error(401, "esta ruta necesita un sujeto"),
            SinIdentidad::Invalida(m) => Respuesta::error(401, m),
        })
    }

    fn con_sujeto(&self, sujeto: &Identidad, seg: &[&str]) -> Respuesta {
        match seg {
            ["organizaciones"] => self.organizaciones(sujeto),
            _ => Respuesta::error(404, "no hay nada en ese camino"),
        }
    }

    fn organizaciones(&self, sujeto: &Identidad) -> Respuesta {
        let Ok(mut base) = self.base.lock() else {
            return Respuesta::error(500, "la conexion quedo envenenada");
        };
        let mut tx = match Tx::abrir(&mut base, sujeto) {
            Ok(t) => t,
            Err(e) => return Respuesta::error(502, e),
        };

        // ⛔ Sólo las SUYAS. Un listado que devolviera todas seria una fuga con
        //   forma de comodidad — y la consola no sabria que la tuvo.
        let filas = match tx.filas(
            "select o.id, o.nombre, o.estado, p.papel
               from iam.organizacion o
               join iam.pertenencia p on p.organizacion = o.id
              where p.persona = (select id from iam.persona where sub = $1)
              order by o.nombre",
            &[&sujeto.persona],
        ) {
            Ok(f) => f,
            Err(e) => return Respuesta::error(500, e),
        };

        let lista: Vec<Json> = filas
            .iter()
            .map(|f| {
                Json::obj([
                    ("id", Json::s(f.get::<_, String>(0))),
                    ("nombre", Json::s(f.get::<_, String>(1))),
                    ("estado", Json::s(f.get::<_, String>(2))),
                    ("papel", Json::s(f.get::<_, String>(3))),
                ])
            })
            .collect();

        if let Err(e) = tx.anotar(
            "organizacion:listar",
            "*",
            Json::obj([("cuantas", Json::Int(lista.len() as i64))]),
        ) {
            return Respuesta::error(500, e);
        }
        if let Err(e) = tx.confirmar() {
            return Respuesta::error(500, e);
        }
        Respuesta::ok(Json::obj([("organizaciones", Json::Arr(lista))]))
    }
}

pub fn mapa(con_identidad: bool) -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("GET", "/salud", true),
        ("GET", "/organizaciones", con_identidad),
    ]
}
