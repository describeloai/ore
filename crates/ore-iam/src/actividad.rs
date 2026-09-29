//! **La actividad de una organización** (0047 A6.2): `GET
//! /organizaciones/{o}/actividad`, lo que la huella dice que pasó en ella.
//!
//! # Hasta dónde ves lo decide esto, y lo dice
//!
//! Con `actividad:leer-toda` (ORGADMIN, SECURITYADMIN), la de todos; si no, la
//! tuya, que es lo que da pertenecer. Y la respuesta lleva `alcance`: enseñar
//! tres filas sin decir que son sólo las tuyas dejaría creer que la organización
//! entera hizo tres cosas.
//!
//! # Sin el ruido
//!
//! El 99 % de la huella es sondeo y máquina (M6 § 4; medido otra vez el
//! 2026-09-29: 10.641 filas en 7 días). Aquí no sale: ni los listados (`*:listar`,
//! la consola al cargar), ni lo que el aprovisionador y el informador de cada
//! celda cuentan cada pocos minutos, ni leer esta misma actividad. Siguen en la
//! huella; esto sirve los actos.
//!
//! # Desde cuándo
//!
//! Por organización sólo hay lo que se anotó con ella en su columna (la `044`, y
//! lo de `ore-iam` desde A6.1). Las filas de antes no se pueden completar —la
//! `039` no deja editar la huella, a nadie—, así que la respuesta dice `desde`.

use crate::base::Tx;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::collections::BTreeMap;

/// Cuántas filas por página, si no se dice; y el techo.
const POR_DEFECTO: i64 = 50;
const TECHO: i64 = 200;

/// Lo que no es un acto de la organización: sondeo, máquina, y leer esto.
const NO_SON_ACTOS: &[&str] = &["celda:aprovisionada", "celda:informa", "actividad:leer"];

impl Servidor {
    /// `GET /organizaciones/{o}/actividad?desde=<id>&limite=<n>&clase=persona|agente&celda=<id>`.
    /// `desde` es el cursor: el `id` de la última fila de la página anterior.
    pub(crate) fn actividad(
        &self,
        s: &Identidad,
        org: &str,
        consulta: &BTreeMap<String, String>,
    ) -> Respuesta {
        let limite = match consulta.get("limite").map(|l| l.parse::<i64>()) {
            None => POR_DEFECTO,
            Some(Ok(n)) if (1..=TECHO).contains(&n) => n,
            Some(_) => {
                return Respuesta::error(422, format!("`limite` es un número de 1 a {TECHO}"));
            }
        };
        let desde = match consulta.get("desde").map(|d| d.parse::<i64>()) {
            None => None,
            Some(Ok(n)) => Some(n),
            Some(Err(_)) => return Respuesta::error(422, "`desde` es el id de una fila"),
        };
        let clase = match consulta.get("clase").map(String::as_str) {
            None => None,
            Some(c @ ("persona" | "agente")) => Some(c.to_string()),
            Some(otra) => {
                return Respuesta::error(
                    422,
                    format!("`clase` es `persona` o `agente`, no `{otra}`"),
                );
            }
        };
        let celda = consulta.get("celda").cloned();
        let org = org.to_string();
        self.en_transaccion(s, move |tx, emisor| {
            let mias = crate::potestad::potestades_de(tx, emisor, &s.persona, &org)?;
            // ⛔ «No perteneces» con el mismo mensaje que `exige`: decir otra cosa
            //   confirmaría que la organización existe.
            if mias.is_empty() {
                return Err("no puedes hacer eso en esa organizacion".into());
            }
            let toda = mias.contains("actividad:leer-toda");
            let filas = leer(tx, emisor, &org, &s.persona, toda, desde, limite + 1, clase.as_deref(), celda.as_deref())?;
            let hay_mas = filas.len() as i64 > limite;
            let filas: Vec<Json> = filas.into_iter().take(limite as usize).collect();
            let siguiente = if hay_mas {
                filas.last().and_then(|f| match f {
                    Json::Obj(m) => m.get("id").cloned(),
                    _ => None,
                })
            } else {
                None
            };
            let primera: Option<String> = tx
                .uno(
                    "select to_char(min(cuando) at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                       from iam.huella where organizacion = $1",
                    &[&org],
                )?
                .and_then(|f| f.get(0));
            let alcance = if toda { "organizacion" } else { "propia" };
            tx.en(&org);
            tx.anotar(
                "actividad:leer",
                &org,
                Json::obj([
                    ("alcance", Json::s(alcance)),
                    ("cuantas", Json::Int(filas.len() as i64)),
                ]),
            )?;
            Ok(Json::obj([
                ("alcance", Json::s(alcance)),
                ("desde", primera.map(Json::s).unwrap_or(Json::Bool(false))),
                ("actividad", Json::Arr(filas)),
                ("siguiente", siguiente.unwrap_or(Json::Bool(false))),
            ]))
        })
    }
}

/// Las filas, de la más nueva a la más vieja, sobre el índice de la `044`.
#[allow(clippy::too_many_arguments)]
fn leer(
    tx: &mut Tx,
    emisor: &str,
    org: &str,
    yo: &str,
    toda: bool,
    desde: Option<i64>,
    limite: i64,
    clase: Option<&str>,
    celda: Option<&str>,
) -> Result<Vec<Json>, String> {
    let no_son: Vec<String> = NO_SON_ACTOS.iter().map(|s| s.to_string()).collect();
    // ⭐ Un agente es quien actúa por alguien (`agente`) o lo que el custodio
    //   anota como tal (`detalle.clase`): un Job que resuelve su credencial.
    let filas = tx.filas(
        "select h.id,
                to_char(h.cuando at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'),
                h.quien, p.nombre, h.agente, h.operacion, coalesce(h.sobre, ''),
                coalesce(h.celda, ''),
                coalesce(h.detalle->>'resultado',
                         case when h.operacion = 'acceso:negado' then 'negado' else 'hecho' end),
                coalesce(h.detalle, '{}'::jsonb)::text
           from iam.huella h
           left join iam.persona p on p.emisor = $2 and p.sub = h.quien
          where h.organizacion = $1
            and h.operacion not like '%:listar'
            and h.operacion <> all($3)
            and ($4 or h.quien = $5)
            and ($6::bigint is null
                 or (h.cuando, h.id) < (select cuando, id from iam.huella where id = $6))
            and ($7::text is null
                 or ($7 = 'agente') = (h.agente is not null or h.detalle->>'clase' = 'agente'))
            and ($8::text is null or h.celda = $8)
          order by h.cuando desc, h.id desc
          limit $9",
        &[
            &org, &emisor, &no_son, &toda, &yo, &desde, &clase, &celda, &limite,
        ],
    )?;
    Ok(filas
        .iter()
        .map(|f| {
            let detalle: String = f.get(9);
            Json::obj([
                ("id", Json::s(f.get::<_, i64>(0).to_string())),
                ("cuando", Json::s(f.get::<_, String>(1))),
                ("quien", Json::s(f.get::<_, String>(2))),
                (
                    "nombre",
                    Json::s(f.get::<_, Option<String>>(3).unwrap_or_default()),
                ),
                (
                    "agente",
                    f.get::<_, Option<String>>(4)
                        .map(Json::s)
                        .unwrap_or(Json::Bool(false)),
                ),
                ("operacion", Json::s(f.get::<_, String>(5))),
                ("sobre", Json::s(f.get::<_, String>(6))),
                ("celda", Json::s(f.get::<_, String>(7))),
                ("resultado", Json::s(f.get::<_, String>(8))),
                (
                    "detalle",
                    ore_core::parse::parse(&detalle)
                        .ok()
                        .and_then(|n| crate::rutas::nodo_a_json(&n).ok())
                        .unwrap_or(Json::Bool(false)),
                ),
            ])
        })
        .collect())
}
