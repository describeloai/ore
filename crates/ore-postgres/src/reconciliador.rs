//! **El reconciliador** (0058 P4·2): lleva lo observado hacia lo deseado.
//!
//! Recorre las operaciones `en-curso` cuya hora (`siguiente`) ya llegó y da a
//! cada una **un intento entero**: todos sus pasos, cada uno idempotente
//! ([`crate::almacen`]). Si sale, la operación queda `hecha` y el recurso,
//! `listo`. Si no:
//!
//! - lo pasajero (el tenant aún no está activo, el controller no contesta) se
//!   reintenta, esperando más cuantos más intentos lleva (1, 2, 4… hasta 60 s);
//! - lo definitivo, o pasados [`INTENTOS`], deja la operación `fallida` con el
//!   motivo, y el recurso, `fallido`.
//!
//! ⭐ Un reinicio no pierde nada: lo que estaba en curso sigue en la base, con
//!   sus ids de tenant y timeline ya guardados, y se retoma donde iba.
//!
//! ⛔ Uno solo: `ore-postgres` es una réplica con `Recreate`. Dos
//!   reconciliadores sobre el mismo estado no son más disponibilidad.

use crate::almacen::{Almacen, Fallo, Origen};
use crate::base::{conectar, mal};
use crate::computos::{Computos, Vm};
use crate::especificacion::{CU_DE_LAS_CONEXIONES, Datos};
use postgres::Client;
use std::sync::Arc;
use std::time::Duration;

/// Cuántos intentos antes de rendirse. Con la espera creciente, unos 30 min.
pub const INTENTOS: i32 = 40;

/// Cada cuánto mira si hay algo que hacer.
const CADA: Duration = Duration::from_secs(1);

/// Y con operaciones en curso (P6·4: un despertar tiene a un cliente esperando).
const CADA_OCUPADO: Duration = Duration::from_millis(200);

/// Lo más que se espera a algo que «va bien y lleva su tiempo» (una VM que
/// arranca: ~35 s medidos; un nodo nuevo del pool: ~3,5 min, P3·2).
pub const PLAZO_ESPERA: f64 = 15.0 * 60.0;

/// En un hilo, para siempre. Con su propia conexión: la del API no se comparte.
pub fn arrancar(url: String, almacen: Arc<dyn Almacen>, computos: Arc<dyn Computos>) {
    std::thread::spawn(move || {
        let mut base: Option<Client> = None;
        let mut ultima_vigilancia = std::time::Instant::now();
        loop {
            if base.as_ref().is_none_or(|c| c.is_closed()) {
                base = match conectar(&url) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        eprintln!("reconciliador · {e}");
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };
            }
            // Con operaciones en curso, la vuelta siguiente enseguida (un despertar espera por
            // ellas); sin nada, cada segundo.
            let mut ocupado = false;
            if let Some(c) = base.as_mut() {
                match vuelta(c, almacen.as_ref(), computos.as_ref()) {
                    Ok(n) => ocupado = n > 0,
                    Err(e) => eprintln!("reconciliador · {e}"),
                }
            }
            // P6·3: la actividad de los cómputos encendidos, y quién duerme.
            if ultima_vigilancia.elapsed() >= VIGILAR_CADA {
                ultima_vigilancia = std::time::Instant::now();
                if let Some(c) = base.as_mut()
                    && let Err(e) = vigilar(c, computos.as_ref())
                {
                    eprintln!("reconciliador · vigilar: {e}");
                }
            }
            std::thread::sleep(if ocupado { CADA_OCUPADO } else { CADA });
        }
    });
}

/// P6·3 · Cada cuánto se pregunta su `last_active` a los cómputos encendidos.
pub const VIGILAR_CADA: Duration = Duration::from_secs(5);

/// P6·3 · **Vigilar**: pregunta su `last_active` a cada cómputo encendido y lo apunta; a los que
/// llevan más de su `dormir_tras` sin actividad (0 = nunca), una operación `dormir-endpoint`.
/// Es una operación a propósito: respeta el cerco de una por proyecto (no duerme a uno que se
/// está configurando) y queda en el historial. Devuelve cuántos mandó a dormir.
pub fn vigilar(c: &mut Client, k: &dyn Computos) -> Result<usize, String> {
    let vivos = c
        .query(
            "select e.organizacion, e.proyecto, e.id, e.vm, e.ip_pod from plano.endpoint e
              where e.deseado = 'vivo' and e.observado = 'listo' and e.ip_pod is not null",
            &[],
        )
        .map_err(mal)?;
    for f in &vivos {
        let (org, p, id, vm, ip): (String, String, String, String, String) =
            (f.get(0), f.get(1), f.get(2), f.get(3), f.get(4));
        // Si no contesta, no se apunta nada: un cómputo que no contesta no se duerme por eso.
        if let Ok(Some(t)) = k.actividad(&vm, &ip) {
            apuntar_actividad(c, &org, &p, &id, &t)?;
        }
    }
    let mut dormidos = 0;
    for f in c
        .query(
            "select e.organizacion, e.proyecto, e.rama, e.id from plano.endpoint e
              where e.deseado = 'vivo' and e.observado = 'listo' and e.dormir_tras > 0
                and e.ultima_actividad < now() - make_interval(secs => e.dormir_tras)",
            &[],
        )
        .map_err(mal)?
    {
        let (org, p, rama, id): (String, String, String, String) =
            (f.get(0), f.get(1), f.get(2), f.get(3));
        match c.execute(
            "insert into plano.operacion (id, organizacion, proyecto, tipo, celda, rama, endpoint)
             values ('op_' || replace(gen_random_uuid()::text, '-', ''), $1, $2, 'dormir-endpoint',
                     'reconciliador', $3, $4)",
            &[&org, &p, &rama, &id],
        ) {
            Ok(_) => dormidos += 1,
            // Otra operación en curso en el proyecto: se mira en la vigilancia siguiente.
            Err(e) if crate::base::choca(&e, "una_en_curso_por_proyecto") => {}
            Err(e) => return Err(mal(e)),
        }
    }
    Ok(dormidos)
}

/// Apunta un `last_active`; sólo hacia delante.
fn apuntar_actividad(c: &mut Client, org: &str, p: &str, id: &str, t: &str) -> Result<(), String> {
    c.execute(
        "update plano.endpoint
            set ultima_actividad = greatest(coalesce(ultima_actividad, $4::text::timestamptz),
                                            $4::text::timestamptz)
          where organizacion = $1 and proyecto = $2 and id = $3",
        &[&org, &p, &id, &t],
    )
    .map(|_| ())
    .map_err(mal)
}

/// Una vuelta: lo que toca ahora. Devuelve cuántas operaciones miró.
pub fn vuelta(c: &mut Client, a: &dyn Almacen, k: &dyn Computos) -> Result<usize, String> {
    let pendientes = c
        .query(
            "select id, tipo, organizacion, proyecto, rama, intentos, endpoint,
                    extract(epoch from now() - creada)::float8
               from plano.operacion
              where estado = 'en-curso' and siguiente <= now()
              order by creada limit 20",
            &[],
        )
        .map_err(mal)?;
    for f in &pendientes {
        let op = Op {
            id: f.get(0),
            tipo: f.get(1),
            organizacion: f.get(2),
            proyecto: f.get(3),
            rama: f.get(4),
            intentos: f.get(5),
            endpoint: f.get(6),
            edad: f.get(7),
        };
        let resultado = intentar(c, a, k, &op);
        cerrar(c, &op, resultado)?;
    }
    Ok(pendientes.len())
}

struct Op {
    id: String,
    tipo: String,
    organizacion: String,
    proyecto: String,
    rama: Option<String>,
    intentos: i32,
    endpoint: Option<String>,
    /// Segundos desde que se pidió.
    edad: f64,
}

/// Un endpoint, con lo que hace falta para su VM.
struct Ep {
    id: String,
    vm: String,
    rama: String,
    timeline: String,
    lectura: bool,
    cu_min: f64,
    cu_max: f64,
    generacion: i64,
    /// De ellas salen las conexiones ([`crate::especificacion::CU_DE_LAS_CONEXIONES`]).
    cu_conexiones: f64,
}

fn endpoints(
    c: &mut Client,
    org: &str,
    p: &str,
    uno: Option<&str>,
) -> Result<Vec<Ep>, postgres::Error> {
    Ok(c.query(
        &format!(
            "select e.id, e.vm, e.rama, r.timeline, e.tipo = 'lectura', e.cu_min, e.cu_max, e.generacion,
                    {CU_DE_LAS_CONEXIONES}
               from plano.endpoint e
               join plano.rama r on r.organizacion = e.organizacion and r.proyecto = e.proyecto and r.id = e.rama
              where e.organizacion = $1 and e.proyecto = $2 and ($3::text is null or e.id = $3)
              order by e.creado"
        ),
        &[&org, &p, &uno],
    )?
    .iter()
    .map(|f| Ep {
        id: f.get(0),
        vm: f.get(1),
        rama: f.get(2),
        timeline: f.get(3),
        lectura: f.get(4),
        cu_min: f.get(5),
        cu_max: f.get(6),
        generacion: f.get(7),
        cu_conexiones: f.get(8),
    })
    .collect())
}

/// Lleva un endpoint hasta `listo`: su VM existe y su `compute_ctl` dice
/// `running`. Cada paso es idempotente; lo que lleva tiempo es `Esperar`.
fn asegurar_endpoint(
    c: &mut Client,
    a: &dyn Almacen,
    k: &dyn Computos,
    org: &str,
    p: &str,
    tenant: &str,
    ep: &Ep,
) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    let marcar = |c: &mut Client, observado: &str| {
        c.execute(
            "update plano.endpoint set observado = $4
              where organizacion = $1 and proyecto = $2 and id = $3",
            &[&org, &p, &ep.id, &observado],
        )
        .map_err(bd)
    };
    let estado = match k.estado(&ep.vm)? {
        Some(e) => e,
        None => {
            // ⭐ El cerco, capa 2: no nace mientras quede un runner con su nombre.
            if k.runner_vivo(&ep.vm)? {
                return Err(Fallo::Esperar(
                    "queda el runner de una VM anterior con este nombre".into(),
                ));
            }
            let pageserver = a.pageserver_de(tenant)?;
            let datos = datos_de(c, org, p, &ep.rama)?;
            let configuracion = k
                .configuracion(
                    &ep.vm,
                    tenant,
                    &ep.timeline,
                    &pageserver,
                    p,
                    ep.lectura,
                    &datos,
                    ep.cu_conexiones,
                )
                .pretty();
            k.crear(
                &Vm {
                    nombre: &ep.vm,
                    organizacion: org,
                    proyecto: p,
                    endpoint: &ep.id,
                    cu_min: ep.cu_min,
                    cu_max: ep.cu_max,
                    generacion: ep.generacion,
                },
                &configuracion,
            )?;
            marcar(c, "arrancando")?;
            // Se mira otra vez ya: casi siempre seguirá naciendo (y se espera), pero
            // así una VM que nace lista no cuesta una vuelta más.
            k.estado(&ep.vm)?
                .ok_or_else(|| Fallo::Esperar("la VM está naciendo".into()))?
        }
    };
    match estado {
        e if e.fase == "Failed" || e.fase == "Succeeded" => Err(Fallo::Definitivo(format!(
            "la VM {} (rama {}) terminó ({})",
            ep.vm, ep.rama, e.fase
        ))),
        e => {
            let (true, Some(ip)) = (e.fase == "Running", e.ip_pod.as_deref()) else {
                return Err(Fallo::Esperar(format!("la VM está {}", e.fase)));
            };
            if !k.listo(&ep.vm, ip)? {
                return Err(Fallo::Esperar("compute_ctl aún no dice running".into()));
            }
            c.execute(
                "update plano.endpoint set observado = 'listo', direccion = $4, ip_pod = $5,
                        ultima_actividad = now(), dormido_en = null
                  where organizacion = $1 and proyecto = $2 and id = $3",
                &[&org, &p, &ep.id, &e.ip_overlay, &ip],
            )
            .map_err(bd)?;
            // Un cómputo de escritura arrancó con lo borrado en su especificación: aplicado.
            if !ep.lectura {
                let datos = datos_de(c, org, p, &ep.rama)?;
                purgar(c, org, p, &ep.rama, &datos)?;
            }
            Ok(())
        }
    }
}

/// Quita un endpoint: su VM y su ConfigMap, espera a que su runner se vaya, y
/// entonces la fila.
fn quitar_endpoint(
    c: &mut Client,
    k: &dyn Computos,
    org: &str,
    p: &str,
    ep: &Ep,
) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    k.borrar(&ep.vm)?;
    if k.estado(&ep.vm)?.is_some() || k.runner_vivo(&ep.vm)? {
        c.execute(
            "update plano.endpoint set observado = 'borrando'
              where organizacion = $1 and proyecto = $2 and id = $3",
            &[&org, &p, &ep.id],
        )
        .map_err(bd)?;
        return Err(Fallo::Esperar("esperando a que su runner se vaya".into()));
    }
    c.execute(
        "delete from plano.endpoint where organizacion = $1 and proyecto = $2 and id = $3",
        &[&org, &p, &ep.id],
    )
    .map_err(bd)?;
    Ok(())
}

/// Los roles y las bases de una rama, para su especificación (P4·4).
pub fn datos_de(c: &mut Client, org: &str, p: &str, rama: &str) -> Result<Datos, Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    let mut d = Datos::default();
    for f in c
        .query(
            "select nombre, verificador, deseado = 'vivo' from plano.rol
              where organizacion = $1 and proyecto = $2 and rama = $3 order by nombre",
            &[&org, &p, &rama],
        )
        .map_err(bd)?
    {
        if f.get::<_, bool>(2) {
            d.roles.push((f.get(0), f.get(1)));
        } else {
            d.roles_borrados.push(f.get(0));
        }
    }
    for f in c
        .query(
            "select nombre, dueno, deseado = 'vivo' from plano.base
              where organizacion = $1 and proyecto = $2 and rama = $3 order by nombre",
            &[&org, &p, &rama],
        )
        .map_err(bd)?
    {
        if f.get::<_, bool>(2) {
            d.bases.push((f.get(0), f.get(1)));
        } else {
            d.bases_borradas.push(f.get(0));
        }
    }
    Ok(d)
}

/// Lo borrado que un cómputo de escritura ya aplicó: ahora sí, fuera la fila.
fn purgar(c: &mut Client, org: &str, p: &str, rama: &str, d: &Datos) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    c.execute(
        "delete from plano.base where organizacion = $1 and proyecto = $2 and rama = $3
            and deseado = 'borrado' and nombre = any($4)",
        &[&org, &p, &rama, &d.bases_borradas],
    )
    .map_err(bd)?;
    c.execute(
        "delete from plano.rol where organizacion = $1 and proyecto = $2 and rama = $3
            and deseado = 'borrado' and nombre = any($4)",
        &[&org, &p, &rama, &d.roles_borrados],
    )
    .map_err(bd)?;
    Ok(())
}

fn tenant_de(c: &mut Client, org: &str, p: &str) -> Result<Option<String>, Fallo> {
    Ok(c.query_opt(
        "select tenant from plano.proyecto where organizacion = $1 and id = $2",
        &[&org, &p],
    )
    .map_err(|e| Fallo::Reintentar(format!("la base: {}", mal(e))))?
    .and_then(|f| f.get(0)))
}

/// Un intento entero de una operación. `Ok` es que ya está.
fn intentar(c: &mut Client, a: &dyn Almacen, k: &dyn Computos, op: &Op) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    match op.tipo.as_str() {
        "crear-proyecto" => {
            let f = c
                .query_one(
                    "select p.tenant, r.timeline from plano.proyecto p
                       join plano.rama r on r.organizacion = p.organizacion and r.proyecto = p.id
                      where p.organizacion = $1 and p.id = $2 and r.padre is null",
                    &[&op.organizacion, &op.proyecto],
                )
                .map_err(bd)?;
            let (tenant, main): (String, String) = (f.get(0), f.get(1));
            a.asegurar_tenant(&tenant)?;
            a.asegurar_timeline(&tenant, &main, None)?;
            c.execute(
                "update plano.rama set observado = 'lista'
                  where organizacion = $1 and proyecto = $2 and padre is null",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            // P4·3·3: y su endpoint de escritura en main.
            for ep in endpoints(c, &op.organizacion, &op.proyecto, None).map_err(bd)? {
                asegurar_endpoint(c, a, k, &op.organizacion, &op.proyecto, &tenant, &ep)?;
            }
            c.execute(
                "update plano.proyecto set observado = 'listo' where organizacion = $1 and id = $2",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            Ok(())
        }
        "borrar-proyecto" => {
            // Primero sus cómputos: que nadie escriba en un tenant que se borra.
            for ep in endpoints(c, &op.organizacion, &op.proyecto, None).map_err(bd)? {
                quitar_endpoint(c, k, &op.organizacion, &op.proyecto, &ep)?;
            }
            if let Some(t) = tenant_de(c, &op.organizacion, &op.proyecto)? {
                a.borrar_tenant(&t)?;
            }
            // Las ramas se van con él (on delete cascade).
            c.execute(
                "delete from plano.proyecto where organizacion = $1 and id = $2",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            Ok(())
        }
        "crear-rama" => {
            let rama = op.rama.as_deref().unwrap_or_default();
            let f = c
                .query_one(
                    "select p.tenant, r.timeline, pa.timeline, r.lsn_origen,
                            to_char(r.instante_origen at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')
                       from plano.rama r
                       join plano.proyecto p on p.organizacion = r.organizacion and p.id = r.proyecto
                       join plano.rama pa on pa.organizacion = r.organizacion and pa.proyecto = r.proyecto
                                         and pa.id = r.padre
                      where r.organizacion = $1 and r.proyecto = $2 and r.id = $3",
                    &[&op.organizacion, &op.proyecto, &rama],
                )
                .map_err(bd)?;
            let (tenant, timeline, padre): (String, String, String) =
                (f.get(0), f.get(1), f.get(2));
            let mut lsn: Option<String> = f.get(3);
            // El instante se resuelve UNA vez y se guarda: un reintento sale del
            // mismo LSN aunque entretanto se haya escrito más.
            if lsn.is_none()
                && let Some(instante) = f.get::<_, Option<String>>(4)
            {
                let l = a.lsn_en_instante(&tenant, &padre, &instante)?;
                c.execute(
                    "update plano.rama set lsn_origen = $4
                      where organizacion = $1 and proyecto = $2 and id = $3",
                    &[&op.organizacion, &op.proyecto, &rama, &l],
                )
                .map_err(bd)?;
                lsn = Some(l);
            }
            a.asegurar_timeline(
                &tenant,
                &timeline,
                Some(Origen {
                    timeline: &padre,
                    lsn: lsn.as_deref(),
                }),
            )?;
            c.execute(
                "update plano.rama set observado = 'lista'
                  where organizacion = $1 and proyecto = $2 and id = $3",
                &[&op.organizacion, &op.proyecto, &rama],
            )
            .map_err(bd)?;
            Ok(())
        }
        "borrar-rama" => {
            let rama = op.rama.as_deref().unwrap_or_default();
            let f = c
                .query_opt(
                    "select p.tenant, r.timeline from plano.rama r
                       join plano.proyecto p on p.organizacion = r.organizacion and p.id = r.proyecto
                      where r.organizacion = $1 and r.proyecto = $2 and r.id = $3",
                    &[&op.organizacion, &op.proyecto, &rama],
                )
                .map_err(bd)?;
            if let Some(f) = f {
                let (tenant, timeline): (String, String) = (f.get(0), f.get(1));
                a.borrar_timeline(&tenant, &timeline)?;
            }
            c.execute(
                "delete from plano.rama where organizacion = $1 and proyecto = $2 and id = $3",
                &[&op.organizacion, &op.proyecto, &rama],
            )
            .map_err(bd)?;
            Ok(())
        }
        // P4·4: roles o bases de una rama cambiaron. Se le aplica la especificación
        // nueva a su cómputo de escritura, si hay uno en marcha; si no, la llevará
        // el siguiente que arranque. Lo borrado sólo se purga cuando se aplicó.
        "configurar-rama" => {
            let rama = op.rama.as_deref().unwrap_or_default();
            let tenant = tenant_de(c, &op.organizacion, &op.proyecto)?
                .ok_or_else(|| Fallo::Definitivo("el proyecto ya no está".into()))?;
            let escritor = c
                .query_opt(
                    "select e.vm, e.ip_pod, r.timeline, e.cu_max from plano.endpoint e
                       join plano.rama r on r.organizacion = e.organizacion and r.proyecto = e.proyecto
                                        and r.id = e.rama
                      where e.organizacion = $1 and e.proyecto = $2 and e.rama = $3
                        and e.tipo = 'lectura-escritura' and e.deseado = 'vivo' and e.observado = 'listo'",
                    &[&op.organizacion, &op.proyecto, &rama],
                )
                .map_err(bd)?;
            let Some(f) = escritor else {
                return Ok(());
            };
            let (vm, ip, timeline, cu): (String, Option<String>, String, f64) =
                (f.get(0), f.get(1), f.get(2), f.get(3));
            let ip = ip.ok_or_else(|| Fallo::Reintentar("el endpoint no tiene IP".into()))?;
            let datos = datos_de(c, &op.organizacion, &op.proyecto, rama)?;
            let pageserver = a.pageserver_de(&tenant)?;
            let configuracion = k.configuracion(
                &vm,
                &tenant,
                &timeline,
                &pageserver,
                &op.proyecto,
                false,
                &datos,
                cu,
            );
            k.configurar(&vm, &ip, &configuracion)?;
            purgar(c, &op.organizacion, &op.proyecto, rama, &datos)
        }
        // P6·4: despertar es crear su cómputo otra vez, con la especificación de ahora (las CU
        // y las conexiones que se cambiaron dormido valen aquí, P6·2).
        "crear-endpoint" | "borrar-endpoint" | "despertar-endpoint" => {
            let id = op.endpoint.as_deref().unwrap_or_default();
            let Some(ep) = endpoints(c, &op.organizacion, &op.proyecto, Some(id))
                .map_err(bd)?
                .pop()
            else {
                // Ya no está la fila: si era borrar, está hecho; si era crear, alguien lo borró.
                return Ok(());
            };
            if op.tipo == "borrar-endpoint" {
                return quitar_endpoint(c, k, &op.organizacion, &op.proyecto, &ep);
            }
            let tenant = tenant_de(c, &op.organizacion, &op.proyecto)?
                .ok_or_else(|| Fallo::Definitivo("el proyecto ya no está".into()))?;
            asegurar_endpoint(c, a, k, &op.organizacion, &op.proyecto, &tenant, &ep)
        }
        // P5·6: quién entra lo comprueba el proxy, no el cómputo. No hay nada que
        // hacer aquí: al quedar hecha, `olvidar` le dice al proxy que lo relea.
        "configurar-acceso" => Ok(()),
        // P6·2: los límites del endpoint. Las CU y `max_connections` valen en el siguiente
        // arranque de su cómputo (no se reinicia uno vivo por esto); `dormir_tras` lo lee el
        // reconciliador al decidir si duerme (P6·3).
        "configurar-endpoint" => Ok(()),
        // P6·3 · Dormir: se mira otra vez (una consulta pudo empezar desde que se decidió), se
        // para limpio (`/terminate`, con su LSN), se borra el cómputo y, cuando no queda nada con
        // su nombre, el endpoint queda `dormido`: sin cómputo, coste 0; los datos, en el almacén.
        "dormir-endpoint" => dormir(c, k, op),
        otro => Err(Fallo::Definitivo(format!(
            "este reconciliador no sabe hacer `{otro}`"
        ))),
    }
}

/// P6·3 · La operación `dormir-endpoint`. Idempotente: `durmiendo` es «ya se paró, falta que su
/// cómputo desaparezca».
fn dormir(c: &mut Client, k: &dyn Computos, op: &Op) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    let id = op.endpoint.as_deref().unwrap_or_default();
    let (org, p) = (op.organizacion.as_str(), op.proyecto.as_str());
    let inactivo = |c: &mut Client| -> Result<bool, Fallo> {
        Ok(c.query_one(
            "select coalesce(dormir_tras > 0
                    and ultima_actividad < now() - make_interval(secs => dormir_tras), false)
               from plano.endpoint where organizacion = $1 and proyecto = $2 and id = $3",
            &[&org, &p, &id],
        )
        .map_err(bd)?
        .get(0))
    };
    let Some(f) = c
        .query_opt(
            "select vm, ip_pod, observado from plano.endpoint
              where organizacion = $1 and proyecto = $2 and id = $3 and deseado = 'vivo'",
            &[&org, &p, &id],
        )
        .map_err(bd)?
    else {
        return Ok(());
    };
    let (vm, ip, observado): (String, Option<String>, String) = (f.get(0), f.get(1), f.get(2));
    match observado.as_str() {
        "listo" => {
            let ip = ip.unwrap_or_default();
            if let Ok(Some(t)) = k.actividad(&vm, &ip) {
                apuntar_actividad(c, org, p, id, &t).map_err(Fallo::Reintentar)?;
            }
            if !inactivo(c)? {
                return Ok(()); // hubo actividad entretanto (o cambió dormir_tras): no se duerme
            }
            // Que no conteste a /terminate (ya parado, o roto) no impide dormirlo.
            let lsn = k.terminar(&vm, &ip).ok().flatten();
            c.execute(
                "update plano.endpoint set observado = 'durmiendo', lsn_al_dormir = $4
                  where organizacion = $1 and proyecto = $2 and id = $3",
                &[&org, &p, &id, &lsn],
            )
            .map_err(bd)?;
        }
        "durmiendo" => {}
        _ => return Ok(()),
    }
    k.borrar(&vm)?;
    if k.estado(&vm)?.is_some() || k.runner_vivo(&vm)? {
        return Err(Fallo::Esperar("esperando a que su runner se vaya".into()));
    }
    c.execute(
        "update plano.endpoint
            set observado = 'dormido', dormido_en = now(), direccion = null, ip_pod = null
          where organizacion = $1 and proyecto = $2 and id = $3",
        &[&org, &p, &id],
    )
    .map_err(bd)?;
    Ok(())
}

/// Apunta lo que salió.
fn cerrar(c: &mut Client, op: &Op, r: Result<(), Fallo>) -> Result<(), String> {
    match r {
        Ok(()) => {
            c.execute(
                "update plano.operacion set estado = 'hecha', terminada = now(), error = null
                  where id = $1",
                &[&op.id],
            )
            .map_err(mal)?;
        }
        // Lo que va bien y tarda: en 2 s otra vez, sin gastar intentos, dentro del plazo. Un
        // despertar, en 200 ms: hay un cliente esperando al otro lado del proxy (P6·4).
        Err(Fallo::Esperar(m)) if op.edad < PLAZO_ESPERA => {
            let espera = if op.tipo == "despertar-endpoint" {
                0.2
            } else {
                2.0
            };
            c.execute(
                "update plano.operacion
                    set error = $2, siguiente = now() + make_interval(secs => $3)
                  where id = $1",
                &[&op.id, &m, &espera],
            )
            .map_err(mal)?;
        }
        Err(Fallo::Reintentar(m)) if op.intentos + 1 < INTENTOS => {
            let espera = 2f64.powi(op.intentos.min(6)).min(60.0);
            c.execute(
                "update plano.operacion
                    set intentos = intentos + 1, error = $2,
                        siguiente = now() + make_interval(secs => $3)
                  where id = $1",
                &[&op.id, &m, &espera],
            )
            .map_err(mal)?;
        }
        Err(f) => {
            let m = format!("{} (tras {} intentos)", f.motivo(), op.intentos + 1);
            c.execute(
                "update plano.operacion
                    set estado = 'fallida', terminada = now(), error = $2, intentos = intentos + 1
                  where id = $1",
                &[&op.id, &m],
            )
            .map_err(mal)?;
            if op.tipo == "crear-proyecto" {
                c.execute(
                    "update plano.proyecto set observado = 'fallido' where organizacion = $1 and id = $2",
                    &[&op.organizacion, &op.proyecto],
                )
                .map_err(mal)?;
            }
            if op.tipo == "crear-endpoint" {
                c.execute(
                    "update plano.endpoint set observado = 'fallido'
                      where organizacion = $1 and proyecto = $2 and id = $3",
                    &[&op.organizacion, &op.proyecto, &op.endpoint],
                )
                .map_err(mal)?;
            }
            if op.tipo == "crear-rama" {
                c.execute(
                    "update plano.rama set observado = 'fallida'
                      where organizacion = $1 and proyecto = $2 and id = $3",
                    &[&op.organizacion, &op.proyecto, &op.rama],
                )
                .map_err(mal)?;
            }
            eprintln!("reconciliador · {} {} fallida: {m}", op.tipo, op.id);
        }
    }
    Ok(())
}
