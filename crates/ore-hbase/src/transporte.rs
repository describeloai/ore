//! **Hablar con HBase**: abrir el cliente nativo con lo que dice la URL, traducir sus errores a los siete codigos
//! del protocolo, y **muestrear** una tabla para saber que columnas tiene (ficha 0072, §7.5).

use crate::esquema::{Columna, Tabla, Tipo, es_texto};
use crate::url::{Destino, analizar};
use ore_driver::{Codigo, Fallo};
use ore_hbase_rpc::{Cancelacion, Cliente, Consulta, Error};
use std::collections::BTreeMap;

/// Una conexion: el cliente (con su cache de regiones) y a donde apunta.
pub struct Conexion {
    pub cliente: Cliente,
    pub destino: Destino,
}

impl Conexion {
    /// Entiende la URL y prepara el cliente. **No toca la red**: los fallos de red llegan con la primera lectura.
    pub fn abrir(url: &str) -> Result<Conexion, Fallo> {
        let destino = analizar(url).map_err(Fallo::operador)?;
        Conexion::de(destino, None)
    }

    pub fn de(destino: Destino, timeout_ms: Option<u64>) -> Result<Conexion, Fallo> {
        let o = destino.opciones(timeout_ms).map_err(Fallo::operador)?;
        let cliente = Cliente::nuevo(&destino.zk, &destino.znode, o).map_err(fallo_de)?;
        Ok(Conexion { cliente, destino })
    }

    /// Otro cliente con el plazo de **esta** lectura (el de la peticion), sin tocar el de la sesion que `servir`
    /// reutiliza.
    pub fn con_tiempo(&self, ms: u64) -> Result<Conexion, Fallo> {
        Conexion::de(self.destino.clone(), Some(ms))
    }
}

/// El texto de un error de HBase para quien lee: sin la traza de Java.
fn texto(clase: &str, mensaje: &str) -> String {
    format!("{}: {mensaje}", clase.rsplit('.').next().unwrap_or(clase))
}

/// Un error del cliente, como uno de los siete codigos.
pub fn fallo_de(e: Error) -> Fallo {
    match e {
        Error::Cancelada => Fallo::origen("lectura cancelada"),
        Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            Fallo::new(Codigo::Tiempo, "HBase no contesto a tiempo")
        }
        Error::Io(io) => Fallo::new(Codigo::Conexion, format!("no se llega a HBase: {io}")),
        Error::Zk(m) => Fallo::new(Codigo::Conexion, format!("ZooKeeper: {m}")),
        Error::RegionSinServidor(r) => Fallo::new(
            Codigo::Conexion,
            format!("la region `{r}` no tiene servidor asignado (se esta moviendo)"),
        ),
        Error::Protocolo(m) => {
            Fallo::origen(format!("HBase contesto algo que no se entiende: {m}"))
        }
        Error::Peticion(m) => Fallo::operador(m),
        Error::Autenticacion(m) => Fallo::new(Codigo::Credencial, m),
        Error::TablaSinRegiones(t) => Fallo::new(
            Codigo::Objeto,
            format!("la tabla `{t}` no existe (o no tiene regiones)"),
        ),
        Error::TablaDeshabilitada(t) => Fallo::origen(format!(
            "la tabla `{t}` esta deshabilitada: habilitala para leerla"
        )),
        Error::Servidor { clase, mensaje, .. } => {
            let corta = clase.rsplit('.').next().unwrap_or(&clase);
            let codigo = match corta {
                "AccessDeniedException" | "BadAuthException" | "SecurityException" => {
                    Codigo::Credencial
                }
                "TableNotFoundException" | "UnknownRegionException" => Codigo::Objeto,
                "NoSuchColumnFamilyException" => Codigo::Esquema,
                "CallTimeoutException" | "RpcTimeoutException" => Codigo::Tiempo,
                _ => Codigo::Origen,
            };
            Fallo::new(codigo, texto(&clase, &mensaje))
        }
        Error::Agotado { intentos, ultimo } => {
            let f = fallo_de(*ultimo);
            // Lo que se reintentaba (una region, la red) y no volvio es un problema de conexion, no del origen.
            let codigo = if f.codigo == Codigo::Origen {
                Codigo::Conexion
            } else {
                f.codigo
            };
            Fallo::new(
                codigo,
                format!(
                    "tras {intentos} reintentos seguidos sin avanzar: {}",
                    f.mensaje
                ),
            )
        }
    }
}

/// Cuantas filas se miran para catalogar una tabla.
pub const FILAS_DE_MUESTRA: usize = 1_000;

/// **Muestrea** una tabla: unas filas del principio de cada region (repartidas, no todas de la primera) y lo que
/// tienen. De ahi salen las columnas y su tipo (`String` si todo lo visto es texto, `Opaque` si no).
pub fn muestrear(
    c: &Cliente,
    espacio: &str,
    nombre: &str,
    presupuesto: usize,
    cancelacion: &Cancelacion,
) -> Result<Tabla, Error> {
    let completo = crate::esquema::en_hbase(espacio, nombre);
    let regiones = c.regiones(&completo)?;
    let por_region = (presupuesto / regiones.len().max(1)).clamp(1, 1_000);
    // (familia, cualificador) -> (filas con valor, todos texto)
    let mut vistas: BTreeMap<(Vec<u8>, Vec<u8>), (usize, bool)> = BTreeMap::new();
    let (mut filas, mut clave_texto) = (0usize, true);
    for r in &regiones {
        let consulta = Consulta {
            inicio: r.inicio.clone(),
            fin: r.fin.clone(),
            filas_por_llamada: u32::try_from(por_region).unwrap_or(1_000),
            ..Consulta::default()
        };
        for f in c.leer(&completo, &consulta, cancelacion)?.take(por_region) {
            let f = f?;
            filas += 1;
            clave_texto &= es_texto(&f.clave);
            for celda in &f.celdas {
                let v = vistas
                    .entry((celda.familia.clone(), celda.cualificador.clone()))
                    .or_insert((0, true));
                v.0 += 1;
                v.1 &= es_texto(&celda.valor);
            }
        }
    }
    let mut avisos = Vec::new();
    let mut columnas = Vec::new();
    for ((familia, cualificador), (n, texto)) in vistas {
        if !es_texto(&familia) || !es_texto(&cualificador) || familia.is_empty() {
            avisos.push(format!(
                "una columna de {n} fila(s) tiene un nombre que no es texto (`{}`:`{}`): no se cataloga",
                String::from_utf8_lossy(&familia),
                String::from_utf8_lossy(&cualificador)
            ));
            continue;
        }
        columnas.push(Columna {
            nombre: format!(
                "{}:{}",
                String::from_utf8_lossy(&familia),
                String::from_utf8_lossy(&cualificador)
            ),
            tipo: if texto { Tipo::texto() } else { Tipo::opaco() },
            filas_con_valor: n,
        });
    }
    if filas == 0 {
        avisos.push("no tiene filas: solo se conoce su clave".to_string());
    }
    Ok(Tabla {
        espacio: espacio.to_string(),
        nombre: nombre.to_string(),
        clave: if clave_texto {
            Tipo::texto()
        } else {
            Tipo::opaco()
        },
        columnas,
        filas_muestreadas: filas,
        regiones: regiones.len(),
        avisos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn servidor(clase: &str) -> Error {
        Error::Servidor {
            clase: format!("org.apache.hadoop.hbase.{clase}"),
            mensaje: "m".into(),
            no_reintentar: false,
        }
    }

    #[test]
    fn cada_error_del_cliente_cae_en_uno_de_los_siete_codigos() {
        let codigo = |e: Error| fallo_de(e).codigo;
        assert_eq!(
            codigo(servidor("security.AccessDeniedException")),
            Codigo::Credencial
        );
        assert_eq!(codigo(servidor("TableNotFoundException")), Codigo::Objeto);
        assert_eq!(
            codigo(servidor("regionserver.NoSuchColumnFamilyException")),
            Codigo::Esquema
        );
        assert_eq!(codigo(servidor("RaraException")), Codigo::Origen);
        assert_eq!(codigo(Error::TablaSinRegiones("t".into())), Codigo::Objeto);
        assert_eq!(codigo(Error::Zk("x".into())), Codigo::Conexion);
        assert_eq!(
            codigo(Error::Io(std::io::ErrorKind::ConnectionRefused.into())),
            Codigo::Conexion
        );
        assert_eq!(
            codigo(Error::Io(std::io::ErrorKind::TimedOut.into())),
            Codigo::Tiempo
        );
        assert_eq!(codigo(Error::Peticion("x".into())), Codigo::Operador);
        assert_eq!(codigo(Error::Autenticacion("x".into())), Codigo::Credencial);
    }

    #[test]
    fn lo_que_se_agoto_reintentando_es_de_conexion_aunque_el_ultimo_fuera_del_origen() {
        let f = fallo_de(Error::Agotado {
            intentos: 15,
            ultimo: Box::new(servidor("NotServingRegionException")),
        });
        assert_eq!(f.codigo, Codigo::Conexion);
        assert!(
            f.mensaje.contains("15 reintentos") && f.mensaje.contains("NotServingRegionException"),
            "{}",
            f.mensaje
        );
        // pero una credencial rechazada sigue siendolo
        let f = fallo_de(Error::Agotado {
            intentos: 2,
            ultimo: Box::new(servidor("AccessDeniedException")),
        });
        assert_eq!(f.codigo, Codigo::Credencial);
    }

    #[test]
    fn el_mensaje_no_lleva_la_traza_ni_el_paquete_de_java() {
        let f = fallo_de(servidor("regionserver.WrongRegionException"));
        assert_eq!(f.mensaje, "WrongRegionException: m");
    }

    #[test]
    fn abrir_no_toca_la_red_pero_si_entiende_la_url() {
        assert!(Conexion::abrir("hbase://nadie.invalido:2181?simple=acepto").is_ok());
        assert_eq!(
            Conexion::abrir("hbase://zk").err().unwrap().codigo,
            Codigo::Operador
        );
    }
}
