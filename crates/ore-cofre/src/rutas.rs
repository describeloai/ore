//! Lo que el custodio sirve, y las tres reglas que no se negocian.
//!
//! ```text
//!   POST /organizaciones/{org}/secretos            emitir
//!   GET  /organizaciones/{org}/secretos            listar   (NOMBRES, no valores)
//!   GET  /organizaciones/{org}/secretos/{nombre}   resolver
//! ```
//!
//! # ① El motor de autorización es el de `ore-iam`
//!
//! No hay aquí ni una decisión sobre quién puede qué: `potestad::exige` y
//! `iam.concesion_viva`. Una segunda respuesta a esa pregunta sería el segundo
//! motor que la `0023` rechaza — y peor si lo escribimos nosotros por descuido.
//!
//! # ② «No existe» y «no puedes» son el MISMO error
//!
//! Es la regla de `potestad::exige` y de la sonda entre inquilinos que cazó la
//! `0021`, y aquí pesa más que en ningún sitio: si resolver un secreto ajeno
//! diera «no tienes acceso» y uno inventado diera «no existe», cualquiera con
//! una cuenta tendría **un directorio de los secretos de los demás**,
//! consultable nombre a nombre.
//!
//! # ③ El valor sale por UNA puerta, y esa puerta deja huella
//!
//! Listar no lo devuelve. Emitir no lo devuelve. Sólo `resolver`, y su huella se
//! confirma en la misma transacción que la lectura — si anotar falla, no se
//! contesta.

use crate::kms::Kms;
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::{Identidad, Proveedor, SinIdentidad};
use ore_iam::base::{Tx, nuevo_id};
use ore_iam::{potestad, verbos};
use postgres::Client;
use std::sync::Mutex;

/// Las clases que admite `cofre.secreto`. La lista vive en la base —un `check`—
/// y aquí sólo se dice para poder contestar con una frase en vez de con una
/// restricción violada.
const CLASES: &[&str] = &["contrasena", "testigo", "clave-api", "conexion"];

pub struct Servidor {
    pub base: Mutex<Client>,
    pub emisor: String,
    pub identidad: Option<Proveedor>,
    pub kms: Kms,
}

impl Servidor {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        match (p.metodo.as_str(), seg.as_slice()) {
            ("GET", ["salud"]) => Respuesta::ok(Json::obj([("ok", Json::Bool(true))])),
            _ => match self.quien(p) {
                Err(r) => r,
                Ok(s) => self.con_sujeto(p, &s, &seg),
            },
        }
    }

    /// Sin proveedor esto contesta **404 y no 401**: la ruta no está porque no
    /// se montó, y decir «no autorizado» insinuaría que existe. Misma regla que
    /// `ore-serve` y `ore-iam`.
    fn quien(&self, p: &Peticion) -> Result<Identidad, Respuesta> {
        let Some(proveedor) = self.identidad.as_ref() else {
            return Err(Respuesta::error(
                404,
                "sin proveedor de identidad configurado, las rutas del cofre no se montan",
            ));
        };
        proveedor(&p.cabeceras).map_err(|e| match e {
            SinIdentidad::Ausente => Respuesta::error(401, "esta ruta necesita un sujeto"),
            SinIdentidad::Invalida(m) => Respuesta::error(401, m),
        })
    }

    fn con_sujeto(&self, p: &Peticion, s: &Identidad, seg: &[&str]) -> Respuesta {
        match (p.metodo.as_str(), seg) {
            ("POST", ["organizaciones", org, "secretos"]) => self.emitir(s, org, &p.cuerpo.clone()),
            ("GET", ["organizaciones", org, "secretos"]) => self.listar(s, org),
            ("GET", ["organizaciones", org, "secretos", nombre]) => self.resolver(s, org, nombre),
            ("GET", _) | ("POST", _) => Respuesta::error(404, "no hay nada en ese camino"),
            _ => Respuesta::error(405, "método no admitido"),
        }
    }

    /// ⛔ Hasta un `GET` abre transacción, y por lo mismo que en `ore-iam`: lo
    /// que se leyó y quién lo leyó se confirman juntos. Aquí es más fuerte
    /// todavía — lo que se lee es un secreto.
    fn en_transaccion(
        &self,
        s: &Identidad,
        f: impl FnOnce(&mut Tx, &str) -> Result<Json, String>,
    ) -> Respuesta {
        let Ok(mut base) = self.base.lock() else {
            return Respuesta::error(500, "la conexión quedó envenenada");
        };
        let mut tx = match Tx::abrir(&mut base, s) {
            Ok(t) => t,
            Err(e) => return Respuesta::error(502, e),
        };
        // ⚠️ Y aquí NO se refresca el nombre de la persona, al revés que en
        //   `ore-iam`. No es un olvido: `ore_cofre` sólo tiene `select` sobre
        //   `iam.persona`, a propósito. El custodio lee quién eres; quién eres
        //   lo escribe el plano de identidad.
        match f(&mut tx, &self.emisor) {
            Err(e) => Respuesta::error(422, e),
            Ok(j) => match tx.confirmar() {
                Ok(()) => Respuesta::ok(j),
                Err(e) => Respuesta::error(500, e),
            },
        }
    }

    // ── emitir ──────────────────────────────────────────────────────────────

    fn emitir(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        let kms = &self.kms;
        self.en_transaccion(s, move |tx, emisor| {
            // ① ¿Puede emitir? Es una POTESTAD de la organización: no habla de
            //    ningún secreto concreto porque todavía no existe.
            potestad::exige(tx, emisor, &s.persona, &org, "secreto:emitir")?;

            let c = analizar(&cuerpo)?;
            let nombre = campo(&c, "nombre").ok_or("falta `nombre`")?;
            let clase = campo(&c, "clase").ok_or("falta `clase`")?;
            let valor = campo(&c, "valor").ok_or("falta `valor`")?;

            if !CLASES.contains(&clase.as_str()) {
                return Err(format!(
                    "`{clase}` no es una clase de secreto. Son: {}",
                    CLASES.join(", ")
                ));
            }
            // El mismo alfabeto que `concesion.recurso` exige para `secreto/…`.
            // Si no fueran el mismo, la concesión que se crea abajo no
            // alcanzaría al secreto que se acaba de crear.
            potestad::recurso(&format!("secreto/{nombre}"))?;
            if valor.is_empty() {
                return Err("un secreto vacio no es un secreto".into());
            }

            // ② Con qué llave se cierra. Sale de `iam`, que es quien la nombra.
            let kek: String = tx
                .uno("select kek from iam.organizacion where id = $1", &[&org])?
                .ok_or("esa organizacion no existe")?
                .get(0);

            // ③ Y se cierra ANTES de escribir nada. Si el KMS dice que no, no
            //    queda una fila de catálogo apuntando a un material que no está.
            let cifrado = kms.cerrar(&kek, valor.as_bytes())?;

            let id = nuevo_id("sec");
            let quien = verbos::persona_id(tx, emisor, &s.persona)?;
            tx.ejecutar(
                "insert into cofre.secreto (id, organizacion, nombre, clase, emitio)
                 values ($1, $2, $3, $4, $5)",
                &[&id, &org, &nombre, &clase, &quien],
            )?;
            tx.ejecutar(
                "insert into cofre.material (secreto, version, cifrado, kek)
                 values ($1, 1, $2, $3)",
                &[&id, &cifrado, &kek],
            )?;

            // ④ ⭐⭐ Y NACE CON SU DUEÑO. La `0023`: quien emite queda `owner`
            //    de lo que emitió y de nada más. Un secreto sin nadie que pueda
            //    darlo no se lo puede dar nadie nunca — es la organización
            //    huérfana otra vez, y se arregla igual: en el mismo acto.
            //
            //    ⛔ Y va por `iam.conceder_de_secreto`, no por un `insert`: el
            //      custodio NO tiene permiso para escribir en `iam.concesion`.
            //      Esa función sólo sabe conceder sobre `secreto/…`, y el `if`
            //      que lo comprueba lo ejecuta la base, no una revisión.
            let con = nuevo_id("con");
            let recurso = format!("secreto/{nombre}");
            tx.ejecutar(
                "select iam.conceder_de_secreto($1, $2, $3, 'owner', $4, $5)",
                &[&con, &quien, &recurso, &quien, &org],
            )?;

            tx.anotar(
                "secreto:emitir",
                &id,
                Json::obj([
                    ("organizacion", Json::s(&org)),
                    ("nombre", Json::s(&nombre)),
                    ("clase", Json::s(&clase)),
                    ("kek", Json::s(&kek)),
                ]),
            )?;

            // ⛔ Y NO devuelve el valor. Quien lo acaba de escribir ya lo tiene;
            //   devolverlo sería una segunda copia por un camino que no es el de
            //   resolver, y por tanto una que se audita distinto.
            Ok(Json::obj([
                ("secreto", Json::s(id)),
                ("nombre", Json::s(nombre)),
                ("version", Json::Int(1)),
                ("concesion", Json::s(con)),
            ]))
        })
    }

    // ── listar ──────────────────────────────────────────────────────────────

    fn listar(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            // ⚠️ Ver los nombres de TODOS —incluidos los que no te han
            //   concedido— destapa qué sistemas hay y cómo se llaman. Por eso
            //   cuelga de una potestad y no del estado por defecto, igual que
            //   `invitacion:listar` con los correos.
            potestad::exige(tx, emisor, &s.persona, &org, "secreto:listar")?;
            let filas = tx.filas(
                "select s.nombre, s.clase, s.en, (s.retirado_en is not null) as retirado,
                        coalesce(max(m.version), 0) as versiones
                   from cofre.secreto s
                   left join cofre.material m on m.secreto = s.id
                  where s.organizacion = $1
                  group by s.nombre, s.clase, s.en, s.retirado_en
                  order by s.nombre",
                &[&org],
            )?;
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("nombre", Json::s(f.get::<_, String>(0))),
                        ("clase", Json::s(f.get::<_, String>(1))),
                        ("retirado", Json::Bool(f.get::<_, bool>(3))),
                        ("versiones", Json::Int(f.get::<_, i64>(4))),
                    ])
                })
                .collect();
            tx.anotar(
                "secreto:listar",
                &org,
                Json::obj([("cuantos", Json::Int(lista.len() as i64))]),
            )?;
            // ⛔ Nombres, clases y cuántas versiones. Ni un valor.
            Ok(Json::obj([("secretos", Json::Arr(lista))]))
        })
    }

    // ── resolver ────────────────────────────────────────────────────────────

    fn resolver(&self, s: &Identidad, org: &str, nombre: &str) -> Respuesta {
        let (org, nombre) = (org.to_string(), nombre.to_string());
        let kms = &self.kms;
        self.en_transaccion(s, move |tx, emisor| {
            let quien = verbos::persona_id(tx, emisor, &s.persona)?;
            let recurso = format!("secreto/{nombre}");

            // ⛔⛔ UNA SOLA CONSULTA, Y UN SOLO ERROR. Se pregunta por el
            //   material Y por la concesión a la vez: así no hay un camino en el
            //   que el código sepa que el secreto existe antes de saber si quien
            //   pregunta puede. Y el mensaje es el mismo en los tres casos —no
            //   existe, no es tuyo, no te lo han concedido— porque distinguirlos
            //   convierte esta ruta en un directorio de lo ajeno.
            let f = tx
                .uno(
                    "select v.cifrado, v.kek, v.version, c.rol
                       from cofre.secreto s
                       join cofre.vigente v on v.secreto = s.id
                       join iam.concesion_viva c
                         on c.recurso = $3 and c.organizacion = s.organizacion
                        and c.sujeto = $4 and c.rol in ('usar', 'lector', 'owner')
                      where s.organizacion = $1 and s.nombre = $2
                        and s.retirado_en is null",
                    &[&org, &nombre, &recurso, &quien],
                )?
                .ok_or("ese secreto no existe o no es tuyo")?;

            let (cifrado, kek, version, rol): (Vec<u8>, String, i32, String) =
                (f.get(0), f.get(1), f.get(2), f.get(3));

            // ⭐ Se abre con la llave CON LA QUE SE CERRÓ, no con la de ahora. Si
            //   la organización cambió de KEK, lo viejo sigue abriéndose.
            let claro = kms.abrir(&kek, &cifrado)?;
            let claro = String::from_utf8(claro)
                .map_err(|_| "lo que devolvio el KMS no es texto".to_string())?;

            // ⛔ La huella ANTES de contestar, y en la misma transacción: si
            //   anotar falla, el valor no sale. Un custodio que abriera sin
            //   dejar rastro sería peor que uno que no abre.
            tx.anotar(
                "secreto:resolver",
                &nombre,
                Json::obj([
                    ("organizacion", Json::s(&org)),
                    // ⭐ CON QUÉ ROL. Es lo que separa «se conectó» de «se la
                    //   llevó» el día que alguien pregunte.
                    ("rol", Json::s(&rol)),
                    ("version", Json::Int(version as i64)),
                ]),
            )?;

            Ok(Json::obj([
                ("nombre", Json::s(&nombre)),
                ("version", Json::Int(version as i64)),
                ("rol", Json::s(&rol)),
                ("valor", Json::s(claro)),
                // ⚠️ Lo que `usar` TODAVÍA no es, dicho en la respuesta y no en
                //   una documentación: hoy los tres roles devuelven el valor por
                //   aquí, y lo que los distingue es la huella. `usar` de verdad
                //   —que el valor llegue a un proceso sin pasar por los ojos de
                //   nadie— es la ENTREGA, y es otra pieza.
                (
                    "siguiente",
                    Json::s(
                        "la entrega —montar el valor en el proceso que lo usa, sin \
                         que pase por un `Secret` ni por una pantalla— es otra pieza",
                    ),
                ),
            ]))
        })
    }
}

fn analizar(cuerpo: &str) -> Result<Node, String> {
    if cuerpo.trim().is_empty() {
        return Err("el cuerpo está vacío".into());
    }
    parse::parse(cuerpo).map_err(|e| format!("el cuerpo no analiza: {e:?}"))
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

pub fn mapa(con: bool) -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("GET", "/salud", true),
        ("POST", "/organizaciones/{org}/secretos", con),
        ("GET", "/organizaciones/{org}/secretos", con),
        ("GET", "/organizaciones/{org}/secretos/{nombre}", con),
    ]
}
