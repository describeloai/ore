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
    /// ⭐ La celda que la plataforma da a quien pide (0025 E6): `ORE_CELDA*`.
    ///   Sin ella, `POST /organizaciones` funda sin celda y `POST …/celdas` se
    ///   niega diciendolo — que es mejor que inventarse un cluster.
    pub celda: Option<crate::fundar::CeldaPlataforma>,
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
        // ── ⭐⭐ EL APROVISIONADOR TIENE DOS VERBOS Y NINGUNO MAS (0025 E5) ──
        //
        // Es un sujeto de MAQUINA con el claim `rubix_tipo=aprovisionador`, que
        // el IdP estampa a su cliente. Registra el agente de una celda y dice
        // que la celda esta aprovisionada. No lista organizaciones, no invita,
        // no concede: un aprovisionador robado puede decir «esta celda esta
        // lista» y registrar un sujeto sin potestad, y nada mas.
        //
        // Y al reves: esos dos verbos NO los hace una persona, por muy admin
        // que sea. Registrar un agente es un acto del reconciliador con lo que
        // el IdP acaba de crear; hacerlo a mano era el paso 0 del ⑨ del
        // aprovisionador, que esta etapa retira.
        let es_aprovisionador = s.tipo.as_deref() == Some("aprovisionador");
        let de_aprovisionador = matches!(
            (p.metodo.as_str(), seg),
            ("POST", ["organizaciones", _, "agentes"]) | ("POST", ["celdas", _, "aprovisionada"])
        );
        if es_aprovisionador != de_aprovisionador {
            return Respuesta::error(
                403,
                if es_aprovisionador {
                    "el aprovisionador solo registra agentes y da celdas por aprovisionadas"
                } else {
                    "eso lo hace el aprovisionador, no una persona"
                },
            );
        }
        match (p.metodo.as_str(), seg) {
            ("GET", ["organizaciones"]) => self.organizaciones(s),
            ("GET", ["organizaciones", o, "miembros"]) => self.miembros(s, o),
            ("GET", ["organizaciones", o, "roles"]) => self.roles(s, o),
            // ⭐ DÓNDE CORRE. La 0024 ④: el clúster es un hecho del plano de
            //   control y se emite desde aquí. Lo que NO dice es si responde —
            //   eso se pregunta por el camino, al `/salud` de su `ore-serve`.
            ("GET", ["organizaciones", o, "celdas"]) => self.celdas(s, o),
            ("GET", ["organizaciones", o, "invitaciones"]) => self.invitaciones(s, o),
            ("POST", ["organizaciones", o, "invitaciones"]) => self.invitar(s, o, &p.cuerpo),
            ("POST", ["organizaciones", o, "concesiones"]) => self.conceder(s, o, &p.cuerpo),
            ("POST", ["invitaciones", "admitir"]) => self.admitir(s, &p.cuerpo),
            ("POST", ["concesiones", c, "revocar"]) => self.revocar(s, c),
            ("POST", ["organizaciones", o, "agentes"]) => self.registrar_agente(s, o, &p.cuerpo),
            ("POST", ["celdas", c, "aprovisionada"]) => self.aprovisionada(s, c),
            // ⭐⭐ LOS DOS VERBOS DE LA 0025 E6: la cuenta, y una celda mas.
            ("POST", ["organizaciones"]) => self.fundar(s, &p.cuerpo),
            ("POST", ["organizaciones", o, "perfil"]) => self.editar_perfil(s, o, &p.cuerpo),
            ("POST", ["organizaciones", o, "celdas"]) => self.crear_celda(s, o, &p.cuerpo),
            ("POST", ["celdas", c, "retirar"]) => self.retirar_celda(s, c),
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
                // ⭐ `roles` en plural desde la `016`. Una persona puede tener
                //   varios cargos en la misma organizacion.
                // ⭐ `arbol` y `entrada` salen de la CELDA (029): la que se llama
                //   como la organizacion, que es la primera. Con N celdas esto es
                //   «la de casa»; la consola elige celda por `/celdas` (0025-5) y
                //   estas dos columnas se retiran de aqui con la 030.
                "select o.id, o.nombre, o.estado,
                        coalesce(array_agg(pr.rol order by pr.rol)
                                 filter (where pr.rol is not null), '{}'),
                        coalesce(c.arbol, ''), coalesce(c.entrada, ''),
                        o.titulo, o.logo
                   from iam.organizacion o
                   join iam.pertenencia pe on pe.organizacion = o.id
                   join iam.persona     p  on p.id = pe.persona
                   left join iam.pertenencia_rol pr
                     on pr.persona = pe.persona and pr.organizacion = pe.organizacion
                   left join iam.celda c on c.organizacion = o.id and c.nombre = o.nombre
                  where p.emisor = $1 and p.sub = $2
                  group by o.id, o.nombre, o.estado, c.arbol, c.entrada, o.titulo, o.logo
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
                        // ⭐ `roles` en plural desde la `016`: la lista vacia es
                        //   «pertenece y nada mas», sin necesitar un nulo.
                        (
                            "roles",
                            Json::Arr(
                                f.get::<_, Vec<String>>(3)
                                    .into_iter()
                                    .map(Json::s)
                                    .collect(),
                            ),
                        ),
                        // ⭐⭐ CÓMO SE LLAMA SU ÁRBOL, desde la `017`. Se devuelve
                        //   aquí porque si no la columna sería de sólo escritura,
                        //   que es la misma figura que este árbol lleva
                        //   encontrando una y otra vez: algo que se escribe y
                        //   nadie lee.
                        //
                        // ⚠️ Es un NOMBRE, no una dirección, y no dice que el
                        //   repositorio exista. Quien lo consuma tiene que
                        //   componer la URL con la forja que le toque — igual
                        //   que `EMISOR` y `DIRECCION` son dos cosas.
                        ("arbol", Json::s(f.get::<_, String>(4))),
                        // ⭐⭐ Y SU PUERTA, desde la `022`. Está aquí por el
                        //   mismo argumento de arriba —una columna que nadie
                        //   lee es de sólo escritura— y por uno más fuerte:
                        //   **es la respuesta a «¿a qué URL le pregunto por el
                        //   árbol de esta organización?»**, y quien tiene que
                        //   contestarla es el plano de control, no una
                        //   constante en la consola.
                        //
                        // ⚠️ Es un HOST, no una URL: sin esquema, sin puerto y
                        //   sin camino. Quien lo consuma compone `https://`,
                        //   igual que con `arbol` compone la URL de clon. Es la
                        //   misma distinción que `EMISOR` y `DIRECCION`, y ya
                        //   van cuatro.
                        ("entrada", Json::s(f.get::<_, String>(5))),
                        // ⭐ El PERFIL (035): titulo y logo, o vacio. `nombre` es el
                        //   identificador; esto es lo que la gente ve.
                        (
                            "titulo",
                            Json::s(f.get::<_, Option<String>>(6).unwrap_or_default()),
                        ),
                        (
                            "logo",
                            Json::s(f.get::<_, Option<String>>(7).unwrap_or_default()),
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
    fn celdas(&self, s: &Identidad, org: &str) -> Respuesta {
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            // ⭐ Basta con pertenecer, y se comprueba con la misma unión que
            //   `organizaciones`: la celda es un atributo de la organización, y
            //   «pertenecer ya da lectura» (`014`). Sin potestad nueva.
            // ⭐ Con su tier DENTRO —título, promesa y cuota— desde la `026`: es
            //   lo que la consola tiene que decir de cada celda, y decirlo desde
            //   aquí es lo que evita una segunda copia de la cuota en la consola.
            // ⭐ Y SIN LAS RETIRADAS (033): retirar es dejar de tenerla. La fila
            //   se queda —el reconciliador la desmonta por ella, y el nombre sigue
            //   reservado mientras tanto—, pero ya no es una celda de la
            //   organización: listarla era enseñar «Abrir» y «Retirar» sobre algo
            //   que se está desmontando, y como nunca la aprovisionan, la consola
            //   la pintaba «Provisioning» para siempre.
            let filas = tx.filas(
                "select c.id, c.nombre, c.tier, c.proveedor, c.region, c.estado, c.creada_en::text,
                        t.titulo, t.promesa, t.cuota_cpu, t.cuota_memoria, t.cuota_jobs,
                        c.puerta, c.cluster, c.arbol, c.entrada, c.aprovisionada::text
                   from iam.celda c
                   join iam.tier        t  on t.nombre = c.tier
                   join iam.pertenencia pe on pe.organizacion = c.organizacion
                   join iam.persona     p  on p.id = pe.persona
                  where c.organizacion = $1 and c.estado <> 'retirada'
                    and p.emisor = $2 and p.sub = $3
                  order by c.creada_en",
                &[&org, &emisor, &s.persona],
            )?;
            // ⛔ Cero filas puede ser «no perteneces» o «no tiene celda», y se
            //   contestan igual a propósito: decir cuál revelaría que la
            //   organización existe a quien no es de ella.
            let lista: Vec<Json> = filas
                .iter()
                .map(|f| {
                    let mut campos = vec![
                        ("id", Json::s(f.get::<_, String>(0))),
                        ("nombre", Json::s(f.get::<_, String>(1))),
                        ("tier", Json::s(f.get::<_, String>(2))),
                        ("proveedor", Json::s(f.get::<_, String>(3))),
                        ("region", Json::s(f.get::<_, String>(4))),
                        ("estado", Json::s(f.get::<_, String>(5))),
                        ("creada_en", Json::s(f.get::<_, String>(6))),
                        ("titulo", Json::s(f.get::<_, String>(7))),
                        ("promesa", Json::s(f.get::<_, String>(8))),
                        // ⭐ La puerta de la celda (027): un NOMBRE al que la
                        //   entrada de la organización tiene que resolver. La
                        //   consola coteja que el mundo converja — es la mitad
                        //   «por el camino» de la 0024-④, aplicada al DNS.
                        ("puerta", Json::s(f.get::<_, String>(12))),
                        // ⭐ Desde la 029 (0025): `nombre` es el de la CELDA; el
                        //   cluster va aparte, y el arbol y la entrada son suyos.
                        //   Es lo que la consola necesita para elegir celda (⑤).
                        ("cluster", Json::s(f.get::<_, String>(13))),
                        ("arbol", Json::s(f.get::<_, String>(14))),
                        ("entrada", Json::s(f.get::<_, String>(15))),
                    ];
                    // ⭐ Cuando el aprovisionador dio la ultima pasada entera
                    //   (032, 0025 E5). Se OMITE si todavia ninguna —como la
                    //   cuota—: la consola lo pinta «Provisioning» sin preguntar
                    //   por el camino.
                    if let Some(cuando) = f.get::<_, Option<String>>(16) {
                        campos.push(("aprovisionada", Json::s(cuando)));
                    }
                    // ⭐ La SALIDA: por que IPs sale la celda hacia las fuentes
                    //   del cliente (lo que abre en su firewall). Es del cluster
                    //   fisico y la sabe este servidor por `ORE_CELDA_SALIDA`;
                    //   solo para las celdas de SU cluster, y se omite si no la
                    //   hay: una lista vacia diria «sale por ninguna».
                    if let Some(p) = self
                        .celda
                        .as_ref()
                        .filter(|p| !p.salida.is_empty() && f.get::<_, String>(13) == p.cluster)
                    {
                        campos.push((
                            "salida",
                            Json::Arr(p.salida.iter().cloned().map(Json::s).collect()),
                        ));
                    }
                    // ⚠️ La cuota se OMITE cuando el tier no la tiene —dedicado,
                    //   byoc—. Un objeto con nulos diría «tiene cuota y no sé
                    //   cuál»; ausente dice lo cierto: no hay cuota de plataforma.
                    if let (Some(c), Some(m), Some(j)) = (
                        f.get::<_, Option<String>>(9),
                        f.get::<_, Option<String>>(10),
                        f.get::<_, Option<String>>(11),
                    ) {
                        campos.push((
                            "cuota",
                            Json::obj([
                                ("cpu", Json::s(c)),
                                ("memoria", Json::s(m)),
                                ("jobs", Json::s(j)),
                            ]),
                        ));
                    }
                    Json::obj(campos)
                })
                .collect();
            // Toda lectura deja huella: es la regla de `confirmar()`, y CI la
            // hizo valer — sin esto, 500 «no dejó huella. No se confirma».
            tx.anotar(
                "celda:listar",
                &org,
                Json::obj([("cuantas", Json::Int(lista.len() as i64))]),
            )?;
            Ok(Json::obj([("celdas", Json::Arr(lista))]))
        })
    }

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

            // ⭐⭐ EL CATALOGO DE POTESTADES, CON `ejercida`.
            //
            //   La columna existe para que la pantalla pueda decir «todavia
            //   no» en vez de ofrecer algo que devuelve 404, y sin devolverla
            //   no servia de nada. Es lo que hace que `SECURITYADMIN` se lea
            //   como lo que es: un rol con UNA potestad que aun no se ejerce,
            //   no un rol vacio.
            let potestades: std::collections::BTreeMap<String, Json> = tx
                .filas(
                    "select nombre, que_hace, ejercida from iam.potestad order by nombre",
                    &[],
                )?
                .iter()
                .map(|f| {
                    (
                        f.get::<_, String>(0),
                        Json::obj([
                            ("que_hace", Json::s(f.get::<_, String>(1))),
                            ("ejercida", Json::Bool(f.get::<_, bool>(2))),
                        ]),
                    )
                })
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
                        ("potestades", Json::Obj(potestades)),
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

    // ── los de la 0025 E6 ──────────────────────────────────────────────────

    /// `POST /organizaciones` `{nombre}`: fundar por HTTP. Quien pide es el
    /// dueño — el argumento «fundar es de operador porque decide el dueño» se
    /// disolvio al medirlo: el dueño es quien pide, y el token ya trae quien es.
    ///
    /// ⚠️ Quien puede pedir: cualquier persona con sesion, UNA organizacion por
    ///   peticion. Es una decision y no codigo, y esta dicha en la 0025: sin
    ///   cuota de organizaciones por persona es abrir cuentas gratis.
    fn fundar(&self, s: &Identidad, cuerpo: &str) -> Respuesta {
        let cuerpo = cuerpo.to_string();
        self.en_transaccion_si_cambia(s, move |tx, emisor| {
            let n = analizar(&cuerpo)?;
            let nombre = campo(&n, "nombre").ok_or("falta `nombre`: el de la organizacion")?;
            // ⛔ Por HTTP «ya estaba» no es un «ya»: es OTRA organizacion con ese
            //   nombre, y quien pide no tiene por que ser de ella. Se niega.
            if tx
                .uno(
                    "select 1 from iam.organizacion where nombre = $1",
                    &[&nombre],
                )?
                .is_some()
            {
                return Err(format!("ya hay una organizacion que se llama `{nombre}`"));
            }
            let celda = self.celda.as_ref().map(|c| c.como_celda());
            let titulo = campo(&n, "titulo");
            let p = crate::fundar::Peticion {
                organizacion: &nombre,
                emisor,
                sub: &s.persona,
                correo: s.correo.as_deref(),
                kek: None,
                titulo: titulo.as_deref(),
                arbol: None,
                entrada: None,
                celda,
            };
            crate::fundar::fundar_en(tx, &p)
        })
    }

    /// `POST /organizaciones/{org}/perfil` `{titulo?, logo?}`: como se llama y
    /// como se ve. `organizacion:editar` (ORGADMIN).
    fn editar_perfil(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        self.en_transaccion(s, move |tx, emisor| {
            let n = analizar(&cuerpo)?;
            let titulo = campo(&n, "titulo");
            let logo = campo(&n, "logo");
            crate::fundar::editar_perfil_en(tx, emisor, s, &org, titulo.as_deref(), logo.as_deref())
        })
    }

    /// `POST /organizaciones/{org}/celdas` `{nombre, tier}`: una celda mas.
    fn crear_celda(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        self.en_transaccion(s, move |tx, emisor| {
            let n = analizar(&cuerpo)?;
            let nombre = campo(&n, "nombre").ok_or("falta `nombre`: el de la celda")?;
            let tier = campo(&n, "tier").unwrap_or_else(|| "compartido".into());
            let plataforma = self.celda.as_ref().ok_or(
                "este servidor no tiene celda de plataforma configurada (ORE_CELDA*): no puede dar celdas",
            )?;
            crate::fundar::crear_celda_en(tx, emisor, s, &org, &nombre, &tier, plataforma)
        })
    }

    /// `POST /celdas/{celda}/retirar`: la fila pasa a `retirada`; el
    /// reconciliador desmonta.
    fn retirar_celda(&self, s: &Identidad, celda: &str) -> Respuesta {
        let celda = celda.to_string();
        self.en_transaccion_si_cambia(s, move |tx, emisor| {
            crate::fundar::retirar_celda_en(tx, emisor, s, &celda)
        })
    }

    // ── los del aprovisionador ──────────────────────────────────────────────

    /// `POST /organizaciones/{org}/agentes` `{sub, nombre}`: el mismo nucleo
    /// que `ore-iam agente`, con la huella del aprovisionador. Idempotente.
    fn registrar_agente(&self, s: &Identidad, org: &str, cuerpo: &str) -> Respuesta {
        let (org, cuerpo) = (org.to_string(), cuerpo.to_string());
        self.en_transaccion_si_cambia(s, move |tx, emisor| {
            let n = analizar(&cuerpo)?;
            let sub =
                campo(&n, "sub").ok_or("falta `sub`: el de la cuenta de servicio del cliente")?;
            let nombre = campo(&n, "nombre");
            crate::fundar::registrar_agente_en(tx, &org, emisor, &sub, nombre.as_deref())
        })
    }

    /// `POST /celdas/{celda}/aprovisionada`: la ultima pasada entera del
    /// aprovisionador sobre esa celda acabo ahora. Es lo que el patron de
    /// operador llama `status`: lo escribe quien reconcilia, no quien pide.
    fn aprovisionada(&self, s: &Identidad, celda: &str) -> Respuesta {
        let celda = celda.to_string();
        self.en_transaccion(s, move |tx, _| {
            let f = tx
                .uno(
                    "update iam.celda set aprovisionada = now()
                      where nombre = $1
                  returning id, aprovisionada::text",
                    &[&celda],
                )?
                .ok_or_else(|| format!("no hay ninguna celda `{celda}`"))?;
            let (id, cuando): (String, String) = (f.get(0), f.get(1));
            tx.anotar(
                "celda:aprovisionada",
                &id,
                Json::obj([("celda", Json::s(&celda))]),
            )?;
            Ok(Json::obj([
                ("celda", Json::s(celda)),
                ("aprovisionada", Json::s(cuando)),
            ]))
        })
    }

    /// Como `en_transaccion`, para un verbo que puede no cambiar nada: si `f`
    /// dice `false`, la transaccion se suelta sin confirmar y se contesta igual.
    /// Es lo que un reconciliador pide: llamar sin mirar, y que «ya estaba» no
    /// sea ni un error ni una huella.
    fn en_transaccion_si_cambia(
        &self,
        s: &Identidad,
        f: impl FnOnce(&mut Tx, &str) -> Result<(Json, bool), String>,
    ) -> Respuesta {
        let Ok(mut base) = self.base.lock() else {
            return Respuesta::error(500, "la conexión quedó envenenada");
        };
        let mut tx = match Tx::abrir(&mut base, s) {
            Ok(t) => t,
            Err(e) => return Respuesta::error(502, e),
        };
        if let Err(e) = crate::verbos::refrescar_nombre(&mut tx, &self.emisor, s) {
            return Respuesta::error(500, e);
        }
        match f(&mut tx, &self.emisor) {
            Err(e) => Respuesta::error(422, e),
            Ok((j, false)) => Respuesta::ok(j),
            Ok((j, true)) => match tx.confirmar() {
                Ok(()) => Respuesta::ok(j),
                Err(e) => Respuesta::error(500, e),
            },
        }
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
        ("GET", "/organizaciones/{org}/celdas", con),
        ("GET", "/organizaciones/{org}/invitaciones", con),
        ("POST", "/organizaciones/{org}/invitaciones", con),
        ("POST", "/organizaciones/{org}/concesiones", con),
        ("POST", "/invitaciones/admitir", con),
        ("POST", "/concesiones/{id}/revocar", con),
        ("POST", "/organizaciones/{org}/agentes", con),
        ("POST", "/celdas/{celda}/aprovisionada", con),
        ("POST", "/organizaciones", con),
        ("POST", "/organizaciones/{org}/perfil", con),
        ("POST", "/organizaciones/{org}/celdas", con),
        ("POST", "/celdas/{celda}/retirar", con),
    ]
}
