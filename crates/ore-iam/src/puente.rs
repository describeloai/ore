//! **El puente** (0047 A2): lo que `ore-iam` contesta a una celda.
//!
//! ```text
//!   POST /access/v1/evaluation    ¿puede?            AuthZEN 1.0
//!   POST /access/v1/evaluations   ¿puede?, en lote   AuthZEN 1.0
//!   POST /access/v1/eventos       lo que hizo        a la huella, con organización y celda
//!   POST /access/v1/quien         su handle          `user:<handle>`, el dueño de lo que crea (la 048)
//!   POST /access/v1/celda         de qué celda       lo pregunta un PRODUCTO con el token que la celda le dio (0058)
//! ```
//!
//! # Dos tokens en cada llamada (0047 § «El contrato»)
//!
//! | cabecera | qué | de qué emisor |
//! |---|---|---|
//! | `Authorization` | **la celda**: el token de Workload Identity de su `ore-serve`, `aud` la nuestra | el de las celdas (`--emisor-celdas`) |
//! | `Ore-Sujeto` | **quien pide**: el token del realm que trajo a `ore-serve`, tal cual | el realm (`--emisor`) |
//!
//! ⛔ **La organización no viaja nunca.** Sale de la celda —`(emisor, sub)` de su
//!   token → `iam.celda`, que escribe el aprovisionador—, y una celda sólo puede
//!   preguntar por la suya porque es lo único que se deduce de ella.
//!
//! ⭐ Las rutas del puente son de la clase `celda` y de ninguna otra: un token
//!   del realm en `Authorization` —el de una persona, el del aprovisionador— es
//!   un 403 de clase, como en la tabla de 0025 E5 y 0026.

use crate::base::{Tx, nuevo_id};
use crate::potestad;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::{Identidad, SinIdentidad};
use std::collections::BTreeMap;

/// Los segundos que `puede` puede guardar una respuesta (0047 § El contrato,
/// `context.vale`). **⟨M2⟩, de partida 30**: con esto, una revocación tarda como
/// mucho medio minuto en valer.
pub const VALE: i64 = 30;

/// Cuánto se guardan las decisiones de escritura para los reintentos de `hizo`
/// (M2 Q3: decenas al día por celda; 24 h sobra).
const RETENCION: &str = "24 hours";

/// Los resultados que un evento puede decir.
const RESULTADOS: &[&str] = &["hecho", "negado", "fallido", "en-curso"];

/// La celda que pregunta, ya resuelta.
struct Celda {
    id: String,
    organizacion: String,
}

/// Lo que se pregunta.
struct Pregunta {
    sujeto_id: String,
    accion: String,
    recurso_tipo: String,
    recurso_id: String,
    ruta: String,
    /// ⭐ `context.consulta: true` (0047 A5): «¿podría?», para que una pantalla
    ///   sepa qué botón enseñar. Se contesta igual, pero una denegación NO va a la
    ///   huella —nadie intentó nada— y un permiso no se guarda como decisión: no
    ///   deja pasar ningún acto.
    consulta: bool,
}

/// Lo que toda celda pregunta antes de atender (0047 A9′): ¿pertenece quien llega
/// a la organización de esta celda? Es la potestad que da pertenecer
/// (`iam.por_defecto`); a un agente se la da estar registrado en ella.
pub const PERTENENCIA: &str = "organizacion:leer";

impl Servidor {
    /// La puerta del puente. `None` si el camino no es del puente.
    pub fn puente(&self, p: &Peticion, seg: &[&str]) -> Option<Respuesta> {
        let ["access", "v1", resto @ ..] = seg else {
            return None;
        };
        let Some(celdas) = self.celdas.as_ref() else {
            return Some(Respuesta::error(
                404,
                "sin emisor de celdas configurado (`--emisor-celdas`), las rutas del puente no se montan",
            ));
        };
        if p.metodo != "POST" {
            return Some(Respuesta::error(405, "el puente sólo atiende POST"));
        }
        if resto == ["celda"] {
            return Some(self.de_que_celda(p, celdas));
        }
        // ① LA CELDA, por su token.
        let autorizacion = match p.cabeceras.get("authorization") {
            Some(a) => a,
            None => {
                return Some(Respuesta::error(
                    401,
                    "el puente necesita el token de la celda",
                ));
            }
        };
        let de_la_celda = match celdas.verificar(autorizacion, ahora()) {
            Ok(i) => i,
            Err(e) => {
                // ¿Es un token del realm? Entonces no es que no valga: es de otra clase.
                if self
                    .identidad
                    .as_ref()
                    .is_some_and(|realm| realm(&p.cabeceras).is_ok())
                {
                    return Some(Respuesta::error(
                        403,
                        "el puente es de las celdas: ni una persona ni el aprovisionador preguntan por aquí",
                    ));
                }
                return Some(Respuesta::error(
                    401,
                    match e {
                        SinIdentidad::Ausente => "el puente necesita el token de la celda".into(),
                        SinIdentidad::Invalida(m) => format!("el token de la celda no vale: {m}"),
                    },
                ));
            }
        };
        // ② QUIEN PIDE, si lo trae: el token del realm, verificado aquí.
        let sujeto = match p.cabeceras.get("ore-sujeto") {
            None => None,
            Some(t) => {
                let Some(realm) = self.identidad.as_ref() else {
                    return Some(Respuesta::error(
                        500,
                        "sin proveedor del realm no se puede verificar `Ore-Sujeto`",
                    ));
                };
                let t = t.trim();
                let bearer = if t.starts_with("Bearer ") || t.starts_with("bearer ") {
                    t.to_string()
                } else {
                    format!("Bearer {t}")
                };
                let cabeceras = BTreeMap::from([("authorization".to_string(), bearer)]);
                match realm(&cabeceras) {
                    Ok(s) => Some(s),
                    Err(SinIdentidad::Ausente) => None,
                    Err(SinIdentidad::Invalida(m)) => {
                        return Some(Respuesta::error(
                            401,
                            format!("el token de `Ore-Sujeto` no vale: {m}"),
                        ));
                    }
                }
            }
        };
        let cuerpo = match analizar(&p.cuerpo) {
            Ok(n) => n,
            Err(m) => return Some(Respuesta::error(400, m)),
        };
        let Ok(mut base) = self.base.lock() else {
            return Some(Respuesta::error(500, "la conexión quedó envenenada"));
        };
        // La transacción se abre a nombre de la celda; lo que se anota dice quién.
        let mut tx = match Tx::abrir(&mut base, &de_la_celda) {
            Ok(t) => t,
            Err(e) => return Some(Respuesta::error(502, e)),
        };
        let celda = match celda_de(&mut tx, &celdas.iss, &de_la_celda.persona) {
            Ok(Some(c)) => c,
            Ok(None) => {
                return Some(Respuesta::error(
                    401,
                    "esta celda no está registrada en `ore-iam`, o está retirada",
                ));
            }
            Err(e) => return Some(Respuesta::error(500, e)),
        };
        Some(match resto {
            ["evaluation"] => self.evaluation(tx, &celda, sujeto.as_ref(), &cuerpo),
            ["evaluations"] => self.evaluations(tx, &celda, sujeto.as_ref(), &cuerpo),
            ["eventos"] => self.eventos(tx, &celda, sujeto.as_ref(), &cuerpo),
            ["quien"] => self.su_handle(tx, &celda, sujeto.as_ref(), &cuerpo),
            _ => Respuesta::error(404, "no hay nada en ese camino del puente"),
        })
    }

    // ── celda ──────────────────────────────────────────────────────────────

    /// **¿De qué celda es este token?** (0058 P4·1): lo pregunta un PRODUCTO de la
    /// plataforma —`ore-postgres`— al que una celda se presentó.
    ///
    /// `Authorization` es el token de Workload Identity que la celda le dio al
    /// producto: mismo emisor que el de las celdas, pero con la audiencia DEL
    /// PRODUCTO (`--audiencias-productos`), no la nuestra. El producto lo reenvía
    /// tal cual y aquí se verifica entero; la respuesta es la celda y su
    /// organización, que es lo único que `iam.celda` sabe y el producto no.
    ///
    /// ⛔ Por qué la audiencia del producto y no la nuestra: un token con
    ///   audiencia `ore-iam` en manos de un producto le dejaría preguntar `puede`
    ///   y contar `hizo` como si fuera la celda. Con la suya sólo sirve para esto,
    ///   que no da nada: decir de quién es lo que ya tiene en la mano.
    ///
    /// `vence` es el `exp` del token: quien guarde la respuesta no la guarda más
    /// allá. `vale` es el mismo techo que `puede`: una celda retirada deja de
    /// valer, como mucho, en ese plazo.
    fn de_que_celda(&self, p: &Peticion, celdas: &ore_entrada::oidc::Emisor) -> Respuesta {
        let Some(autorizacion) = p.cabeceras.get("authorization") else {
            return Respuesta::error(401, "falta el token de la celda que se presentó");
        };
        if self.productos.is_empty() {
            return Respuesta::error(
                404,
                "sin audiencias de producto (`--audiencias-productos`), nadie pregunta por aquí",
            );
        }
        let ahora = ahora();
        let mut motivo = String::from("ningún producto conocido");
        let mut hallado = None;
        for producto in &self.productos {
            match producto.verificar_con_vence(autorizacion, ahora) {
                Ok((i, vence)) => {
                    hallado = Some((producto.aud.clone(), i, vence));
                    break;
                }
                Err(SinIdentidad::Ausente) => {
                    return Respuesta::error(401, "falta el token de la celda que se presentó");
                }
                Err(SinIdentidad::Invalida(m)) => motivo = m,
            }
        }
        let Some((audiencia, de_la_celda, vence)) = hallado else {
            return Respuesta::error(
                401,
                format!("no es el token de una celda para un producto de la plataforma: {motivo}"),
            );
        };
        let Ok(mut base) = self.base.lock() else {
            return Respuesta::error(500, "la conexión quedó envenenada");
        };
        let mut tx = match Tx::abrir(&mut base, &de_la_celda) {
            Ok(t) => t,
            Err(e) => return Respuesta::error(502, e),
        };
        let celda = match celda_de(&mut tx, &celdas.iss, &de_la_celda.persona) {
            Ok(Some(c)) => c,
            Ok(None) => {
                return Respuesta::error(
                    401,
                    "esta celda no está registrada en `ore-iam`, o está retirada",
                );
            }
            Err(e) => return Respuesta::error(500, e),
        };
        let nombre = match tx.uno("select nombre from iam.celda where id = $1", &[&celda.id]) {
            Ok(f) => f.map(|f| f.get::<_, String>(0)).unwrap_or_default(),
            Err(e) => return Respuesta::error(500, e),
        };
        cerrar(
            tx,
            false,
            Json::obj([
                (
                    "celda",
                    Json::obj([("id", Json::s(&celda.id)), ("nombre", Json::s(nombre))]),
                ),
                ("organizacion", Json::s(&celda.organizacion)),
                ("producto", Json::s(audiencia)),
                ("vence", Json::Int(vence)),
                ("vale", Json::Int(VALE)),
            ]),
        )
    }

    // ── quien ──────────────────────────────────────────────────────────────

    /// **El handle de quien crea** (0052 · Ownership; la 048): lo que una celda
    /// escribe en el `owner` de lo que alguien crea, `user:<handle>`.
    ///
    /// `subject.id` es el `sub` de la persona. Si `Ore-Sujeto` es el token de esa
    /// misma persona, el handle sale del nombre de usuario que trae (si aún no
    /// tenía); si no —desde un puesto llama el agente, y la persona es la que lo
    /// abrió—, se lee el que tiene.
    ///
    /// ⛔ Sólo de personas de la organización de la celda: un agente no es dueño
    ///   de nada, y una celda no pregunta por gente de otra. Las dos cosas son el
    ///   mismo 404.
    fn su_handle(
        &self,
        mut tx: Tx,
        celda: &Celda,
        sujeto: Option<&Identidad>,
        c: &Node,
    ) -> Respuesta {
        let Some(sub) = c
            .get("subject")
            .and_then(|(_, s)| s.get("id"))
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            return Respuesta::error(400, "falta `subject.id`: el `sub` de la persona");
        };
        if let Some(s) = sujeto.filter(|s| s.persona == sub && s.agente.is_none())
            && let Err(e) = crate::handle::asegurar(&mut tx, &self.emisor, s)
        {
            return Respuesta::error(500, e);
        }
        match crate::handle::de_la_persona(&mut tx, &self.emisor, sub, &celda.organizacion) {
            Err(e) => Respuesta::error(500, e),
            Ok(None) => Respuesta::error(
                404,
                format!(
                    "`{sub}` no es una persona de esta organización: sólo una persona es dueña de lo que crea"
                ),
            ),
            Ok(Some(h)) => cerrar(
                tx,
                false,
                Json::obj([
                    ("subject", Json::obj([("id", Json::s(sub))])),
                    ("owner", Json::s(format!("user:{h}"))),
                    ("handle", Json::s(h)),
                ]),
            ),
        }
    }

    // ── puede ──────────────────────────────────────────────────────────────

    fn evaluation(
        &self,
        mut tx: Tx,
        celda: &Celda,
        sujeto: Option<&Identidad>,
        c: &Node,
    ) -> Respuesta {
        let Some(sujeto) = sujeto else {
            return Respuesta::error(401, "`puede` necesita `Ore-Sujeto`: el token de quien pide");
        };
        let pregunta = match pregunta_de(c, None) {
            Ok(q) => q,
            Err(m) => return Respuesta::error(400, m),
        };
        match self.decidir(&mut tx, celda, sujeto, &pregunta) {
            Err(Fallo(codigo, m)) => Respuesta::error(codigo, m),
            Ok((respuesta, anotado)) => cerrar(tx, anotado, respuesta),
        }
    }

    /// El lote de AuthZEN: `subject`, `action`, `resource` y `context` de arriba
    /// valen por defecto para cada evaluación, y cada una puede cambiarlos.
    /// Semántica `execute_all`: se contestan todas, en orden.
    fn evaluations(
        &self,
        mut tx: Tx,
        celda: &Celda,
        sujeto: Option<&Identidad>,
        c: &Node,
    ) -> Respuesta {
        let Some(sujeto) = sujeto else {
            return Respuesta::error(401, "`puede` necesita `Ore-Sujeto`: el token de quien pide");
        };
        let Some((_, lista)) = c.get("evaluations") else {
            return Respuesta::error(400, "falta `evaluations`");
        };
        let mut salida = Vec::new();
        let mut anotado = false;
        for e in lista.items() {
            let pregunta = match pregunta_de(e, Some(c)) {
                Ok(q) => q,
                Err(m) => return Respuesta::error(400, m),
            };
            match self.decidir(&mut tx, celda, sujeto, &pregunta) {
                Err(Fallo(codigo, m)) => return Respuesta::error(codigo, m),
                Ok((r, a)) => {
                    salida.push(r);
                    anotado |= a;
                }
            }
        }
        cerrar(tx, anotado, Json::obj([("evaluations", Json::Arr(salida))]))
    }

    /// Decide una pregunta. Devuelve la respuesta de AuthZEN y si dejó huella.
    ///
    /// - Una DENEGACIÓN va a la huella (`acceso:negado`) en el acto, sin esperar
    ///   a que el módulo lo cuente (§ 13 de la industria).
    /// - Un PERMISO no deja huella por su cuenta —la deja el acto, con el `id`
    ///   de esta decisión dentro—; se guarda un día en `iam.decision`, para que
    ///   un reintento de `hizo` sepa de quién era.
    fn decidir(
        &self,
        tx: &mut Tx,
        celda: &Celda,
        sujeto: &Identidad,
        q: &Pregunta,
    ) -> Result<(Json, bool), Fallo> {
        if q.sujeto_id != sujeto.persona {
            return Err(Fallo(
                400,
                "`subject.id` no es el sujeto del token de `Ore-Sujeto`: quien pregunta tiene un fallo".into(),
            ));
        }
        let (decision, motivo) = self.motivo(tx, celda, sujeto, q)?;
        let id = nuevo_id("dec");
        let version = version_de(tx, &celda.organizacion)?;
        if decision && !q.consulta {
            tx.ejecutar(
                "insert into iam.decision (id, quien, agente, organizacion, celda, accion, recurso_tipo, recurso_id, decision)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, true)",
                &[
                    &id,
                    &sujeto.persona,
                    &sujeto.agente,
                    &celda.organizacion,
                    &celda.id,
                    &q.accion,
                    &q.recurso_tipo,
                    &q.recurso_id,
                ],
            )?;
            tx.ejecutar(
                &format!("delete from iam.decision where cuando < now() - interval '{RETENCION}'"),
                &[],
            )?;
        } else if !decision && !q.consulta {
            tx.anotar_por(
                &sujeto.persona,
                sujeto.agente.as_deref(),
                "acceso:negado",
                &q.accion,
                Json::obj([
                    ("decision", Json::s(&id)),
                    ("accion", Json::s(&q.accion)),
                    (
                        "recurso",
                        Json::s(format!("{}/{}", q.recurso_tipo, q.recurso_id)),
                    ),
                    ("ruta", Json::s(&q.ruta)),
                    ("motivo", Json::s(&motivo)),
                ]),
                &celda.organizacion,
                &celda.id,
            )?;
        }
        let mut ctx = vec![
            ("id", Json::s(id)),
            ("version", Json::s(version)),
            ("vale", Json::Int(VALE)),
        ];
        if !decision {
            ctx.push(("motivo", Json::s(motivo)));
        }
        Ok((
            Json::obj([
                ("decision", Json::Bool(decision)),
                ("context", Json::obj(ctx)),
            ]),
            !decision && !q.consulta,
        ))
    }

    /// La decisión y, si niega, lo que le falta. «No perteneces» y «no puedes»
    /// dicen lo mismo, como en el resto de `ore-iam`: el motivo no destapa quién
    /// está dentro.
    fn motivo(
        &self,
        tx: &mut Tx,
        celda: &Celda,
        s: &Identidad,
        q: &Pregunta,
    ) -> Result<(bool, String), Fallo> {
        let conocida = tx
            .uno("select 1 from iam.potestad where nombre = $1", &[&q.accion])?
            .is_some();
        if !conocida {
            return Ok((
                false,
                format!(
                    "potestad desconocida: `{}` no está en el catálogo",
                    q.accion
                ),
            ));
        }
        if q.recurso_tipo != "organizacion" {
            return Ok((
                false,
                format!(
                    "este motor sólo decide sobre la organización todavía (0047 A8); `{}` no",
                    q.recurso_tipo
                ),
            ));
        }
        // ⭐ A9′: la PERTENENCIA de un agente (`organizacion:leer`, lo que toda celda
        //   pregunta antes de atender). Un agente pertenece si está registrado en la
        //   organización de ESTA celda (`iam.agente`, que da de alta el aprovisionador);
        //   lo demás, un agente no lo tiene.
        if s.tipo.as_deref() == Some("agente") || s.tipo.as_deref() == Some("aprovisionador") {
            if q.accion == PERTENENCIA && s.tipo.as_deref() == Some("agente") {
                let suyo = tx
                    .uno(
                        "select 1 from iam.agente where emisor = $1 and sub = $2 and organizacion = $3",
                        &[&self.emisor, &s.persona, &celda.organizacion],
                    )?
                    .is_some();
                return Ok(if suyo {
                    (true, String::new())
                } else {
                    (false, "este agente no es de esta organización".into())
                });
            }
            return Ok((
                false,
                "un agente no tiene potestades de organización".into(),
            ));
        }
        Ok(
            match potestad::exige(tx, &self.emisor, &s.persona, &celda.organizacion, &q.accion) {
                Ok(_) => (true, String::new()),
                Err(_) => (
                    false,
                    format!("no tienes `{}` en esta organización", q.accion),
                ),
            },
        )
    }

    // ── hizo ───────────────────────────────────────────────────────────────

    /// `POST /access/v1/eventos`: lo que un módulo hizo, a la huella con su
    /// organización y su celda. Idempotente por `id`: el mismo evento dos veces
    /// es uno, y el segundo contesta 200 con el mismo `id`.
    ///
    /// Quién lo hizo sale del token de `Ore-Sujeto`, o —en un reintento, cuando
    /// ese token ya caducó— de la decisión que `ore-iam` tomó (`decision`), si
    /// está viva y es de esta organización. Sin ninguna de las dos, 400: una
    /// celda no puede atribuirle a nadie algo que `ore-iam` no autorizó.
    fn eventos(
        &self,
        mut tx: Tx,
        celda: &Celda,
        sujeto: Option<&Identidad>,
        c: &Node,
    ) -> Respuesta {
        let texto = |k: &str| c.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
        let Some(id) = texto("id").filter(|i| id_valido(i)) else {
            return Respuesta::error(
                400,
                "falta `id`, o no es un identificador (letras, cifras, `-`, `_`; hasta 80)",
            );
        };
        let Some(operacion) = texto("operacion").filter(|o| !o.is_empty()) else {
            return Respuesta::error(400, "falta `operacion`");
        };
        let Some(resultado) = texto("resultado").filter(|r| RESULTADOS.contains(&r.as_str()))
        else {
            return Respuesta::error(
                400,
                format!("`resultado` es uno de: {}", RESULTADOS.join(", ")),
            );
        };
        let sobre = texto("sobre").unwrap_or_default();
        let decision = texto("decision");

        // ¿Ya estaba? Entonces no se anota otra vez.
        match tx.uno(
            "select 1 from iam.huella where organizacion = $1 and detalle->>'evento' = $2",
            &[&celda.organizacion, &id],
        ) {
            Ok(Some(_)) => {
                return Respuesta::ok(Json::obj([("id", Json::s(id)), ("ya", Json::Bool(true))]));
            }
            Ok(None) => {}
            Err(e) => return Respuesta::error(500, e),
        }

        let (quien, agente) = match (sujeto, &decision) {
            (Some(s), _) => (s.persona.clone(), s.agente.clone()),
            (None, Some(d)) => match tx.uno(
                &format!(
                    "select quien, agente from iam.decision
                      where id = $1 and organizacion = $2 and cuando >= now() - interval '{RETENCION}'"
                ),
                &[d, &celda.organizacion],
            ) {
                Ok(Some(f)) => (f.get::<_, String>(0), f.get::<_, Option<String>>(1)),
                Ok(None) => {
                    return Respuesta::error(
                        400,
                        "sin `Ore-Sujeto`, el evento tiene que nombrar una decisión viva de esta organización",
                    );
                }
                Err(e) => return Respuesta::error(500, e),
            },
            (None, None) => {
                return Respuesta::error(400, "el evento necesita `Ore-Sujeto` o la `decision` que lo dejó pasar");
            }
        };

        let mut detalle = vec![("evento", Json::s(&id)), ("resultado", Json::s(&resultado))];
        for k in ["decision", "commit", "abre", "cuando"] {
            if let Some(v) = texto(k) {
                detalle.push((k, Json::s(v)));
            }
        }
        if let Some((_, d)) = c.get("detalle") {
            detalle.push(("detalle", Json::de_node(d)));
        }
        if let Err(e) = tx.anotar_por(
            &quien,
            agente.as_deref(),
            &operacion,
            &sobre,
            Json::obj(detalle),
            &celda.organizacion,
            &celda.id,
        ) {
            return Respuesta::error(500, e);
        }
        match tx.confirmar() {
            Ok(()) => Respuesta::creado(Json::obj([("id", Json::s(id))])),
            Err(e) => Respuesta::error(500, e),
        }
    }
}

// ── de apoyo ───────────────────────────────────────────────────────────────

/// Un fallo con su código. Lo que llega como `String` de la base es un 500.
struct Fallo(u16, String);

impl From<String> for Fallo {
    fn from(m: String) -> Fallo {
        Fallo(500, m)
    }
}

/// Confirma: con huella si la hubo (una denegación), como observación si no.
fn cerrar(tx: Tx, anotado: bool, cuerpo: Json) -> Respuesta {
    let r = if anotado {
        tx.confirmar()
    } else {
        tx.confirmar_observacion()
    };
    match r {
        Ok(()) => Respuesta::ok(cuerpo),
        Err(e) => Respuesta::error(500, e),
    }
}

/// La celda de un token: `(emisor, sub)` → `iam.celda`, no retirada.
fn celda_de(tx: &mut Tx, emisor: &str, sub: &str) -> Result<Option<Celda>, String> {
    Ok(tx
        .uno(
            "select id, organizacion from iam.celda
              where identidad_emisor = $1 and identidad_sub = $2 and estado <> 'retirada'",
            &[&emisor, &sub],
        )?
        .map(|f| Celda {
            id: f.get(0),
            organizacion: f.get(1),
        }))
}

/// El estado de la política de una organización: cambia cuando cambian sus
/// pertenencias, sus cargos o sus concesiones (altas, bajas y revocaciones mueven
/// el recuento o la fecha), o los roles del catálogo. Opaco: nadie lo lee hoy;
/// existe para poder pasar un día a copia local sin cambiar a quien pregunta (H4).
fn version_de(tx: &mut Tx, org: &str) -> Result<String, String> {
    Ok(tx
        .uno(
            "select md5(concat_ws('|',
                 (select count(*) || '/' || coalesce(max(desde)::text, '') from iam.pertenencia where organizacion = $1),
                 (select count(*) || '/' || coalesce(max(desde)::text, '') from iam.pertenencia_rol where organizacion = $1),
                 (select count(*) || '/' || coalesce(max(greatest(desde, revocada_en))::text, '') from iam.concesion where organizacion = $1),
                 (select count(*) from iam.rol_potestad)))",
            &[&org],
        )?
        .map(|f| f.get::<_, String>(0))
        .unwrap_or_default())
}

/// Una pregunta de AuthZEN, con lo de arriba como valor por defecto (el lote).
fn pregunta_de(n: &Node, arriba: Option<&Node>) -> Result<Pregunta, String> {
    let de = |k: &str, sub: &str| -> Option<String> {
        n.get(k)
            .or_else(|| arriba.and_then(|a| a.get(k)))
            .and_then(|(_, o)| o.get(sub))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    };
    Ok(Pregunta {
        sujeto_id: de("subject", "id").ok_or("falta `subject.id`")?,
        accion: de("action", "name").ok_or("falta `action.name`")?,
        recurso_tipo: de("resource", "type").ok_or("falta `resource.type`")?,
        recurso_id: de("resource", "id").ok_or("falta `resource.id`")?,
        ruta: de("context", "ruta").unwrap_or_default(),
        consulta: de("context", "consulta").as_deref() == Some("true"),
    })
}

fn id_valido(i: &str) -> bool {
    !i.is_empty()
        && i.len() <= 80
        && i.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn analizar(cuerpo: &str) -> Result<Node, String> {
    if cuerpo.trim().is_empty() {
        return Err("el cuerpo está vacío".into());
    }
    parse::parse(cuerpo).map_err(|e| format!("el cuerpo no analiza: {e:?}"))
}

fn ahora() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
