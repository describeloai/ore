//! `ore-cofre retirar-huerfanos`: las credenciales de conexión cuya fuente ya
//! no está en el árbol, fuera del custodio. De operador, y corre EN el
//! inquilino, como `mudar` — y por lo mismo: borrar del almacén exige el
//! prefijo de ESE inquilino.
//!
//! # Por qué existe (038)
//!
//! `DELETE /fuentes/{n}` retira la credencial con la fuente. Pero lo que quedó
//! de antes —19 en `demo`, 2 en `victor` el 2026-09-18— es de fuentes que ya no
//! están en ningún manifiesto: nadie las va a pulsar desde una ficha que no
//! existe. Y desde fuera no se puede: el custodio decide con el testigo de una
//! persona, y un operador con la llave de todos sería la concentración que la
//! `0023` desmontó.
//!
//! # Qué es huérfano
//!
//! Un secreto vivo de clase `conexion` llamado `fuente-<n>` cuya `<n>` NO está
//! en la lista de fuentes declaradas que el operador trae (`--declaradas`, del
//! `ontology.config.yaml` del inquilino). El custodio no ve el árbol; quien lo
//! ve, se lo dice. Y `--seco` enseña qué se iría sin tocar nada.
//!
//! # Qué hace con cada uno, en UNA transacción
//!
//!   1. la fila con `retirado_en` y `retiro_agente = "ore-cofre retirar-huerfanos"`
//!      — no una persona que no lo hizo
//!   2. sus concesiones revocadas con fecha (`iam.revocar_de_secreto`)
//!   3. el material fuera del almacén de la celda
//!   4. huella `secreto:retirar` con `como: operador`, sin el valor

use crate::almacen::{Almacen, nombre_en_almacen};
use ore_core::json::Json;
use ore_entrada::identidad::Identidad;
use ore_iam::base::Tx;
use postgres::Client;

pub const VERBO: &str = "ore-cofre retirar-huerfanos";

pub fn retirar(
    mut c: Client,
    org: &str,
    declaradas: &[String],
    almacen: &Almacen,
    seco: bool,
) -> Result<usize, String> {
    let operador = Identidad {
        persona: "operador".into(),
        agente: Some(VERBO.into()),
        correo: None,
        nombre: None,
        tipo: None,
    };
    let mut tx = Tx::abrir(&mut c, &operador)?;
    let f = tx
        .uno(
            "select id, nombre from iam.organizacion where nombre = $1 or id = $1",
            &[&org],
        )?
        .ok_or_else(|| format!("no hay ninguna organizacion `{org}`"))?;
    let (org_id, inquilino): (String, String) = (f.get(0), f.get(1));

    let filas = tx.filas(
        "select s.id, s.nombre, ce.nombre
           from cofre.secreto s
           join iam.celda ce on ce.id = s.celda
          where s.organizacion = $1 and s.clase = 'conexion'
            and s.nombre like 'fuente-%' and s.retirado_en is null
          order by s.nombre",
        &[&org_id],
    )?;
    let mut cuantos = 0usize;
    for f in &filas {
        let (id, nombre, celda): (String, String, String) = (f.get(0), f.get(1), f.get(2));
        let fuente = nombre.trim_start_matches("fuente-");
        if declaradas.iter().any(|d| d == fuente) {
            continue;
        }
        if seco {
            eprintln!("  · {nombre} se iría (su fuente `{fuente}` no está declarada)");
            cuantos += 1;
            continue;
        }
        tx.ejecutar(
            "update cofre.secreto set retirado_en = now(), retiro_agente = $2 where id = $1",
            &[&id, &VERBO],
        )?;
        let recurso = format!("secreto/{nombre}");
        let revocadas: i32 = tx
            .uno(
                "select iam.revocar_de_secreto_operador($1, $2, $3)",
                &[&recurso, &org_id, &VERBO],
            )?
            .map(|r| r.get(0))
            .unwrap_or(0);
        almacen.borrar(&nombre_en_almacen(&celda, &nombre))?;
        tx.anotar(
            "secreto:retirar",
            &id,
            Json::obj([
                ("organizacion", Json::s(&org_id)),
                ("nombre", Json::s(&nombre)),
                ("como", Json::s("operador")),
                (
                    "motivo",
                    Json::s(format!(
                        "la fuente `{fuente}` ya no está declarada en el árbol"
                    )),
                ),
                ("concesiones_revocadas", Json::Int(revocadas as i64)),
            ]),
        )?;
        eprintln!("  ✓ {nombre} retirada ({revocadas} concesion(es) revocadas)");
        cuantos += 1;
    }
    if seco || cuantos == 0 {
        eprintln!(
            "  · `{inquilino}`: {cuantos} huérfana(s){}",
            if seco { " · en seco, nada tocado" } else { "" }
        );
        return Ok(cuantos);
    }
    tx.confirmar()?;
    Ok(cuantos)
}
