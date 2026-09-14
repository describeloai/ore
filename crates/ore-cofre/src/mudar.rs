//! `ore-cofre mudar`: lo que `cofre.material` guardaba en la base central, al
//! almacén de la celda. De UNA vez por inquilino, y corre EN el inquilino.
//!
//! # Por qué corre dentro y no desde fuera
//!
//! Abrir lo viejo exige la llave de ESA organización, y escribir lo nuevo exige
//! el prefijo de ESE inquilino. Las dos cosas las tiene sólo `ore-cofre-<inq>`,
//! por Workload Identity, en su namespace. Un operador desde fuera con una cuenta
//! que pudiera abrir todas las llaves sería exactamente la concentración que la
//! `0023` desmontó.
//!
//! # Qué hace con cada secreto vivo
//!
//!   1. abre la versión vigente con la KEK con la que se cerró (`kms::abrir`)
//!   2. crea el secreto en el almacén con esa misma KEK como CMEK, idempotente
//!   3. añade el valor como versión — entrada estándar, sin tocar el disco
//!   4. borra sus filas de `cofre.material`
//!   5. deja huella `secreto:mudar` con el nombre en el almacén y la versión
//!
//! Todo en UNA transacción sobre la base: si el almacén dice que no a mitad, no
//! se borra nada y se vuelve a correr. Lo que ya estuviera en el almacén se
//! reutiliza (`crear` no falla si existe), así que repetir es seguro.
//!
//! ⚠️ Y la `028` sólo borra `cofre.material` cuando esté VACÍA. Hasta que esto
//!   haya corrido en cada inquilino con secretos, la migración se niega — no se
//!   borra lo que no se ha mudado.

use crate::almacen::{Almacen, nombre_en_almacen};
use crate::kms::Kms;
use ore_core::json::Json;
use ore_entrada::identidad::Identidad;
use ore_iam::base::Tx;
use postgres::Client;

pub fn mudar(mut c: Client, org: &str, kms: &Kms, almacen: &Almacen) -> Result<usize, String> {
    let operador = Identidad {
        persona: "operador".into(),
        agente: Some("ore-cofre mudar".into()),
        correo: None,
        nombre: None,
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
        "select s.id, s.nombre, v.cifrado, v.kek, v.version
           from cofre.secreto s
           join cofre.vigente v on v.secreto = s.id
          where s.organizacion = $1 and s.retirado_en is null
          order by s.nombre",
        &[&org_id],
    )?;
    if filas.is_empty() {
        eprintln!("  · `{inquilino}` no tiene material en la base central: nada que mudar");
        // Nada cambió: no se confirma. Leer no es un acto.
        return Ok(0);
    }

    let mut cuantos = 0;
    for f in &filas {
        let (id, nombre, cifrado, kek, version): (String, String, Vec<u8>, String, i32) =
            (f.get(0), f.get(1), f.get(2), f.get(3), f.get(4));
        // ① con la llave con la que se cerró
        let claro = kms.abrir(&kek, &cifrado)?;
        // ② y ③: al almacén, bajo el prefijo del inquilino, con esa KEK como CMEK
        let en_almacen = nombre_en_almacen(&inquilino, &nombre);
        almacen.crear(&en_almacen, &kek, &inquilino)?;
        let nueva = almacen.anadir(&en_almacen, &claro)?;
        // ④ fuera de la base central
        tx.ejecutar("delete from cofre.material where secreto = $1", &[&id])?;
        // ⑤ y queda escrito
        tx.anotar(
            "secreto:mudar",
            &id,
            Json::obj([
                ("organizacion", Json::s(&org_id)),
                ("nombre", Json::s(&nombre)),
                ("kek", Json::s(&kek)),
                ("version_vieja", Json::Int(version as i64)),
                ("almacen", Json::s(&en_almacen)),
                ("version", Json::Int(nueva)),
            ]),
        )?;
        eprintln!("  ✓ {nombre} → {en_almacen} (version {nueva})");
        cuantos += 1;
    }
    tx.confirmar()?;
    Ok(cuantos)
}
