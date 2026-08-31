//! El **artefacto de topología**: las aristas del grafo, fuera de las fuentes.
//!
//! Lo decidió el [ADR 0006](../../docs/decisions/0006-el-artefacto-de-topologia.md):
//! ORE no opera ninguna base de datos. La topología es **la misma clase de cosa
//! que el plano de contexto** — se construye una vez, se firma, se distribuye y
//! se mapea— y cambia el tamaño y la cadencia, no la naturaleza.
//!
//! # Por qué esto es lo que hace asequible la ley de §2
//!
//! > **El índice convierte escaneos en búsquedas por clave.**
//!
//! La travesía ocurre **en local, sobre las aristas materializadas**, así que
//! cuando el motor abre una conexión ya sabe exactamente qué claves pide. Por eso
//! puede permitirse no compensar lo que la fuente no sabe hacer: casi nunca lo
//! necesita.
//!
//! # CSR, y por qué su limitación documentada es nuestro modo de operación
//!
//! *Compressed sparse row*: para cada relación, las claves origen **ordenadas**,
//! un array de desplazamientos y un array de destinos. La literatura dice que no
//! admite actualizaciones dinámicas sin reconstruir el array de aristas entero —
//! y eso es un **defecto** si estás construyendo una base de datos y un
//! **no-problema** si tu índice se reconstruye por ventana.
//!
//! # La forma del fichero
//!
//! Todo entero es little-endian de 32 bits, y las claves se **internan** en una
//! tabla ordenada: una arista es un par de índices, no un par de cadenas.
//!
//! ```text
//! "ORETOPO1"          8 bytes
//! digest              longitud + bytes   ← contra qué bundle se construyó
//! marca de agua       longitud + bytes   ← hasta cuándo era cierto
//! tabla de claves     n, luego (longitud + bytes) ORDENADAS
//! relaciones          n, y por cada una:
//!                       nombre           longitud + bytes
//!                       n_origenes
//!                       origenes[]       índices en la tabla, ASCENDENTES
//!                       offsets[n+1]
//!                       destinos[]       índices en la tabla
//! ```
//!
//! La tabla ordenada y los orígenes ascendentes son lo que hace el fichero
//! **determinista**: dos construcciones sobre la misma instantánea dan los mismos
//! bytes. Es G1 otra vez, y no es cosmética — un índice que difiera entre nodos
//! hace que dos nodos contesten distinto a la misma pregunta.
//!
//! # Lo que v1 NO hace, y se dice
//!
//! **No se mapea en memoria: se lee entero.** El formato está preparado —anchuras
//! fijas, sin analizar nada— pero `mmap` es una dependencia, y se paga cuando el
//! artefacto sea lo bastante grande para que se note. Afirmar que se mapea sin
//! mapearlo sería exactamente la clase de promesa que este proyecto no hace.

use std::collections::{BTreeMap, BTreeSet};

/// Los tres arrays de CSR: orígenes ordenados, desplazamientos y destinos.
type Csr = (Vec<u32>, Vec<u32>, Vec<u32>);

const MAGIA: &[u8; 8] = b"ORETOPO1";

/// Una arista: `(relación cualificada, clave origen, clave destino)`.
pub type Arista = (String, String, String);

#[derive(Debug, Default)]
pub struct Topologia {
    /// Contra qué bundle se construyó. Un plan que use un índice de otro bundle
    /// es una **condición nombrada**, no una junta silenciosa.
    pub digest: String,
    /// Hasta cuándo era cierto (`05-ejecutor` §7).
    pub marca: String,
    claves: Vec<String>,
    /// relación → sus tres arrays de CSR.
    relaciones: BTreeMap<String, Csr>,
}

fn u32le(v: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn cadena(s: &str, out: &mut Vec<u8>) {
    u32le(s.len() as u32, out);
    out.extend_from_slice(s.as_bytes());
}

impl Topologia {
    /// Construye el índice a partir de las aristas. **Es una función pura**: no
    /// abre nada. Leer las aristas de una fuente es el otro acto, y se pide por
    /// separado porque falla por separado — la misma figura que `discover`.
    pub fn construir(digest: &str, marca: &str, aristas: &[Arista]) -> Topologia {
        // La tabla se ordena: es de donde sale el determinismo.
        let mut vocabulario: BTreeSet<&str> = BTreeSet::new();
        for (_, o, d) in aristas {
            vocabulario.insert(o);
            vocabulario.insert(d);
        }
        let claves: Vec<String> = vocabulario.iter().map(|s| s.to_string()).collect();
        let indice: BTreeMap<&str, u32> = claves
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i as u32))
            .collect();

        // Agrupadas por relación y por origen, con los destinos ordenados y sin
        // repetir: dos veces la misma arista es una arista.
        let mut por_relacion: BTreeMap<&str, BTreeMap<u32, BTreeSet<u32>>> = BTreeMap::new();
        for (rel, o, d) in aristas {
            por_relacion
                .entry(rel.as_str())
                .or_default()
                .entry(indice[o.as_str()])
                .or_default()
                .insert(indice[d.as_str()]);
        }

        let relaciones = por_relacion
            .into_iter()
            .map(|(rel, mapa)| {
                let mut origenes = Vec::with_capacity(mapa.len());
                let mut offsets = Vec::with_capacity(mapa.len() + 1);
                let mut destinos = Vec::new();
                offsets.push(0u32);
                for (o, ds) in mapa {
                    origenes.push(o);
                    destinos.extend(ds.iter().copied());
                    offsets.push(destinos.len() as u32);
                }
                (rel.to_string(), (origenes, offsets, destinos))
            })
            .collect();

        Topologia {
            digest: digest.to_string(),
            marca: marca.to_string(),
            claves,
            relaciones,
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIA);
        cadena(&self.digest, &mut out);
        cadena(&self.marca, &mut out);
        u32le(self.claves.len() as u32, &mut out);
        for k in &self.claves {
            cadena(k, &mut out);
        }
        u32le(self.relaciones.len() as u32, &mut out);
        for (rel, (origenes, offsets, destinos)) in &self.relaciones {
            cadena(rel, &mut out);
            u32le(origenes.len() as u32, &mut out);
            for o in origenes {
                u32le(*o, &mut out);
            }
            for f in offsets {
                u32le(*f, &mut out);
            }
            u32le(destinos.len() as u32, &mut out);
            for d in destinos {
                u32le(*d, &mut out);
            }
        }
        out
    }

    pub fn leer(b: &[u8]) -> Result<Topologia, String> {
        let mut i = 0usize;
        let tomar = |i: &mut usize, n: usize, b: &[u8]| -> Result<Vec<u8>, String> {
            if *i + n > b.len() {
                return Err("el artefacto está truncado".into());
            }
            let v = b[*i..*i + n].to_vec();
            *i += n;
            Ok(v)
        };
        if tomar(&mut i, 8, b)? != MAGIA {
            return Err("esto no es un artefacto de topología".into());
        }
        let u32de = |i: &mut usize| -> Result<u32, String> {
            let v = tomar(i, 4, b)?;
            Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        };
        let cadena_de = |i: &mut usize| -> Result<String, String> {
            let n = u32de(i)? as usize;
            String::from_utf8(tomar(i, n, b)?).map_err(|e| format!("clave no UTF-8: {e}"))
        };

        let digest = cadena_de(&mut i)?;
        let marca = cadena_de(&mut i)?;
        let n = u32de(&mut i)? as usize;
        let mut claves = Vec::with_capacity(n);
        for _ in 0..n {
            claves.push(cadena_de(&mut i)?);
        }
        let nrel = u32de(&mut i)? as usize;
        let mut relaciones = BTreeMap::new();
        for _ in 0..nrel {
            let rel = cadena_de(&mut i)?;
            let no = u32de(&mut i)? as usize;
            let mut origenes = Vec::with_capacity(no);
            for _ in 0..no {
                origenes.push(u32de(&mut i)?);
            }
            let mut offsets = Vec::with_capacity(no + 1);
            for _ in 0..=no {
                offsets.push(u32de(&mut i)?);
            }
            let nd = u32de(&mut i)? as usize;
            let mut destinos = Vec::with_capacity(nd);
            for _ in 0..nd {
                destinos.push(u32de(&mut i)?);
            }
            relaciones.insert(rel, (origenes, offsets, destinos));
        }
        Ok(Topologia {
            digest,
            marca,
            claves,
            relaciones,
        })
    }

    /// Los vecinos directos de una clave por una relación.
    fn vecinos(&self, relacion: &str, clave: &str) -> Vec<&str> {
        let Some((origenes, offsets, destinos)) = self.relaciones.get(relacion) else {
            return Vec::new();
        };
        let Ok(k) = self.claves.binary_search(&clave.to_string()) else {
            return Vec::new();
        };
        // Búsqueda binaria: los orígenes están ascendentes, que es media razón
        // de que el formato los ordene.
        let Ok(pos) = origenes.binary_search(&(k as u32)) else {
            return Vec::new();
        };
        destinos[offsets[pos] as usize..offsets[pos + 1] as usize]
            .iter()
            .filter_map(|d| self.claves.get(*d as usize).map(String::as_str))
            .collect()
    }

    /// La travesía: desde una raíz, N saltos, un conjunto de **claves**.
    ///
    /// No abre nada. Es lo que hace que la fase ② de `05-ejecutor` §3 pueda
    /// ocurrir antes de que exista ninguna conexión.
    pub fn travesia(&self, relacion: &str, desde: &str, saltos: usize) -> BTreeSet<String> {
        let mut vistos: BTreeSet<String> = BTreeSet::new();
        let mut frontera: Vec<String> = vec![desde.to_string()];
        for _ in 0..saltos {
            let mut siguiente = Vec::new();
            for k in &frontera {
                for v in self.vecinos(relacion, k) {
                    if vistos.insert(v.to_string()) {
                        siguiente.push(v.to_string());
                    }
                }
            }
            if siguiente.is_empty() {
                break;
            }
            frontera = siguiente;
        }
        vistos
    }

    /// Los ancestros de una clave: la travesía sin límite de profundidad.
    ///
    /// Es lo que cierra el hueco de M0. `principal in Employee::"…"` significa
    /// *«el que pregunta está bajo esa cadena»*, y `in` en Cedar es
    /// **alcanzabilidad transitiva**: darle los ancestros como padres del
    /// principal es exactamente lo que espera.
    pub fn ancestros(&self, relacion: &str, desde: &str) -> BTreeSet<String> {
        // El límite es el número de claves: un ciclo no puede visitar más.
        self.travesia(relacion, desde, self.claves.len().max(1))
    }

    pub fn relaciones(&self) -> Vec<&str> {
        self.relaciones.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cadena_de_mando() -> Vec<Arista> {
        vec![
            ("hr.Employee.manager".into(), "emp-7".into(), "emp-3".into()),
            ("hr.Employee.manager".into(), "emp-9".into(), "emp-3".into()),
            ("hr.Employee.manager".into(), "emp-3".into(), "ceo".into()),
        ]
    }

    /// **G1 aplicado al índice.** Un índice que difiera entre nodos hace que dos
    /// nodos contesten distinto a la misma pregunta, que es la peor forma de
    /// fallar.
    #[test]
    fn dos_construcciones_sobre_la_misma_instantanea_dan_los_mismos_bytes() {
        let a = Topologia::construir("sha256:x", "2026-08-31T10:00:00Z", &cadena_de_mando());
        // Y el orden de llegada de las aristas no puede cambiar el resultado: la
        // tabla se ordena, que es de donde sale el determinismo.
        let mut revueltas = cadena_de_mando();
        revueltas.reverse();
        let b = Topologia::construir("sha256:x", "2026-08-31T10:00:00Z", &revueltas);
        assert_eq!(a.bytes(), b.bytes());
    }

    #[test]
    fn la_travesia_no_abre_nada_y_devuelve_claves() {
        let t = Topologia::construir("sha256:x", "w", &cadena_de_mando());
        let uno = t.travesia("hr.Employee.manager", "emp-7", 1);
        assert_eq!(uno, ["emp-3".to_string()].into());
        let dos = t.travesia("hr.Employee.manager", "emp-7", 2);
        assert_eq!(dos, ["emp-3".to_string(), "ceo".to_string()].into());
    }

    /// Y la cadena entera, que es lo que Cedar espera de `in`.
    #[test]
    fn los_ancestros_son_la_cadena_completa() {
        let t = Topologia::construir("sha256:x", "w", &cadena_de_mando());
        assert_eq!(
            t.ancestros("hr.Employee.manager", "emp-9"),
            ["emp-3".to_string(), "ceo".to_string()].into()
        );
        // El CEO no reporta a nadie.
        assert!(t.ancestros("hr.Employee.manager", "ceo").is_empty());
    }

    #[test]
    fn el_artefacto_va_y_vuelve() {
        let a = Topologia::construir("sha256:abc", "2026-08-31", &cadena_de_mando());
        let b = Topologia::leer(&a.bytes()).expect("se lee");
        assert_eq!(b.digest, "sha256:abc");
        assert_eq!(b.marca, "2026-08-31");
        assert_eq!(b.relaciones(), vec!["hr.Employee.manager"]);
        assert_eq!(b.bytes(), a.bytes());
    }

    /// Un fichero que no es esto se dice, en vez de leerse a medias.
    #[test]
    fn lo_que_no_es_un_artefacto_no_se_intenta_interpretar() {
        assert!(Topologia::leer(b"otra cosa").is_err());
        let a = Topologia::construir("d", "w", &cadena_de_mando()).bytes();
        assert!(Topologia::leer(&a[..a.len() / 2]).is_err());
    }
}
