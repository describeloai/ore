//! El log de transparencia: la aritmética, que es lo único que vive aquí.
//!
//! # Qué añade sobre una firma
//!
//! Una firma dice **de quién** es un paquete. No dice nada sobre si esa clave le
//! ha dicho lo mismo a todo el mundo. Quien tenga la clave puede firmar dos
//! `gdpr 0.2.0` distintos —uno para el auditor y otro para ti— y las dos firmas
//! verifican. Ninguna comprobación local puede distinguirlas, porque el defecto
//! no está en lo que tienes: está en lo que **no** tienes.
//!
//! Un log de transparencia es la respuesta que ya dieron la base de sumas de Go
//! y Sigstore: **todo lo que se firma se publica en una lista que solo crece, y
//! cualquiera puede comprobar que su copia es un prefijo de la tuya.** No impide
//! que alguien firme algo malo; garantiza que no lo pueda hacer **en privado**.
//!
//! Y es lo que hace demostrable la pregunta que el sector regulado hace de
//! verdad: *«¿qué decía la definición de dato personal el 14 de marzo?»*.
//!
//! # Dos pruebas, y hacen falta las dos
//!
//! | | Qué demuestra | Sin ella |
//! |---|---|---|
//! | **inclusión** | esta versión está en el log, en esta posición | el log podría no haberla visto nunca |
//! | **consistencia** | el log de hoy **extiende** el de ayer | el log podría haber reescrito el pasado |
//!
//! Con solo la primera, un log puede enseñarle a cada uno un árbol distinto y
//! todas las pruebas de inclusión cuadran: es la bifurcación, y es exactamente
//! el ataque contra el que existe la segunda.
//!
//! # Por qué esto vive DENTRO y el log fuera
//!
//! Por lo mismo que verificar una firma vive dentro: **una prueba de inclusión
//! se comprueba con SHA-256 y nada más**. No hay red, ni reloj, ni credencial —
//! solo hashes que ya están en el árbol. Servir el log es otra cosa, y vive
//! fuera como vive fuera traer un paquete.
//!
//! # La forma es la de RFC 6962
//!
//! Y no es una elección: es lo que permite que un log existente sirva a este
//! motor y que un tercero verifique con las herramientas que ya tiene. Un árbol
//! de Merkle propio habría sido el mismo cómputo con menos gente que lo sabe
//! leer.
//!
//! ```text
//! hoja(d) = SHA256(0x00 ‖ d)      nodo(a,b) = SHA256(0x01 ‖ a ‖ b)
//! ```
//!
//! Los prefijos separan los dos dominios, y no son decorativos: sin ellos, la
//! hoja cuyo contenido es la concatenación de dos hashes tendría el mismo hash
//! que el nodo que los une, y una hoja podría hacerse pasar por un subárbol.

use crate::json::Json;
use sha2::{Digest, Sha256};

/// Un hash del árbol. 32 bytes, como los del digest.
pub type Hash = [u8; 32];

/// `SHA256(0x00 ‖ datos)`.
pub fn hoja(datos: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update([0x00]);
    h.update(datos);
    h.finalize().into()
}

/// `SHA256(0x01 ‖ izquierda ‖ derecha)`.
pub fn nodo(izquierda: &Hash, derecha: &Hash) -> Hash {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(izquierda);
    h.update(derecha);
    h.finalize().into()
}

/// Lo que se anota en el log: **el enunciado firmado y quién lo firmó**.
///
/// No es el `.oob` ni su digest a secas. Lo que hay que poder demostrar es que
/// *esta clave dijo esto*, así que la hoja lleva las dos cosas — y lleva el
/// mismo enunciado que se firma, no una segunda forma de decir lo mismo. Dos
/// formas serían dos semánticas, y la que se anota tiene que ser la que se
/// comprueba.
pub fn entrada(enunciado: &str, key_id: &str, firma: &str) -> String {
    Json::obj([
        ("keyId", Json::s(key_id)),
        ("signature", Json::s(firma)),
        ("statement", Json::s(enunciado)),
    ])
    .jcs()
}

/// **La cabeza firmada**: lo que el log afirma de sí mismo en un momento dado.
///
/// Una prueba de inclusión demuestra que una hoja está en un árbol **con esta
/// raíz**, y no dice de dónde salió la raíz. Sin una cabeza firmada, cualquiera
/// construye un árbol con la hoja que quiera, calcula su raíz y presenta una
/// prueba impecable de algo que ningún log ha visto.
///
/// Es el mismo agujero que `trustedKeys` cierra para un paquete, un piso más
/// arriba: la firma del log es lo que convierte una raíz en **una afirmación de
/// alguien**.
///
/// Lleva el `logId` dentro para que la cabeza de un log no valga como cabeza de
/// otro, igual que el enunciado de una firma lleva la coordenada.
pub fn cabeza(log_id: &str, tamano: u64, raiz: &Hash) -> String {
    Json::obj([
        ("logId", Json::s(log_id)),
        ("root", Json::s(a_hex(raiz))),
        ("treeSize", Json::Int(tamano as i64)),
    ])
    .jcs()
}

/// Por qué una prueba no vale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalida {
    /// El índice cae fuera del árbol, o los tamaños no tienen sentido.
    Tamano,
    /// La prueba tiene más o menos hashes de los que ese árbol necesita.
    ///
    /// Se distingue de que no case porque es un fallo de **forma**: alguien
    /// construyó mal la prueba, no la falsificó. El tamaño del camino está
    /// determinado por el índice y el tamaño del árbol, así que esto se sabe
    /// antes de hashear nada.
    Longitud,
    /// Los hashes están, encajan en número, y **la raíz sale otra**.
    NoCasa,
}

impl Invalida {
    pub const fn como_texto(self) -> &'static str {
        match self {
            Invalida::Tamano => "el índice o el tamaño del árbol no tienen sentido",
            Invalida::Longitud => "la prueba no tiene los hashes que ese árbol necesita",
            Invalida::NoCasa => "la raíz que sale de la prueba no es la que se declara",
        }
    }
}

/// **Esta hoja está en este árbol, en esta posición.**
///
/// Es la mitad que contesta *«el log vio esta versión»*. No dice nada sobre si
/// el log es el mismo que vio otro — eso es [`consistencia`].
pub fn inclusion(
    hoja: &Hash,
    indice: u64,
    tamano: u64,
    camino: &[Hash],
    raiz: &Hash,
) -> Result<(), Invalida> {
    if indice >= tamano {
        return Err(Invalida::Tamano);
    }
    let (dentro, borde) = descomponer(indice, tamano);
    if camino.len() as u64 != dentro + borde {
        return Err(Invalida::Longitud);
    }
    let r = cadena_dentro(*hoja, &camino[..dentro as usize], indice);
    let r = cadena_borde(r, &camino[dentro as usize..]);
    if r == *raiz {
        Ok(())
    } else {
        Err(Invalida::NoCasa)
    }
}

/// **El árbol de ahora extiende el de antes.**
///
/// Es la mitad que contesta *«el log no ha reescrito nada»*, y sin ella la otra
/// no significa gran cosa: un log que pueda tener dos historias enseña a cada
/// uno la suya, y todas las pruebas de inclusión cuadran.
pub fn consistencia(
    antes: u64,
    raiz_antes: &Hash,
    ahora: u64,
    raiz_ahora: &Hash,
    prueba: &[Hash],
) -> Result<(), Invalida> {
    if antes > ahora {
        return Err(Invalida::Tamano);
    }
    if antes == ahora {
        // Nada creció. La prueba sobra, y una que viniera con hashes estaría
        // afirmando un paso que no se ha dado.
        if !prueba.is_empty() {
            return Err(Invalida::Longitud);
        }
        return if raiz_antes == raiz_ahora {
            Ok(())
        } else {
            Err(Invalida::NoCasa)
        };
    }
    if antes == 0 {
        // Todo árbol extiende al vacío, y no hay nada que demostrar.
        return if prueba.is_empty() {
            Ok(())
        } else {
            Err(Invalida::Longitud)
        };
    }

    let (mut dentro, borde) = descomponer(antes - 1, ahora);
    let ceros = antes.trailing_zeros() as u64;
    dentro -= ceros;

    // Cuando `antes` es potencia de dos, su raíz **es** el subárbol completo y
    // la prueba no lo repite: la semilla es la raíz que ya se tiene.
    let (semilla, desde) = if antes == 1 << ceros {
        (*raiz_antes, 0)
    } else {
        (*prueba.first().ok_or(Invalida::Longitud)?, 1)
    };
    if prueba.len() as u64 != desde + dentro + borde {
        return Err(Invalida::Longitud);
    }
    let resto = &prueba[desde as usize..];
    let mascara = (antes - 1) >> ceros;

    let a = cadena_dentro_derecha(semilla, &resto[..dentro as usize], mascara);
    let a = cadena_borde(a, &resto[dentro as usize..]);
    if a != *raiz_antes {
        return Err(Invalida::NoCasa);
    }
    let b = cadena_dentro(semilla, &resto[..dentro as usize], mascara);
    let b = cadena_borde(b, &resto[dentro as usize..]);
    if b == *raiz_ahora {
        Ok(())
    } else {
        Err(Invalida::NoCasa)
    }
}

/// Cuántos hashes del camino son nodos internos y cuántos son del borde
/// derecho.
///
/// Un árbol de RFC 6962 no está equilibrado cuando su tamaño no es potencia de
/// dos: el flanco derecho son subárboles completos de tamaños decrecientes que
/// se encadenan. Los dos tramos se recorren distinto, y por eso se cuentan
/// aparte en vez de mirar bit a bit dentro del bucle.
fn descomponer(indice: u64, tamano: u64) -> (u64, u64) {
    let dentro = u64::BITS as u64 - (indice ^ (tamano - 1)).leading_zeros() as u64;
    (dentro, (indice >> dentro).count_ones() as u64)
}

/// Sube por los nodos internos: el bit del índice dice de qué lado va el hermano.
fn cadena_dentro(mut h: Hash, camino: &[Hash], indice: u64) -> Hash {
    for (i, p) in camino.iter().enumerate() {
        h = if (indice >> i) & 1 == 0 {
            nodo(&h, p)
        } else {
            nodo(p, &h)
        };
    }
    h
}

/// Igual, pero **solo** cuando el hermano está a la izquierda.
///
/// Es lo que reconstruye la raíz VIEJA desde la misma semilla que reconstruye la
/// nueva: los pasos que en el árbol nuevo suben por la derecha son justo los que
/// el viejo todavía no tenía.
fn cadena_dentro_derecha(mut h: Hash, camino: &[Hash], indice: u64) -> Hash {
    for (i, p) in camino.iter().enumerate() {
        if (indice >> i) & 1 == 1 {
            h = nodo(p, &h);
        }
    }
    h
}

/// El flanco derecho: siempre se cuelga a la derecha de lo que ya se lleva.
fn cadena_borde(mut h: Hash, camino: &[Hash]) -> Hash {
    for p in camino {
        h = nodo(p, &h);
    }
    h
}

// ── Construir el árbol ──────────────────────────────────────────────────────
//
// Vive aquí y no en quien sirve el log, por la misma razón que `publicables`
// vive en `link`: el que construye y el que verifica tienen que estar de acuerdo
// sobre qué es el árbol. Con dos definiciones, la misma lista daría dos raíces —
// y eso ya pasó una vez en este proyecto, con el digest de un paquete.

/// La raíz de una lista de hojas. El árbol vacío es `SHA256("")`, como RFC 6962.
pub fn raiz(hojas: &[Hash]) -> Hash {
    match hojas.len() {
        0 => Sha256::new().finalize().into(),
        1 => hojas[0],
        n => {
            let k = division(n as u64) as usize;
            nodo(&raiz(&hojas[..k]), &raiz(&hojas[k..]))
        }
    }
}

/// El camino de inclusión de la hoja `indice`.
pub fn camino_de_inclusion(hojas: &[Hash], indice: u64) -> Vec<Hash> {
    let n = hojas.len() as u64;
    if indice >= n {
        return Vec::new();
    }
    if n == 1 {
        return Vec::new();
    }
    let k = division(n);
    if indice < k {
        let mut c = camino_de_inclusion(&hojas[..k as usize], indice);
        c.push(raiz(&hojas[k as usize..]));
        c
    } else {
        let mut c = camino_de_inclusion(&hojas[k as usize..], indice - k);
        c.push(raiz(&hojas[..k as usize]));
        c
    }
}

/// La prueba de que el árbol de `hojas` extiende al de sus primeras `antes`.
pub fn prueba_de_consistencia(hojas: &[Hash], antes: u64) -> Vec<Hash> {
    consistencia_desde(hojas, antes, true)
}

fn consistencia_desde(hojas: &[Hash], antes: u64, es_raiz: bool) -> Vec<Hash> {
    let n = hojas.len() as u64;
    if antes == n {
        // El subárbol viejo llega entero hasta aquí. En la raíz no hace falta
        // decirlo —quien verifica ya tiene esa raíz— y más abajo sí.
        return if es_raiz {
            Vec::new()
        } else {
            vec![raiz(hojas)]
        };
    }
    if antes == 0 || antes > n {
        return Vec::new();
    }
    let k = division(n);
    if antes <= k {
        // El indicador se **propaga** bajando por la izquierda y solo se apaga
        // al torcer a la derecha. Apagarlo siempre metía la raíz vieja en la
        // prueba —que quien verifica ya tiene— y la prueba salía con un hash de
        // más para todo tamaño potencia de dos.
        let mut p = consistencia_desde(&hojas[..k as usize], antes, es_raiz);
        p.push(raiz(&hojas[k as usize..]));
        p
    } else {
        let mut p = consistencia_desde(&hojas[k as usize..], antes - k, false);
        p.push(raiz(&hojas[..k as usize]));
        p
    }
}

/// El punto de corte de RFC 6962: la mayor potencia de dos **estrictamente**
/// menor que `n`.
///
/// Es lo que hace que el subárbol izquierdo esté siempre completo, y de ahí sale
/// que una lista que crece nunca reescriba un nodo que ya existía.
fn division(n: u64) -> u64 {
    1 << (u64::BITS - 1 - (n - 1).leading_zeros())
}

/// Hexadecimal, con el mismo criterio que la firma: minúsculas y longitud fija.
pub fn a_hex(h: &Hash) -> String {
    crate::firma::a_hex(h)
}

/// Y la vuelta. Solo 32 bytes exactos: un hash corto no es un hash.
pub fn de_hex(s: &str) -> Option<Hash> {
    let v = crate::firma::hex(s)?;
    (v.len() == 32).then(|| {
        let mut h = [0u8; 32];
        h.copy_from_slice(&v);
        h
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hojas(n: u64) -> Vec<Hash> {
        (0..n).map(|i| hoja(&i.to_be_bytes())).collect()
    }

    /// Para cada tamaño hasta 16 y cada hoja, la prueba que se genera es la que
    /// se verifica. Se recorre entero en vez de con tres casos porque los
    /// árboles de RFC 6962 se rompen justo donde no son potencia de dos, y un
    /// solo tamaño elegido a mano tenía toda la pinta de ser uno cómodo.
    #[test]
    fn toda_hoja_de_todo_arbol_prueba_su_inclusion() {
        for n in 1..=16u64 {
            let hs = hojas(n);
            let r = raiz(&hs);
            for i in 0..n {
                let c = camino_de_inclusion(&hs, i);
                assert_eq!(
                    inclusion(&hs[i as usize], i, n, &c, &r),
                    Ok(()),
                    "hoja {i} de {n}"
                );
            }
        }
    }

    /// Y la misma prueba **no** vale para otra hoja: es lo que hace que
    /// demuestre algo.
    #[test]
    fn el_camino_de_una_hoja_no_prueba_otra() {
        let hs = hojas(7);
        let r = raiz(&hs);
        let c = camino_de_inclusion(&hs, 3);
        assert!(inclusion(&hs[4], 3, 7, &c, &r).is_err());
        assert!(inclusion(&hs[3], 4, 7, &c, &r).is_err());
    }

    /// Un hash cambiado en el camino no cuela, que es el ataque directo.
    #[test]
    fn un_camino_manipulado_no_prueba_nada() {
        let hs = hojas(9);
        let r = raiz(&hs);
        let mut c = camino_de_inclusion(&hs, 5);
        c[0][0] ^= 1;
        assert_eq!(inclusion(&hs[5], 5, 9, &c, &r), Err(Invalida::NoCasa));
    }

    /// La longitud del camino la fija el árbol, así que sobra o falta se sabe
    /// **antes** de hashear. Es un fallo de forma y se cuenta aparte de una
    /// falsificación.
    #[test]
    fn un_camino_de_otra_longitud_es_un_fallo_de_forma() {
        let hs = hojas(9);
        let r = raiz(&hs);
        let mut c = camino_de_inclusion(&hs, 5);
        c.pop();
        assert_eq!(inclusion(&hs[5], 5, 9, &c, &r), Err(Invalida::Longitud));
    }

    /// Toda pareja de tamaños: el árbol grande extiende al pequeño.
    #[test]
    fn todo_arbol_prueba_que_extiende_a_los_anteriores() {
        for n in 1..=16u64 {
            let hs = hojas(n);
            let r = raiz(&hs);
            for m in 1..=n {
                let vieja = raiz(&hs[..m as usize]);
                let p = prueba_de_consistencia(&hs, m);
                assert_eq!(consistencia(m, &vieja, n, &r, &p), Ok(()), "{m} → {n}");
            }
        }
    }

    /// **La bifurcación, que es el ataque que este código existe para parar.**
    ///
    /// Dos historias que comparten prefijo hasta cierto punto y luego divergen:
    /// las pruebas de inclusión de las dos cuadran contra su propia raíz, y solo
    /// la consistencia ve que una no extiende a la otra.
    #[test]
    fn un_log_que_reescribio_el_pasado_no_prueba_consistencia() {
        let buenas = hojas(8);
        let mut torcidas = buenas.clone();
        torcidas[2] = hoja(b"otra cosa"); // el pasado, cambiado

        let vieja = raiz(&buenas[..5]);
        let nueva = raiz(&torcidas);
        let p = prueba_de_consistencia(&torcidas, 5);
        assert_eq!(
            consistencia(5, &vieja, 8, &nueva, &p),
            Err(Invalida::NoCasa),
            "aceptó un log que reescribió una entrada vieja"
        );

        // Y la inclusión sola no lo habría visto: dentro del árbol torcido, la
        // hoja 2 tiene su prueba y cuadra perfectamente.
        let c = camino_de_inclusion(&torcidas, 2);
        assert_eq!(inclusion(&torcidas[2], 2, 8, &c, &nueva), Ok(()));
    }

    /// Sin crecimiento no hay nada que probar, y una prueba con hashes estaría
    /// afirmando un paso que no se dio.
    #[test]
    fn el_mismo_tamano_exige_la_misma_raiz_y_ninguna_prueba() {
        let hs = hojas(6);
        let r = raiz(&hs);
        assert_eq!(consistencia(6, &r, 6, &r, &[]), Ok(()));
        assert_eq!(consistencia(6, &r, 6, &r, &[r]), Err(Invalida::Longitud));
        let otra = raiz(&hojas(5));
        assert_eq!(consistencia(6, &otra, 6, &r, &[]), Err(Invalida::NoCasa));
    }

    /// Un log no encoge. Que lo diga en vez de calcular sobre índices al revés
    /// es lo que separa un error de una respuesta absurda.
    #[test]
    fn un_arbol_no_puede_extender_a_uno_mayor() {
        let hs = hojas(8);
        assert_eq!(
            consistencia(8, &raiz(&hs), 5, &raiz(&hs[..5]), &[]),
            Err(Invalida::Tamano)
        );
    }

    /// Los dos prefijos son distintos dominios, y sin ellos una hoja podría
    /// hacerse pasar por el nodo que une dos hashes.
    #[test]
    fn una_hoja_no_puede_hacerse_pasar_por_un_nodo() {
        let (a, b) = (hoja(b"a"), hoja(b"b"));
        let mut concatenados = Vec::new();
        concatenados.extend_from_slice(&a);
        concatenados.extend_from_slice(&b);
        assert_ne!(hoja(&concatenados), nodo(&a, &b));
    }

    /// La entrada es el enunciado firmado **y quién lo firmó**: el log demuestra
    /// que una clave dijo algo, no solo que algo existió.
    #[test]
    fn la_entrada_lleva_el_enunciado_y_su_firmante() {
        let e = crate::firma::enunciado("oos.dev/regulatory/gdpr", "0.2.0", "sha256:abc");
        let con = entrada(&e, "oos.dev", "aa");
        assert_ne!(con, entrada(&e, "otro", "aa"), "el firmante no entra");
        assert_ne!(con, entrada(&e, "oos.dev", "bb"), "la firma no entra");
    }

    /// La cabeza de un log no vale como cabeza de otro, ni para otro tamaño.
    /// Sin el `logId` dentro, una firma de un log complaciente valdría para
    /// suplantar a otro con el mismo árbol.
    #[test]
    fn la_cabeza_nombra_su_log_y_su_tamano() {
        let r = raiz(&hojas(4));
        assert_ne!(cabeza("a", 4, &r), cabeza("b", 4, &r));
        assert_ne!(cabeza("a", 4, &r), cabeza("a", 5, &r));
    }

    #[test]
    fn el_hexadecimal_de_un_hash_va_y_vuelve() {
        let h = hoja(b"x");
        assert_eq!(de_hex(&a_hex(&h)), Some(h));
        assert_eq!(de_hex("00ff"), None, "32 bytes o nada");
    }
}
