//! Las rutas, y la puerta que las monta o no.
//!
//! # Qué se sirve, y por qué justo esto
//!
//! El primer flujo de una consola: **dar de alta un origen y ver lo que trajo**.
//! Y ese flujo cae partido exactamente por donde el sustrato ya estaba partido:
//!
//! ```text
//!   dar de alta la fuente     `ore source add`   AQUÍ    no abre un socket
//!   ver qué contiene          `ore source catalog`  un JOB   habla con el origen
//!   inducir la ontología      `ore discover`     AQUÍ    lee un fichero
//!   cerrar las decisiones     `ore review`       AQUÍ    las respuestas vienen de fuera
//! ```
//!
//! ⇒ **El botón «añadir origen» lo sirve el plano de control.** El de «ver qué
//! tiene» no, y no por una regla que hayamos escrito: porque el binario que
//! corre aquí **no lleva cliente TLS**.
//!
//! # Dónde está el árbol, y qué cambia según dónde
//!
//! Un [`Arbol`], y son dos cosas distintas a propósito:
//!
//! - **un directorio**, que es lo que sirve en una máquina y lo que ejercita la
//!   prueba de fuego. Leer es leer y escribir es escribir; no hay historia y no
//!   hay nadie más;
//! - **la forja**, que es lo que corre en el clúster. Cada petición **clona**, y
//!   la que escribe **empuja**. El servidor no se queda el árbol.
//!
//! Y el segundo modo trae gratis lo que el primero no puede tener: dos personas
//! contestando la misma cola clonan el mismo commit, y **la segunda en empujar
//! es rechazada**. Eso llega al cliente como un `409` en vez de como una
//! ontología con las dos respuestas mezcladas.
//!
//! # Las dos cosas que este módulo se niega a hacer
//!
//! - **No acepta una URL con credencial dentro SI NO HAY DÓNDE GUARDARLA.**
//!   Esto se negaba siempre, y la negativa venía con su fecha de caducidad
//!   escrita: *«el día que haya un sitio de verdad donde ponerla, esta negativa
//!   es lo que hay que quitar»*. Ese sitio es el custodio, y existe desde el
//!   2026-09-10.
//!
//!   ⇒ Así que ya no se niega: se pregunta **si hay custodio**, que es lo único
//!   que la negativa quería saber. Con él, la credencial va allí y una fuente
//!   Postgres se da de alta como cualquier otra. Sin él, sigue el `422` — un
//!   `ore-serve` sin `--cofre` aceptando la contraseña de producción de alguien
//!   y perdiéndola en `.env.local` es exactamente el agujero que esto evitaba.
//!
//!   ⚠️ Y si el custodio la rechaza, el alta contesta `502` y no `201`: la
//!   fuente queda declarada —el árbol se escribió antes— pero un tick verde
//!   encima de un secreto perdido es peor que un error.
//! - **No deja que un nombre de la URL toque el sistema de ficheros.** Un
//!   segmento se valida contra un alfabeto cerrado antes de convertirse en un
//!   camino; `..` no es un caso especial que haya que recordar, es algo que el
//!   alfabeto ya no admite.

use crate::arbol;
use crate::cola;
use crate::documentos;
use crate::git;
use crate::mando;
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::Loaded;
#[cfg(test)]
use ore_core::link::Package;
use ore_core::parse::{self, Node, Style};
use ore_entrada::http::{self, Peticion, Respuesta, Salida};
use ore_entrada::identidad::{Identidad, Proveedor, SinIdentidad};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Dónde vive el árbol.
pub enum Arbol {
    Directorio(PathBuf),
    Forja(git::Forja),
}

pub struct Servidor {
    /// El binario `ore`. Es una ruta y no un nombre para que un despliegue no
    /// dependa del `PATH` del contenedor.
    pub binario: PathBuf,
    pub arbol: Arbol,
    /// El puerto de identidad. `None` ⇒ **las rutas de datos no se montan**.
    pub identidad: Option<Proveedor>,
    /// ⭐⭐ DÓNDE VIVE LA CREDENCIAL DE UNA FUENTE, y por fin en algún sitio.
    ///
    /// `host:puerto` del custodio. `None` ⇒ el alta de una fuente SIN credencial
    /// en la URL sigue funcionando —y la credencial que no hay no se pierde—,
    /// pero una que sí la traiga se rechaza con `422`: `ore source add` la
    /// escribe en `.env.local`, `.env.local` está en el `.gitignore`, y el clon
    /// se tira al terminar la petición.
    ///
    /// ⇒ Medido: el Job de catálogo del primer inquilino murió con «`PRUEBA_BQ_URL`
    ///   no está definida», y el cofre llevaba días desplegado con CERO secretos
    ///   dentro. Tenía cliente desde el principio y nadie se lo había dado.
    /// ⭐⭐ LA COLA DE TRABAJO de este inquilino. `None` ⇒ el alta funciona y el
    /// Job de catálogo lo rinde la convergencia, con la latencia del cron.
    ///
    /// ⛔ Y es un repositorio DISTINTO del árbol y del compartimento. Del árbol
    /// porque ahí van las declaraciones del cliente, no manifiestos de
    /// Kubernetes; del compartimento porque ése contiene el `Deployment` de este
    /// mismo proceso, y escribir ahí sería el gobernado escribiendo su gobierno.
    pub cola: Option<git::Forja>,
    pub cofre: Option<String>,
    /// De quién es este árbol. El custodio guarda POR ORGANIZACIÓN, y este
    /// proceso sirve UNA — su namespace es el del inquilino.
    pub organizacion: Option<String>,
    /// La cuenta de Google de los drivers (`--cuenta-driver`): la que el
    /// cliente autoriza en su origen. Se enseña, no se usa aquí.
    pub cuenta_driver: Option<String>,
    /// El gateway de modelos (0027 E1): dónde se suscribe y qué puerta se contesta.
    pub modelos: Option<crate::modelos::Modelos>,
    /// La lista de certificación de un fichero (el banco); si no, de la cola.
    pub perfiles: Option<PathBuf>,
    /// La API de la forja del árbol (0030 W2): ramas y propuestas. `None` ⇒
    /// las rutas de ramas y propuestas contestan 422 y el árbol es sólo `main`.
    pub forja_api: Option<crate::forja::Api>,
    /// Los puestos vivos (0031 W3.1): todo el estado de las sesiones, en memoria.
    ///
    /// ⭐ En un `Arc` desde 0037 ②: un flujo abierto (`GET …/flujo`) sigue
    ///   escribiendo después de que la petición que lo abrió haya vuelto, así
    ///   que lo que mira tiene que poder vivir por su cuenta.
    pub puestos: std::sync::Arc<crate::puestos::Puestos>,
    /// El índice de assets, por cabeza (0034 ⑤): `GET /assets` de memoria.
    pub assets_cache: crate::assets::Cache,
    /// En qué se diferencia una rama de la de por defecto (ramas globales,
    /// fase 2), por las dos cabezas: `GET /ramas/{r}/cambios` de memoria.
    pub cambios_cache: crate::assets::Cache,
    /// ⭐ El puente a `ore-iam` (0047 A5). Sin él, P2 no existe y todo sigue como
    ///   era: quien tiene sesión da de alta fuentes, protege `main` y fusiona según
    ///   la política.
    pub acceso: Option<std::sync::Arc<ore_acceso::Acceso>>,
    /// ⭐ Y su buzón (0047 A6.4): lo que se escribe va a la actividad sin que la
    ///   respuesta espere a `ore-iam`. Con el puente, siempre; sin él, no hay.
    pub buzon: Option<ore_acceso::Buzon>,
    /// ⭐ La última pertenencia confirmada de cada sujeto, para la gracia (A9′).
    pub pertenencias: crate::acceso::Pertenencias,
}

/// **Desde un puesto sólo entran los verbos** (0031 W3.7 gobierno ①).
///
/// Medido antes: desde una celda, `PUT /arbol/conduits.yaml` (`low` → `high`)
/// era 200 firmado por el agente, y el `owner` de un paquete, otro 200: el
/// gobierno mismo se reescribía desde un puesto. El testigo que una celda
/// tiene es el del agente, y lo que un agente escribe en este servidor es lo
/// que los verbos dicen —leer (`/puestos/{id}/datos`), escribir (`/v1`, el
/// catálogo; `confirmar`), declarar (`/documentos`), crear una base
/// (`POST /paquetes`, 0039)— más lo suyo del puesto
/// (`pendiente`, `salida`). Todo lo demás que no sea leer, un agente no lo
/// hace: 403. Se decide AQUÍ y no ruta a ruta, en una lista de permitidos
/// como la de `mando.rs`: el verbo que se añada mañana llega negado (P4).
/// Leer sigue abierto: el árbol se lee por el agente (las medidas, el índice)
/// y quitar la cabecera `x-ore-puesto` no cambia nada, porque lo que decide
/// es el sujeto.
fn puerta_del_agente(p: &Peticion, sujeto: &Identidad, seg: &[&str]) -> Option<Respuesta> {
    if p.metodo == "GET" || !crate::puestos::es_agente(sujeto) {
        return None;
    }
    let entra = matches!(
        seg,
        ["puestos", ..]
            | ["v1", ..]
            | ["documentos", ..]
            | ["conceptos", ..]
            // 0039: `create … database` en un guion
            | ["paquetes"]
            | ["datasets", _, _, "confirmar"]
            | ["datasets", _, _, _, "confirmar"]
    );
    if entra {
        return None;
    }
    Some(Respuesta::error(
        403,
        format!(
            "desde un puesto sólo entran los verbos —leer, escribir, declarar— por `/puestos/{{id}}/datos`, `/v1`, `/documentos` y `/conceptos`; `{} /{}` no",
            p.metodo,
            seg.join("/")
        ),
    ))
}

impl Servidor {
    /// Como [`Servidor::atender`], pero dejando que una ruta conteste con un
    /// flujo abierto en vez de con una respuesta que termina (0037 ②).
    ///
    /// La identidad se resuelve igual y la puerta del agente se cruza igual:
    /// lo único distinto es la forma de contestar.
    pub fn atender_flujo(&self, p: &Peticion) -> Salida {
        let seg = p.segmentos();
        let es_flujo = matches!(
            (p.metodo.as_str(), seg.as_slice()),
            ("GET", ["puestos", _, "flujo"])
                | ("GET", ["puestos", _, "lsp", "agente"])
                | ("GET", ["puestos", _, "lsp", "consola"])
        );
        if es_flujo {
            let sujeto = match self.quien(p) {
                Ok(s) => s,
                Err(r) => return Salida::Una(r),
            };
            if let Some(r) = puerta_del_agente(p, &sujeto, &seg) {
                return Salida::Una(r);
            }
            // ⛔ Por cabecera y no por la URL: esta entrada tira la cadena de
            //   consulta a propósito (ningún dato entra por la URL), y
            //   `last-event-id` es justo lo que un `EventSource` manda solo.
            let desde = p
                .cabeceras
                .get("last-event-id")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(0);
            return match seg.as_slice() {
                ["puestos", id, "flujo"] => self.flujo_del_puesto(&sujeto, id, desde),
                // El agente recoge lo que el editor manda; no hay nada que
                // retomar, porque lo que recoge SE CONSUME.
                ["puestos", id, "lsp", "agente"] => self.flujo_lsp_del_agente(&sujeto, id),
                ["puestos", id, "lsp", "consola"] => {
                    self.flujo_lsp_de_la_consola(&sujeto, id, desde)
                }
                _ => Salida::Una(Respuesta::error(404, "esa ruta no existe")),
            };
        }
        Salida::Una(self.atender(p))
    }

    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        match (p.metodo.as_str(), seg.as_slice()) {
            // ── Sin identidad: sólo lo que no dice nada del árbol ────────────
            ("GET", ["salud"]) => Respuesta::ok(Json::obj([("ok", Json::Bool(true))])),
            ("GET", ["version"]) => self.version(),

            // ── Con identidad ────────────────────────────────────────────────
            _ => match self.quien(p) {
                Err(r) => r,
                Ok(sujeto) => self.con_sujeto(p, &sujeto, &seg),
            },
        }
    }

    /// Quién pregunta, o la respuesta que hay que darle.
    ///
    /// Sin proveedor configurado esto contesta **404 y no 401**: la ruta no está
    /// porque no se montó, y decir «no autorizado» insinuaría que existe y que
    /// con la credencial correcta contestaría.
    ///
    /// ⛔ Y con el puente (0047 A9′), **quien llega tiene que ser de la organización
    ///   de esta celda**: un token del realm vale en todas, y la pertenencia la sabe
    ///   `ore-iam`. Es la única puerta de las rutas con sujeto, flujos incluidos.
    fn quien(&self, p: &Peticion) -> Result<Identidad, Respuesta> {
        let Some(proveedor) = self.identidad.as_ref() else {
            return Err(Respuesta::error(
                404,
                "sin proveedor de identidad configurado, las rutas de datos no se montan",
            ));
        };
        let sujeto = proveedor(&p.cabeceras).map_err(|e| match e {
            SinIdentidad::Ausente => Respuesta::error(401, "esta ruta necesita un sujeto"),
            SinIdentidad::Invalida(m) => Respuesta::error(401, m),
        })?;
        self.pertenece(&sujeto)?;
        Ok(sujeto)
    }

    fn con_sujeto(&self, p: &Peticion, sujeto: &Identidad, seg: &[&str]) -> Respuesta {
        if let Some(r) = puerta_del_agente(p, sujeto, seg) {
            return r;
        }
        // La rama en la que el editor lee o escribe el árbol (0030 W2); sin
        // cabecera, `main`. Sólo las rutas del árbol la miran.
        let rama = p
            .cabeceras
            .get(crate::propuestas::CABECERA_RAMA)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        match (p.metodo.as_str(), seg) {
            // Ramas globales, fase 1: lo que el catálogo lee, EN la rama de
            // `x-ore-rama`; sin ella, la de por defecto, como siempre.
            ("GET", ["fuentes"]) => self.leyendo_en(rama, fuentes),
            // Qué credencial usa una familia y qué roles concede el cliente:
            // lo que el paso de conexión del wizard enseña (`credenciales`).
            // «Comprobar acceso» antes del alta: un Job con la cuenta del
            // driver ejecuta `check`; su resultado vuelve por el informador.
            // Volver a leer el origen: tras conceder un rol que faltaba, el
            // catálogo que falló no se reintenta solo (su Job ya existe).
            ("POST", ["fuentes", n, "catalogar"]) => {
                let n = n.to_string();
                Respuesta::ok(Json::obj([(
                    "encolado",
                    Json::s(self.encolar_catalogo_corrida(&n, sujeto, true)),
                )]))
            }
            ("POST", ["fuentes", "comprobaciones"]) => {
                let cuerpo = p.cuerpo.clone();
                self.comprobar_fuente(&cuerpo, sujeto)
            }
            ("GET", ["fuentes", "credenciales", tipo]) => {
                Respuesta::ok(crate::credenciales::de(tipo, self.cuenta_driver.as_deref()))
            }
            ("POST", ["fuentes"]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Dar de alta una fuente") {
                    return r;
                }
                // ⭐ P2 (0047 A5): una conexión se gobierna con potestad, no con
                //   revisión (0044 B.7). Sin puente, lo de siempre.
                let decision = match self.exigir(sujeto, "fuente:crear", "POST /fuentes") {
                    Ok(d) => d,
                    Err(r) => return r,
                };
                let cuerpo = p.cuerpo.clone();
                // ⛔ EL TESTIGO DE QUIEN PIDIO, y no uno nuestro. El custodio
                //   decide con `concesion_viva` si esa persona puede emitir, y
                //   con una credencial de servicio esa pregunta no se haria:
                //   emitiria siempre el servidor. La `018` puso `secreto:emitir`
                //   en una PERSONA a proposito.
                let testigo = p
                    .cabeceras
                    .get("authorization")
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(str::to_string);
                let r = self.escribiendo(sujeto, "alta de una fuente", |r| {
                    self.alta_de_fuente(r, &cuerpo, testigo.as_deref(), sujeto)
                });
                if r.codigo == 201 {
                    let nombre = match &r.cuerpo {
                        Json::Obj(m) => match m.get("name") {
                            Some(Json::Str(n)) => n.clone(),
                            _ => String::new(),
                        },
                        _ => String::new(),
                    };
                    self.contar(crate::acceso::evento(
                        "fuente:crear",
                        &format!("fuente/{nombre}"),
                        "hecho",
                        decision,
                        crate::acceso::commit_de(&r),
                    ));
                }
                r
            }
            // ⭐⭐ EN QUE ESTADO ESTA UNA FUENTE, que son TRES y no dos.
            //
            // La ficha del catalogo decia «todavia no hay catalogo: el Job aun
            // no ha corrido» tanto cuando nadie lo habia encolado como cuando
            // se estaba leyendo el origen en ese momento. Son dos situaciones
            // con arreglos distintos —una espera, la otra no va a pasar sola— y
            // pintarlas igual manda a mirar el sitio equivocado.
            ("GET", ["fuentes", n, "estado"]) => {
                let n = n.to_string();
                self.estado(&n)
            }
            // ⭐ Retirar una fuente (0027, «el catálogo de la conexión»): la
            //   conexión fuera del manifiesto, su catálogo fuera del árbol, su
            //   Job de catálogo fuera de la cola. 409 si alguna database sale
            //   de ella. La credencial del custodio se dice y no se toca: el
            //   cofre no tiene baja todavía, y es un acto con su propia huella.
            ("DELETE", ["fuentes", n]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Retirar una fuente") {
                    return r;
                }
                let n = n.to_string();
                // Con el testigo de quien pide, como el alta: el custodio decide
                // si esa persona puede retirar (`owner`, o `secreto:retirar`).
                let testigo = p
                    .cabeceras
                    .get("authorization")
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(str::to_string);
                self.escribiendo(sujeto, &format!("retirar la fuente `{n}`"), |r| {
                    self.retirar_fuente(r, &n, testigo.as_deref(), sujeto)
                })
            }
            ("GET", ["paquetes"]) => self.leyendo_en(rama, paquetes),
            // ── 0027 E1 · los verbos del modelo (`modelos.rs`) ────────────
            // (E3: la lista de certificación tal como Bastion la publica; no
            // toca el árbol, pero sí dice qué se puede pedir: con identidad)
            ("GET", ["perfiles"]) => self.perfiles_publicados(),
            ("GET", ["modelos"]) => self.leyendo_en(rama, |r| self.modelos(r)),
            ("POST", ["modelos"]) => {
                let cuerpo = p.cuerpo.clone();
                self.escribiendo(sujeto, "alta de un modelo", |r| {
                    self.alta_de_modelo(r, &cuerpo)
                })
            }
            ("GET", ["modelos", n]) => {
                let n = n.to_string();
                self.leyendo_en(rama, move |r| self.modelo(r, &n))
            }
            ("DELETE", ["modelos", n]) => {
                let n = n.to_string();
                self.escribiendo(sujeto, &format!("retirar el modelo `{n}`"), |r| {
                    self.retirar_modelo(r, &n, sujeto)
                })
            }
            // ⭐⭐ CREAR UNA BASE ELIGIENDO QUE ENTRA. Es lo que el modal de la
            //   consola lleva meses pidiendo con casillas: schemas y tablas de
            //   un origen ya descubierto, marcadas. Hasta hoy la seleccion
            //   moria en el estado del navegador.
            //
            // ⭐ Y va POR AQUI y no por la cola de trabajo, aunque el catalogo
            //   de una fuente vaya por un Job. La diferencia es que esto es
            //   HERMETICO: `discover --from` induce desde el catalogo que el Job
            //   ya dejo en el arbol, sin credencial, sin red y sin driver. Es
            //   exactamente lo mismo que `review` hace tres rutas mas abajo, y
            //   por el mismo camino: un clon, `ore`, un commit.
            // ⭐ 0039: y desde un puesto (`create … database` en un guion),
            //   con `x-ore-puesto`: quien la crea es la persona y va a su rama,
            //   como lo que una celda declara o escribe.
            ("POST", ["paquetes"]) => {
                let (sujeto, rama) = match self.sujeto_del_puesto(p, sujeto, rama) {
                    Ok(x) => x,
                    Err(r) => return r,
                };
                let cuerpo = p.cuerpo.clone();
                self.escribiendo_en(rama.as_deref(), &sujeto, "alta de una base", |r| {
                    self.alta_de_paquete(r, &cuerpo, &sujeto)
                })
            }
            // ⭐⭐ EL ESQUEMA DESCUBIERTO, que hasta hoy no salia por ningun
            //   sitio. `/paquetes` daba nombre, version y cuantas decisiones
            //   quedan abiertas — util para una lista, inutil para una ficha.
            //   La consola pintaba un esquema de mentira porque no habia de
            //   donde sacar el de verdad.
            ("GET", ["paquetes", n, "esquema"]) => {
                let n = n.to_string();
                self.leyendo_en(rama, move |r| esquema(r, &n))
            }
            ("GET", ["paquetes", n, "decisiones"]) => {
                let n = n.to_string();
                self.leyendo_en(rama, move |r| decisiones(r, &n))
            }
            // ── 0027 P1 I2 · la decisión de la copia (`copia.rs`) ────────
            ("GET", ["paquetes", n, "copias"]) => {
                let n = n.to_string();
                self.leyendo_en(rama, move |r| self.copias(r, &n))
            }
            // ── retirar una base: el paquete fuera del arbol, y la cola al dia ──
            ("DELETE", ["paquetes", n]) => {
                let n = n.to_string();
                self.escribiendo_en(rama, sujeto, &format!("retirar la base `{n}`"), |r| {
                    self.retirar_paquete(r, &n, sujeto)
                })
            }
            // ── 0038 P6 · los schemas de una base (`schemas.rs`) ─────────
            //   Crear y renombrar: lo que la consola hacía en su estado local.
            ("POST", ["paquetes", n, "schemas"]) => {
                let n = n.to_string();
                let cuerpo = p.cuerpo.clone();
                let que = analizar(&cuerpo)
                    .ok()
                    .and_then(|c| {
                        c.get("name")
                            .and_then(|(_, v)| v.as_str().map(String::from))
                    })
                    .unwrap_or_default();
                self.escribiendo_en(
                    rama,
                    sujeto,
                    &format!("`{n}`: crear el schema `{que}`"),
                    |r| self.crear_schema(r, &n, &cuerpo),
                )
            }
            ("POST", ["paquetes", n, "schemas", s, "renombrar"]) => {
                let (n, s) = (n.to_string(), s.to_string());
                let cuerpo = p.cuerpo.clone();
                let a = analizar(&cuerpo)
                    .ok()
                    .and_then(|c| c.get("to").and_then(|(_, v)| v.as_str().map(String::from)))
                    .unwrap_or_default();
                self.escribiendo_en(
                    rama,
                    sujeto,
                    &format!("`{n}`: el schema `{s}` pasa a llamarse `{a}`"),
                    |r| self.renombrar_schema(r, &n, &s, &cuerpo),
                )
            }
            // ── 0027 P1 C2 · modelar una tabla de una base (`ore model`) ──
            ("POST", ["paquetes", n, "tablas", o, "modelar"]) => {
                let (n, o) = (n.to_string(), o.to_string());
                self.escribiendo_en(rama, sujeto, &format!("`{n}`: modelar `{o}`"), |r| {
                    self.modelar(r, &n, &o, sujeto)
                })
            }
            // ── copiar UNA tabla de una base foránea (`ore copy`) ─────────
            ("POST", ["paquetes", n, "tablas", o, "copiar"]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Copiar una tabla") {
                    return r;
                }
                let (n, o) = (n.to_string(), o.to_string());
                self.escribiendo(sujeto, &format!("`{n}`: copiar `{o}` a la celda"), |r| {
                    self.copiar_tabla(r, &n, &o, sujeto)
                })
            }
            // ── 0027 P1 I4b · ascender una base foránea a estándar ────────
            // 0030 W1 · rehacer la copia: escribe la cola, no el árbol.
            ("POST", ["paquetes", n, "copia", "rehacer"]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Rehacer la copia") {
                    return r;
                }
                let n = n.to_string();
                self.leyendo(move |r| self.rehacer_copia(r, &n, sujeto))
            }
            ("POST", ["paquetes", n, "copia"]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Ascender a base estándar") {
                    return r;
                }
                let n = n.to_string();
                self.escribiendo(sujeto, &format!("`{n}` pasa a base estándar"), |r| {
                    self.ascender(r, &n, sujeto)
                })
            }
            ("POST", ["paquetes", n, "decisiones"]) => {
                if let Some(r) = self.solo_en_la_de_por_defecto(rama, "Contestar las decisiones") {
                    return r;
                }
                let n = n.to_string();
                let cuerpo = p.cuerpo.clone();
                self.escribiendo(sujeto, &format!("decisiones de `{n}`"), |r| {
                    self.responder(r, &n, &cuerpo, sujeto)
                })
            }
            // ── 0030 W0 · el árbol por ruta (`arbol.rs`): lo que el editor abre ──
            //   Y desde W2, EN LA RAMA que diga `X-Ore-Rama` (`propuestas.rs`);
            //   sin cabecera, `main`, como siempre.
            // ⭐ 0036 ④: `X-Ore-Raiz` acota el índice a la carpeta de un
            //   repositorio. Sin ella, la celda entera, como siempre.
            ("GET", ["arbol"]) => {
                let raiz = match crate::entorno::alcance_valido(
                    p.cabeceras.get("x-ore-raiz").map(String::as_str),
                ) {
                    Ok(a) => a,
                    Err(r) => return r,
                };
                self.leyendo_en(rama, move |d| {
                    if let Some(a) = &raiz
                        && !d.join(a).is_dir()
                    {
                        return Respuesta::error(404, format!("no hay `{a}` en el árbol"));
                    }
                    arbol::indice_en(d, raiz.as_deref())
                })
            }
            ("GET", ["arbol", "diagnosticos"]) => {
                self.leyendo_en(rama, |r| self.diagnosticos_del_arbol(r))
            }
            // ⭐ Las versiones de un fichero (0030 W2, *Version history*): git las tiene.
            ("GET", ["arbol", "historia", ruta @ ..]) => {
                let ruta = ruta.join("/");
                self.leyendo_en(rama, move |r| self.historia_del_fichero(r, &ruta))
            }
            ("GET", ["arbol", "version", hash, ruta @ ..]) => {
                let (hash, ruta) = (hash.to_string(), ruta.join("/"));
                self.leyendo_en(rama, move |r| self.version_del_fichero(r, &hash, &ruta))
            }
            ("GET", ["arbol", ruta @ ..]) => {
                let ruta = ruta.join("/");
                self.leyendo_en(rama, move |r| arbol::leer(r, &ruta))
            }
            // ⭐ Varios ficheros en UN commit con mensaje (0030 W2), o en seco
            //   lo que ese commit sería: lo que el panel de Commit enseña.
            ("POST", ["arbol", "commit"]) => {
                let cuerpo = p.cuerpo.clone();
                let si_commit = p.cabeceras.get("if-match").cloned();
                let (seco, mensaje) = arbol::intencion_del_commit(&cuerpo);
                if seco {
                    self.leyendo_en(rama, |r| {
                        self.commit_del_arbol(r, &cuerpo, si_commit.as_deref())
                    })
                } else {
                    self.escribiendo_en(rama, sujeto, &mensaje, |r| {
                        self.commit_del_arbol(r, &cuerpo, si_commit.as_deref())
                    })
                }
            }
            ("PUT", ["arbol", ruta @ ..]) => {
                let ruta = ruta.join("/");
                let cuerpo = p.cuerpo.clone();
                let si_commit = p.cabeceras.get("if-match").cloned();
                self.escribiendo_en(rama, sujeto, &format!("escribir `{ruta}`"), |r| {
                    self.escribir_fichero(r, &ruta, &cuerpo, si_commit.as_deref())
                })
            }
            ("DELETE", ["arbol", ruta @ ..]) => {
                let ruta = ruta.join("/");
                let si_commit = p.cabeceras.get("if-match").cloned();
                self.escribiendo_en(rama, sujeto, &format!("retirar `{ruta}`"), |r| {
                    self.retirar_fichero(r, &ruta, si_commit.as_deref())
                })
            }
            // ── 0030 W2 · ramas y propuestas (`propuestas.rs`) ──
            ("GET", ["ramas"]) => self.ramas(),
            // Ramas globales, fase 2: en qué se diferencia la rama de la de por
            // defecto, por activo y con su significado (`cambios.rs`).
            ("GET", ["ramas", resto @ .., "cambios"]) if !resto.is_empty() => {
                self.cambios(&resto.join("/"))
            }
            ("POST", ["ramas"]) => self.crear_rama(sujeto, &p.cuerpo),
            // P1.4 · la política de `main`: protegerla, o proponer dejarla libre.
            ("PUT", ["ramas", resto @ .., "proteccion"]) if !resto.is_empty() => {
                // ⭐ P2 (0047 A5): cambiar la política es de quien tenga
                //   `rama:proteger` (0044 B.7), en las dos direcciones.
                let rama = resto.join("/");
                let decision =
                    match self.exigir(sujeto, "rama:proteger", "PUT /ramas/{r}/proteccion") {
                        Ok(d) => d,
                        Err(r) => return r,
                    };
                let r = self.proteger(sujeto, &rama, &p.cuerpo);
                let cambiada = !matches!(&r.cuerpo, Json::Obj(m) if matches!(m.get("cambiada"), Some(Json::Bool(false))));
                if r.codigo < 300 && cambiada {
                    self.contar(crate::acceso::evento(
                        "rama:proteger",
                        &format!("rama/{rama}"),
                        "hecho",
                        decision,
                        crate::acceso::commit_de(&r),
                    ));
                }
                r
            }
            ("DELETE", ["ramas", nombre @ ..]) => self.retirar_rama(&nombre.join("/")),
            // ⭐ Traer OTRA rama a ésta (el «Merge» del menú): git merge en un clon de
            //   la rama, el gate de siempre, y el empujón. `main` no: eso es una propuesta.
            ("POST", ["ramas", resto @ .., "fusionar"]) if !resto.is_empty() => {
                self.fusionar_en_rama(sujeto, &resto.join("/"), &p.cuerpo)
            }
            // ── 0031 W3.1 · el puesto: la sesión viva (`puestos.rs`) ──────
            // La persona abre, manda celdas y espera salidas; el agente del
            // pod pide trabajo, entrega salidas y resuelve datos. Sin árbol
            // salvo `datos`, que lee el informe de la copia en la rama.
            // ── 0031 W3.2 · el entorno: lo que el árbol declara, y su capa ──
            // ⭐ 0037 ③c: el entorno tiene lenguaje. Sin él, `python`, que es
            //   lo que ha significado `/entorno` desde W3.2.
            ("GET", ["entorno"]) => self.entorno(
                rama,
                p.cabeceras.get("x-ore-raiz").map(String::as_str),
                crate::entorno::PYTHON,
            ),
            ("POST", ["entorno"]) => self.resolver_entorno(
                sujeto,
                rama,
                p.cabeceras.get("x-ore-raiz").map(String::as_str),
                crate::entorno::PYTHON,
            ),
            ("GET", ["entorno", e]) => match crate::entorno::entorno_valido(e) {
                Ok(e) => self.entorno(rama, p.cabeceras.get("x-ore-raiz").map(String::as_str), e),
                Err(r) => r,
            },
            ("POST", ["entorno", e]) => match crate::entorno::entorno_valido(e) {
                Ok(e) => self.resolver_entorno(
                    sujeto,
                    rama,
                    p.cabeceras.get("x-ore-raiz").map(String::as_str),
                    e,
                ),
                Err(r) => r,
            },
            ("GET", ["puestos"]) => self.puestos_de(sujeto),
            ("POST", ["puestos"]) => self.abrir_puesto(sujeto, &p.cuerpo),
            // ── el trabajo (0031 §9, W3.7 ④): un fichero del árbol como Job ──
            ("GET", ["trabajos"]) => self.trabajos_de(sujeto),
            ("POST", ["trabajos"]) => self.abrir_trabajo(sujeto, &p.cuerpo),
            ("GET", ["trabajos", id]) => self.trabajo(sujeto, id),
            ("GET", ["puestos", id]) => self.puesto(sujeto, id),
            ("DELETE", ["puestos", id]) => self.cerrar_puesto(sujeto, id),
            ("POST", ["puestos", id, "ejecutar"]) => self.ejecutar_en_puesto(sujeto, id, &p.cuerpo),
            ("GET", ["puestos", id, "celdas", n]) => match n.parse::<u64>() {
                Ok(n) => self.celda_del_puesto(sujeto, id, n),
                Err(_) => Respuesta::error(422, "la celda es un número"),
            },
            ("GET", ["puestos", id, "pendiente"]) => self.pendiente_del_puesto(sujeto, id),
            // ⭐ El servidor de lenguaje (0037 ③a): el editor manda por aquí y
            //   el agente entrega por aquí; los dos flujos van en `atender_flujo`.
            ("POST", ["puestos", id, "lsp"]) => self.lsp_de_la_consola(sujeto, id, &p.cuerpo),
            ("POST", ["puestos", id, "lsp", "salida"]) => {
                self.lsp_del_servidor(sujeto, id, &p.cuerpo)
            }
            ("POST", ["puestos", id, "celdas", n, "salida"]) => match n.parse::<u64>() {
                Ok(n) => self.salida_del_puesto(sujeto, id, n, &p.cuerpo),
                Err(_) => Respuesta::error(422, "la celda es un número"),
            },
            ("GET", ["puestos", id, "datos", vista]) => self.datos_del_puesto(sujeto, id, vista),
            // `sql()` sin regex: el texto entero, y cada nombre del árbol resuelto.
            ("POST", ["puestos", id, "sql"]) => self.sql_del_puesto(sujeto, id, &p.cuerpo),
            // Lo que el transform declara, dicho al servidor (W3.7 gobierno ⑤).
            ("POST", ["puestos", id, "transform"]) => {
                self.declarar_transform(sujeto, id, &p.cuerpo)
            }
            ("DELETE", ["puestos", id, "transform"]) => self.retirar_transform(sujeto, id),
            // ── 0031 §11 · el catálogo REST de Iceberg (`catalogo.rs`) ──────
            (_, ["v1", resto @ ..]) => self.catalogo(p, sujeto, rama, resto),
            // ── 0031 §10 · los datasets (`datasets.rs`): la lista, la ficha y el swap ──
            // ── 0034 ⑤ · el índice de assets: el árbol compilado, de memoria por cabeza ──
            ("GET", ["assets"]) => self.assets(rama, None),
            ("GET", ["assets", commit]) => self.assets(rama, Some(commit)),
            // ── 0035 ② · los proyectos: escribirlos ─────────────────────────
            // Leerlos NO tiene ruta: `GET /assets` ya los trae (0035 ①). Aquí
            // sólo lo que el árbol desnudo no sabe: la forma del manifiesto, el
            // nombre cogido (409) y borrar la LENTE sin borrar lo que nombraba.
            // Un proyecto lo crea una persona: `/proyectos` no está en la
            // puerta del agente, así que desde un puesto es 403.
            ("POST", ["proyectos"]) => {
                let cuerpo = p.cuerpo.clone();
                self.escribiendo_en(rama, sujeto, "crear un proyecto", |r| {
                    self.crear_proyecto(r, sujeto, &cuerpo)
                })
            }
            ("PUT", ["proyectos", id]) => {
                let (id, cuerpo) = (id.to_string(), p.cuerpo.clone());
                self.escribiendo_en(rama, sujeto, &format!("escribir el proyecto `{id}`"), |r| {
                    self.escribir_proyecto(r, sujeto, &id, &cuerpo)
                })
            }
            // ── 0035 ⑥ · 0036 ② · los repositorios: la unidad de trabajo ────
            // Leerlos va en `/assets` (①) y borrarlos, en `DELETE /arbol/<carpeta>`
            // (0035 ③b): aquí sólo nacer entero —manifiesto + semilla en UN
            // commit— y reescribir el manifiesto.
            ("POST", ["repositorios"]) => {
                let cuerpo = p.cuerpo.clone();
                self.escribiendo_en(rama, sujeto, "crear un repositorio", |r| {
                    self.crear_repositorio(r, &cuerpo)
                })
            }
            // ⭐ Actualizar la plantilla (⑧b): una RAMA y una PROPUESTA con su
            //   diff, nunca un pisotón — esos ficheros los ha editado alguien.
            ("POST", ["repositorios", resto @ .., "actualizar"]) if !resto.is_empty() => {
                let ruta = resto.join("/");
                self.actualizar_plantilla(sujeto, &ruta)
            }
            ("PUT", ["repositorios", resto @ ..]) if !resto.is_empty() => {
                let (ruta, cuerpo) = (resto.join("/"), p.cuerpo.clone());
                self.escribiendo_en(
                    rama,
                    sujeto,
                    &format!("escribir el repositorio `{ruta}`"),
                    |r| self.escribir_repositorio(r, &ruta, &cuerpo),
                )
            }
            ("DELETE", ["proyectos", id]) => {
                let id = id.to_string();
                self.escribiendo_en(rama, sujeto, &format!("retirar el proyecto `{id}`"), |r| {
                    self.retirar_proyecto(r, &id)
                })
            }
            // 0046 E8·1d: la colección, como el dataset —lista, ficha con su
            // historia de transacciones, e ítems por estado y paginados—.
            ("GET", ["colecciones"]) => self.colecciones(rama),
            ("GET", ["colecciones", b, n]) => {
                self.ficha_de_la_coleccion(rama, b, ore_core::normalize::SCHEMA_POR_DEFECTO, n)
            }
            ("GET", ["colecciones", b, s, n]) => self.ficha_de_la_coleccion(rama, b, s, n),
            ("GET", ["colecciones", b, s, n, "items"]) => {
                self.items_de_la_coleccion(rama, b, s, n, &p.consulta)
            }
            // 0046 E9·2: servir un ítem por su huella —el valor de un `Media<c>`—
            // con una URL firmada, o un lote de ellas (una lista, una galería).
            ("GET", ["colecciones", b, s, n, "items", h]) => {
                match crate::datasets::sin_porcentajes(h) {
                    Some(h) => self.servir_items(rama, p, b, s, n, &[h], None, true),
                    None => Respuesta::error(422, "la huella no es UTF-8"),
                }
            }
            ("POST", ["colecciones", b, s, n, "items", "resolver"]) => {
                let c = match ore_core::parse::parse(&p.cuerpo) {
                    Ok(c) if !p.cuerpo.trim().is_empty() => c,
                    _ => {
                        return Respuesta::error(
                            400,
                            "el cuerpo no es JSON: `{huellas: [...], ttl?}`",
                        );
                    }
                };
                let huellas: Vec<String> = c
                    .get("huellas")
                    .map(|(_, v)| v.items())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                let ttl = c
                    .get("ttl")
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|t| t.parse().ok());
                self.servir_items(rama, p, b, s, n, &huellas, ttl, false)
            }
            ("GET", ["datasets"]) => self.datasets(rama),
            // 0038: `{ns}/{n}` es de `default`; `{base}/{schema}/{n}`, de su schema.
            ("GET", ["datasets", ns, n]) => {
                self.ficha_del_dataset(rama, ns, ore_core::normalize::SCHEMA_POR_DEFECTO, n)
            }
            ("GET", ["datasets", b, s, n]) => self.ficha_del_dataset(rama, b, s, n),
            // El techo de la clase también aquí (0036 ⑤): `confirmar` mueve el
            // puntero de un dataset, que es escribir.
            ("POST", ["datasets", _, _, "confirmar"] | ["datasets", _, _, _, "confirmar"])
                if p.cabeceras
                    .get(crate::puestos::PUESTO)
                    .and_then(|id| self.clase_de(id.trim()))
                    .is_some_and(|c| !c.escribe) =>
            {
                Respuesta::error(
                    403,
                    "este puesto vive en un repositorio que no escribe datos (0036 ⑤): confirmar mueve el puntero de un dataset",
                )
            }
            ("POST", ["datasets", ns, n, "confirmar"]) => self.confirmar_dataset(
                rama,
                sujeto,
                ns,
                ore_core::normalize::SCHEMA_POR_DEFECTO,
                n,
                &p.cuerpo,
            ),
            ("POST", ["datasets", b, s, n, "confirmar"]) => {
                self.confirmar_dataset(rama, sujeto, b, s, n, &p.cuerpo)
            }
            // ⭐ 0036 ④: con `X-Ore-Raiz`, sólo las que tocan SUS ficheros —la
            //   pestaña «Pull requests» de un repositorio—.
            ("GET", ["propuestas"]) => {
                match crate::entorno::alcance_valido(
                    p.cabeceras.get("x-ore-raiz").map(String::as_str),
                ) {
                    Ok(a) => self.propuestas(a.as_deref()),
                    Err(r) => r,
                }
            }
            ("POST", ["propuestas"]) => self.proponer(sujeto, &p.cuerpo),
            ("GET", ["propuestas", n]) => match n.parse::<u64>() {
                Ok(n) => self.propuesta(sujeto, n),
                Err(_) => Respuesta::error(404, format!("`{n}` no es un número de propuesta")),
            },
            ("POST", ["propuestas", n, "revisar"]) => match n.parse::<u64>() {
                Ok(n) => self.revisar(sujeto, n, &p.cuerpo),
                Err(_) => Respuesta::error(404, format!("`{n}` no es un número de propuesta")),
            },
            ("POST", ["propuestas", n, "fusionar"]) => match n.parse::<u64>() {
                Ok(n) => self.fusionar(sujeto, n),
                Err(_) => Respuesta::error(404, format!("`{n}` no es un número de propuesta")),
            },
            ("DELETE", ["propuestas", n]) => match n.parse::<u64>() {
                Ok(n) => self.cerrar_propuesta(n),
                Err(_) => Respuesta::error(404, format!("`{n}` no es un número de propuesta")),
            },
            // ── 0029 F4a I3 · las funciones y su invocación (`funciones.rs`) ──
            ("GET", ["funciones"]) => self.leyendo_en(rama, |r| self.funciones(r)),
            // 0038 P6c: `{ns}/{n}` es de `default`; `{b}/{s}/{n}`, de su schema.
            ("GET", ["funciones", ns, n, "resultados"])
            | ("GET", ["funciones", ns, _, n, "resultados"]) => {
                let schema = match seg {
                    [_, _, s, _, _] => s.to_string(),
                    _ => ore_core::normalize::SCHEMA_POR_DEFECTO.to_string(),
                };
                let (ns, n) = (ns.to_string(), n.to_string());
                self.leyendo(move |r| self.resultados(r, &ns, &schema, &n))
            }
            // Invocar no escribe el árbol: escribe la cola. Se lee el árbol
            // para decidir, y el Job hace el resto.
            ("POST", ["funciones", ns, n, "invocar"])
            | ("POST", ["funciones", ns, _, n, "invocar"]) => {
                let schema = match seg {
                    [_, _, s, _, _] => s.to_string(),
                    _ => ore_core::normalize::SCHEMA_POR_DEFECTO.to_string(),
                };
                let (ns, n) = (ns.to_string(), n.to_string());
                self.leyendo(move |r| self.invocar(r, &ns, &schema, &n, sujeto))
            }
            // ── 0030 W1 ④ · la pregunta, servida (`preguntar.rs`) ──────────
            // Síncrono y de lectura: clona, `ore ask`, y devuelve las filas.
            ("POST", ["vistas", ns, n, "ejecutar"]) => {
                let (ns, n) = (ns.to_string(), n.to_string());
                let cuerpo = p.cuerpo.clone();
                self.leyendo(move |r| {
                    self.ejecutar(r, &ns, ore_core::normalize::SCHEMA_POR_DEFECTO, &n, &cuerpo)
                })
            }
            // 0038: con su schema, `/vistas/{base}/{schema}/{n}/ejecutar`.
            ("POST", ["vistas", b, s, n, "ejecutar"]) => {
                let (b, s, n) = (b.to_string(), s.to_string(), n.to_string());
                let cuerpo = p.cuerpo.clone();
                self.leyendo(move |r| self.ejecutar(r, &b, &s, &n, &cuerpo))
            }
            // ── Ontology Forge · los documentos, por kind (`documentos.rs`) ──
            // El kind se resuelve contra `documentos::KINDS`: un kind que no
            // esté en la tabla es 404 con la lista de los que sí. La medida
            // lee la misma tabla, así que lo que no se sirve no cuenta.
            //
            // **Y desde un puesto** (W3.7 ①, «declarar»): con `x-ore-puesto`
            // el sujeto es la persona que lo abrió y la rama la del puesto,
            // como en `/v1` y `/datasets`; lo que una celda declara lo firma
            // quien la escribió, y va a su rama.
            ("GET" | "PUT" | "DELETE", ["documentos", ..]) | ("GET", ["conceptos"]) => {
                let (sujeto, rama) = match self.sujeto_del_puesto(p, sujeto, rama) {
                    Ok(x) => x,
                    Err(r) => return r,
                };
                self.documentos(p, &sujeto, rama.as_deref(), seg)
            }
            ("GET", _) | ("POST", _) | ("PUT", _) | ("DELETE", _) => {
                Respuesta::error(404, "no hay nada en ese camino")
            }
            _ => Respuesta::error(405, "método no admitido"),
        }
    }

    // ── Dónde se trabaja ────────────────────────────────────────────────────

    /// Le da a `f` un árbol para leer. Con la forja, lo último de ella —el
    /// árbol de su commit en el espejo, compartido (0046 E5b·2)—; con un
    /// directorio, el de siempre.
    ///
    /// ⛔ Para LEER: el árbol se comparte con las demás peticiones de ese
    ///   commit. Quien necesite escribir va por `escribiendo`.
    pub(crate) fn leyendo(&self, f: impl FnOnce(&Path) -> Respuesta) -> Respuesta {
        match &self.arbol {
            Arbol::Directorio(d) => f(d),
            Arbol::Forja(forja) => match forja.para_leer(None) {
                Err(e) => Respuesta::error(502, e.to_string()),
                Ok(arbol) => f(arbol.ruta()),
            },
        }
    }

    /// Igual, y además **publica lo que `f` haya cambiado**.
    ///
    /// Sólo publica si la respuesta fue buena: una petición que acaba en `422`
    /// deja el clon a medias, y el clon se tira. Es lo que hace que un error no
    /// pueda dejar el árbol a medio escribir — no hay nada que deshacer porque
    /// no se llegó a escribir en ningún sitio duradero.
    /// Las rutas de los documentos por kind, ya con el sujeto y la rama
    /// resueltos (los del puesto, si la petición viene de uno).
    fn documentos(
        &self,
        p: &Peticion,
        sujeto: &Identidad,
        rama: Option<&str>,
        seg: &[&str],
    ) -> Respuesta {
        match (p.metodo.as_str(), seg) {
            ("GET", ["documentos", kind]) => {
                let kind = kind.to_string();
                self.leyendo_en(rama, move |r| documentos::listar(r, &kind))
            }
            // Los conceptos del árbol y los importados de `vendor/*.oob`, con
            // quién los habla: lo que la sección Concepts pinta.
            ("GET", ["conceptos"]) => self.leyendo_en(rama, documentos::conceptos),
            // 0038: `{ns}/{n}` es de `default`; `{base}/{schema}/{n}`, de su schema.
            ("GET" | "PUT" | "DELETE", ["documentos", kind, ns, n])
            | ("GET" | "PUT" | "DELETE", ["documentos", kind, ns, _, n]) => {
                let schema = match seg {
                    [_, _, _, s, _] => s.to_string(),
                    _ => ore_core::normalize::SCHEMA_POR_DEFECTO.to_string(),
                };
                let (kind, ns, n) = (kind.to_string(), ns.to_string(), n.to_string());
                let corto = ore_core::normalize::corto(&ns, &schema, &n);
                let que = documentos::kind_de(&kind).map_or(kind.clone(), |k| k.articulo.into());
                let si_commit = p.cabeceras.get("if-match").cloned();
                match p.metodo.as_str() {
                    "GET" => {
                        self.leyendo_en(rama, move |r| documentos::uno(r, &kind, &ns, &schema, &n))
                    }
                    "PUT" => {
                        let cuerpo = p.cuerpo.clone();
                        self.escribiendo_en(
                            rama,
                            sujeto,
                            &format!("escribir {que} `{corto}`"),
                            |r| {
                                self.escribir_documento(
                                    r,
                                    &kind,
                                    &ns,
                                    &schema,
                                    &n,
                                    &cuerpo,
                                    si_commit.as_deref(),
                                )
                            },
                        )
                    }
                    _ => self.escribiendo_en(
                        rama,
                        sujeto,
                        &format!("retirar {que} `{corto}`"),
                        |r| {
                            self.retirar_documento(
                                r,
                                &kind,
                                &ns,
                                &schema,
                                &n,
                                si_commit.as_deref(),
                                sujeto,
                            )
                        },
                    ),
                }
            }
            ("GET", _) | ("POST", _) | ("PUT", _) | ("DELETE", _) => {
                Respuesta::error(404, "no hay nada en ese camino")
            }
            _ => Respuesta::error(405, "método no admitido"),
        }
    }

    pub(crate) fn escribiendo(
        &self,
        sujeto: &Identidad,
        mensaje: &str,
        f: impl FnOnce(&Path) -> Respuesta,
    ) -> Respuesta {
        match &self.arbol {
            Arbol::Directorio(d) => f(d),
            Arbol::Forja(forja) => {
                let prestado = match forja.clonar() {
                    Ok(p) => p,
                    Err(e) => return Respuesta::error(502, e.to_string()),
                };
                let mut r = f(prestado.ruta());
                if r.codigo >= 300 || !forja.hay_cambios(prestado.ruta()) {
                    return r;
                }
                match forja.publicar(prestado.ruta(), sujeto, mensaje) {
                    Ok(commit) => {
                        if let Json::Obj(m) = &mut r.cuerpo {
                            m.insert("commit".into(), Json::s(commit));
                        }
                        r
                    }
                    Err(git::Fallo::Adelantado(m)) => {
                        Respuesta::error(409, git::Fallo::Adelantado(m).to_string())
                    }
                    Err(e) => Respuesta::error(502, e.to_string()),
                }
            }
        }
    }

    // ── Lo que no toca el árbol ─────────────────────────────────────────────

    fn version(&self) -> Respuesta {
        let aqui = std::env::temp_dir();
        match mando::correr(&self.binario, &aqui, &[String::from("--version")]) {
            Ok(s) if s.bien() => Respuesta::ok(Json::obj([
                ("motor", Json::s(s.stdout.trim())),
                ("plano", Json::s("control")),
                (
                    "arbol",
                    Json::s(match &self.arbol {
                        Arbol::Directorio(_) => "directorio",
                        Arbol::Forja(_) => "forja",
                    }),
                ),
            ])),
            Ok(s) => Respuesta::error(500, format!("`ore --version` falló: {}", s.stderr.trim())),
            Err(e) => Respuesta::error(500, e.to_string()),
        }
    }

    // ── Las fuentes ─────────────────────────────────────────────────────────

    /// Manda el valor de la fuente al custodio. Devuelve una frase que dice qué
    /// pasó — nunca un fallo que tumbe el alta, porque el árbol ya está escrito
    /// y negar el 201 sería mentir sobre lo que sí ocurrió.
    ///
    /// ⛔ El nombre del secreto es `fuente-<nombre>` y NO el de la variable de
    /// entorno. La variable la deriva `ore source add` de `metadata.name`, así
    /// que renombrar la organización la cambiaría — y un secreto cuyo nombre
    /// cambia cuando cambia otra cosa es un secreto que se pierde. Quien lo
    /// consuma lee `connectionEnv` del manifiesto y sabe bajo qué nombre
    /// exportarlo.
    ///
    /// ⚠️ Y con GUION y no con barra: el nombre viaja como un SEGMENTO de la
    /// ruta cuando alguien lo lee — `GET /organizaciones/{org}/secretos/{nombre}`
    /// — y una barra lo partiria en dos. El alfabeto de `concesion.recurso` lo
    /// admitiria; la ruta no.
    ///
    /// ⭐ La clase es `conexion`, que el custodio ya tenia en su lista desde el
    /// primer dia y nadie habia usado. Es literalmente lo que se pidio cuando se
    /// diseno: *«credenciales de bases de datos, de sources»*.
    fn guardar_credencial(
        &self,
        nombre: &str,
        url: &str,
        testigo: Option<&str>,
    ) -> Result<String, (Option<u16>, String)> {
        let (Some(cofre), Some(org)) = (&self.cofre, &self.organizacion) else {
            return Err((None, "NO guardada: este servidor no sabe de ningun custodio (`--cofre` y `--organizacion`)"
                .into()));
        };
        let Some(t) = testigo else {
            return Err((
                None,
                "NO guardada: la peticion no traia testigo que reenviar".into(),
            ));
        };
        let cuerpo = Json::obj([
            ("nombre", Json::s(format!("fuente-{nombre}"))),
            ("clase", Json::s("conexion")),
            ("valor", Json::s(url)),
        ]);
        match http::pedir(
            "POST",
            cofre,
            &format!("/organizaciones/{org}/secretos"),
            Some(t),
            Some(&cuerpo),
        ) {
            Err(e) => Err((None, format!("NO guardada: {e}"))),
            Ok((c, _)) if (200..300).contains(&c) => Ok(format!(
                "guardada en el custodio como `fuente-{nombre}`, clase `conexion`"
            )),
            Ok((c, b)) => Err((
                Some(c),
                format!(
                    "NO guardada: el custodio contesto {c} · {}",
                    b.trim().chars().take(90).collect::<String>()
                ),
            )),
        }
    }

    /// **Retirar una fuente**: `DELETE /fuentes/{n}` — el inverso del alta,
    /// servido. Tres cosas en un acto, y una cuarta que se dice:
    ///
    /// 1. la conexión fuera de `ontology.config.yaml` (`ore source remove`);
    /// 2. su catálogo (`packages/<n>`, manifiesto + `discover.catalog.json`)
    ///    fuera del árbol — es lo que el Job de catálogo dejó, y sin conexión
    ///    no es de nadie; el reconciliador no lo re-cataloga porque la fuente
    ///    ya no está declarada;
    /// 3. su Job de catálogo fuera de la cola, si seguía ahí;
    /// 4. la credencial fuera del custodio (`DELETE …/secretos/fuente-<n>`, con
    ///    el testigo de quien pide: el cofre decide y deja su huella). Si el
    ///    custodio dice que no, la fuente ya no está y se DICE lo que quedó.
    ///
    /// **409 si alguna database sale de ella**: una base es un paquete con
    /// alcance cuyo `source` es esta fuente, y sus tablas la nombran como
    /// `datasource`. Retirar la fuente las dejaría sin compilar; se retiran
    /// antes, y aquí se dice cuáles. Y la puerta de siempre: si el árbol
    /// empeora por algo que no la nombra, no se escribe nada.
    fn retirar_fuente(
        &self,
        raiz: &Path,
        nombre: &str,
        testigo: Option<&str>,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(nombre) {
            return Respuesta::error(422, format!("nombre de fuente: {m}"));
        }
        let manifiesto = raiz.join("ontology.config.yaml");
        let declarada = std::fs::read_to_string(&manifiesto)
            .ok()
            .and_then(|t| parse::parse(&t).ok())
            .and_then(|n| {
                n.get("datasources").map(|(_, v)| {
                    v.items()
                        .iter()
                        .any(|d| d.get("name").and_then(|(_, x)| x.as_str()) == Some(nombre))
                })
            })
            .unwrap_or(false);
        if !declarada {
            return Respuesta::error(404, format!("no hay ninguna fuente `{nombre}` declarada"));
        }
        // Los paquetes que salen de ella: 409 con la lista. ⭐ 0045 P2: por
        // alcance, por una `Table` con `datasource: <n>` y por lo que lee una
        // `Table` de su paquete (`punteros::bases_que_salen_de`). Antes solo lo
        // segundo, en carpetas `tables/`: con los punteros en la fuente no veía
        // a nadie y la retiraba con ellos dentro.
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        let bases = crate::punteros::bases_que_salen_de(raiz, &pkg, nombre);
        drop(pkg);
        if !bases.is_empty() {
            return Respuesta::error(
                409,
                format!(
                    "de `{nombre}` salen {} database(s): {}. Retíralas antes; sus tablas nombran esta fuente como `datasource`",
                    bases.len(),
                    bases.join(", ")
                ),
            );
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        // ① la conexión. `--keep-secret`: en un árbol servido no hay
        //   `.env.local`; la credencial está en el custodio.
        let salida = mando::correr(
            &self.binario,
            raiz,
            &[
                "source".into(),
                "remove".into(),
                nombre.into(),
                "--path".into(),
                ".".into(),
                "--keep-secret".into(),
            ],
        );
        match salida {
            Err(e) => return Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => {
                return Respuesta::error(
                    422,
                    format!(
                        "`ore source remove` devolvió {}: {}",
                        s.codigo,
                        primera_linea(&s.stdout, &s.stderr)
                    ),
                );
            }
            Ok(_) => {}
        }
        // ② su catálogo. Sólo si es eso —manifiesto y catálogo, sin alcance—;
        //   un paquete con alcance que se llame como la fuente sería una base,
        //   y ya se habría contestado 409.
        let dir = raiz.join("packages").join(nombre);
        let catalogo_retirado = if dir.join("discover.catalog.json").is_file()
            && !dir.join("discover.scope.json").is_file()
        {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                return Respuesta::error(
                    500,
                    format!("no se pudo retirar `packages/{nombre}`: {e}"),
                );
            }
            true
        } else {
            false
        };
        // La puerta: sólo puede empeorar por lo que la NOMBRA.
        let nombra = |d: &Json| !crate::copia::texto_de(d).contains(nombre);
        if let Err(r) = self.empeora_salvo(
            raiz,
            &antes,
            &format!("retirar la fuente `{nombre}`"),
            nombra,
        ) {
            return r;
        }
        // ③ la cola.
        let desencolado = self.desencolar_catalogo(nombre, sujeto);
        // ④ la credencial.
        let secreto = self.retirar_credencial(nombre, testigo);
        Respuesta::ok(Json::obj([
            ("source", Json::s(nombre)),
            ("retirada", Json::Bool(true)),
            ("catalogo", Json::Bool(catalogo_retirado)),
            ("desencolado", Json::s(desencolado)),
            ("secreto", Json::s(secreto)),
        ]))
    }

    /// `DELETE /organizaciones/{org}/secretos/fuente-<n>` en el custodio, con el
    /// testigo de quien pide. Lo que dice va en la respuesta; no deshace la
    /// baja de la fuente, que ya está escrita.
    fn retirar_credencial(&self, nombre: &str, testigo: Option<&str>) -> String {
        let (Some(cofre), Some(org)) = (&self.cofre, &self.organizacion) else {
            return format!(
                "`fuente-{nombre}` sigue en el custodio: este servidor no sabe de ninguno (`--cofre` y `--organizacion`)"
            );
        };
        let Some(t) = testigo else {
            return format!(
                "`fuente-{nombre}` sigue en el custodio: la petición no traía testigo que reenviar"
            );
        };
        match http::pedir(
            "DELETE",
            cofre,
            &format!("/organizaciones/{org}/secretos/fuente-{nombre}"),
            Some(t),
            None,
        ) {
            Err(e) => format!("`fuente-{nombre}` sigue en el custodio: {e}"),
            Ok((c, _)) if (200..300).contains(&c) => {
                format!("`fuente-{nombre}` retirada del custodio")
            }
            Ok((c, b)) => format!(
                "`fuente-{nombre}` sigue en el custodio: contestó {c} · {}",
                b.trim().chars().take(90).collect::<String>()
            ),
        }
    }

    /// El Job de catálogo de una fuente fuera de la cola, si seguía ahí. Lo
    /// que dice va en la respuesta; no tumba la baja, que ya está escrita.
    fn desencolar_catalogo(&self, fuente: &str, sujeto: &Identidad) -> String {
        let Some(forja) = &self.cola else {
            return "sin cola (`--cola`): nada que desencolar".into();
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("NO desencolado: {e}"),
        };
        let dir = prestado.ruta();
        let fichero = format!("44-el-catalogo-{}.yaml", cola::nombre_de_objeto(fuente));
        if !dir.join(&fichero).is_file() {
            return format!("`{fichero}` no estaba en la cola");
        }
        if let Err(e) = std::fs::remove_file(dir.join(&fichero)) {
            return format!("NO desencolado: {e}");
        }
        match forja.publicar(
            dir,
            sujeto,
            &format!("Desencolar el catálogo de `{fuente}`"),
        ) {
            Ok(c) => format!("`{fichero}` fuera de la cola · commit {c}"),
            Err(e) => format!("NO desencolado: {e}"),
        }
    }

    fn alta_de_fuente(
        &self,
        raiz: &Path,
        cuerpo: &str,
        testigo: Option<&str>,
        sujeto: &Identidad,
    ) -> Respuesta {
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let campo = |k: &str| {
            cuerpo
                .get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string)
        };
        let Some(nombre) = campo("name") else {
            return Respuesta::error(422, "falta `name`");
        };
        let Some(url) = campo("url") else {
            return Respuesta::error(422, "falta `url`");
        };
        // ⭐ 0045 P1.5: el nombre, con la regla que todo lo que llega a ser
        //   admite, y lo que ya lo ocupa, ANTES de escribir nada.
        if let Err(m) = nombre_de_fuente(&nombre) {
            return con_sugerencia(422, m, sugerir_nombre_de_fuente(&nombre));
        }
        if fuente_declarada(raiz, &nombre) {
            return con_sugerencia(
                409,
                format!("ya hay una fuente `{nombre}`"),
                nombre_libre(raiz, &nombre),
            );
        }
        // ⛔ Un paquete con ese nombre haría nacer la fuente «catalogada» —
        //   `GET /fuentes/{n}/estado` mira si existe `packages/<n>/`— y el Job
        //   de catálogo se la saltaría para siempre.
        if raiz.join("packages").join(&nombre).exists() {
            return con_sugerencia(
                409,
                format!(
                    "ya hay un paquete `{nombre}` en el árbol, y una fuente es también su paquete"
                ),
                nombre_libre(raiz, &nombre),
            );
        }
        // ── LA GUARDA, QUE AHORA DEPENDE DE QUE HAYA DONDE GUARDARLA ──────
        //
        // Esto rechazaba SIEMPRE una URL con credencial dentro, y el modulo
        // dejo escrito por que y hasta cuando: *«el dia que haya un sitio de
        // verdad donde ponerla, esta negativa es lo que hay que quitar»*.
        //
        // Ese sitio existe desde hoy: el custodio. Asi que la negativa deja de
        // ser incondicional y pasa a preguntar lo unico que importaba —**si hay
        // donde ponerla**—. Sin custodio configurado sigue diciendo que no, y
        // con el mismo motivo de siempre: `ore source add` la mandaria a
        // `.env.local`, que en este servidor vive en un clon que se tira.
        //
        // ⛔ Quitarla a secas habria reabierto el agujero exactamente donde
        //   nadie mira: un `ore-serve` sin `--cofre` aceptando la contraseña de
        //   produccion de alguien y perdiendola en silencio.
        let trae_credencial = sin_credencial(&url).is_err();
        if trae_credencial && self.cofre.is_none() {
            return Respuesta::error(
                422,
                "esta URL trae una credencial dentro y este servidor no sabe de ningun \
                 custodio (`--cofre` y `--organizacion`). `ore source add` la mandaria a \
                 `.env.local`, y un fichero en el disco de un pod no es un secreto guardado.",
            );
        }

        let mut args = vec![
            "source".into(),
            "add".into(),
            "--name".into(),
            nombre.clone(),
            url.clone(),
        ];
        if let Some(t) = campo("type") {
            args.push("--type".into());
            args.push(t);
        }
        if let Some(d) = campo("description") {
            args.push("--description".into());
            args.push(d);
        }

        match mando::correr(&self.binario, raiz, &args) {
            Err(e) => Respuesta::error(500, e.to_string()),
            // ⭐ 0045 P1.5: 422. Lo que ocupa el nombre ya se ha mirado arriba
            //   (409); lo que `ore source add` rechaza es lo que se le dio —la
            //   URL, el tipo—, y eso no es un conflicto con nada.
            Ok(s) if !s.bien() => Respuesta::error(
                422,
                format!(
                    "`ore source add` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(s) => {
                // ── ⭐⭐ Y AHORA LA CREDENCIAL, QUE ANTES SE PERDÍA ────────
                //
                // `ore source add` la escribe en `.env.local`, que está en el
                // `.gitignore` — así que vivía dentro de un clon que se tira al
                // volver de esta función. El manifiesto declaraba dónde
                // buscarla y **no había dónde**.
                //
                // ⇒ Va al custodio, que se construyó para esto y llevaba días
                //   con cero secretos dentro. Éste es su primer cliente.
                //
                // ⛔ Y DESPUÉS de escribir el árbol, pero ANTES de publicarlo:
                //   `escribiendo` sólo empuja si esta función contesta 2xx. Si
                //   el custodio falla, la respuesta no es 2xx y el árbol no se
                //   publica: no queda ni fuente sin credencial ni credencial sin
                //   fuente. (✏️ 2026-09-29: aquí se decía que la fuente quedaba
                //   declarada igual; dejó de ser verdad cuando el empuje pasó a
                //   depender del código, y el mensaje del 502 lo repetía en falso.)
                let guardada = self.guardar_credencial(&nombre, &url, testigo);

                // ⛔⛔ Y SI TRAIA CREDENCIAL Y NO SE GUARDO, ESTO NO ES UN 201.
                //
                // Para BigQuery la URL es inocua —la credencial la presta la
                // nube— y perderla no pierde nada. Para Postgres es la
                // contraseña de produccion de alguien: contestar 201 pintaria
                // un tick verde encima de un secreto que ya no existe en ningun
                // sitio, y el fallo aparecería media hora despues en el
                // registro de otro Job.
                //
                // ⭐ 0045 P1.5: un 409 del custodio es un secreto VIVO
                //   `fuente-<n>` que ninguna fuente declara: un huérfano, de un
                //   alta que guardó la credencial y no llegó a publicar el
                //   árbol. Uno retirado ya no ocupa el nombre (la `043`). El
                //   nombre está ocupado y se dice como los demás; el árbol no
                //   se publica y nada se encola.
                if let Err((Some(409), m)) = &guardada {
                    return con_sugerencia(
                        409,
                        format!(
                            "el custodio ya guarda un secreto vivo `fuente-{nombre}` que \
                             ninguna fuente declara (lo retira `ore-cofre \
                             retirar-huerfanos`): {m}"
                        ),
                        nombre_libre(raiz, &format!("{nombre}_2")),
                    );
                }
                let guardada = match guardada {
                    Ok(g) => g,
                    Err((_, m)) => m,
                };
                if trae_credencial && !guardada.starts_with("guardada") {
                    return Respuesta::error(
                        502,
                        format!(
                            "la fuente `{nombre}` no se dio de alta: su credencial no se \
                             guardó ({guardada}), y sin ella el árbol no se publica. \
                             Se puede volver a intentar con el mismo nombre."
                        ),
                    );
                }
                // Se encola cuando la fuente va a quedar: después de la
                // credencial, no antes (un 502 no publica el árbol, y un Job
                // encolado para una fuente que no existe se quedaba en la cola).
                let encolado = self.encolar_catalogo(&nombre, sujeto);

                Respuesta::creado(Json::obj([
                    ("name", Json::s(nombre)),
                    ("informe", Json::s(s.stdout.trim())),
                    // ⚠️ Se dice SIEMPRE, y con lo que pasó. Un alta que
                    //   contesta 201 y calla que la credencial no se guardó
                    //   deja el fallo para el Job de catálogo, media hora más
                    //   tarde y en otro registro.
                    ("credencial", Json::s(guardada)),
                    // ⭐ Y si se encoló o por qué no. Se dice SIEMPRE, por lo
                    //   mismo que `credencial`: un alta que contesta bien y calla
                    //   que el trabajo no quedó encolado deja el fallo para una
                    //   pantalla vacía media hora después.
                    ("encolado", Json::s(encolado)),
                    // Lo que sigue, dicho en la respuesta y no en la
                    // documentación: leer el origen NO se hace aquí, y quien
                    // pintó el botón tiene que saberlo sin ir a buscarlo.
                    (
                        "siguiente",
                        Json::s(
                            "leer el catálogo del origen no corre en el plano de control: \
                             necesita un Job con la imagen de drivers",
                        ),
                    ),
                ]))
            }
        }
    }

    /// ⭐⭐ LOS TRES ESTADOS DE UNA FUENTE, y por que son tres.
    ///
    /// `catalogada`  su paquete esta en el arbol. Hay esquema que enseñar.
    /// `encolada`    su Job esta escrito en la cola: se esta leyendo el origen
    ///               AHORA, o esta a punto. Se espera y aparece solo.
    /// `pendiente`   nadie lo ha encolado. Esperar no sirve de nada.
    ///
    /// ⛔ La ficha las pintaba todas igual —«el Job aun no ha corrido»— y esa
    ///   frase es cierta en dos de los tres casos y engañosa en uno: mientras el
    ///   Job LEE el origen, decir que no ha corrido manda a buscar un fallo que
    ///   no existe.
    ///
    /// ⚠️ Y saber si esta encolada CUESTA UN CLON de la cola, asi que solo se
    ///   hace cuando no hay paquete. Quien pregunta esta mirando una ficha sin
    ///   esquema y deja de preguntar en cuanto aparece — la ventana es la que
    ///   tarda un catalogo, un par de minutos.
    ///
    /// ⭐⭐ Y UN CUARTO: `fallida`. El Job que falla deja su diagnóstico en el
    ///   árbol (`cola::Fallo`); si el Job de la cola sigue siendo ese, la
    ///   fuente está fallida y se dice por qué. Si es otro, alguien pulsó
    ///   «Reintentar» y está `encolada` otra vez.
    fn estado(&self, fuente: &str) -> Respuesta {
        let cola = self.cola.as_ref();
        let fallida = |f: cola::Fallo| {
            Respuesta::ok(Json::obj([
                ("estado", Json::s("fallida")),
                (
                    "dice",
                    Json::s("el catalogo fallo; su diagnostico esta en el arbol"),
                ),
                ("job", Json::s(f.job)),
                ("fin", Json::s(f.fin)),
                ("log", Json::s(f.log)),
            ]))
        };
        // ⭐ TODO dentro del mismo clon del arbol: preguntar por el paquete y
        //   por la cola son dos preguntas, pero una sola visita a la forja.
        self.leyendo(move |raiz| {
            // ① El paquete, que es lo definitivo.
            if raiz.join("packages").join(fuente).is_dir() {
                return Respuesta::ok(Json::obj([
                    ("estado", Json::s("catalogada")),
                    ("dice", Json::s("su catalogo esta en el arbol")),
                ]));
            }
            // ⛔ `fuente` viene de la URL: solo nombres, nunca un camino.
            let fallo = fuente
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                .then(|| {
                    std::fs::read_to_string(
                        raiz.join(".fallos").join(format!("{fuente}.catalogo.txt")),
                    )
                    .ok()
                })
                .flatten()
                .and_then(|t| cola::fallo_de(&t));
            // ② La cola. Sin `--cola` NO se contesta `pendiente`: eso afirmaria
            //    que nadie lo encolo, y lo cierto es que no se puede saber.
            let Some(cola) = cola else {
                if let Some(f) = fallo {
                    return fallida(f);
                }
                return Respuesta::ok(Json::obj([
                    ("estado", Json::s("desconocido")),
                    (
                        "dice",
                        Json::s(
                            "este servidor no sabe de ninguna cola, asi que no puede \
                            decir si hay trabajo encolado",
                        ),
                    ),
                ]));
            };
            let prestado = match cola.clonar() {
                Ok(p) => p,
                Err(e) => {
                    return Respuesta::ok(Json::obj([
                        ("estado", Json::s("desconocido")),
                        ("dice", Json::s(format!("no se pudo leer la cola: {e}"))),
                    ]));
                }
            };
            let fichero = format!("44-el-catalogo-{}.yaml", cola::nombre_de_objeto(fuente));
            let en_cola = std::fs::read_to_string(prestado.ruta().join(&fichero)).ok();
            match (fallo, en_cola) {
                // El Job que falló es el que sigue en la cola: nadie reintentó.
                (Some(f), Some(t)) if cola::job_de(&t) == Some(f.job.as_str()) => fallida(f),
                (Some(f), None) => fallida(f),
                (_, Some(_)) => Respuesta::ok(Json::obj([
                    ("estado", Json::s("encolada")),
                    ("dice", Json::s("se esta leyendo el origen")),
                ])),
                (None, None) => Respuesta::ok(Json::obj([
                    ("estado", Json::s("pendiente")),
                    ("dice", Json::s("nadie ha encolado su catalogo todavia")),
                ])),
            }
        })
    }

    /// **`POST /fuentes/comprobaciones {url}`**: encola la comprobación de
    /// acceso (`malla/54-la-comprobacion.yaml`) y devuelve el nombre del Job.
    ///
    /// ⛔ ore-serve no contesta la pregunta: no habla con Google y su cuenta no
    ///   es la del driver. La contesta el Job, con la identidad que leerá de
    ///   verdad, y su JSON vuelve por el informador a `ore-iam` (0026): la
    ///   consola lo lee en `/celdas` buscando este `job`.
    fn comprobar_fuente(&self, cuerpo: &str, sujeto: &Identidad) -> Respuesta {
        let n = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let Some(url) = n.get("url").and_then(|(_, v)| v.as_str()).map(str::trim) else {
            return Respuesta::error(400, "falta `url`: `bigquery://<proyecto>/<dataset>`");
        };
        let tipo = match cola::url_sin_secreto(url) {
            Ok(t) => t,
            Err(e) => return Respuesta::error(422, e),
        };
        let Some(forja) = &self.cola else {
            return Respuesta::error(
                503,
                "este servidor no sabe de ninguna cola (`--cola`): no hay dónde comprobar",
            );
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return Respuesta::error(503, format!("no se pudo leer la cola: {e}")),
        };
        let dir = prestado.ruta();
        let Ok(plantilla) = std::fs::read_to_string(dir.join(cola::PLANTILLA_COMPROBACION)) else {
            return Respuesta::error(
                503,
                format!(
                    "la cola no trae `{}`: hay que converger este inquilino",
                    cola::PLANTILLA_COMPROBACION
                ),
            );
        };
        let corrida = crate::funciones::corrida_ahora();
        let (fichero, texto) = match cola::rendir_comprobacion(&plantilla, tipo, url, &corrida) {
            Ok(v) => v,
            Err(e) => return Respuesta::error(422, e),
        };
        let job = texto
            .lines()
            .find_map(|l| l.trim().strip_prefix("name: comprobar-"))
            .map(|h| format!("comprobar-{h}"))
            .unwrap_or_default();
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return Respuesta::error(500, format!("no se pudo escribir `{fichero}`: {e}"));
        }
        match forja.publicar(dir, sujeto, &format!("Comprobar el acceso a `{url}`")) {
            Ok(c) => Respuesta::ok(Json::obj([
                ("job", Json::s(&job)),
                ("estado", Json::s("encolada")),
                ("commit", Json::s(c)),
                (
                    "dice",
                    Json::s(
                        "la comprobación corre con la cuenta del driver; su resultado llega en \
                         el estado de la celda (`/celdas`), en el log de este Job",
                    ),
                ),
            ])),
            Err(e) => Respuesta::error(503, format!("no se pudo encolar: {e}")),
        }
    }

    /// ⭐⭐ ENCOLAR EL CATALOGO, EN EL MISMO ACTO DEL ALTA.
    ///
    /// Medido el 2026-09-10: un alta a las 19:24 no tenia su Job hasta las 20:17,
    /// porque quien lo rendia era la convergencia y solo la llamaba el cron.
    /// Escribir aqui la cola hace que el webhook de la forja dispare, el
    /// `Receiver` la reconcilie y Flux cree el Job — segundos, y ni un actor
    /// nuevo.
    ///
    /// ⛔ Y NO se deshace el alta si esto falla. La fuente esta en el arbol y el
    ///   commit existe; ademas la convergencia sigue rindiendo lo que falte, asi
    ///   que un fallo aqui es LENTITUD, no perdida. Se dice y se sigue.
    ///
    /// ⚠️ La plantilla la deja el aprovisionador en la propia cola. Si no esta
    ///   —un inquilino aprovisionado antes de que esto existiera— se dice con esa
    ///   frase, que es la que manda a converger.
    fn encolar_catalogo(&self, fuente: &str, sujeto: &Identidad) -> String {
        self.encolar_catalogo_corrida(fuente, sujeto, false)
    }

    /// Con `otra_vez`, una corrida dentro: otro Job aunque el anterior exista
    /// (`cola::rendir_corrida`).
    fn encolar_catalogo_corrida(&self, fuente: &str, sujeto: &Identidad, otra_vez: bool) -> String {
        let Some(forja) = &self.cola else {
            return "NO encolado: este servidor no sabe de ninguna cola (`--cola`); \
            lo rendira la convergencia"
                .into();
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("NO encolado: {e}"),
        };
        let dir = prestado.ruta();
        let plantilla = match std::fs::read_to_string(dir.join(cola::PLANTILLA)) {
            Ok(t) => t,
            Err(_) => {
                return format!(
                    "NO encolado: la cola no trae `{}`; hay que converger este inquilino",
                    cola::PLANTILLA
                );
            }
        };
        let corrida = otra_vez.then(crate::funciones::corrida_ahora);
        let (fichero, texto) = match cola::rendir_corrida(&plantilla, fuente, corrida.as_deref()) {
            Ok(v) => v,
            Err(e) => return format!("NO encolado: {e}"),
        };
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return format!("NO encolado: no se pudo escribir `{fichero}`: {e}");
        }
        // ⭐ Si el fichero ya estaba igual, no hay commit. El nombre lleva el
        //   resumen del contenido, asi que encolar dos veces la misma fuente es
        //   idempotente por construccion — y la historia de la cola no se llena
        //   de commits que no cambian nada.
        if !forja.hay_cambios(dir) {
            return format!("ya encolado como `{fichero}`");
        }
        match forja.publicar(dir, sujeto, &format!("Catalogar `{fuente}`")) {
            Ok(c) => format!("encolado como `{fichero}` · commit {c}"),
            Err(e) => format!("NO encolado: {e}"),
        }
    }

    /// **Una base es un paquete con alcance**, inducido del catalogo de otro.
    ///
    /// `source` es el paquete del que sale el catalogo — el que el Job dejo al
    /// descubrir la fuente entera, con `discover.catalog.json` al lado. `name`
    /// es el paquete nuevo, y `only` los objetos fisicos que entran, tal como
    /// el catalogo los nombra: `public.pedidos`.
    ///
    /// ⭐ Y `type` (0027 P1 I4b): `foreign` —un espejo, lo de siempre, y lo que
    ///   se entiende si falta— o `standard`: la regla va al alcance y el
    ///   inductor la aplica (`discover --type standard`): cada tabla del `only`
    ///   con clave nace con copia; las demas esperan la suya. Despues, el
    ///   conducto y el Job (`copia::tras_inducir`).
    ///
    /// ⛔ NO se vuelve a leer el origen. Eso lo hizo el Job, con credencial y
    ///   dentro de su red; esto induce de lo que aquel dejo escrito. Si el
    ///   origen cambio desde entonces lo dira `drift-detect`, que es su trabajo.
    fn alta_de_paquete(&self, raiz: &Path, cuerpo: &str, sujeto: &Identidad) -> Respuesta {
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let campo = |k: &str| {
            cuerpo
                .get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string)
        };
        let Some(nombre) = campo("name") else {
            return Respuesta::error(422, "falta `name`");
        };
        if let Err(m) = token(&nombre) {
            return Respuesta::error(422, format!("`name`: {m}"));
        }
        // ⛔ El nombre es el espacio de nombres de lo inducido (OOS2030). Con
        //   guion, `discover` escribia una base entera que no compila — medido
        //   en `victor` (`test-standard`). Se dice aqui, antes.
        if !ore_core::pertenencia::puede_ser_namespace(&nombre) {
            return Respuesta::error(
                422,
                format!(
                    "`name`: `{nombre}` no puede ser un espacio de nombres — una letra y luego letras, dígitos y `_` (sin guiones)"
                ),
            );
        }
        let tipo = match campo("type").as_deref() {
            None | Some("foreign") => "foreign",
            Some("standard") => "standard",
            Some(otro) => {
                return Respuesta::error(
                    422,
                    format!("`type` es `standard` o `foreign`, no `{otro}`"),
                );
            }
        };
        // ⭐ 0039: sin origen, una standard database vacía (`create standard
        //   database b` en un guion): lo que tenga se escribirá en el lago.
        let Some(fuente) = campo("source") else {
            return self.base_vacia(raiz, &nombre, campo("type").as_deref(), sujeto);
        };
        let objetos: Vec<String> = cuerpo
            .get("only")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|n| n.as_str())
            .map(str::to_string)
            .collect();
        if objetos.is_empty() {
            // ⛔ Sin `only` no hay base: seria copiar el paquete de la fuente
            //   con otro nombre, y ya existe.
            return Respuesta::error(422, "`only` está vacío: una base es lo que se elige");
        }

        let origen = match paquete_de(raiz, &fuente) {
            Ok(d) => d,
            Err(mut r) => {
                if r.codigo == 404 {
                    r = Respuesta::error(404, format!("no hay paquete `{fuente}` del que inducir"));
                }
                return r;
            }
        };
        let catalogo = origen.join("discover.catalog.json");
        if !catalogo.is_file() {
            return Respuesta::error(
                409,
                format!("`{fuente}` no tiene `discover.catalog.json`: no salió de un `discover`"),
            );
        }
        if raiz.join("packages").join(&nombre).exists() {
            return Respuesta::error(409, format!("ya hay un paquete `{nombre}`"));
        }
        // `s.*` es el schema entero del origen (0039, `include (s.*)`).
        let objetos = match expandir(&catalogo, objetos) {
            Ok(o) => o,
            Err(r) => return r,
        };

        // La lista, FUERA del arbol: es la entrada de una peticion, no un
        // documento del repositorio. `discover` la copia a `discover.scope.json`,
        // que si es del repositorio y si va en el commit.
        let lista = temporal("alcance", "txt");
        if std::fs::write(&lista, objetos.join("\n")).is_err() {
            return Respuesta::error(500, "no se pudo escribir la lista de objetos");
        }
        let mut args = vec![
            "discover".into(),
            "--from".into(),
            catalogo.to_string_lossy().into_owned(),
            "--out".into(),
            raiz.join("packages")
                .join(&nombre)
                .to_string_lossy()
                .into_owned(),
            "--name".into(),
            nombre.clone(),
            "--only-file".into(),
            lista.to_string_lossy().into_owned(),
            "--type".into(),
            tipo.into(),
            // ⭐ El catalogo no modela (0027 P1 C1): una base nace con sus
            //   tablas y vistas, sin entidades. Modelar es otro acto.
            "--no-model".into(),
        ];
        // ⭐⭐ EL DUEÑO ES LA ORGANIZACIÓN. `owner` en OOS es «quién responde»
        //   como handle de forja (`team:x`, contra CODEOWNERS), y la CLI no lo
        //   deriva porque no sabe quién la ejecuta. Este servidor SÍ sabe de
        //   quién es el árbol (0022: el inquilino es el repositorio de la
        //   organización; `--organizacion`), así que contesta `dueno` con
        //   `team:<organización>` y la base nace compilando —y, si es estándar,
        //   con la copia encolada—. Quién PULSÓ ya va en el commit (`sub`+`act`);
        //   quién RESPONDE es la organización. Preguntárselo a la persona era
        //   pedirle que inventase una cadena que no resuelve contra nada.
        //   Transferir la propiedad a un equipo, cuando IAM los tenga, será otro
        //   acto (contestar `dueno` de nuevo).
        let dueno = self.dueno_del_arbol();
        if let Some(o) = &dueno {
            args.push("--owner".into());
            args.push(o.clone());
        }
        let salida = mando::correr(&self.binario, raiz, &args);
        let _ = std::fs::remove_file(&lista);

        match salida {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                422,
                format!(
                    "`ore discover` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(s) => {
                let dir = raiz.join("packages").join(&nombre);
                let mut campos = vec![
                    ("name", Json::s(&nombre)),
                    ("source", Json::s(&fuente)),
                    ("type", Json::s(tipo)),
                    ("informe", Json::s(s.stdout.trim())),
                    ("quedan", Json::Int(pendientes(&dir) as i64)),
                ];
                match &dueno {
                    Some(o) => campos.push(("owner", Json::s(o))),
                    None => campos.push((
                        "owner",
                        Json::s("cambiame: este servidor no tiene organización (o su nombre no es un handle); contesta `dueno`"),
                    )),
                }
                campos.extend(self.tras_inducir(raiz, &nombre, sujeto));
                Respuesta::ok(Json::obj(campos))
            }
        }
    }

    /// **Una standard database vacía** (0039): `ore package new`, con el dueño
    /// de las bases —la organización— o, si no la hay, quien la crea.
    fn base_vacia(
        &self,
        raiz: &Path,
        nombre: &str,
        tipo: Option<&str>,
        sujeto: &Identidad,
    ) -> Respuesta {
        if tipo == Some("foreign") {
            return Respuesta::error(
                422,
                "una foreign database se lee en su origen: falta `source` (y `only`)",
            );
        }
        if raiz.join("packages").join(nombre).exists() {
            return Respuesta::error(409, format!("ya hay un paquete `{nombre}`"));
        }
        let dueno = self.dueno_del_arbol().or_else(|| {
            let h = sujeto
                .persona
                .split_once(':')
                .map_or(sujeto.persona.as_str(), |(_, h)| h);
            let o = format!("user:{h}");
            ore_core::pertenencia::es_handle(&o).then_some(o)
        });
        let mut args: Vec<String> = vec![
            "package".into(),
            "new".into(),
            nombre.into(),
            "--path".into(),
            raiz.to_string_lossy().into_owned(),
        ];
        if let Some(o) = &dueno {
            args.extend(["--owner".into(), o.clone()]);
        }
        match mando::correr(&self.binario, raiz, &args) {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                if s.codigo == 65 { 409 } else { 422 },
                format!(
                    "`ore package new` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(_) => Respuesta::ok(Json::obj([
                ("name", Json::s(nombre)),
                ("type", Json::s("standard")),
                (
                    "owner",
                    dueno.map(Json::s).unwrap_or_else(|| Json::s("cambiame")),
                ),
            ])),
        }
    }

    /// `team:<organización>`, si este servidor sabe de quién es el árbol y el
    /// nombre puede ser un handle. Si no, nadie: el paquete nace con `cambiame`
    /// y la decisión `dueno` en la cola, como siempre.
    pub(crate) fn dueno_del_arbol(&self) -> Option<String> {
        let o = format!("team:{}", self.organizacion.as_deref()?);
        ore_core::pertenencia::es_handle(&o).then_some(o)
    }

    fn responder(&self, raiz: &Path, paquete: &str, cuerpo: &str, sujeto: &Identidad) -> Respuesta {
        let dir = match paquete_de(raiz, paquete) {
            Ok(d) => d,
            Err(r) => return r,
        };
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let Some((_, respuestas)) = cuerpo.get("answers") else {
            return Respuesta::error(422, "falta `answers`");
        };
        if respuestas.entries().is_empty() {
            return Respuesta::error(422, "`answers` está vacío: no hay nada que cerrar");
        }

        // Se escribe como JSON y `ore review` lo lee como YAML. No es un truco:
        // JSON es un subconjunto de YAML, y es la misma economía por la que este
        // árbol no lleva analizador de JSON (ADR 0002).
        //
        // Y va FUERA del árbol a propósito: es la entrada de una petición, no un
        // documento del repositorio. Dentro, acabaría en un commit.
        let fichero = temporal("respuestas", "json");
        let texto = Json::obj([("answers", de_node(respuestas))]).jcs();
        if std::fs::write(&fichero, texto).is_err() {
            return Respuesta::error(500, "no se pudo escribir el fichero de respuestas");
        }

        let args = vec![
            "review".into(),
            dir.to_string_lossy().into_owned(),
            "--answers".into(),
            fichero.to_string_lossy().into_owned(),
        ];
        let salida = mando::correr(&self.binario, raiz, &args);
        let _ = std::fs::remove_file(&fichero);

        match salida {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                422,
                format!(
                    "`ore review` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(s) => {
                // ⭐ Contestar `clave` en una base estandar trae la copia de esa
                //   tabla (el inductor la aplica): el conducto y el Job, aqui.
                let mut campos = vec![
                    ("informe", Json::s(s.stdout.trim())),
                    ("quedan", Json::Int(pendientes(&dir) as i64)),
                ];
                campos.extend(self.tras_inducir(raiz, paquete, sujeto));
                Respuesta::ok(Json::obj(campos))
            }
        }
    }
}

// ── Lo que se lee del árbol ─────────────────────────────────────────────────

const COLA: &str = "discover.pending.json";

/// Vista → objeto físico, resolviendo los dos saltos de una vez.
///
/// Devuelve un mapa de **nombre de vista** a `spec.object` de su tabla. Lo que
/// no resuelve —una vista sin `from.table`, una tabla sin `object`— no entra
/// en el mapa, y quien lo consulte omite el campo.
/// **Las tablas de una base, tal como el catalogo las tiene**: por cada
/// `Table`, su objeto fisico, su fuente, sus columnas con el `physicalType` que
/// el origen dijo, la vista trivial que la expone, y si esta modelada (tiene
/// `Entity`, y entonces cual). Ordenadas por objeto.
///
/// ⭐ Y el paquete de UNA FUENTE no tiene `tables/` (0027, «el catalogo de la
///   conexion»): el Job deja solo `discover.catalog.json` y el manifiesto, y
///   lo gobernado nace al crear una database. Su esquema —lo que la ficha de
///   la conexion y el modal de nueva database ensenan— se lee del catalogo,
///   con la misma forma: nada modelado, nada copiado, sin vista.
#[cfg(test)]
fn tablas_del_paquete(raiz: &Path, pkg: &Package, dir: &Path) -> Vec<Json> {
    tablas_con(&crate::punteros::Indice::nuevo(raiz, pkg), dir)
}

/// [`tablas_del_paquete`] con el índice de la petición (0046 E5b): quien
/// pinta varios paquetes —`GET /paquetes`— lo calcula una vez.
fn tablas_con(idx: &crate::punteros::Indice, dir: &Path) -> Vec<Json> {
    let este = dir
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or_default()
        .to_string();
    // ⭐ 0045 P2: la FUENTE enseña siempre su catálogo entero —lo que la ficha
    //   de la conexión y el modal de nueva database ofrecen—, anotado con su
    //   puntero y con quién lo usa. Si enseñara sus Tables, dejaría de ofrecer
    //   los objetos que ninguna database ha usado todavía.
    if idx.es_fuente(dir) {
        return tablas_de_la_fuente(idx, dir, &este);
    }
    let pkg = idx.arbol();
    let suyo = |d: &&Loaded| idx.paquete(d) == Some(este.as_str());
    let en = |d: &Node, padre: &str, k: &str| -> Option<String> {
        d.get(padre)
            .and_then(|(_, m)| m.get(k))
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    };
    // Lo que lee una tabla: una vista (la pregunta) o un dataset mantenido (la
    // copia, 0033), `(nombre, es dataset)`, por la ruta de la tabla.
    //
    // ⭐ 0045 P2: la tabla se RESUELVE por el árbol —`from.table`, o la única
    //   cosa que lee una vista SQL (0040)—, esté en este paquete o en el de su
    //   fuente. Antes se buscaba por el último trozo del nombre en la carpeta
    //   hermana, y con el puntero en la fuente no se encontraba nada.
    let mut lector: std::collections::BTreeMap<PathBuf, (String, bool)> = Default::default();
    for d in pkg
        .docs
        .iter()
        .filter(suyo)
        .filter(|d| matches!(d.kind, Kind::View | Kind::Dataset))
    {
        let (Some(t), Some(n)) = (idx.tabla_que_lee(d), en(&d.root, "metadata", "name")) else {
            continue;
        };
        let es_dataset = d.kind == Kind::Dataset;
        // Un dataset gana a una vista sobre la misma tabla: es lo que se
        // tiene, y es lo que el catálogo enseña.
        if es_dataset || !lector.contains_key(&t.path) {
            lector.insert(t.path.clone(), (n, es_dataset));
        }
    }
    let mut entidad_de_vista: std::collections::BTreeMap<String, String> = Default::default();
    for e in pkg
        .docs
        .iter()
        .filter(suyo)
        .filter(|d| d.kind == Kind::Entity)
    {
        if let (Some(n), Some(v)) = (
            en(&e.root, "metadata", "name"),
            en(&e.root, "spec", "backedBy"),
        ) {
            entidad_de_vista.insert(v, n);
        }
    }
    // Las tablas: las suyas, y las que leen las suyas estén donde estén.
    let mut tablas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(suyo)
        .filter(|d| d.kind == Kind::Table)
        .collect();
    let ya: std::collections::HashSet<&Path> = tablas.iter().map(|t| t.path.as_path()).collect();
    for p in lector.keys() {
        if !ya.contains(p.as_path())
            && let Some(t) = idx.por_ruta(p)
        {
            tablas.push(t);
        }
    }
    tablas.sort_by(|a, b| a.path.cmp(&b.path));
    // Sin tablas ni nadie que las lea: lo que el catálogo guardado dice.
    // ⚠️ De una base, sólo lo que eligió: una base de solo colecciones
    //   (v1alpha16) no tiene tablas, y sin esto enseñaba el bucket entero.
    if tablas.is_empty() {
        let mut del_catalogo = idx
            .leer(&dir.join("discover.catalog.json"))
            .map(|c| filas_del_catalogo(&c))
            .unwrap_or_default();
        if let Some(elegidos) = elegidos_de(dir) {
            del_catalogo.retain(|f| {
                matches!(f, Json::Obj(m) if matches!(m.get("object"), Some(Json::Str(o)) if elegidos.contains(o)))
            });
        }
        if !del_catalogo.is_empty() {
            return del_catalogo;
        }
    }
    let mut ubicaciones: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, String>,
    > = Default::default();
    let mut salida: Vec<(String, Json)> = Vec::new();
    for tabla in tablas {
        let t = &tabla.root;
        let Some(nombre) = en(t, "metadata", "name") else {
            continue;
        };
        let objeto = en(t, "spec", "object").unwrap_or_default();
        // v1alpha16: de un bucket, `object` es dónde está; la fila dice el
        // nombre del catálogo, que es lo que se elige y se copia, y `location`.
        let fuente_de = en(t, "spec", "datasource").unwrap_or_default();
        let (objeto, ubicacion) = match ubicaciones
            .entry(fuente_de.clone())
            .or_insert_with(|| nombres_por_ubicacion(idx, &fuente_de))
            .get(&objeto)
        {
            Some(n) => (n.clone(), Some(objeto)),
            None => (objeto, None),
        };
        let mut columnas = Vec::new();
        if let Some((_, spec)) = t.get("spec")
            && let Some((_, c)) = spec.get("columns")
            && let Node::Mapping { entries, .. } = c
        {
            for (k, v) in entries {
                let Some(k) = k.as_str() else { continue };
                let mut campos = vec![("name", Json::s(k))];
                if let Some(pt) = v.get("physicalType").and_then(|(_, p)| p.as_str()) {
                    campos.push(("physicalType", Json::s(pt)));
                }
                if let Some(ty) = v.get("type").and_then(|(_, t)| t.as_str()) {
                    campos.push(("type", Json::s(ty)));
                }
                columnas.push(Json::obj(campos));
            }
        }
        let (vista, copiada) = match lector.get(&tabla.path) {
            Some((v, c)) => (Some(v.clone()), *c),
            None => (None, false),
        };
        let entidad = vista
            .as_ref()
            .and_then(|v| entidad_de_vista.get(v).cloned());
        let mut campos = vec![
            ("name", Json::s(&nombre)),
            // 0038: su schema (`discover` lleva el del origen)
            (
                "schema",
                Json::s(
                    en(t, "metadata", "schema")
                        .unwrap_or_else(|| ore_core::normalize::SCHEMA_POR_DEFECTO.to_string()),
                ),
            ),
            ("object", Json::s(&objeto)),
            (
                "datasource",
                Json::s(en(t, "spec", "datasource").unwrap_or_default()),
            ),
            ("columns", Json::Arr(columnas)),
            ("modeled", Json::Bool(entidad.is_some())),
            // ⭐ Si tiene dataset (0033): por la clase de la base o una a una.
            ("copied", Json::Bool(copiada)),
        ];
        if let Some(v) = &vista {
            // `view` sigue siendo el nombre de lo que la lee, para quien ya lo
            // leía; `dataset` dice que es una copia con su documento.
            campos.push(("view", Json::s(v)));
            if copiada {
                campos.push(("dataset", Json::s(v)));
            }
        }
        if let Some(e) = &entidad {
            campos.push(("entity", Json::s(e)));
        }
        if let Some(l) = ubicacion {
            campos.push(("location", Json::s(l)));
        }
        salida.push((objeto, Json::obj(campos)));
    }
    salida.sort_by(|a, b| a.0.cmp(&b.0));
    salida.into_iter().map(|(_, j)| j).collect()
}

/// ⭐ 0045 P2 · **El esquema de una fuente**: su catálogo entero, y en cada
/// objeto su puntero (`table`, si ya está en la fuente) y quién lo usa
/// (`usedBy`: las databases que lo leen, en cualquiera de las dos
/// disposiciones). Las dos claves se ordenan detrás de `object`, y solo están
/// si dicen algo: la forma de antes no cambia.
fn tablas_de_la_fuente(idx: &crate::punteros::Indice, dir: &Path, este: &str) -> Vec<Json> {
    let mut filas = idx
        .leer(&dir.join("discover.catalog.json"))
        .map(|c| filas_del_catalogo(&c))
        .unwrap_or_default();
    for f in filas.iter_mut() {
        let Json::Obj(m) = f else { continue };
        // v1alpha16: la `Table` de un bucket lleva en `object` dónde está
        // (`location`), no el nombre del catálogo.
        let Some(Json::Str(o)) = m.get("location").or_else(|| m.get("object")).cloned() else {
            continue;
        };
        let mut usan: std::collections::BTreeSet<String> = Default::default();
        let puntero = if m.get("kind") == Some(&Json::s("objects")) {
            let patron = match m.get("match") {
                Some(Json::Str(p)) => Some(p.clone()),
                _ => None,
            };
            let Some(t) = idx.objetos(este, &o, patron.as_deref()) else {
                continue;
            };
            // Un contenedor no tiene colección que lo lea: lo usa la base que
            // lo eligió.
            if let Some(Json::Str(nombre)) = m.get("object") {
                usan.extend(idx.eligen(este, nombre));
            }
            Some(t)
        } else {
            // La disposición de antes de 0045: una base con su propia `Table`.
            usan.extend(idx.con_tabla(este, &o).map(String::from));
            idx.tabla(este, &o)
        };
        if let Some(t) = puntero {
            usan.extend(idx.lectores(t).map(String::from));
            m.insert("table".into(), Json::s(t.qname().unwrap_or_default()));
        }
        usan.remove(este);
        if !usan.is_empty() {
            m.insert(
                "usedBy".into(),
                Json::Arr(usan.into_iter().map(Json::s).collect()),
            );
        }
    }
    filas
}

/// Lo que una base eligió (`only` de `discover.scope.json`), si es una base.
fn elegidos_de(dir: &Path) -> Option<std::collections::BTreeSet<String>> {
    let t = std::fs::read_to_string(dir.join("discover.scope.json")).ok()?;
    let n = parse::parse(&t).ok()?;
    Some(
        n.get("only")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
    )
}

/// v1alpha16 · de dónde está (`tables[].object`) al nombre del catálogo, en el
/// catálogo del paquete de una fuente. Vacío si la fuente no tiene paquete o
/// sus tablas no dicen dónde están (todas menos las de un bucket).
fn nombres_por_ubicacion(
    idx: &crate::punteros::Indice,
    fuente: &str,
) -> std::collections::BTreeMap<String, String> {
    let Some(cat) = idx.leer(
        &idx.raiz()
            .join("packages")
            .join(fuente)
            .join("discover.catalog.json"),
    ) else {
        return Default::default();
    };
    cat.get("tables")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| {
            Some((
                t.get("object")?.1.as_str()?.to_string(),
                t.get("name")?.1.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// El esquema de una fuente, desde `discover.catalog.json`: `name` es el objeto
/// fisico (`public.pedidos`), y `physicalType` es el `sourceType` que el
/// origen dijo —si no lo dijo, el tipo de OOS que el lector dedujo va en `type`
/// y `physicalType` se omite, como en una `Table`—.
fn filas_del_catalogo(cat: &Node) -> Vec<Json> {
    let fuente = cat
        .get("source")
        .and_then(|(_, v)| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut salida: Vec<(String, Json)> = Vec::new();
    for t in cat.get("tables").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let Some(objeto) = t.get("name").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        let mut columnas = Vec::new();
        for c in t.get("columns").map(|(_, v)| v.items()).unwrap_or(&[]) {
            let Some(n) = c.get("name").and_then(|(_, v)| v.as_str()) else {
                continue;
            };
            let mut campos = vec![("name", Json::s(n))];
            if let Some(pt) = c.get("sourceType").and_then(|(_, v)| v.as_str()) {
                campos.push(("physicalType", Json::s(pt)));
            }
            if let Some(ty) = c.get("type").and_then(|(_, v)| v.as_str()) {
                campos.push(("type", Json::s(ty)));
            }
            columnas.push(Json::obj(campos));
        }
        let mut campos = vec![
            ("name", Json::s(objeto)),
            ("object", Json::s(objeto)),
            ("datasource", Json::s(&fuente)),
            ("columns", Json::Arr(columnas)),
            ("modeled", Json::Bool(false)),
            ("copied", Json::Bool(false)),
        ];
        // v1alpha16 (0046 E5): de un bucket, `object` sigue siendo el nombre
        // del catálogo —lo que se elige en `only`, `<schema>.<nombre>`— y
        // `location` dice dónde está: la clave o el prefijo de sus ficheros.
        if let Some(l) = t.get("object").and_then(|(_, v)| v.as_str()) {
            campos.push(("location", Json::s(l)));
        }
        salida.push((objeto.to_string(), Json::obj(campos)));
    }
    // v1alpha16: los conjuntos de objetos, que se eligen igual. Sus columnas
    // son las fijas del listado (`01-object-table` §1).
    for o in cat.get("objects").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let Some(nombre) = o.get("name").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        let texto = |k: &str| o.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let entero = |k: &str| Json::Int(texto(k).and_then(|n| n.parse().ok()).unwrap_or(0));
        let columnas = ore_core::document::COLUMNAS_DE_OBJETO
            .iter()
            .map(|(n, t)| Json::obj([("name", Json::s(*n)), ("type", Json::s(*t))]))
            .collect();
        let mut campos = vec![
            ("name", Json::s(nombre)),
            ("object", Json::s(nombre)),
            ("kind", Json::s("objects")),
            ("datasource", Json::s(&fuente)),
            ("columns", Json::Arr(columnas)),
            ("modeled", Json::Bool(false)),
            ("copied", Json::Bool(false)),
            ("media", Json::s(texto("media").unwrap_or_default())),
            ("location", Json::s(texto("prefix").unwrap_or_default())),
            ("count", entero("count")),
            ("bytes", entero("bytes")),
        ];
        if let Some(m) = texto("match") {
            campos.push(("match", Json::s(m)));
        }
        salida.push((nombre.to_string(), Json::obj(campos)));
    }
    salida.sort_by(|a, b| a.0.cmp(&b.0));
    salida.into_iter().map(|(_, j)| j).collect()
}

/// Cuantas tablas tiene un paquete y cuantas estan modeladas (tienen `Entity`).
fn tablas_y_modeladas(idx: &crate::punteros::Indice, dir: &Path) -> (usize, usize) {
    let t = tablas_con(idx, dir);
    let m = t
        .iter()
        .filter(|j| matches!(j, Json::Obj(o) if o.get("modeled") == Some(&Json::Bool(true))))
        .count();
    (t.len(), m)
}

/// **Los `.yaml` de la carpeta de un kind en un paquete** (`tables`,
/// `datasets`…): la de la raíz y la de cada schema declarado (0038 P5:
/// `discover` deja lo del origen en `<schema>/tables/…`), ordenados.
pub(crate) fn yamls_del_kind(paquete: &Path, carpeta: &str) -> Vec<PathBuf> {
    let mut dirs = vec![paquete.join(carpeta)];
    if let Ok(es) = std::fs::read_dir(paquete) {
        let mut schemas: Vec<PathBuf> = es
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("schema.yaml").is_file())
            .collect();
        schemas.sort();
        dirs.extend(schemas.into_iter().map(|s| s.join(carpeta)));
    }
    let mut out: Vec<PathBuf> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|es| es.flatten().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("yaml"))
        .collect();
    out.sort();
    out
}

/// Vista (o dataset) → el objeto físico de la tabla que lee, esté donde esté la
/// tabla (0045 P2: resuelta por el árbol, no por la carpeta hermana).
fn objetos_fisicos(
    idx: &crate::punteros::Indice,
    paquete: &Path,
) -> std::collections::BTreeMap<String, String> {
    let pkg = idx.arbol();
    let este = paquete
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    let mut salida: std::collections::BTreeMap<String, String> = Default::default();
    let mut ubicaciones: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, String>,
    > = Default::default();
    for d in pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::View | Kind::Dataset) && idx.paquete(d) == Some(este))
    {
        let tabla = idx.tabla_que_lee(d);
        let (Some(nombre), Some(o)) = (
            d.meta("name").and_then(|v| v.as_str()),
            tabla
                .and_then(|t| t.section("object"))
                .and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        // v1alpha16: de un bucket, el nombre del catálogo, no dónde está.
        let fuente = tabla
            .and_then(|t| t.section("datasource"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let o = ubicaciones
            .entry(fuente.clone())
            .or_insert_with(|| nombres_por_ubicacion(idx, &fuente))
            .get(o)
            .cloned()
            .unwrap_or_else(|| o.to_string());
        // Una vista gana a un dataset con el mismo nombre (lo que `backedBy`
        // nombraba antes de 0033).
        if d.kind == Kind::View || !salida.contains_key(nombre) {
            salida.insert(nombre.to_string(), o);
        }
    }
    salida
}

/// Lo que el manifiesto declara. **Nunca un secreto**: el manifiesto no tiene
/// ninguno —`connectionEnv` dice dónde buscarlo, no qué es— y esto no mira el
/// entorno para completarlo.
fn fuentes(raiz: &Path) -> Respuesta {
    let manifiesto = raiz.join("ontology.config.yaml");
    let texto = match std::fs::read_to_string(&manifiesto) {
        Ok(t) => t,
        Err(_) => return Respuesta::error(404, "este directorio no es un repositorio ontológico"),
    };
    let arbol = match parse::parse(&texto) {
        Ok(n) => n,
        Err(e) => return Respuesta::error(500, format!("el manifiesto no analiza: {e:?}")),
    };
    let lista = match arbol.get("datasources") {
        Some((_, n)) => n
            .items()
            .iter()
            .map(|f| {
                let campo = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
                Json::obj([
                    ("name", Json::s(campo("name"))),
                    ("type", Json::s(campo("type"))),
                    ("connectionEnv", Json::s(campo("connectionEnv"))),
                    ("description", Json::s(campo("description"))),
                ])
            })
            .collect(),
        None => Vec::new(),
    };
    Respuesta::ok(Json::obj([("datasources", Json::Arr(lista))]))
}

fn paquetes(raiz: &Path) -> Respuesta {
    let dir = raiz.join("packages");
    let Ok(entradas) = std::fs::read_dir(&dir) else {
        return Respuesta::ok(Json::obj([("packages", Json::Arr(Vec::new()))]));
    };
    // ⭐ 0045 P2: el árbol, una vez para todos: la tabla de una database puede
    //   vivir en el paquete de su fuente.
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    // ⭐ 0046 E5b: y el índice, una vez para todos los paquetes.
    let idx = crate::punteros::Indice::nuevo(raiz, &pkg);
    let mut lista: Vec<(String, Json)> = Vec::new();
    for e in entradas.flatten() {
        let manifiesto = e.path().join("package.yaml");
        let Ok(texto) = std::fs::read_to_string(&manifiesto) else {
            continue;
        };
        let Ok(arbol) = parse::parse(&texto) else {
            continue;
        };
        let campo = |k: &str| {
            arbol
                .get("metadata")
                .and_then(|(_, m)| m.get(k))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let nombre = e.file_name().to_string_lossy().into_owned();
        let abiertas = pendientes(&e.path());
        // ⭐ De que fuente sale, y si fue ELEGIDO. Un paquete con
        //   `discover.scope.json` es una base que alguien creo marcando que
        //   entra; sin el, es la fuente entera tal como el Job la leyo. La
        //   consola pinta lo primero bajo su conexion y lo segundo como el
        //   esquema descubierto de esta — y sin este campo no podria
        //   distinguirlos.
        let (fuente, elegido) = idx.origen(&e.path());
        // ⭐ Y su CLASE (0027 P1 I4a): `standard` copia todo lo que entra a la
        //   celda; `foreign` es un espejo, cada lectura va al origen. Con las
        //   copias que declara y las que están hechas, para que la ficha diga
        //   «8/8 copied» — o vea la deriva.
        let mut campos = vec![
            ("name", Json::s(nombre.clone())),
            ("version", Json::s(campo("version"))),
            ("decisionesPendientes", Json::Int(abiertas as i64)),
            ("scoped", Json::Bool(elegido)),
            ("type", Json::s(crate::copia::clase_de(&e.path()))),
            ("copias", crate::copia::copias_de(raiz, &nombre)),
        ];
        // ⭐ Y cuantas tablas tiene y cuantas estan modeladas (0027 P1 C1): el
        //   catalogo no modela, asi que una base recien nacida es N/0.
        let (tablas, modeladas) = tablas_y_modeladas(&idx, &e.path());
        campos.push(("tablas", Json::Int(tablas as i64)));
        campos.push(("modeladas", Json::Int(modeladas as i64)));
        if let Some(f) = fuente {
            campos.push(("source", Json::s(f)));
        }
        lista.push((nombre, Json::obj(campos)));
    }
    lista.sort_by(|a, b| a.0.cmp(&b.0));
    Respuesta::ok(Json::obj([(
        "packages",
        Json::Arr(lista.into_iter().map(|(_, j)| j).collect()),
    )]))
}

/// ⭐⭐ LO QUE `ore discover` ENCONTRO DE VERDAD.
///
/// Un paquete guarda una entidad por fichero en `entities/`, y cada una es un
/// documento OOS: `metadata.name`, `metadata.namespace`, `spec.properties`.
/// Esto las lee y las devuelve tal cual — sin resumir, sin ordenar por nada que
/// no sea el nombre, y sin inventar lo que el fichero no dice.
///
/// ⛔ Y NO se rellena lo que no consta. Una ficha de catalogo suele querer
///   «filas estimadas», y aqui no hay: el inductor lee el ESQUEMA, no cuenta
///   filas. Devolver un cero seria afirmar que la tabla esta vacia, y devolver
///   un numero inventado es peor. Lo que falta se omite, y quien pinte decide
///   como se dice «no lo se».
///
/// ⚠️ Un fichero que no analiza se SALTA y se cuenta. Un paquete a medias tiene
///   que poder verse a medias — negarse entero porque una entidad esta rota
///   esconde las diecinueve que estan bien.
fn esquema(raiz: &Path, paquete: &str) -> Respuesta {
    let dir = match paquete_de(raiz, paquete) {
        Ok(d) => d,
        Err(r) => return r,
    };
    // ⭐⭐ EL ESQUEMA FISICO, desde `tables/` (0027 P1 C2). El catalogo no
    //   modela: una base recien nacida no tiene `entities/`, y este esquema es
    //   el del catalogo de activos —todas las columnas, con el tipo del
    //   origen— no el del modelo. `entities` sigue debajo hasta que la
    //   consola lea `tables` (C3).
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let idx = crate::punteros::Indice::nuevo(raiz, &pkg);
    let tablas = tablas_con(&idx, &dir);
    let entradas = yamls_del_kind(&dir, "entities");
    if entradas.is_empty() {
        return Respuesta::ok(Json::obj([
            ("tables", Json::Arr(tablas)),
            ("entities", Json::Arr(Vec::new())),
        ]));
    }

    // ⭐⭐ EL NOMBRE FISICO, que es lo que hace falta para ELEGIR. Una entidad
    //   se llama `Pedidos`; lo que `discover --only` entiende es `public.pedidos`,
    //   y entre los dos hay dos saltos: la entidad dice `backedBy: pedidos`, la
    //   vista `pedidos` dice `from: { table: public_pedidos }`, y la tabla
    //   `public_pedidos` dice `object: public.pedidos`. Se resuelven aqui,
    //   una vez, en vez de en cada consola.
    let objeto_de = objetos_fisicos(&idx, &dir);

    let mut rotos = 0usize;
    let mut lista: Vec<(String, Json)> = Vec::new();
    for camino in entradas {
        let Ok(texto) = std::fs::read_to_string(&camino) else {
            rotos += 1;
            continue;
        };
        let Ok(doc) = parse::parse(&texto) else {
            rotos += 1;
            continue;
        };
        let en = |padre: &str, k: &str| {
            doc.get(padre)
                .and_then(|(_, m)| m.get(k))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let nombre = en("metadata", "name");
        if nombre.is_empty() {
            rotos += 1;
            continue;
        }

        // Las propiedades: `nombre: { type: … }` o `nombre: { type: …, labels: … }`.
        let mut props: Vec<Json> = Vec::new();
        if let Some((_, spec)) = doc.get("spec")
            && let Some((_, p)) = spec.get("properties")
            && let Node::Mapping { entries, .. } = p
        {
            for (k, v) in entries {
                let Some(k) = k.as_str() else { continue };
                let tipo = v
                    .get("type")
                    .and_then(|(_, t)| t.as_str())
                    .unwrap_or_default();
                props.push(Json::obj([("name", Json::s(k)), ("type", Json::s(tipo))]));
            }
        }

        // ⚠️ La clave primaria es una LISTA, y se devuelve como tal: decir solo
        //   la primera columna de una clave compuesta seria una media verdad
        //   que ademas parece entera.
        let mut clave: Vec<Json> = Vec::new();
        if let Some((_, spec)) = doc.get("spec")
            && let Some((_, pk)) = spec.get("primaryKey")
            && let Node::Sequence { items, .. } = pk
        {
            for it in items {
                if let Some(c) = it.as_str() {
                    clave.push(Json::s(c));
                }
            }
        }

        let respaldo = doc
            .get("spec")
            .and_then(|(_, sp)| sp.get("backedBy"))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or_default()
            .to_string();
        let schema = match en("metadata", "schema") {
            s if s.is_empty() => ore_core::normalize::SCHEMA_POR_DEFECTO.to_string(),
            s => s,
        };
        let mut campos = vec![
            ("name", Json::s(&nombre)),
            ("namespace", Json::s(en("metadata", "namespace"))),
            ("schema", Json::s(&schema)),
            ("backedBy", Json::s(&respaldo)),
            ("primaryKey", Json::Arr(clave)),
            ("properties", Json::Arr(props)),
        ];
        // Se OMITE si no se pudo resolver, no se pone vacio: una entidad sin
        // objeto fisico es legal —una vista que agrupa, por ejemplo— y una
        // cadena vacia diria que lo tiene y se llama asi.
        if let Some(o) = objeto_de.get(&respaldo) {
            campos.push(("object", Json::s(o)));
        }
        lista.push((
            format!("{}/{}", en("metadata", "namespace"), nombre),
            Json::obj(campos),
        ));
    }
    lista.sort_by(|a, b| a.0.cmp(&b.0));

    let mut salida = vec![
        ("tables", Json::Arr(tablas)),
        (
            "entities",
            Json::Arr(lista.into_iter().map(|(_, j)| j).collect()),
        ),
    ];
    if rotos > 0 {
        // ⭐ Se dice cuantas se saltaron. Una lista mas corta de lo que deberia
        //   y en silencio es la peor forma de contestar.
        salida.push(("ilegibles", Json::Int(rotos as i64)));
    }
    Respuesta::ok(Json::obj(salida))
}

/// La cola tal como el inductor la dejó: cada decisión con su `id`, su `because`
/// y sus `options`. **Es un formulario servido en JSON**, y por eso esta ruta no
/// la reescribe — reordenar u omitir aquí sería una segunda opinión sobre lo que
/// hay que preguntar.
fn decisiones(raiz: &Path, paquete: &str) -> Respuesta {
    let dir = match paquete_de(raiz, paquete) {
        Ok(d) => d,
        Err(r) => return r,
    };
    match std::fs::read_to_string(dir.join(COLA)) {
        Err(_) => Respuesta::ok(Json::obj([
            ("pending", Json::Arr(Vec::new())),
            (
                "nota",
                Json::s("no hay cola: o no se indujo, o ya se cerró entera"),
            ),
        ])),
        Ok(t) => match parse::parse(&t) {
            Ok(n) => Respuesta::ok(de_node(&n)),
            Err(e) => Respuesta::error(500, format!("la cola no analiza: {e:?}")),
        },
    }
}

/// **Cuántas decisiones quedan abiertas.**
///
/// Y se cuentan, no se mira si el fichero está: medido contra la forja, `ore
/// review` **deja la cola escrita con la lista vacía** cuando las cierra todas.
/// Un `existe(fichero)` decía «hay decisiones pendientes» para siempre, y una
/// consola que enseñe ese aviso sobre un árbol ya decidido enseña una alarma
/// que nadie puede apagar.
fn pendientes(paquete: &Path) -> usize {
    let Ok(t) = std::fs::read_to_string(paquete.join(COLA)) else {
        return 0;
    };
    let Ok(n) = parse::parse(&t) else {
        return 0;
    };
    n.get("pending").map(|(_, v)| v.items().len()).unwrap_or(0)
}

/// El directorio de un paquete, con el nombre ya comprobado.
fn paquete_de(raiz: &Path, nombre: &str) -> Result<PathBuf, Respuesta> {
    token(nombre).map_err(|m| Respuesta::error(422, format!("nombre de paquete: {m}")))?;
    let dir = raiz.join("packages").join(nombre);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(Respuesta::error(404, "no hay tal paquete"))
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

/// Un nombre que puede convertirse en un camino sin sorpresas.
///
/// El alfabeto es cerrado, así que `..`, `/`, `\` y los nombres reservados de
/// Windows no son casos que haya que acordarse de excluir: no están.
/// ⭐⭐ 0045 P1.5 · **EL NOMBRE DE UNA FUENTE**: `^[a-z][a-z0-9]*(_[a-z0-9]+)*$`,
/// hasta 30.
///
/// Es la intersección de todo lo que ese nombre llega a ser, medida:
///
/// | dónde | su regla |
/// |---|---|
/// | el secreto `fuente-<n>` del custodio | minúsculas, `[a-z0-9_-]`, `≤ 63` → `n ≤ 56` |
/// | el fichero y el Job de la cola (`cola::nombre_de_objeto`) | minúsculas, lo demás `-`, corta a 30 |
/// | el paquete de la fuente (`OOS2030`) | una letra, luego letras, dígitos y `_` |
///
/// ⛔ Antes valía `^[A-Za-z][A-Za-z0-9_]{0,127}$`, y así `Ventas` fallaba al
///   guardar la credencial (el custodio no admite mayúsculas) y `Ventas` y
///   `ventas`, `a_b` y `a__b` —o dos nombres largos con el mismo prefijo—
///   caían en el MISMO fichero de la cola: el segundo catálogo pisaba al
///   primero. Minúsculas, sin `__` ni `_` en los bordes y hasta 30 hacen
///   `nombre_de_objeto` biyectivo.
pub fn nombre_de_fuente(n: &str) -> Result<(), String> {
    let bien = n.len() <= 30
        && n.starts_with(|c: char| c.is_ascii_lowercase())
        && n.split('_').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if bien {
        Ok(())
    } else {
        Err(format!(
            "`{n}` no puede ser el nombre de una fuente: minúsculas, dígitos y `_` \
             (sin `__` ni `_` al principio o al final), empezando por letra y hasta 30"
        ))
    }
}

/// **El nombre que sale de lo que alguien escribió**: sin acentos, en
/// minúsculas, lo demás `_`, sin `_` repetidos ni en los bordes, cortado a 30
/// en un `_`. Si empieza por dígito, `f_` delante. Lo mismo hace la consola en
/// vivo, debajo del nombre de la conexión; aquí sirve para sugerirlo.
pub fn sugerir_nombre_de_fuente(humano: &str) -> String {
    let mut s = String::new();
    for c in humano.chars().flat_map(sin_acento) {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    let mut s = s.trim_matches('_').to_string();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s = format!("f_{s}");
    }
    if s.is_empty() {
        s = "fuente".into();
    }
    if s.len() > 30 {
        let corte = s[..30].rfind('_').filter(|i| *i > 0).unwrap_or(30);
        s = s[..corte].trim_end_matches('_').to_string();
    }
    s
}

/// Lo que una letra con tilde es sin ella. Lo justo para los nombres que se
/// escriben aquí; lo que no está, se va por `_`.
fn sin_acento(c: char) -> Option<char> {
    Some(match c {
        'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
        'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
        'ñ' | 'Ñ' => 'n',
        'ç' | 'Ç' => 'c',
        c => c,
    })
}

/// ¿Declara el manifiesto de `raiz` una fuente `nombre`?
fn fuente_declarada(raiz: &Path, nombre: &str) -> bool {
    std::fs::read_to_string(raiz.join("ontology.config.yaml"))
        .ok()
        .and_then(|t| parse::parse(&t).ok())
        .and_then(|n| {
            n.get("datasources").map(|(_, v)| {
                v.items()
                    .iter()
                    .any(|d| d.get("name").and_then(|(_, x)| x.as_str()) == Some(nombre))
            })
        })
        .unwrap_or(false)
}

/// Un nombre libre a partir de `n`: él, o `n_2`, `n_3`… — ni declarado ni con
/// paquete. Para decirlo en el 409 en vez de dejar al que llama adivinando.
fn nombre_libre(raiz: &Path, n: &str) -> String {
    let ocupado = |x: &str| fuente_declarada(raiz, x) || raiz.join("packages").join(x).exists();
    if !ocupado(n) {
        return n.to_string();
    }
    (2..)
        .map(|i| {
            let sufijo = format!("_{i}");
            let base = &n[..n.len().min(30 - sufijo.len())];
            format!("{}{sufijo}", base.trim_end_matches('_'))
        })
        .find(|x| !ocupado(x))
        .unwrap_or_else(|| n.to_string())
}

/// Un error con el nombre que sí valdría al lado: la consola lo ofrece.
fn con_sugerencia(codigo: u16, motivo: String, sugerencia: String) -> Respuesta {
    Respuesta {
        codigo,
        cuerpo: Json::obj([
            ("error", Json::s(motivo)),
            ("sugerencia", Json::s(sugerencia)),
        ]),
    }
}

pub fn token(v: &str) -> Result<(), String> {
    if v.is_empty() || v.len() > 64 {
        return Err("tiene que medir entre 1 y 64 caracteres".into());
    }
    if !v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("sólo letras ASCII, dígitos, `_` y `-`".into());
    }
    Ok(())
}

/// Una URL que **no** trae la credencial dentro.
///
/// ⚠️ Desde el 2026-09-10 esto ya no DECIDE: DETECTA. Quien decide es el alta,
/// mirando si hay custodio donde poner lo que se detecte. La distinción importa
/// porque el mensaje de abajo se sigue leyendo como una negativa y ya no lo es
/// por sí solo.
///
/// Lo que se busca es la autoridad con `usuario:clave@`, y de paso las dos
/// formas de meterla en la consulta. No pretende ser exhaustivo: pretende que
/// el caso normal no pase inadvertido, y el caso raro que pase no crea un
/// secreto donde no lo había — `ore source add` lo sacaría del manifiesto igual.
pub fn sin_credencial(url: &str) -> Result<(), String> {
    let motivo = "esta URL trae una credencial dentro.\n\
        El plano de control no tiene hoy dónde guardarla: `ore source add` la \
        mandaría a `.env.local`, y un fichero en el disco de un pod no es un \
        secreto guardado.\n\
        Las fuentes cuya credencial la presta la nube —BigQuery por Workload \
        Identity, por ejemplo— se dan de alta sin ella.";
    if let Some(resto) = url.split_once("://").map(|(_, r)| r) {
        let autoridad = resto.split(['/', '?', '#']).next().unwrap_or("");
        if autoridad.contains('@') {
            return Err(motivo.into());
        }
    }
    // ⭐ 0046 F0 · Un parámetro de la consulta cuyo NOMBRE parece una
    //   credencial, con la regla con la que `ore source add` la tapa al
    //   informar (`fuente.rs::sensible`), más `sig` (la SAS de Azure). Antes se
    //   buscaba `secret=` y `token=` a la letra: `secret_access_key=` de una URL
    //   de S3 no lo es, y la guarda que exige el custodio no saltaba.
    let consulta = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let consulta = consulta.split('#').next().unwrap_or("");
    for p in consulta.split('&') {
        if let Some((k, v)) = p.split_once('=')
            && !v.is_empty()
            && parametro_de_credencial(k)
        {
            return Err(motivo.into());
        }
    }
    Ok(())
}

/// El nombre de un parámetro que lleva una credencial.
fn parametro_de_credencial(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    ["pass", "pwd", "secret", "token", "key", "credential"]
        .iter()
        .any(|s| k.contains(s))
        || k == "sig"
}

// ── Utilidades ──────────────────────────────────────────────────────────────

pub(crate) fn analizar(cuerpo: &str) -> Result<Node, Respuesta> {
    if cuerpo.trim().is_empty() {
        return Err(Respuesta::error(400, "el cuerpo está vacío"));
    }
    parse::parse(cuerpo).map_err(|e| Respuesta::error(400, format!("el cuerpo no analiza: {e:?}")))
}

/// De lo analizado a la forma canónica.
///
/// El estilo del escalar decide el tipo, igual que en `ore dev`: un `1` sin
/// comillas vuelve como número y un `"1"` como cadena. Es lo que hace que la
/// cola que sale por aquí sea **el mismo JSON** que el inductor escribió.
pub(crate) fn de_node(n: &Node) -> Json {
    match n {
        Node::Mapping { entries, .. } => Json::Obj(
            entries
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), de_node(v))))
                .collect(),
        ),
        Node::Sequence { items, .. } => Json::Arr(items.iter().map(de_node).collect()),
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => match raw.as_str() {
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            _ => raw.parse::<i64>().map(Json::Int).unwrap_or(Json::s(raw)),
        },
        Node::Scalar { raw, .. } => Json::s(raw),
    }
}

pub(crate) fn primera_linea(stdout: &str, stderr: &str) -> String {
    let fuente = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    fuente
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

static CUENTA: AtomicU64 = AtomicU64::new(0);

fn temporal(nombre: &str, extension: &str) -> PathBuf {
    let n = CUENTA.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ore-serve-{}-{}-{}.{}",
        std::process::id(),
        n,
        nombre,
        extension
    ))
}

/// Lo que hay montado, para poder decirlo al arrancar. Las de `/documentos`
/// salen de `documentos::KINDS`: una por kind servido.
pub fn mapa(con_identidad: bool) -> Vec<(&'static str, String, bool)> {
    let mut m: Vec<(&'static str, String, bool)> = [
        ("GET", "/salud", true),
        ("GET", "/version", true),
        ("GET", "/fuentes", con_identidad),
        ("POST", "/fuentes", con_identidad),
        ("DELETE", "/fuentes/{nombre}", con_identidad),
        ("GET", "/fuentes/{nombre}/estado", con_identidad),
        ("GET", "/fuentes/credenciales/{tipo}", con_identidad),
        ("POST", "/fuentes/comprobaciones", con_identidad),
        ("POST", "/fuentes/{nombre}/catalogar", con_identidad),
        ("GET", "/paquetes", con_identidad),
        ("GET", "/paquetes/{nombre}/esquema", con_identidad),
        ("GET", "/paquetes/{nombre}/decisiones", con_identidad),
        ("POST", "/paquetes/{nombre}/decisiones", con_identidad),
        ("GET", "/paquetes/{nombre}/copias", con_identidad),
        ("POST", "/paquetes/{nombre}/copia", con_identidad),
        ("DELETE", "/paquetes/{nombre}", con_identidad),
        ("GET", "/arbol", con_identidad),
        ("GET", "/arbol/diagnosticos", con_identidad),
        ("GET", "/arbol/{ruta}", con_identidad),
        ("PUT", "/arbol/{ruta}", con_identidad),
        ("DELETE", "/arbol/{ruta}", con_identidad),
        ("POST", "/arbol/commit", con_identidad),
        ("GET", "/arbol/historia/{ruta}", con_identidad),
        ("GET", "/arbol/version/{hash}/{ruta}", con_identidad),
        ("POST", "/ramas/{nombre}/fusionar", con_identidad),
        ("GET", "/ramas", con_identidad),
        ("POST", "/ramas", con_identidad),
        ("DELETE", "/ramas/{nombre}", con_identidad),
        ("GET", "/entorno", con_identidad),
        ("POST", "/entorno", con_identidad),
        ("GET", "/puestos", con_identidad),
        ("POST", "/puestos", con_identidad),
        ("GET", "/puestos/{id}", con_identidad),
        ("DELETE", "/puestos/{id}", con_identidad),
        ("POST", "/puestos/{id}/ejecutar", con_identidad),
        ("GET", "/puestos/{id}/celdas/{n}", con_identidad),
        ("GET", "/puestos/{id}/pendiente", con_identidad),
        ("POST", "/puestos/{id}/celdas/{n}/salida", con_identidad),
        ("GET", "/puestos/{id}/datos/{vista}", con_identidad),
        ("GET", "/propuestas", con_identidad),
        ("POST", "/propuestas", con_identidad),
        ("GET", "/propuestas/{n}", con_identidad),
        ("POST", "/propuestas/{n}/revisar", con_identidad),
        ("POST", "/propuestas/{n}/fusionar", con_identidad),
        ("DELETE", "/propuestas/{n}", con_identidad),
        ("GET", "/colecciones", con_identidad),
        (
            "GET",
            "/colecciones/{base}/{schema}/{nombre}",
            con_identidad,
        ),
        (
            "GET",
            "/colecciones/{base}/{schema}/{nombre}/items",
            con_identidad,
        ),
        (
            "GET",
            "/colecciones/{base}/{schema}/{nombre}/items/{huella}",
            con_identidad,
        ),
        (
            "POST",
            "/colecciones/{base}/{schema}/{nombre}/items/resolver",
            con_identidad,
        ),
        ("GET", "/funciones", con_identidad),
        ("GET", "/funciones/{ns}/{nombre}/resultados", con_identidad),
        ("POST", "/funciones/{ns}/{nombre}/invocar", con_identidad),
        (
            "GET",
            "/funciones/{base}/{schema}/{nombre}/resultados",
            con_identidad,
        ),
        (
            "POST",
            "/funciones/{base}/{schema}/{nombre}/invocar",
            con_identidad,
        ),
        ("POST", "/vistas/{ns}/{nombre}/ejecutar", con_identidad),
        ("POST", "/paquetes/{nombre}/copia/rehacer", con_identidad),
        (
            "POST",
            "/paquetes/{nombre}/tablas/{objeto}/modelar",
            con_identidad,
        ),
        (
            "POST",
            "/paquetes/{nombre}/tablas/{objeto}/copiar",
            con_identidad,
        ),
        ("POST", "/paquetes/{nombre}/schemas", con_identidad),
        (
            "POST",
            "/paquetes/{nombre}/schemas/{schema}/renombrar",
            con_identidad,
        ),
        ("POST", "/proyectos", con_identidad),
        ("POST", "/repositorios", con_identidad),
        ("PUT", "/repositorios/{ruta}", con_identidad),
        ("PUT", "/proyectos/{id}", con_identidad),
        ("DELETE", "/proyectos/{id}", con_identidad),
        ("GET", "/perfiles", con_identidad),
        ("GET", "/modelos", con_identidad),
        ("POST", "/modelos", con_identidad),
        ("GET", "/modelos/{nombre}", con_identidad),
        ("DELETE", "/modelos/{nombre}", con_identidad),
    ]
    .into_iter()
    .map(|(v, r, b)| (v, r.to_string(), b))
    .collect();
    m.push(("GET", "/conceptos".to_string(), con_identidad));
    for k in documentos::KINDS {
        m.push(("GET", format!("/documentos/{}", k.nombre), con_identidad));
        // Lo que escribe otro verbo (0041: el `Model`) sólo se lee por aquí.
        let verbos: &[&str] = if k.escribe.is_some() {
            &["GET"]
        } else {
            &["GET", "PUT", "DELETE"]
        };
        for &verbo in verbos {
            m.push((
                verbo,
                format!("/documentos/{}/{{ns}}/{{nombre}}", k.nombre),
                con_identidad,
            ));
        }
    }
    m
}

pub fn ruta_de(p: &Path) -> String {
    p.display().to_string()
}

/// **`include (s.*)`** (0039): un objeto `s.*` es el schema `s` entero del
/// catálogo del origen (sus `tables[].name` que empiezan por `s.`). Lo demás,
/// tal cual: si no está, lo dice `discover`.
fn expandir(catalogo: &Path, objetos: Vec<String>) -> Result<Vec<String>, Respuesta> {
    if !objetos.iter().any(|o| o.ends_with(".*")) {
        return Ok(objetos);
    }
    let nombres: Vec<String> = std::fs::read_to_string(catalogo)
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .map(|n| {
            // v1alpha16: `s.*` también trae los conjuntos de objetos de `s`.
            let de = |k: &str| {
                n.get(k)
                    .map(|(_, t)| t.items().to_vec())
                    .unwrap_or_default()
            };
            de("tables")
                .into_iter()
                .chain(de("objects"))
                .collect::<Vec<_>>()
                .iter()
                .filter_map(|t| {
                    t.get("name")
                        .and_then(|(_, v)| v.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for o in objetos {
        match o.strip_suffix(".*") {
            None => {
                if !out.contains(&o) {
                    out.push(o)
                }
            }
            Some(s) => {
                let de_s: Vec<&String> = nombres
                    .iter()
                    .filter(|n| n.strip_prefix(s).is_some_and(|r| r.starts_with('.')))
                    .collect();
                if de_s.is_empty() {
                    return Err(Respuesta::error(
                        422,
                        format!("el origen no tiene nada en `{s}`: `{o}` no incluye ningún objeto"),
                    ));
                }
                for n in de_s {
                    if !out.contains(n) {
                        out.push(n.clone());
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod el_nombre_de_una_fuente {
    use super::*;

    /// La regla es la intersección de custodio, cola y namespace: lo que pasa,
    /// pasa en los tres.
    #[test]
    fn la_regla() {
        for bien in [
            "ventas",
            "ventas_produccion",
            "pg",
            "bq2",
            "a_b_c",
            &"a".repeat(30),
        ] {
            assert!(nombre_de_fuente(bien).is_ok(), "{bien}");
        }
        for mal in [
            "Ventas",
            "ventas-prod",
            "a__b",
            "_a",
            "a_",
            "2ventas",
            "",
            "ventas prod",
            &"a".repeat(31),
            "bigquery_20260927_1428x_largooo",
        ] {
            assert!(nombre_de_fuente(mal).is_err(), "{mal}");
        }
    }

    /// Lo que la regla deja pasar no colisiona en la cola: `nombre_de_objeto`
    /// es biyectivo sobre ella.
    #[test]
    fn dos_nombres_validos_no_comparten_fichero_de_cola() {
        let nombres = ["ventas", "ventas_2", "ventas2", "a_b", "ab", "a_bc", "ab_c"];
        let objetos: std::collections::BTreeSet<_> =
            nombres.iter().map(|n| cola::nombre_de_objeto(n)).collect();
        assert_eq!(objetos.len(), nombres.len(), "{objetos:?}");
    }

    #[test]
    fn el_nombre_que_se_sugiere_cumple_la_regla() {
        for (humano, sale) in [
            ("Ventas Producción", "ventas_produccion"),
            ("  CRM -- EU (prod) ", "crm_eu_prod"),
            ("2024 ventas", "f_2024_ventas"),
            ("Ñandú", "nandu"),
            ("¿?", "fuente"),
            (
                "Almacén de datos de la región norte del país",
                "almacen_de_datos_de_la_region",
            ),
        ] {
            let s = sugerir_nombre_de_fuente(humano);
            assert_eq!(s, sale, "{humano}");
            assert!(nombre_de_fuente(&s).is_ok(), "{humano} → {s}");
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn un_nombre_no_puede_salir_de_su_directorio() {
        assert!(token("..").is_err());
        assert!(token("../etc").is_err());
        assert!(token("a/b").is_err());
        assert!(token("a\\b").is_err());
        assert!(token("").is_err());
        assert!(token("ventas").is_ok());
        assert!(token("ventas-2_b").is_ok());
    }

    /// ⚠️ Se DETECTA, que ya no es lo mismo que negarse. Con custodio
    /// configurado, una URL de estas se acepta y la credencial va allí; sin él,
    /// el alta la rechaza. Esta función sólo contesta a «¿la trae?».
    #[test]
    fn una_url_con_credencial_se_detecta() {
        assert!(sin_credencial("postgres://ana:clave@host/db").is_err());
        assert!(sin_credencial("https://host/x?password=abc").is_err());
        assert!(sin_credencial("https://host/x?token=abc").is_err());
    }

    /// Las que no la traen: la credencial la presta la nube (BigQuery por
    /// Workload Identity) o no hace falta. Éstas nunca dependieron del custodio.
    #[test]
    fn las_claves_de_un_almacen_de_objetos_son_credencial() {
        // S3 con sus claves (0046 F0): antes pasaba por «sin credencial».
        assert!(
            sin_credencial("s3://b/p?region=eu-north-1&access_key_id=AKIA&secret_access_key=x/y+z")
                .is_err()
        );
        assert!(sin_credencial("s3://b/p?aws_session_token=abc").is_err());
        // La SAS de Azure.
        assert!(sin_credencial("https://c.blob.core.windows.net/x?sv=2022&sig=abc").is_err());
        // Un parámetro inocente, o uno vacío, no lo es.
        assert!(sin_credencial("s3://b/p?region=eu-north-1").is_ok());
        assert!(sin_credencial("s3://b/p?secret_access_key=").is_ok());
    }

    #[test]
    fn una_url_sin_credencial_pasa() {
        assert!(sin_credencial("bigquery://mi-proyecto/ventas").is_ok());
        assert!(sin_credencial("postgres://host:5432/db").is_ok());
        assert!(sin_credencial("jsonl:///datos/x.jsonl").is_ok());
    }

    #[test]
    fn el_tipo_del_escalar_sobrevive_al_viaje() {
        let n = parse::parse(r#"{"a": 1, "b": "1", "c": true, "d": [1, "x"]}"#).unwrap();
        assert_eq!(de_node(&n).jcs(), r#"{"a":1,"b":"1","c":true,"d":[1,"x"]}"#);
    }

    /// El fallo que la forja destapó: `ore review` deja la cola escrita con la
    /// lista vacía, así que «existe el fichero» y «quedan decisiones» son dos
    /// cosas distintas.
    #[test]
    fn una_cola_vacia_no_son_decisiones_pendientes() {
        let d = std::env::temp_dir().join(format!("ore-serve-cola-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);

        std::fs::write(d.join(COLA), r#"{"pending": []}"#).unwrap();
        assert_eq!(pendientes(&d), 0, "una cola vacía contó como pendiente");

        std::fs::write(d.join(COLA), r#"{"pending": [{"id":"a"},{"id":"b"}]}"#).unwrap();
        assert_eq!(pendientes(&d), 2);

        std::fs::remove_file(d.join(COLA)).unwrap();
        assert_eq!(pendientes(&d), 0, "sin fichero tampoco hay pendientes");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// El fichero de respuestas es entrada de una petición, no un documento del
    /// repositorio: si cayera dentro del árbol acabaría en un commit.
    #[test]
    fn el_fichero_de_respuestas_vive_fuera_del_arbol() {
        let f = temporal("respuestas", "json");
        assert!(f.starts_with(std::env::temp_dir()));
    }
}

#[cfg(test)]
mod el_puntero_en_la_fuente {
    //! 0045 P2: el esquema y los recuentos de una database y de su fuente, con
    //! el puntero en la fuente, dicen lo mismo que con el puntero en la database.
    use super::*;

    fn arbol(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ore-p2-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let w = |rel: &str, t: &str| {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "ontology.config.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
        );
        w(
            "packages/pg/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: pg, version: 0.1.0, status: draft, domain: pg }\nspec: { owner: team:t, exports: [pg.olist.orders] }\n",
        );
        w(
            "packages/pg/discover.catalog.json",
            "{\"source\":\"pg\",\"tables\":[{\"name\":\"olist.customers\",\"columns\":[{\"name\":\"customer_id\",\"type\":\"String\"}]},{\"name\":\"olist.orders\",\"columns\":[{\"name\":\"order_id\",\"sourceType\":\"varchar\",\"type\":\"String\"}]}]}",
        );
        w(
            "packages/pg/olist/schema.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: olist, namespace: pg }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/pg/olist/tables/orders.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Table\nmetadata: { name: orders, namespace: pg, schema: olist }\nspec:\n  datasource: pg\n  object: \"olist.orders\"\n  columns: { order_id: { type: String, physicalType: varchar } }\n  reads: { fullScan: cheap }\n  changes: { mode: none, witness: none, key: [order_id] }\n",
        );
        w(
            "packages/tienda/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: tienda, version: 0.1.0, status: active, domain: tienda }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/tienda/discover.scope.json",
            "{\"source\":\"pg\",\"objects\":[\"olist.orders\"],\"type\":\"standard\"}",
        );
        w(
            "packages/tienda/discover.catalog.json",
            "{\"source\":\"pg\",\"tables\":[{\"name\":\"olist.customers\",\"columns\":[]},{\"name\":\"olist.orders\",\"columns\":[]}]}",
        );
        w(
            "packages/tienda/olist/schema.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: olist, namespace: tienda }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/tienda/olist/datasets/orders.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: orders, namespace: tienda, schema: olist }\nspec:\n  owner: team:t\n  from: { table: pg.olist.orders }\n  fields: { order_id: order_id }\n",
        );
        d
    }

    fn campo<'a>(j: &'a Json, k: &str) -> Option<&'a Json> {
        match j {
            Json::Obj(m) => m.get(k),
            _ => None,
        }
    }

    #[test]
    fn la_database_ensena_su_copia_aunque_el_puntero_este_en_la_fuente() {
        let d = arbol("database");
        let (pkg, _) = ore_core::validate::cargar_paquete(&d);
        let filas = tablas_del_paquete(&d, &pkg, &d.join("packages/tienda"));
        // Una fila, la de lo que copia: no el catálogo entero de la database.
        assert_eq!(filas.len(), 1, "{filas:?}");
        let f = &filas[0];
        assert_eq!(campo(f, "object"), Some(&Json::s("olist.orders")));
        assert_eq!(campo(f, "datasource"), Some(&Json::s("pg")));
        assert_eq!(campo(f, "copied"), Some(&Json::Bool(true)));
        assert_eq!(campo(f, "dataset"), Some(&Json::s("orders")));
        assert_eq!(
            tablas_y_modeladas(
                &crate::punteros::Indice::nuevo(&d, &pkg),
                &d.join("packages/tienda")
            ),
            (1, 0)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn la_fuente_ensena_su_catalogo_entero_con_su_puntero_y_quien_lo_usa() {
        let d = arbol("fuente");
        let (pkg, _) = ore_core::validate::cargar_paquete(&d);
        let filas = tablas_del_paquete(&d, &pkg, &d.join("packages/pg"));
        // Las dos del catálogo, aunque solo una tenga puntero: el modal de nueva
        // database tiene que seguir ofreciendo `customers`.
        assert_eq!(filas.len(), 2, "{filas:?}");
        let orders = filas
            .iter()
            .find(|f| campo(f, "object") == Some(&Json::s("olist.orders")))
            .unwrap();
        assert_eq!(campo(orders, "table"), Some(&Json::s("pg.olist.orders")));
        assert_eq!(
            campo(orders, "usedBy"),
            Some(&Json::Arr(vec![Json::s("tienda")]))
        );
        let customers = filas
            .iter()
            .find(|f| campo(f, "object") == Some(&Json::s("olist.customers")))
            .unwrap();
        assert_eq!(campo(customers, "table"), None);
        assert_eq!(campo(customers, "usedBy"), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// v1alpha16 (0046 E5) · **un bucket**: la tabla de ficheros dice dónde
    /// está (`location`) y se elige por su nombre del catálogo (`object`);
    /// los conjuntos de objetos salen como filas `kind: objects` con sus
    /// columnas fijas, su puntero y quién los usa —por su colección, o por
    /// haberlo elegido, que es lo único que hace una base con un zip—.
    #[test]
    fn un_bucket_ensena_sus_tablas_y_sus_conjuntos_por_su_nombre() {
        let d = std::env::temp_dir().join(format!("ore-e5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let w = |rel: &str, t: &str| {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "ontology.config.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: s3, type: s3, connectionEnv: S3_URL }\n",
        );
        w(
            "packages/s3/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: s3, version: 0.1.0, status: draft, domain: s3 }\nspec: { owner: team:t, exports: [s3.docs.pedidos, s3.docs.contratos, s3.docs.zips] }\n",
        );
        w(
            "packages/s3/discover.catalog.json",
            "{\"source\":\"s3\",\"tables\":[{\"name\":\"docs.pedidos\",\"object\":\"Mi carpeta/pedidos.csv\",\"format\":{\"type\":\"csv\"},\"columns\":[{\"name\":\"id\",\"type\":\"String\"}]}],\
             \"objects\":[{\"name\":\"docs.contratos\",\"prefix\":\"Mi carpeta/contratos/\",\"media\":\"document\",\"count\":4,\"bytes\":100,\"extensions\":[\"pdf\"]},\
             {\"name\":\"docs.zips\",\"prefix\":\"Mi carpeta/\",\"match\":\"*.zip\",\"media\":\"archive\",\"count\":2,\"bytes\":9,\"extensions\":[\"zip\"]}]}",
        );
        w(
            "packages/s3/docs/schema.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: docs, namespace: s3 }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/s3/docs/tables/pedidos.yaml",
            "apiVersion: oos.dev/v1alpha16\nkind: Table\nmetadata: { name: pedidos, namespace: s3, schema: docs }\nspec:\n  datasource: s3\n  object: \"Mi carpeta/pedidos.csv\"\n  format: { type: csv }\n  columns: { id: { type: String } }\n  reads: { fullScan: cheap }\n  changes: { mode: retract, witness: listing }\n",
        );
        for (n, extra) in [
            (
                "contratos",
                "  prefix: \"Mi carpeta/contratos/\"\n  media: document\n",
            ),
            (
                "zips",
                "  prefix: \"Mi carpeta/\"\n  match: \"*.zip\"\n  media: archive\n",
            ),
        ] {
            w(
                &format!("packages/s3/docs/objects/{n}.yaml"),
                &format!(
                    "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: {{ name: {n}, namespace: s3, schema: docs }}\nspec:\n  datasource: s3\n{extra}  changes: {{ mode: retract, witness: listing }}\n"
                ),
            );
        }
        w(
            "packages/fdb/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: fdb, version: 0.1.0, status: draft, domain: fdb }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/fdb/discover.scope.json",
            "{\"source\":\"s3\",\"only\":[\"docs.pedidos\",\"docs.contratos\"]}",
        );
        w(
            "packages/fdb/docs/schema.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: docs, namespace: fdb }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/fdb/docs/datasets/pedidos.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: pedidos, namespace: fdb, schema: docs }\nspec:\n  owner: team:t\n  from: { table: s3.docs.pedidos }\n  fields: { id: id }\n",
        );
        w(
            "packages/fdb/docs/collections/contratos.yaml",
            "apiVersion: oos.dev/v1alpha16\nkind: MediaCollection\nmetadata: { name: contratos, namespace: fdb, schema: docs }\nspec:\n  owner: team:t\n  media: document\n  formats: [pdf]\n  from: { objectTable: s3.docs.contratos }\n  virtual: true\n",
        );
        w(
            "packages/zdb/package.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: zdb, version: 0.1.0, status: draft, domain: zdb }\nspec: { owner: team:t }\n",
        );
        w(
            "packages/zdb/discover.scope.json",
            "{\"source\":\"s3\",\"only\":[\"docs.zips\"]}",
        );
        std::fs::copy(
            d.join("packages/s3/discover.catalog.json"),
            d.join("packages/zdb/discover.catalog.json"),
        )
        .unwrap();

        let (pkg, _) = ore_core::validate::cargar_paquete(&d);
        let de = |filas: &[Json], o: &str| {
            filas
                .iter()
                .find(|f| campo(f, "object") == Some(&Json::s(o)))
                .cloned()
                .unwrap_or_else(|| panic!("sin `{o}`: {filas:?}"))
        };
        let fuente = tablas_del_paquete(&d, &pkg, &d.join("packages/s3"));
        assert_eq!(fuente.len(), 3, "{fuente:?}");
        let p = de(&fuente, "docs.pedidos");
        assert_eq!(
            campo(&p, "location"),
            Some(&Json::s("Mi carpeta/pedidos.csv"))
        );
        assert_eq!(campo(&p, "table"), Some(&Json::s("s3.docs.pedidos")));
        assert_eq!(campo(&p, "usedBy"), Some(&Json::Arr(vec![Json::s("fdb")])));
        let c = de(&fuente, "docs.contratos");
        assert_eq!(campo(&c, "kind"), Some(&Json::s("objects")));
        assert_eq!(campo(&c, "media"), Some(&Json::s("document")));
        assert_eq!(campo(&c, "count"), Some(&Json::Int(4)));
        assert_eq!(campo(&c, "table"), Some(&Json::s("s3.docs.contratos")));
        assert_eq!(campo(&c, "usedBy"), Some(&Json::Arr(vec![Json::s("fdb")])));
        assert!(
            matches!(campo(&c, "columns"), Some(Json::Arr(cs)) if cs.len() == ore_core::document::COLUMNAS_DE_OBJETO.len())
        );
        let z = de(&fuente, "docs.zips");
        assert_eq!(campo(&z, "table"), Some(&Json::s("s3.docs.zips")));
        assert_eq!(campo(&z, "usedBy"), Some(&Json::Arr(vec![Json::s("zdb")])));

        // La base: la fila dice el nombre del catálogo, que es lo que se copia.
        let base = tablas_del_paquete(&d, &pkg, &d.join("packages/fdb"));
        assert_eq!(base.len(), 1, "{base:?}");
        assert_eq!(campo(&base[0], "object"), Some(&Json::s("docs.pedidos")));
        assert_eq!(
            campo(&base[0], "location"),
            Some(&Json::s("Mi carpeta/pedidos.csv"))
        );
        // Una base sin tablas enseña lo que eligió, no el bucket entero.
        let zips = tablas_del_paquete(&d, &pkg, &d.join("packages/zdb"));
        assert_eq!(zips.len(), 1, "{zips:?}");
        assert_eq!(campo(&zips[0], "object"), Some(&Json::s("docs.zips")));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⭐ 0046 E5b · **El esquema de una fuente crece con el árbol, no con su
    /// cuadrado.** Una fuente de `n` tablas con su puntero cada una y una base
    /// con una vista SQL sobre cada puntero; se cronometra solo pintar el
    /// esquema (el árbol ya cargado), con `n` y con `4n`, lo mejor de tres.
    /// Lo de antes —cada fila recorría el árbol y reanalizaba cada consulta—
    /// daba ×46 (y 98 s con 2.000 tablas). Hoy da ×6–8, no ×4: resolver cada
    /// nombre de una consulta (`linaje::resolver` → `Package::table`…) sigue
    /// siendo una búsqueda lineal en ore-core, y eso es lo que queda por hacer
    /// (0046 E5b). El umbral caza la regresión, no ese resto. La razón no
    /// depende de la máquina.
    #[test]
    fn el_esquema_de_una_fuente_crece_lineal() {
        let tiempo = |n: usize| {
            let d = std::env::temp_dir().join(format!("ore-e5b-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            let w = |rel: String, t: String| {
                let p = d.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, t).unwrap();
            };
            w(
                "ontology.config.yaml".into(),
                "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n".into(),
            );
            w(
                "packages/pg/package.yaml".into(),
                "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: pg, version: 0.1.0, status: draft, domain: pg }\nspec: { owner: team:t }\n".into(),
            );
            let filas: Vec<String> = (0..n)
                .map(|i| format!("{{\"name\":\"t{i}\",\"columns\":[{{\"name\":\"id\",\"type\":\"Integer\"}}]}}"))
                .collect();
            w(
                "packages/pg/discover.catalog.json".into(),
                format!("{{\"source\":\"pg\",\"tables\":[{}]}}", filas.join(",")),
            );
            w(
                "packages/b/package.yaml".into(),
                "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: b, version: 0.1.0, status: draft, domain: b }\nspec: { owner: team:t }\n".into(),
            );
            for i in 0..n {
                w(
                    format!("packages/pg/tables/t{i}.yaml"),
                    format!(
                        "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: {{ name: t{i}, namespace: pg }}\nspec:\n  datasource: pg\n  object: t{i}\n  columns: {{ id: {{ type: Integer }} }}\n  reads: {{ fullScan: cheap }}\n  changes: {{ mode: none, witness: none }}\n"
                    ),
                );
                w(
                    format!("packages/b/views/v{i}.yaml"),
                    format!(
                        "apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: {{ name: v{i}, namespace: b }}\nspec:\n  owner: team:t\n  dialect: duckdb\n  sql: SELECT id FROM \"pg\".\"t{i}\"\n  columns: {{ id: {{ type: Integer }} }}\n"
                    ),
                );
            }
            let (pkg, _) = ore_core::validate::cargar_paquete(&d);
            let mut t = std::time::Duration::MAX;
            let mut filas = Vec::new();
            for _ in 0..3 {
                let a = std::time::Instant::now();
                filas = tablas_del_paquete(&d, &pkg, &d.join("packages/pg"));
                t = t.min(a.elapsed());
            }
            assert_eq!(filas.len(), n);
            assert!(
                filas
                    .iter()
                    .all(|f| campo(f, "usedBy") == Some(&Json::Arr(vec![Json::s("b")]))),
                "cada puntero lo lee `b`: {:?}",
                &filas[..1]
            );
            let _ = std::fs::remove_dir_all(&d);
            t
        };
        let (poco, mucho) = (tiempo(100), tiempo(400));
        let razon = mucho.as_secs_f64() / poco.as_secs_f64().max(1e-6);
        assert!(
            razon < 12.0,
            "×4 objetos costó ×{razon:.1} ({poco:?} → {mucho:?}): el esquema de una fuente volvió a ser cuadrático"
        );
    }
}
