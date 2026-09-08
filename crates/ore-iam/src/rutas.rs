//! Lo que `ore-iam` sirve por HTTP.
//!
//! # Todas las rutas dejan huella, incluidas las de leer
//!
//! Es la idea de su `022`, y su frase se toma entera: *«leer lo que hicieron
//! los demás es la potestad más barata del catálogo y también la más íntima»*.
//! Un verbo de lectura que no deja rastro es un agujero con forma de
//! optimización — por eso incluso un `GET` abre transacción: lo que se leyó y
//! quién lo leyó se confirman juntos.
//!
//! # Y el sujeto se busca por `(emisor, sub)`
//!
//! El servidor sabe contra qué emisor valida, así que lo lleva consigo. Buscar
//! por `sub` a secas es el mismo error que aceptar un token sin mirar el `iss`:
//! dos emisores pueden traer el mismo `sub` y serían dos personas distintas
//! viendo la misma organización.

use crate::base::Tx;
use crate::verbos;
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::{Identidad, Proveedor, SinIdentidad};
use postgres::Client;
use std::sync::Mutex;

/// Cuánto vive una invitación. Una semana: lo que tarda alguien en volver de
/// vacaciones, y no tanto como para que un vale olvidado siga sirviendo meses
/// después de que nadie recuerde haberlo emitido.
const DIAS_DE_LA_INVITACION: i64 = 7;

pub struct Servidor {
    pub base: Mutex<Client>,
    /// El emisor contra el que se validan los tokens. Va aquí porque la
    /// identidad de una persona es `(emisor, sub)`, no `sub`.
    pub emisor: String,
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
                Ok(sujeto) => self.con_sujeto(p, &sujeto, &seg),
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

    fn con_sujeto(&self, p: &Peticion, s: &Identidad, seg: &[&str]) -> Respuesta {
        match (p.metodo.as_str(), seg) {
            ("GET", ["organizaciones"]) => self.organizaciones(s),
            ("GET", ["organizaciones", o, "miembros"]) => self.miembros(s, o),
            ("GET", ["organizaciones", o, "roles"]) => self.roles(s, o),
            ("GET", ["organizaciones", o, "invitaciones"]) => self.invitaciones(s, o),
            ("POST", ["organizaciones", o, "invitaciones"]) => self.invitar(s, o, &p.cuerpo),
            ("POST", ["organizaciones", o, "concesiones"]) => self.conceder(s, o, &p.cuerpo),
            ("POST", ["invitaciones", "admitir"]) => self.admitir(s, &p.cuerpo),
            ("POST", ["concesiones", c, "revocar"]) => self.revocar(s, c),
            ("GET" | "POST", _) => Respuesta::error(404, "no hay nada en ese camino"),
            _ => Respuesta::error(405, "método no admitido"),
        }
    }

    /// Abre la transacción, corre `f`, y confirma. Si `f` falla no se confirma
    /// nada — y como la huella va dentro, un error tampoco deja rastro de algo
    /// que no ocurrió.
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
        // ⭐ Antes de nada: el nombre que trae el token. Va aqui y no en cada
        //   verbo porque es de la SESION, no del acto — y asi no hay un camino
        //   por el que alguien entre y su nombre se quede viejo.
        if let Err(e) = crate::verbos::refrescar_nombre(&mut tx, &self.emisor, s) {
            return Respuesta::error(500, e);
        }
        match f(&mut tx, &self.emisor) {
            Err(e) => Respuesta::error(422, e),
            Ok(j) => match tx.confirmar() {
                Ok(()) => Respuesta::ok(j),
                Err(e) => Respuesta::error(500, e),
            },
        }
    }

    // ── leer ────────────────────────────────────────────────────────────────

    fn organizaciones(&self, s: &Identidad) -> Respuesta {
        self.en_transaccion(s, |tx, emisor| {
            // ⛔ Sólo las SUYAS. Un listado que devolviera todas sería una fuga
            //   con forma de comodidad, y la consola no sabría que la tuvo.
            let filas = tx.filas(
                "select o.id, o.nombre, o.estado, pe.rol
                   from iam.organizacion o
                   join iam.pertenencia pe on pe.organizacion = o.id
                   join iam.persona     p  on p.id = pe.persona
                  where p.emisor = $1 and p.sub = $2
                  order by o.nombre",
                &[&emisor, &s.persona],
            )?;
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("id", Json::s(f.get::<_, String>(0))),
                        ("nombre", Json::s(f.get::<_, String>(1))),
                        ("estado", Json::s(f.get::<_, String>(2))),
                        // ⛔ `Option`: desde la `014` `rol` es nulable —`null` es
                        //   «pertenece y nada mas»— y leerlo como `String` panicaria
                        //   en la primera fila sin cargo.
                        (
                            "rol",
                            Json::s(f.get::<_, Option<String>>(3).unwrap_or_default()),
                        ),
                    ])
                })
                .collect();
            tx.anotar(
                "organizacion:listar",
                "*",
                Json::obj([("cuantas", Json::Int(lista.len() as i64))]),
            )?;
            Ok(Json::obj([("organizaciones", Json::Arr(lista))]))
        })
    }

    /// Quien esta dentro, y con que rol.
    ///
    /// ⭐ Basta con PERTENECER. Su `76` §2 lo argumenta y se toma entero:
    /// esconder quien manda es seguridad por oscuridad — y la pantalla que lo
    /// pinta existe para que alguien pueda preguntarle a la persona correcta.
    ///
    /// ⚠️ Y no hay `tipo`. `iam` no modela agentes todavia: `concesion.sujeto`
    /// es texto para que quepa uno, pero aqui no hay ninguno que listar.
    /// Devolver `"persona"` en todas las filas seria afirmar una distincion que
    /// este plano no sabe hacer.
    fn miembros(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            crate::potestad::exige(tx, emisor, &s.persona, &org, "miembro:listar")?;
            // ⭐ `roles` en plural desde la `016`: una persona puede tener
            //   varios cargos, y `array_agg` sobre el `left join` devuelve una
            //   lista vacia para quien solo pertenece — que es lo que
            //   «pertenece y nada mas» significa, sin necesitar un nulo.
            let filas = tx.filas(
                "select p.id, p.nombre, coalesce(p.correo, ''),
                        coalesce(array_agg(pr.rol) filter (where pr.rol is not null), '{}'),
                        to_char(pe.desde at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                   from iam.pertenencia pe
                   join iam.persona p on p.id = pe.persona
                   left join iam.pertenencia_rol pr
                     on pr.persona = pe.persona and pr.organizacion = pe.organizacion
                  where pe.organizacion = $1
                  group by p.id, p.nombre, p.correo, pe.desde
                  order by pe.desde",
                &[&org],
            )?;
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    let nombre: Option<String> = f.get(1);
                    let roles: Vec<String> = f.get(3);
                    Json::obj([
                        ("persona", Json::s(f.get::<_, String>(0))),
                        // ⭐ `conocido` false NO significa inactivo: significa que
                        //   el emisor nunca nos dijo como se llama.
                        ("conocido", Json::Bool(nombre.is_some())),
                        ("nombre", Json::s(nombre.unwrap_or_default())),
                        ("correo", Json::s(f.get::<_, String>(2))),
                        ("roles", Json::Arr(roles.into_iter().map(Json::s).collect())),
                        ("desde", Json::s(f.get::<_, String>(4))),
                    ])
                })
                .collect();
            tx.anotar(
                "miembro:listar",
                &org,
                Json::obj([("cuantos", Json::Int(lista.len() as i64))]),
            )?;
            Ok(Json::obj([("miembros", Json::Arr(lista))]))
        })
    }

    /// El catálogo Y las asignaciones, en una respuesta.
    ///
    /// ⭐ Las dos juntas por su motivo, que sigue siendo bueno: *«si la interfaz
    /// tuviera su propia copia de las potestades habría dos descripciones de lo
    /// mismo, y divergirían el día que se añada una»*.
    ///
    /// ⚠️ Y `porRol` es lo que cada cargo **AÑADE** sobre el estado por defecto,
    /// no todo lo que da. Es la forma que pinta la pantalla —*«qué añade sobre
    /// el estado por defecto»*— y la que contesta la pregunta de quien va a
    /// conceder.
    fn roles(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            // ⭐ Basta con pertenecer: `rol:listar` es del estado por defecto.
            //   Esconder quien manda seria seguridad por oscuridad (`76` §2).
            crate::potestad::exige(tx, emisor, &s.persona, &org, "rol:listar")?;

            let por_defecto: Vec<Json> = tx
                .filas(
                    "select potestad from iam.por_defecto order by potestad",
                    &[],
                )?
                .iter()
                .map(|f| Json::s(f.get::<_, String>(0)))
                .collect();

            // ⚠️ TODOS los roles, tengan o no a alguien: la pantalla explica el
            //   catalogo, y un rol que nadie tiene sigue siendo parte de el.
            //   `SECURITYADMIN` sale con su lista y con su nota.
            let filas = tx.filas(
                "select r.nombre, coalesce(r.que_puede, ''), coalesce(r.nota, ''),
                        coalesce(array_agg(rp.potestad order by rp.potestad)
                                 filter (where rp.potestad is not null), '{}')
                   from iam.rol r
                   left join iam.rol_potestad rp on rp.rol = r.nombre
                  group by r.nombre, r.que_puede, r.nota
                  order by r.nombre",
                &[],
            )?;
            // ⭐ Un `BTreeMap`: `Json::Obj` lo es, y ademas deja las claves
            //   ordenadas —que es lo que la forma canonica (JCS) pide.
            let roles: std::collections::BTreeMap<String, Json> = filas
                .iter()
                .map(|f| {
                    let anade: Vec<String> = f.get(3);
                    (
                        f.get::<_, String>(0),
                        Json::obj([
                            ("que_puede", Json::s(f.get::<_, String>(1))),
                            // ⭐ La nota va a la pantalla. `SECURITYADMIN` dice
                            //   ahi que hoy es una carcasa, y que se vea es la
                            //   unica forma de que un rol vacio no parezca lleno.
                            ("nota", Json::s(f.get::<_, String>(2))),
                            ("anade", Json::Arr(anade.into_iter().map(Json::s).collect())),
                        ]),
                    )
                })
                .collect();

            let asignaciones: Vec<Json> = tx
                .filas(
                    "select p.id, pr.rol,
                            to_char(pr.desde at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
                            pr.otorgo, p.nombre, p.correo
                       from iam.pertenencia_rol pr
                       join iam.persona p on p.id = pr.persona
                      where pr.organizacion = $1
                      order by pr.desde",
                    &[&org],
                )?
                .iter()
                .map(|f| {
                    Json::obj([
                        ("sujeto", Json::s(f.get::<_, String>(0))),
                        ("rol", Json::s(f.get::<_, String>(1))),
                        ("desde", Json::s(f.get::<_, String>(2))),
                        // ⭐ Vacio = EL PROVEEDOR, en el aprovisionamiento. Es el
                        //   unico caso legitimo, y la pantalla lo distingue.
                        (
                            "concedido_por",
                            Json::s(f.get::<_, Option<String>>(3).unwrap_or_default()),
                        ),
                        (
                            "nombre",
                            Json::s(f.get::<_, Option<String>>(4).unwrap_or_default()),
                        ),
                        (
                            "correo",
                            Json::s(f.get::<_, Option<String>>(5).unwrap_or_default()),
                        ),
                    ])
                })
                .collect();

            tx.anotar(
                "rol:listar",
                &org,
                Json::obj([("cuantas", Json::Int(asignaciones.len() as i64))]),
            )?;
            Ok(Json::obj([
                (
                    "catalogo",
                    Json::obj([
                        ("porDefecto", Json::Arr(por_defecto)),
                        // ⛔ `Json::obj` exige claves `&'static str`, y estas son nombres de
                        //   rol que salen de la base. Se construye la variante.
                        ("porRol", Json::Obj(roles)),
                    ]),
                ),
                ("asignaciones", Json::Arr(asignaciones)),
            ]))
        })
    }

    fn invitaciones(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            crate::potestad::exige(tx, emisor, &s.persona, &org, "invitacion:listar")?;
            // ⚠️ `vale_resumen` NO sale. Listar invitaciones no puede ser una
            //   forma de conseguir vales — es justo la propiedad que la `010`
            //   compró al separar el `id` del vale.
            let filas = tx.filas(
                "select id, correo, rol, estado, emitida_en, caduca_en
                   from iam.invitacion_estado
                  where organizacion = $1 order by emitida_en desc",
                &[&org],
            )?;
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("id", Json::s(f.get::<_, String>(0))),
                        ("correo", Json::s(f.get::<_, String>(1))),
                        // ⛔ `Option`: desde la `014` `rol` es nulable —`null` es
                        //   «pertenece y nada mas»— y leerlo como `String` panicaria
                        //   en la primera fila sin cargo.
                        (
                            "rol",
                            Json::s(f.get::<_, Option<String>>(2).unwrap_or_default()),
                        ),
                        ("estado", Json::s(f.get::<_, String>(3))),
                    ])
                })
                .collect();
            tx.anotar(
                "invitacion:listar",
                &org,
                Json::obj([("cuantas", Json::Int(lista.len() as i64))]),
            )?;
            Ok(Json::obj([("invitaciones", Json::Arr(lista))]))
        })
    }

    // ── escribir ────────────────────────────────────────────────────────────

    fn invitar(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        self.en_transaccion(s, move |tx, emisor| {
            let c = analizar(&cuerpo)?;
            let correo = campo(&c, "correo").ok_or("falta `correo`")?;
            // ⭐ Sin `rol` es legitimo: invitar a pertenecer y nada mas. Antes
            //   era obligatorio porque la columna no admitia vacio.
            let rol = campo(&c, "rol");
            verbos::invitar(
                tx,
                s,
                emisor,
                &org,
                &correo,
                rol.as_deref(),
                DIAS_DE_LA_INVITACION,
            )
        })
    }

    fn admitir(&self, s: &Identidad, cuerpo: &str) -> Respuesta {
        let cuerpo = cuerpo.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            let c = analizar(&cuerpo)?;
            let vale = campo(&c, "vale").ok_or("falta `vale`")?;
            verbos::admitir(tx, s, emisor, &vale)
        })
    }

    fn conceder(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        self.en_transaccion(s, move |tx, emisor| {
            let c = analizar(&cuerpo)?;
            let a = campo(&c, "sujeto").ok_or("falta `sujeto`")?;
            let r = campo(&c, "recurso").ok_or("falta `recurso`")?;
            let p = campo(&c, "rol").ok_or("falta `rol`")?;
            verbos::conceder(tx, s, emisor, &org, &a, &r, &p)
        })
    }

    fn revocar(&self, s: &Identidad, id: &str) -> Respuesta {
        let id = id.to_string();
        self.en_transaccion(s, move |tx, emisor| verbos::revocar(tx, s, emisor, &id))
    }
}

// ── el cuerpo ───────────────────────────────────────────────────────────────

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
        ("GET", "/organizaciones", con),
        ("GET", "/organizaciones/{org}/miembros", con),
        ("GET", "/organizaciones/{org}/roles", con),
        ("GET", "/organizaciones/{org}/invitaciones", con),
        ("POST", "/organizaciones/{org}/invitaciones", con),
        ("POST", "/organizaciones/{org}/concesiones", con),
        ("POST", "/invitaciones/admitir", con),
        ("POST", "/concesiones/{id}/revocar", con),
    ]
}
