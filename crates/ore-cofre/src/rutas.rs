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

use crate::almacen::{Almacen, nombre_en_almacen};
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
    /// Dónde vive el material desde la 0024-⑤: el Secret Manager de la celda.
    pub almacen: Almacen,
    /// De qué celda es este cofre (0025-④). Un secreto nace de ella.
    pub celda: String,
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
        let almacen = &self.almacen;
        let self_celda = self.celda.clone();
        self.en_transaccion(s, move |tx, emisor| {
            // ⓪ El nombre o el id, a ID. Ver `canonica`: aqui llegaba `demo` y
            //    todo lo de abajo pregunta por `org_b7b98fdd…`.
            let org = canonica(tx, &org)?;
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

            // ② Con qué llave se cierra —de la ORGANIZACION, que es la cuenta— y
            //    de qué CELDA es el secreto (0025-④): la de este cofre, que tiene
            //    que ser de esa organizacion. El nombre de la celda va DELANTE del
            //    nombre en el almacén porque es lo que la condición IAM evalúa.
            let f = tx
                .uno(
                    "select o.kek, c.id, c.nombre
                       from iam.organizacion o
                       join iam.celda c on c.organizacion = o.id and c.nombre = $2
                      where o.id = $1",
                    &[&org, &self_celda],
                )?
                .ok_or_else(|| {
                    format!("la celda `{self_celda}` de este cofre no es de esa organizacion")
                })?;
            let (kek, celda_id, inquilino): (String, String, String) =
                (f.get(0), f.get(1), f.get(2));

            // ③ El METADATO, en la base del plano de control — y dentro de la
            //    transacción, así que si el almacén dice que no, no queda una
            //    fila apuntando a un material que no está. Con su celda: desde la
            //    029 la fila la lleva, y el disparador que la ponía por defecto
            //    deja de hacer falta.
            let id = nuevo_id("sec");
            let quien = verbos::persona_id(tx, emisor, &s.persona)?;
            tx.ejecutar(
                "insert into cofre.secreto (id, organizacion, nombre, clase, emitio, celda)
                 values ($1, $2, $3, $4, $5, $6)",
                &[&id, &org, &nombre, &clase, &quien, &celda_id],
            )?;

            // ── ⭐⭐ Y EL MATERIAL, EN LA CELDA ──────────────────────────────
            //
            // Es la 0024-⑤. El valor va al Secret Manager de la celda por el
            // cliente de la nube —entrada estándar, sin tocar el disco—, cifrado
            // con la KEK de la organización como CMEK. `cofre.material` deja de
            // existir para lo nuevo; lo viejo lo muda `ore-cofre mudar`.
            //
            // ⚠️ Lo que NO es atómico, dicho: si el almacén escribe y la
            //   confirmación de abajo falla, queda un secreto en el almacén sin
            //   fila. No es una fuga —sólo lo lee `ore-cofre-<inq>`— y `crear` es
            //   idempotente: el siguiente `emitir` con el mismo nombre lo
            //   reutiliza y añade otra versión.
            let en_almacen = nombre_en_almacen(&inquilino, &nombre);
            almacen.crear(&en_almacen, &kek, &inquilino)?;
            let version = almacen.anadir(&en_almacen, valor.as_bytes())?;

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

            // ── ⭐⭐ Y AL AGENTE DEL INQUILINO, `usar` ──────────────────
            //
            // Un secreto de clase `conexion` existe PARA que lo use un Job. Sin
            // esta concesion nace inerte: medido el 2026-09-10, el primer
            // secreto que llego al cofre no lo pudo abrir nadie.
            //
            // ⛔ `usar` y no `owner`: el agente saca el valor y no puede
            //   conceder ni retirar. Es el rol que la `018` metio en
            //   `rol_de_recurso` pensando exactamente en esto.
            //
            // ⚠️ Y SOLO a los agentes de ESTA organizacion. La `024` ata cada
            //   agente a un inquilino, asi que esta consulta no puede alcanzar
            //   al de otro aunque compartan el `sub` del IdP — que hoy lo
            //   comparten, porque `ore-agente` es el mismo cliente en todos los
            //   namespaces.
            //
            // ⭐ Si no hay agente registrado no pasa nada y no se avisa: una
            //   organizacion sin Jobs es un caso legitimo, y un aviso que se
            //   dispara en el caso normal enseña a ignorarlo.
            for f in tx.filas("select id from iam.agente where organizacion = $1", &[&org])? {
                let age: String = f.get(0);
                tx.ejecutar(
                    "select iam.conceder_de_secreto($1, $2, $3, 'usar', $4, $5)",
                    &[&nuevo_id("con"), &age, &recurso, &quien, &org],
                )?;
            }

            tx.anotar(
                "secreto:emitir",
                &id,
                Json::obj([
                    ("organizacion", Json::s(&org)),
                    ("nombre", Json::s(&nombre)),
                    ("clase", Json::s(&clase)),
                    ("kek", Json::s(&kek)),
                    // Dónde quedó, y qué versión le dio el almacén: es lo que
                    // permite cotejar la huella contra el almacén sin abrir nada.
                    ("almacen", Json::s(&en_almacen)),
                    ("version", Json::Int(version)),
                ]),
            )?;

            // ⛔ Y NO devuelve el valor. Quien lo acaba de escribir ya lo tiene;
            //   devolverlo sería una segunda copia por un camino que no es el de
            //   resolver, y por tanto una que se audita distinto.
            Ok(Json::obj([
                ("secreto", Json::s(id)),
                ("nombre", Json::s(nombre)),
                ("version", Json::Int(version)),
                ("concesion", Json::s(con)),
            ]))
        })
    }

    // ── listar ──────────────────────────────────────────────────────────────

    fn listar(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        let self_celda = self.celda.clone();
        self.en_transaccion(s, move |tx, emisor| {
            // ⓪ El nombre o el id, a ID. Ver `canonica`.
            let org = canonica(tx, &org)?;
            // ⚠️ Ver los nombres de TODOS —incluidos los que no te han
            //   concedido— destapa qué sistemas hay y cómo se llaman. Por eso
            //   cuelga de una potestad y no del estado por defecto, igual que
            //   `invitacion:listar` con los correos.
            potestad::exige(tx, emisor, &s.persona, &org, "secreto:listar")?;
            // ⛔ Ya no dice cuántas versiones hay: eso lo sabe el almacén, y
            //   preguntárselo serían N llamadas al cliente para contestar una
            //   lista. Nadie lo leía — medido en la consola y en las pruebas.
            // ⭐ Los de ESTA celda (0025-④): un cofre lista y abre los suyos. Con
            //   dos celdas en la organizacion, una fuente `pg` en cada una son
            //   dos secretos, y cada cofre ve el suyo.
            let filas = tx.filas(
                "select s.nombre, s.clase, (s.retirado_en is not null) as retirado
                   from cofre.secreto s
                   join iam.celda ce on ce.id = s.celda
                  where s.organizacion = $1 and ce.nombre = $2
                  order by s.nombre",
                &[&org, &self_celda],
            )?;
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    Json::obj([
                        ("nombre", Json::s(f.get::<_, String>(0))),
                        ("clase", Json::s(f.get::<_, String>(1))),
                        ("retirado", Json::Bool(f.get::<_, bool>(2))),
                    ])
                })
                .collect();
            tx.anotar(
                "secreto:listar",
                &org,
                Json::obj([("cuantos", Json::Int(lista.len() as i64))]),
            )?;
            // ⛔ Nombres y clases. Ni un valor.
            Ok(Json::obj([("secretos", Json::Arr(lista))]))
        })
    }

    // ── resolver ────────────────────────────────────────────────────────────

    fn resolver(&self, s: &Identidad, org: &str, nombre: &str) -> Respuesta {
        let (org, nombre) = (org.to_string(), nombre.to_string());
        let almacen = &self.almacen;
        let self_celda = self.celda.clone();
        self.en_transaccion(s, move |tx, emisor| {
            // ⓪ El nombre o el id, a ID. Ver `canonica`.
            let org = canonica(tx, &org)?;
            // ── ⭐⭐ QUIEN PIDE PUEDE NO SER UNA PERSONA ────────────────────
            //
            // Aqui ponia `persona_id`, y el primer Job de catalogo que llego a
            // pedir de verdad murio con «quien pide no es una persona conocida
            // aqui». No era un fallo: era el modelo diciendo la verdad. Un Job
            // no es una persona.
            //
            // ⇒ `sujeto_id` acepta las dos puertas de la `024`. Y el agente va
            //   POR INQUILINO: el mismo `sub` del IdP —`ore-agente` es el mismo
            //   cliente en todos los namespaces, medido— da un sujeto distinto
            //   en cada organizacion, asi que conceder `usar` en `demo` no
            //   concede nada en `prueba`.
            //
            // ⛔ Y `emitir` sigue con `persona_id`, sin tocar: la `018` puso
            //   `secreto:emitir` en una PERSONA a proposito. Un agente saca lo
            //   que ya existe; no decide que exista.
            let (quien, clase) = verbos::sujeto_id(tx, emisor, &s.persona, &org)?;
            let recurso = format!("secreto/{nombre}");

            // ⛔⛔ UNA SOLA CONSULTA, Y UN SOLO ERROR. Se pregunta por el
            //   secreto Y por la concesión a la vez: así no hay un camino en el
            //   que el código sepa que el secreto existe antes de saber si quien
            //   pregunta puede. Y el mensaje es el mismo en los tres casos —no
            //   existe, no es tuyo, no te lo han concedido— porque distinguirlos
            //   convierte esta ruta en un directorio de lo ajeno.
            //
            // ⭐ La invariante SOBREVIVE a la mudanza del material (0024-⑤)
            //   porque el metadato se quedó aquí: el `join` sigue devolviendo
            //   nada si no puedes, y SÓLO DESPUÉS se va al almacén de la celda a
            //   por el valor. Era la única invariante que el traslado tocaba,
            //   medida en `medida-el-cofre-y-su-almacen.py`, y así no se toca.
            // ⭐ Y el nombre de la CELDA del secreto (029), no el de la
            //   organizacion: es lo que va delante en el almacén.
            let f = tx
                .uno(
                    "select c.rol, ce.nombre
                       from cofre.secreto s
                       join iam.celda ce on ce.id = s.celda and ce.nombre = $5
                       join iam.concesion_viva c
                         on c.recurso = $3 and c.organizacion = s.organizacion
                        and c.sujeto = $4 and c.rol in ('usar', 'lector', 'owner')
                      where s.organizacion = $1 and s.nombre = $2
                        and s.retirado_en is null",
                    &[&org, &nombre, &recurso, &quien, &self_celda],
                )?
                .ok_or("ese secreto no existe o no es tuyo")?;

            let (rol, inquilino): (String, String) = (f.get(0), f.get(1));

            // ⭐ Del almacén de la celda, la última versión. Con qué llave se
            //   cerró lo sabe el almacén: cada versión lleva la CMEK que había,
            //   y rotar la de la organización no cierra lo viejo.
            let (claro, version) = almacen.leer(&nombre_en_almacen(&inquilino, &nombre))?;
            let claro = String::from_utf8(claro)
                .map_err(|_| "lo que devolvio el almacen no es texto".to_string())?;

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
                    // ⭐ Y DE QUE CLASE es quien lo saco. `huella.quien` guarda
                    //   el `sub` en crudo —no tiene clave ajena— asi que sin
                    //   esto un Job y una persona se leen igual en la auditoria,
                    //   y la `008` separa esas dos preguntas a proposito.
                    ("clase", Json::s(clase)),
                    ("version", Json::Int(version)),
                ]),
            )?;

            Ok(Json::obj([
                ("nombre", Json::s(&nombre)),
                ("version", Json::Int(version)),
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

/// ⭐⭐ EL ID CANONICO DE UNA ORGANIZACION, VENGA POR NOMBRE O POR ID.
///
/// ⛔ Esto faltaba, y costo el primer secreto que este custodio tenia que
///   guardar. `ore-serve` arranca con `--organizacion demo` —el nombre— y todo
///   este fichero trataba el segmento de la URL como el ID:
///
///     potestad::exige(…, "demo", "secreto:emitir")
///       → select … where pp.organizacion = 'demo'
///       → CERO FILAS, porque esa columna guarda `org_b7b98fdd…`
///
///   Y el sintoma no decia nada de eso: «no puedes hacer eso en esa
///   organizacion», o sea un problema de permisos donde habia un problema de
///   unidades. La persona SI tenia `secreto:emitir`; se le preguntaba por otra
///   organizacion que no existe.
///
/// ⭐ Se resuelve AQUI y no en quien llama, y esa es la decision: el nombre es
///   unico y esta en la fila —`017`, `019`, `022`—, asi que convertirlo en el
///   identificador interno es trabajo de quien recibe. Arreglarlo en
///   `ore-serve` habria dejado al siguiente cliente del custodio tropezando con
///   lo mismo, y ademas habria metido un identificador opaco de Keycloak en la
///   plantilla de cada inquilino.
///
/// ⚠️ No hay ambiguedad posible: un id es `org_<hex>` y un nombre no admite `_`
///   —`nombre_valido` lo prohibe—. Aun asi el id gana, escrito y no supuesto.
fn canonica(tx: &mut Tx, org: &str) -> Result<String, String> {
    Ok(tx
        .uno(
            "select id from iam.organizacion
              where id = $1 or nombre = $1
              order by (id = $1) desc
              limit 1",
            &[&org],
        )?
        .ok_or_else(|| format!("`{org}` no es ninguna organizacion"))?
        .get(0))
}
