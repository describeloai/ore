//! **`GET /assets`** — el índice de assets (0034 ⑤): el árbol compilado a una
//! cabeza, proyectado por `ore_core::assets::indice`, y servido **de memoria
//! por commit**.
//!
//! | ruta | qué |
//! |---|---|
//! | `GET /assets` | el índice de la rama por defecto; con la cabecera de rama (0030 W2, como `/arbol`), el de esa rama: lo que un workspace propone, antes de fusionar |
//! | `GET /assets/{commit}` | el catálogo como era. Por el camino y no por la consulta: **ningún dato entra por la URL** más allá de lo que ya viaja en `/arbol/version/{hash}` |
//!
//! # La caché por cabeza
//!
//! Medido (0034 «Lo medido»): cada petición al árbol cuesta lo mismo —0,5 s en
//! demo, 0,9 s en victor— porque es **clonar la forja**, no el tamaño. Aquí el
//! clon se hace **una vez por cabeza**: antes de clonar se pregunta a la forja
//! qué commit tiene la rama (`git ls-remote`, milisegundos), y si es el que ya
//! está en memoria se sirve sin tocar el árbol. Un push cambia la cabeza y el
//! siguiente `GET` recalcula. Nada persiste: la caché son las últimas
//! [`CABEZAS`] por proceso, y un directorio (el banco, los tests) no se cachea.
//!
//! ⭐ F4 (2026-10-15): eran 4 cabezas entre TODAS las ramas, y una rama lleva
//! en la clave la cabeza de `main`: con más de 4 ramas mirándose, o con `main`
//! moviéndose, la caché no acertaba nunca fuera de `main` (L2, L1 en victor) y
//! cada petición rehacía el índice —varias a la vez, el mismo—. Ahora:
//! · la clave sale del espejo ([`crate::git::Forja::clave_para_leer`]: un
//!   `fetch` compartido), no de dos `ls-remote` por petición;
//! · caben [`CABEZAS`] índices mientras no pesen más de [`PESO`] entre todos;
//! · un índice se hace UNA vez por clave: quien lo pide mientras se hace,
//!   espera a ese ([`Cache::turno`]).
//!
//! # `estadisticas`, del catálogo de la fuente (ORE 0062)
//!
//! Un puntero a un origen (`Table`, `ObjectTable` del paquete de una fuente) no
//! tiene transacciones, y su ficha decía «Rows —» siempre. Lo que el Job de
//! catálogo vio está en el árbol —`packages/<fuente>/discover.catalog.json`, en
//! el mismo commit que los punteros—: sus filas **contadas** (`rows`: el pie de
//! un Parquet, `__TABLES__` de BigQuery) o **estimadas** (`rowsEstimate`: un CSV
//! grande, por su muestra), y en un conjunto de objetos cuántos y cuánto pesan.
//! Aquí se cuelga de cada puntero, diciendo cuál de las dos cosas es y de
//! cuándo. No se guarda en ningún otro sitio: es del catálogo, y se rehace con él.
//!
//! # `version`, por fichero
//!
//! El núcleo deja `version: null` (no sabe de git). Aquí, en el clon que ya se
//! tiene, un `git log -1` por fichero rellena `{commit, cuando, sujeto}`. Se
//! mide en el paso 3 del brief.
use crate::rutas::{Arbol, Servidor};
use ore_core::assets::Cabeza;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Cuántas cabezas se recuerdan como mucho (por proceso, entre todas las ramas).
pub(crate) const CABEZAS: usize = 64;

/// Cuánto pueden pesar en memoria, entre todos, los índices recordados ([`Pesa`]).
/// Desde F7a se guardan como texto (un índice de 10.000 activos, ~12 MB), no
/// como árbol: el árbol pesaba ~10 veces su JSON (L6: 80 MiB por 7,6 MB). El más
/// reciente se queda aunque pese más.
pub(crate) const PESO: usize = 64 << 20;

/// `(rama, cabeza)`: lo que identifica un índice.
type Clave = (String, String);

/// Lo que se guarda en la [`Cache`], con lo que ocupa en memoria.
pub(crate) trait Pesa {
    fn peso(&self) -> usize;
}

impl Pesa for Json {
    fn peso(&self) -> usize {
        self.jcs().len()
    }
}

/// La caché: `(rama, cabeza) → índice`, las últimas [`CABEZAS`] que quepan en
/// [`PESO`], y las claves que alguien está haciendo.
pub(crate) struct Cache<T: Pesa = Guardado> {
    entradas: Mutex<Entradas<T>>,
    hecho: std::sync::Condvar,
    /// ⭐ F7a: los índices se HACEN de uno en uno (no los que se sirven). Hacer
    ///   uno de 10.000 activos lleva ore-serve a ~250 MiB (L6); dos a la vez, al
    ///   OOM de L5. Quien llega espera a que acabe el de delante.
    obras: Mutex<()>,
}

impl<T: Pesa> Default for Cache<T> {
    fn default() -> Self {
        Cache {
            entradas: Mutex::new(Entradas {
                lista: VecDeque::new(),
                haciendo: Default::default(),
            }),
            hecho: Default::default(),
            obras: Mutex::new(()),
        }
    }
}

struct Entradas<T> {
    /// Del más viejo al más nuevo, con su peso.
    lista: VecDeque<(Clave, Arc<T>, usize)>,
    haciendo: std::collections::BTreeSet<Clave>,
}

/// Lo que da [`Cache::turno`]: el índice, o el turno de hacerlo.
pub(crate) enum Turno<'a, T: Pesa = Guardado> {
    Hecho(Arc<T>),
    /// Mientras viva, quien pida la misma clave espera; al soltarlo, mira.
    Mio {
        _turno: Haciendo<'a, T>,
    },
}

pub(crate) struct Haciendo<'a, T: Pesa = Guardado> {
    cache: &'a Cache<T>,
    clave: Clave,
}

impl<T: Pesa> Drop for Haciendo<'_, T> {
    fn drop(&mut self) {
        let mut e = self
            .cache
            .entradas
            .lock()
            .unwrap_or_else(|x| x.into_inner());
        e.haciendo.remove(&self.clave);
        self.cache.hecho.notify_all();
    }
}

impl<T: Pesa> std::fmt::Debug for Cache<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cache")
    }
}

impl<T: Pesa> Cache<T> {
    pub(crate) fn lee(&self, clave: &Clave) -> Option<Arc<T>> {
        let e = self.entradas.lock().ok()?;
        e.lista
            .iter()
            .find(|(k, _, _)| k == clave)
            .map(|(_, v, _)| v.clone())
    }

    pub(crate) fn guarda(&self, clave: Clave, j: Arc<T>) {
        let peso = j.peso();
        if let Ok(mut e) = self.entradas.lock() {
            e.lista.retain(|(k, _, _)| *k != clave);
            e.lista.push_back((clave, j, peso));
            while e.lista.len() > 1
                && (e.lista.len() > CABEZAS || e.lista.iter().map(|x| x.2).sum::<usize>() > PESO)
            {
                e.lista.pop_front();
            }
        }
    }

    /// **El índice de `clave`, o el turno de hacerlo** (F4). Si otro lo está
    /// haciendo, se espera a que acabe y se vuelve a mirar: diez peticiones a
    /// la misma rama hacen UN índice, no diez a la vez (L1 en victor: los
    /// índices rehechos en paralelo eran lo que se comía la memoria).
    pub(crate) fn turno(&self, clave: &Clave) -> Turno<'_, T> {
        let mut e = self.entradas.lock().unwrap_or_else(|x| x.into_inner());
        loop {
            if let Some((_, v, _)) = e.lista.iter().find(|(k, _, _)| k == clave) {
                return Turno::Hecho(v.clone());
            }
            if e.haciendo.insert(clave.clone()) {
                return Turno::Mio {
                    _turno: Haciendo {
                        cache: self,
                        clave: clave.clone(),
                    },
                };
            }
            let t = std::time::Instant::now();
            e = self.hecho.wait(e).unwrap_or_else(|x| x.into_inner());
            crate::fases::candado(t.elapsed());
        }
    }

    /// El turno de HACER un índice (F7a): de uno en uno en todo el proceso.
    pub(crate) fn obra(&self) -> std::sync::MutexGuard<'_, ()> {
        let t = std::time::Instant::now();
        let g = self.obras.lock().unwrap_or_else(|x| x.into_inner());
        crate::fases::candado(t.elapsed());
        g
    }
}

// ── F7a · el índice guardado como TEXTO ─────────────────────────────────────
//
// L6 (10.000 activos): el índice como árbol `Json` ocupa ~80 MiB por 7,6 MB de
// JSON, y servirlo de la caché lo CLONABA entero (otros 80 MiB) para ponerle la
// salud de ahora. Con 50 usuarios, OOM. Ahora se guarda su texto con HUECOS
// donde va lo que cambia al servir —la salud (depende del reloj) y
// `desde_cache`—, más una mini-copia con sólo lo que la salud lee. Servir es
// coser el texto con los valores de la salud, que se rehace como mucho cada
// [`CUBO_SALUD`] segundos por índice. La respuesta es la MISMA, byte a byte
// (`servido_es_lo_mismo_que_el_arbol_anotado`).

/// Los kinds que llevan salud: los que [`anotar_salud`] mide.
const CON_SALUD: &[&str] = &["Dataset", "MediaCollection", "View", "Table", "ObjectTable"];

/// Cada cuántos segundos se rehace la salud de un índice guardado. Una promesa
/// de frescura se mide en minutos u horas: 15 s no la cambian.
const CUBO_SALUD: i64 = 15;

/// Un índice guardado: su texto, con huecos, y lo que la salud necesita.
///
/// ⭐ Sólo TEXTO, ningún árbol: lo que pesa es lo que mide. Guardar la
///   mini-copia como `Json` (y su anotada) ocupaba de verdad 2,6 veces y más lo
///   que cualquier estimación decía (L6 con 30 ramas: 210 y 313 MiB).
pub(crate) struct Guardado {
    trozos: Vec<Trozo>,
    /// Qué va en cada hueco.
    lugares: Vec<Lugar>,
    /// La mini-copia, en JSON: los ítems con `kind`, `paquete`, `schema` y, los
    /// que tienen salud, lo que `anotar_salud` lee; y los paquetes por su nombre.
    mini: String,
    peso: usize,
    salud: Mutex<Option<(i64, Arc<Anotado>)>>,
    /// Si alguien está rehaciendo la salud aparte ([`Guardado::reciente`]).
    rehaciendo: std::sync::atomic::AtomicBool,
}

/// La salud de un índice guardado, a una hora: el texto de cada hueco que
/// tiene valor.
pub(crate) struct Anotado {
    valores: std::collections::HashMap<usize, String>,
}

enum Trozo {
    Texto(String),
    /// `coma`: la clave va detrás de otra (`,"clave":…`). Si no, es la primera
    /// del objeto y, sin valor, se escribe `null` —no pasa: ni `salud` ni
    /// `desde_cache` ni las sumas de un paquete son nunca la primera—.
    Hueco {
        n: usize,
        clave: &'static str,
        coma: bool,
    },
}

enum Lugar {
    Raiz(&'static str),
    Paquete(String, &'static str),
    Item(String),
}

impl Lugar {
    fn clave(&self) -> &'static str {
        match self {
            Lugar::Raiz(c) | Lugar::Paquete(_, c) => c,
            Lugar::Item(_) => "salud",
        }
    }
}

impl Pesa for Guardado {
    fn peso(&self) -> usize {
        self.peso
    }
}

fn marca(lugares: &mut Vec<Lugar>, l: Lugar) -> Json {
    lugares.push(l);
    Json::Crudo(format!("\u{1}{}\u{2}", lugares.len() - 1))
}

impl Guardado {
    /// Guarda un índice recién hecho (sin salud ni `desde_cache`).
    pub(crate) fn de(mut j: Json) -> Guardado {
        let mini = mini_de(&j);
        let mut lugares = Vec::new();
        if let Json::Obj(m) = &mut j {
            m.insert("salud".into(), marca(&mut lugares, Lugar::Raiz("salud")));
            m.insert(
                "desde_cache".into(),
                marca(&mut lugares, Lugar::Raiz("desde_cache")),
            );
            if let Some(Json::Arr(ps)) = m.get_mut("paquetes") {
                for p in ps.iter_mut() {
                    let Json::Obj(p) = p else { continue };
                    let nombre = match p.get("name") {
                        Some(Json::Str(x)) => x.clone(),
                        _ => continue,
                    };
                    for c in ["salud", "salud_por_schema", "salud_del_origen"] {
                        p.insert(
                            c.into(),
                            marca(&mut lugares, Lugar::Paquete(nombre.clone(), c)),
                        );
                    }
                }
            }
            if let Some(Json::Obj(items)) = m.get_mut("items") {
                for (r, it) in items.iter_mut() {
                    let Json::Obj(it) = it else { continue };
                    if matches!(it.get("kind"), Some(Json::Str(k)) if CON_SALUD.contains(&k.as_str()))
                    {
                        it.insert("salud".into(), marca(&mut lugares, Lugar::Item(r.clone())));
                    }
                }
            }
        }
        let texto = j.jcs();
        drop(j);
        let mini = mini.jcs();
        let mut trozos = Vec::with_capacity(2 * lugares.len() + 1);
        let mut resto = texto.as_str();
        while let Some(i) = resto.find('\u{1}') {
            let f = i + resto[i..].find('\u{2}').expect("la marca se cierra");
            let n: usize = resto[i + 1..f].parse().expect("la marca es un número");
            let clave = lugares[n].clave();
            let antes = resto[..i]
                .strip_suffix(&format!("\"{clave}\":"))
                .expect("la marca va tras su clave");
            let (antes, coma) = match antes.strip_suffix(',') {
                Some(a) => (a, true),
                None => (antes, false),
            };
            trozos.push(Trozo::Texto(antes.to_string()));
            trozos.push(Trozo::Hueco { n, clave, coma });
            resto = &resto[f + 1..];
        }
        trozos.push(Trozo::Texto(resto.to_string()));
        // El texto, la mini-copia, y lo que ocupará la salud anotada (un valor
        // por hueco) y cada hueco (su ruta).
        let peso = texto.len()
            + mini.len()
            + lugares
                .iter()
                .map(|l| {
                    160 + match l {
                        Lugar::Item(r) | Lugar::Paquete(r, _) => r.len(),
                        Lugar::Raiz(_) => 0,
                    }
                })
                .sum::<usize>();
        Guardado {
            trozos,
            lugares,
            mini,
            peso,
            salud: Mutex::new(None),
            rehaciendo: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// La salud a la hora `ahora`, de la que hay si es del mismo cubo. La de
    /// `GET /health` y las pruebas: exacta, y quien la pide espera a que se haga.
    pub(crate) fn anotado(&self, ahora: i64) -> Arc<Anotado> {
        let cubo = ahora.div_euclid(CUBO_SALUD);
        let mut g = self.salud.lock().unwrap_or_else(|x| x.into_inner());
        if let Some((c, a)) = g.as_ref()
            && *c == cubo
        {
            return a.clone();
        }
        let a = Arc::new(self.calcular(ahora));
        *g = Some((cubo, a.clone()));
        a
    }

    /// **La salud para servir** (stale-while-revalidate). Si la que hay es de
    /// otro cubo, se sirve igual y UN hilo la rehace aparte; sólo la primera vez
    /// se espera. Medido a 10.000 activos con medio CPU: rehacerla son 4,1–4,5 s
    /// (parsear la mini-copia 1,9–2,5 s y `anotar_salud` 1,5–1,7 s) y coserla,
    /// 11 ms: lo pagaba entero quien caía en un cubo nuevo. Una promesa de
    /// frescura se mide en minutos u horas: unos segundos de retraso no cambian
    /// lo que dice.
    pub(crate) fn reciente(self: &Arc<Self>, ahora: i64) -> Arc<Anotado> {
        use std::sync::atomic::Ordering;
        let cubo = ahora.div_euclid(CUBO_SALUD);
        let vieja = {
            let g = self.salud.lock().unwrap_or_else(|x| x.into_inner());
            match g.as_ref() {
                Some((c, a)) if *c == cubo => return a.clone(),
                Some((_, a)) => Some(a.clone()),
                None => None,
            }
        };
        let Some(vieja) = vieja else {
            return self.anotado(ahora);
        };
        if !self.rehaciendo.swap(true, Ordering::SeqCst) {
            let yo = self.clone();
            let llevado = crate::lugar::Llevado::aqui();
            std::thread::spawn(move || {
                llevado.poner();
                let a = Arc::new(yo.calcular(ahora));
                let mut g = yo.salud.lock().unwrap_or_else(|x| x.into_inner());
                if g.as_ref().is_none_or(|(c, _)| *c < cubo) {
                    *g = Some((cubo, a));
                }
                drop(g);
                yo.rehaciendo.store(false, Ordering::SeqCst);
            });
        }
        vieja
    }

    /// La salud a la hora `ahora`, calculada: la mini-copia anotada y, de ella,
    /// el texto de cada hueco.
    fn calcular(&self, ahora: i64) -> Anotado {
        let mini = self.salud_anotada(ahora);
        let mut valores = std::collections::HashMap::new();
        {
            let Json::Obj(m) = &mini else {
                unreachable!("la mini-copia es un objeto")
            };
            let vacio = Default::default();
            let items = match m.get("items") {
                Some(Json::Obj(i)) => i,
                _ => &vacio,
            };
            let paquetes: std::collections::HashMap<
                &str,
                &std::collections::BTreeMap<String, Json>,
            > = match m.get("paquetes") {
                Some(Json::Arr(a)) => a
                    .iter()
                    .filter_map(|p| match p {
                        Json::Obj(p) => match p.get("name") {
                            Some(Json::Str(n)) => Some((n.as_str(), p)),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
                _ => Default::default(),
            };
            for (n, l) in self.lugares.iter().enumerate() {
                let v = match l {
                    Lugar::Raiz("desde_cache") => None,
                    Lugar::Raiz(c) => m.get(*c),
                    Lugar::Paquete(p, c) => paquetes.get(p.as_str()).and_then(|p| p.get(*c)),
                    Lugar::Item(r) => match items.get(r) {
                        Some(Json::Obj(it)) => it.get("salud"),
                        _ => None,
                    },
                };
                if let Some(v) = v {
                    valores.insert(n, v.jcs());
                }
            }
        }
        Anotado { valores }
    }

    /// La mini-copia, anotada con la salud de `ahora`: lo que `GET /health` lee.
    /// Se rehace del texto cada vez: no se guarda ningún árbol.
    pub(crate) fn salud_anotada(&self, ahora: i64) -> Json {
        let mut mini = ore_core::parse::parse(&self.mini)
            .map(|n| Json::de_node_fiel(&n))
            .unwrap_or_else(|_| Json::obj([]));
        anotar_salud(&mut mini, ahora);
        mini
    }

    /// La salud de un ítem (`kind:ns.nombre`) en `a`, si la tiene.
    pub(crate) fn salud_de_item(&self, a: &Anotado, r: &str) -> Option<Json> {
        let n = self
            .lugares
            .iter()
            .position(|l| matches!(l, Lugar::Item(x) if x == r))?;
        let v = a.valores.get(&n)?;
        ore_core::parse::parse(v)
            .ok()
            .map(|n| Json::de_node_fiel(&n))
    }

    /// El cuerpo de la respuesta: el texto cosido con la salud y `desde_cache`.
    pub(crate) fn cuerpo(&self, desde_cache: bool, a: &Anotado) -> String {
        let largo: usize = self
            .trozos
            .iter()
            .map(|t| match t {
                Trozo::Texto(x) => x.len(),
                Trozo::Hueco { .. } => 0,
            })
            .sum::<usize>()
            + a.valores.values().map(|v| v.len() + 16).sum::<usize>()
            + 32;
        let mut s = String::with_capacity(largo);
        for t in &self.trozos {
            match t {
                Trozo::Texto(x) => s.push_str(x),
                Trozo::Hueco { n, clave, coma } => {
                    let v = match &self.lugares[*n] {
                        Lugar::Raiz("desde_cache") => {
                            Some(if desde_cache { "true" } else { "false" })
                        }
                        _ => a.valores.get(n).map(String::as_str),
                    };
                    match (v, coma) {
                        (Some(v), _) => {
                            if *coma {
                                s.push(',');
                            }
                            s.push('"');
                            s.push_str(clave);
                            s.push_str("\":");
                            s.push_str(v);
                        }
                        (None, false) => {
                            s.push('"');
                            s.push_str(clave);
                            s.push_str("\":null");
                        }
                        (None, true) => {}
                    }
                }
            }
        }
        s
    }
}

/// La mini-copia de un índice: cada ítem con `kind`, `paquete` y `schema`, y
/// los que llevan salud con lo que [`anotar_salud`] lee (`define.freshness`,
/// `puntero`, las relaciones `sale_de`); los paquetes, por su nombre.
fn mini_de(j: &Json) -> Json {
    let Json::Obj(m) = j else {
        return Json::obj([]);
    };
    let mut items = std::collections::BTreeMap::new();
    if let Some(Json::Obj(todos)) = m.get("items") {
        for (r, it) in todos {
            let Json::Obj(it) = it else { continue };
            let mut x = std::collections::BTreeMap::new();
            for k in ["kind", "paquete", "schema"] {
                if let Some(v) = it.get(k) {
                    x.insert(k.to_string(), v.clone());
                }
            }
            if matches!(it.get("kind"), Some(Json::Str(k)) if CON_SALUD.contains(&k.as_str())) {
                if let Some(Json::Obj(d)) = it.get("define")
                    && let Some(f) = d.get("freshness")
                {
                    x.insert("define".into(), Json::obj([("freshness", f.clone())]));
                }
                if let Some(p) = it.get("puntero") {
                    x.insert("puntero".into(), p.clone());
                }
                if let Some(Json::Arr(rs)) = it.get("relaciones") {
                    x.insert(
                        "relaciones".into(),
                        Json::Arr(
                            rs.iter()
                                .filter(|r| {
                                    matches!(r, Json::Obj(o) if matches!(o.get("tipo"), Some(Json::Str(t)) if t == "sale_de"))
                                })
                                .cloned()
                                .collect(),
                        ),
                    );
                }
            }
            items.insert(r.clone(), Json::Obj(x));
        }
    }
    let paquetes = match m.get("paquetes") {
        Some(Json::Arr(ps)) => ps
            .iter()
            .filter_map(|p| match p {
                Json::Obj(p) => p.get("name").map(|n| Json::obj([("name", n.clone())])),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Json::obj([
        ("items", Json::Obj(items)),
        ("paquetes", Json::Arr(paquetes)),
    ])
}

/// El índice de un árbol en disco: los punteros de `datasets/`, el paquete
/// compilado, la proyección, y `version` por fichero si hay historia.
#[cfg(test)]
pub(crate) fn indice_de(raiz: &Path, cabeza: Cabeza) -> Json {
    indice_con(raiz, cabeza, false)
}

/// docs/ejecuciones.md X2b · La base `system` y su `executions` llevan
/// `system: true` en el índice: la consola los enseña como System table,
/// globales (iguales en toda rama) y de solo lectura.
fn marcar_lo_del_sistema(j: &mut Json) {
    let Json::Obj(m) = j else { return };
    if let Some(Json::Arr(ps)) = m.get_mut("paquetes") {
        for p in ps.iter_mut() {
            if let Json::Obj(p) = p
                && p.get("name") == Some(&Json::s(ore_core::sistema::BASE))
            {
                p.insert("system".into(), Json::Bool(true));
            }
        }
    }
    if let Some(Json::Obj(items)) = m.get_mut("items") {
        for (k, it) in items.iter_mut() {
            if let Json::Obj(it) = it
                && k.split_once(':')
                    .is_some_and(|(_, qn)| ore_core::sistema::es_tabla_del_sistema(qn))
            {
                it.insert("system".into(), Json::Bool(true));
            }
        }
    }
}

/// [`indice_de`], y con `sistema` (docs/ejecuciones.md X2, con `--executions`)
/// la base `system` con su `executions`: todos la ven en el árbol de catálogo
/// desde el primer momento, aunque no esté en el árbol.
pub(crate) fn indice_con(raiz: &Path, cabeza: Cabeza, sistema: bool) -> Json {
    indice_entero(raiz, cabeza, sistema, true)
}

/// [`indice_con`], y con `punteros = false` **sólo los documentos** (0079 ·5c
/// P5: `/assets/{commit}`, que no tiene clientes; un commit de git no tiene
/// punteros cuando los punteros viven en el catálogo).
fn indice_entero(raiz: &Path, cabeza: Cabeza, sistema: bool, con_punteros: bool) -> Json {
    let (mut pkg, _) = ore_core::validate::cargar_paquete(raiz);
    if sistema
        && !pkg.docs.iter().any(|d| {
            d.kind == ore_core::document::Kind::Package
                && d.meta("name").and_then(|n| n.as_str()) == Some(ore_core::sistema::BASE)
        })
    {
        pkg.docs.extend(ore_core::sistema::documentos(&pkg.root));
    }
    // Por la puerta (0079 ·5b·3), fieles: un `null` sigue nulo (0063 S6b).
    // ⭐ 0044 C.2 ③: en una rama, cada puntero dice de dónde sale.
    let lugar = crate::lugar::Lugar::de(raiz);
    let en_una_rama = lugar.en_una_rama();
    let todos = if con_punteros {
        lugar.punteros().todos().unwrap_or_default()
    } else {
        Default::default()
    };
    let punteros: std::collections::BTreeMap<String, Json> = todos
        .into_iter()
        .map(|(k, l)| {
            let mut v = Json::de_node_fiel(&l.puntero);
            if en_una_rama && let Json::Obj(m) = &mut v {
                let d = match l.de {
                    ore_core::punteros::De::Main => "main",
                    ore_core::punteros::De::Aqui => "rama",
                };
                m.insert("de".into(), Json::s(d));
            }
            (k, v)
        })
        .collect();
    let mut j = ore_core::assets::indice(&pkg, &punteros, &cabeza);
    if sistema {
        marcar_lo_del_sistema(&mut j);
    }
    anotar_estadisticas(raiz, &mut j);
    // `version` por fichero, sólo si el árbol tiene historia. Un proyecto
    // (0035 ①) tiene ruta como cualquier otra cosa —su manifiesto—, así que
    // quién lo creó y cuándo sale del mismo sitio y sin pedirle nada al árbol.
    if crate::documentos::cabeza_de(raiz).is_some()
        && let Json::Obj(m) = &mut j
    {
        // ⭐ F1 (2026-10-15): todas las versiones en UNA pasada de `git log`. Antes
        //   era un `git log -1 -- <ruta>` por ruta: en victor, 438 documentos ×
        //   26 ms = 11,6 s en el pod (0,5 CPU), y `/assets` en una rama tardaba
        //   20–25 s (medido con L1: «índice rehecho 18 s, git-local 474×»). Una
        //   pasada: 190–219 ms en el mismo pod, y la MISMA versión para 599
        //   documentos y 144 carpetas (comparado una a una sobre el árbol de victor).
        let mut rutas = std::collections::BTreeSet::new();
        for clave in ["items", "proyectos", "repositorios"] {
            let lista: Vec<&Json> = match m.get(clave) {
                Some(Json::Obj(o)) => o.values().collect(),
                Some(Json::Arr(a)) => a.iter().collect(),
                _ => Vec::new(),
            };
            for x in lista {
                if let Json::Obj(x) = x
                    && let Some(Json::Str(ruta)) = x.get("ruta")
                {
                    rutas.insert(ruta.trim_end_matches('/').to_string());
                }
            }
        }
        let versiones = versiones_de(raiz, &rutas);
        let de = |ruta: &str| versiones.get(ruta.trim_end_matches('/')).cloned();
        if let Some(Json::Obj(items)) = m.get_mut("items") {
            for it in items.values_mut() {
                if let Json::Obj(it) = it
                    && let Some(Json::Str(ruta)) = it.get("ruta")
                    && let Some(v) = de(ruta)
                {
                    it.insert("version".into(), v);
                }
            }
        }
        if let Some(Json::Arr(ps)) = m.get_mut("proyectos") {
            for p in ps.iter_mut() {
                if let Json::Obj(p) = p
                    && let Some(Json::Str(ruta)) = p.get("ruta")
                    && let Some(v) = de(ruta)
                {
                    p.insert("version".into(), v);
                }
            }
        }
        // Un repositorio es una CARPETA, y `git log -1 -- <carpeta>` dice lo
        // mismo de ella que de un fichero: quien la tocó y cuándo (medido,
        // 0035 ⑥ §2). Es el «last edited by» de la lista, sin inventar nada.
        if let Some(Json::Arr(rs)) = m.get_mut("repositorios") {
            for r in rs.iter_mut() {
                if let Json::Obj(r) = r
                    && let Some(Json::Str(ruta)) = r.get("ruta")
                    && let Some(v) = de(ruta)
                {
                    r.insert("version".into(), v);
                }
            }
        }
    }
    j
}

/// Lo que el catálogo de una fuente dice de un objeto suyo, por `<schema>.<nombre>`.
fn del_catalogo(raiz: &Path, fuente: &str) -> Option<std::collections::BTreeMap<String, Json>> {
    use ore_core::parse::Node;
    let ruta = format!("packages/{fuente}/discover.catalog.json");
    let texto = std::fs::read_to_string(raiz.join(&ruta)).ok()?;
    let cat = ore_core::parse::parse(&texto).ok()?;
    let num = |n: &Node, k: &str| -> Option<i64> { n.get(k)?.1.as_str()?.parse().ok() };
    // De cuándo es: el último commit del catálogo, si el árbol tiene historia.
    let cuando = version_de(raiz, &ruta).and_then(|v| match v {
        Json::Obj(m) => m.get("cuando").cloned(),
        _ => None,
    });
    let mut out = std::collections::BTreeMap::new();
    for t in cat.get("tables").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let Some(nombre) = t.get("name").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        let (filas, son) = match (num(t, "rows"), num(t, "rowsEstimate")) {
            (Some(n), _) => (n, "contadas"),
            (None, Some(n)) => (n, "estimadas"),
            (None, None) => continue,
        };
        let mut m = vec![("filas", Json::Int(filas)), ("filas_son", Json::s(son))];
        if let Some(c) = &cuando {
            m.push(("catalogado", c.clone()));
        }
        out.insert(nombre.to_string(), Json::obj(m));
    }
    for o in cat.get("objects").map(|(_, v)| v.items()).unwrap_or(&[]) {
        let Some(nombre) = o.get("name").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        let (Some(n), Some(b)) = (num(o, "count"), num(o, "bytes")) else {
            continue;
        };
        let mut m = vec![("objetos", Json::Int(n)), ("bytes", Json::Int(b))];
        if let Some(c) = &cuando {
            m.push(("catalogado", c.clone()));
        }
        out.insert(nombre.to_string(), Json::obj(m));
    }
    Some(out)
}

/// ⭐ 0062 · `estadisticas` en cada `Table` y `ObjectTable` de un paquete de
/// fuente, de su catálogo (ver la cabecera). Un catálogo se lee una vez.
/// **0063 S2 · La salud de cada dataset, al servir** (v1alpha30 §6). No va en
/// el índice cacheado: una promesa de frescura se rompe con el reloj aunque el
/// árbol no cambie. Se calcula aquí, con la hora de esta respuesta, de lo que
/// el índice ya trae: el puntero y la `freshness` de `define`.
///
/// **0063 S6c · Y por el linaje, con sus sumas.** Lo que no tiene puntero toma
/// la salud de lo que lee (`ore_core::salud::propagar`, v1alpha30 §2.2): una
/// vista sin copia, lo escrito desde algo degradado. Una tabla en vivo no
/// lleva salud (D-S6-1: «Live»), salvo lo que sus copias vieron de ella. Y cada
/// paquete lleva `salud` (la suma de sus activos) y `salud_por_schema`; el
/// índice, la suma del árbol.
pub(crate) fn anotar_salud(j: &mut Json, ahora: i64) {
    use ore_core::salud::{Nodo, bloque, del_origen, en_vivo, propagar, sumar};
    const CAPAS: (bool, bool) = (false, false);
    let Json::Obj(m) = j else { return };
    let Some(Json::Obj(items)) = m.get_mut("items") else {
        return;
    };
    let texto = |it: &std::collections::BTreeMap<String, Json>, k: &str| match it.get(k) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    };
    // ── La medida propia: el puntero de una copia o de una colección ────────
    let mut nodos: std::collections::BTreeMap<String, Nodo> = Default::default();
    for (r, it) in items.iter() {
        let Json::Obj(it) = it else { continue };
        let kind = texto(it, "kind").unwrap_or_default();
        if !matches!(
            kind.as_str(),
            "Dataset" | "MediaCollection" | "View" | "Table" | "ObjectTable"
        ) {
            continue;
        }
        let promesa = match it.get("define") {
            Some(Json::Obj(d)) => texto(d, "freshness"),
            _ => None,
        };
        let propio = matches!(kind.as_str(), "Dataset" | "MediaCollection").then(|| {
            let puntero = match it.get("puntero") {
                Some(p @ Json::Obj(_)) => ore_core::parse::parse(&p.jcs()).ok(),
                _ => None,
            };
            bloque(puntero.as_ref(), promesa.as_deref(), ahora, CAPAS)
        });
        let entradas = match it.get("relaciones") {
            Some(Json::Arr(a)) => a
                .iter()
                .filter_map(|x| match x {
                    Json::Obj(x) if texto(x, "tipo").as_deref() == Some("sale_de") => {
                        texto(x, "ref")
                    }
                    _ => None,
                })
                .collect(),
            _ => vec![],
        };
        // 0068 C4·4: lo que el build leyó, y de cuándo (`leido_de`, de la
        // procedencia del puntero).
        let leido = match it.get("puntero") {
            Some(Json::Obj(p)) => match p.get("leido_de") {
                Some(Json::Obj(d)) => d
                    .iter()
                    .filter_map(|(k, v)| match v {
                        Json::Str(s) => {
                            Some((ore_core::normalize::a_corto(k).into_owned(), s.clone()))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => Default::default(),
            },
            _ => Default::default(),
        };
        nodos.insert(
            r.clone(),
            Nodo {
                propio,
                de_las_copias: false,
                entradas,
                promesa: (kind == "View").then_some(promesa).flatten(),
                leido,
            },
        );
    }
    // ── Lo que las copias vieron de la tabla en vivo de la que salen ────────
    let mut vistos: std::collections::BTreeMap<String, Vec<Json>> = Default::default();
    for n in nodos.values() {
        let Some(Json::Obj(b)) = &n.propio else {
            continue;
        };
        let Some(Json::Arr(motivos)) = b.get("reasons") else {
            continue;
        };
        for e in n
            .entradas
            .iter()
            .filter(|e| e.starts_with("table:") || e.starts_with("objecttable:"))
        {
            vistos
                .entry(e.clone())
                .or_default()
                .extend(motivos.iter().cloned());
        }
    }
    for (t, motivos) in vistos {
        if let Some(n) = nodos.get_mut(&t)
            && n.propio.is_none()
        {
            n.propio = en_vivo(&motivos, CAPAS);
            n.de_las_copias = n.propio.is_some();
        }
    }
    // ── Cada origen, con lo que vieron sus copias (S6e) ─────────────────────
    let mut por_origen: std::collections::BTreeMap<String, Vec<Json>> = Default::default();
    for n in nodos.values().filter(|n| !n.de_las_copias) {
        let Some(b) = &n.propio else { continue };
        let fuentes: std::collections::BTreeSet<String> = n
            .entradas
            .iter()
            .filter(|e| e.starts_with("table:") || e.starts_with("objecttable:"))
            .filter_map(|e| match items.get(e) {
                Some(Json::Obj(it)) => texto(it, "paquete"),
                _ => None,
            })
            .collect();
        for f in fuentes {
            por_origen.entry(f).or_default().push(b.clone());
        }
    }
    // ── El linaje, y cada salud en su ítem ──────────────────────────────────
    let saludes = propagar(&nodos, ahora, CAPAS);
    // (paquete, schema) → los bloques, para las sumas.
    let mut por_ambito: std::collections::BTreeMap<(String, String), Vec<Json>> =
        Default::default();
    for (r, b) in saludes {
        let Some(Json::Obj(it)) = items.get_mut(&r) else {
            continue;
        };
        let paquete = texto(it, "paquete").unwrap_or_default();
        let schema = texto(it, "schema").unwrap_or_default();
        por_ambito
            .entry((paquete, schema))
            .or_default()
            .push(b.clone());
        it.insert("salud".into(), b);
    }
    // ── Las sumas: el árbol, cada paquete y cada schema (§3.3) ──────────────
    m.insert("salud".into(), sumar(por_ambito.values().flatten()));
    if let Some(Json::Arr(paquetes)) = m.get_mut("paquetes") {
        for p in paquetes.iter_mut() {
            let Json::Obj(p) = p else { continue };
            let nombre = texto(p, "name").unwrap_or_default();
            let suyos: Vec<(&String, &Vec<Json>)> = por_ambito
                .iter()
                .filter(|((pq, _), _)| *pq == nombre)
                .map(|((_, s), b)| (s, b))
                .collect();
            p.insert(
                "salud".into(),
                sumar(suyos.iter().flat_map(|(_, b)| b.iter())),
            );
            p.insert(
                "salud_por_schema".into(),
                Json::Obj(
                    suyos
                        .iter()
                        .map(|(s, b)| ((*s).clone(), sumar(b.iter())))
                        .collect(),
                ),
            );
            if let Some(o) = por_origen
                .get(&nombre)
                .and_then(|c| del_origen(c.iter(), CAPAS))
            {
                p.insert("salud_del_origen".into(), o);
            }
        }
    }
}

/// **`GET /health…`** (0063 S6c), del índice ya anotado:
///
/// | camino | qué |
/// |---|---|
/// | `[]` | la suma del árbol, y la de cada base y cada uno de sus schemas |
/// | `[base]`, `[base, schema]` | la suma de ese ámbito y la salud de cada activo suyo que la tiene; en una fuente, `origen`: lo que vieron sus copias (S6e) |
/// | `[ref]` (`kind:qname`) | la salud de un activo; `null` si es en vivo sin nada sabido («Live») |
///
/// Por el camino y no por la consulta, como `/assets/{commit}`.
pub(crate) fn salud_de(j: &Json, camino: &[&str]) -> Result<Json, Respuesta> {
    let Json::Obj(m) = j else {
        return Err(Respuesta::error(500, "el índice no es un objeto"));
    };
    let vacio = Default::default();
    let items = match m.get("items") {
        Some(Json::Obj(i)) => i,
        _ => &vacio,
    };
    let paquetes: Vec<&std::collections::BTreeMap<String, Json>> = match m.get("paquetes") {
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(|p| match p {
                Json::Obj(p) => Some(p),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    let nombre = |p: &std::collections::BTreeMap<String, Json>| match p.get("name") {
        Some(Json::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let de = |k: &str, o: &std::collections::BTreeMap<String, Json>| {
        o.get(k)
            .cloned()
            .unwrap_or_else(|| Json::Crudo("null".into()))
    };
    match camino {
        [] => Ok(Json::obj([
            ("salud", de("salud", m)),
            (
                "bases",
                Json::Obj(
                    paquetes
                        .iter()
                        .map(|p| {
                            (
                                nombre(p),
                                Json::obj([
                                    ("salud", de("salud", p)),
                                    ("schemas", de("salud_por_schema", p)),
                                    ("origen", de("salud_del_origen", p)),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
        ])),
        [r] if r.contains(':') => match items.get(*r) {
            Some(Json::Obj(it)) => Ok(Json::obj([
                ("ref", Json::s(*r)),
                ("salud", de("salud", it)),
            ])),
            _ => Err(Respuesta::error(404, format!("no hay ningún activo `{r}`"))),
        },
        [base] | [base, _] => {
            let base: &str = base;
            let Some(p) = paquetes.iter().find(|p| nombre(p) == *base) else {
                return Err(Respuesta::error(
                    404,
                    format!("no hay ninguna base `{base}`"),
                ));
            };
            let schema = camino.get(1).copied();
            let suma = match schema {
                None => de("salud", p),
                Some(s) => match p.get("salud_por_schema") {
                    Some(Json::Obj(por)) => por
                        .get(s)
                        .cloned()
                        .unwrap_or_else(|| ore_core::salud::sumar(std::iter::empty())),
                    _ => ore_core::salud::sumar(std::iter::empty()),
                },
            };
            let suyos = items
                .iter()
                .filter_map(|(r, it)| {
                    let Json::Obj(it) = it else { return None };
                    let es = |k: &str, v: &str| matches!(it.get(k), Some(Json::Str(x)) if x == v);
                    let dentro = es("paquete", base) && schema.is_none_or(|s| es("schema", s));
                    (dentro && it.contains_key("salud")).then(|| (r.clone(), de("salud", it)))
                })
                .collect();
            Ok(Json::obj([
                ("salud", suma),
                ("origen", de("salud_del_origen", p)),
                ("items", Json::Obj(suyos)),
            ]))
        }
        _ => Err(Respuesta::error(404, "no hay nada en ese camino")),
    }
}

fn anotar_estadisticas(raiz: &Path, j: &mut Json) {
    let Json::Obj(m) = j else { return };
    let Some(Json::Obj(items)) = m.get_mut("items") else {
        return;
    };
    let mut catalogos: std::collections::BTreeMap<
        String,
        Option<std::collections::BTreeMap<String, Json>>,
    > = Default::default();
    for it in items.values_mut() {
        let Json::Obj(it) = it else { continue };
        if !matches!(it.get("kind"), Some(Json::Str(k)) if k == "Table" || k == "ObjectTable") {
            continue;
        }
        // Lo que una base foránea expone ES el documento de la fuente (`es`,
        // v1alpha27): sus estadísticas, las del catálogo de esa fuente.
        let de_la_fuente = match it.get("es") {
            Some(Json::Str(r)) => r.split_once(':').and_then(|(_, qn)| {
                let partes: Vec<&str> = qn.split('.').collect();
                match partes.as_slice() {
                    [p, s, n] => Some((p.to_string(), s.to_string(), n.to_string())),
                    [p, n] => Some((p.to_string(), "default".to_string(), n.to_string())),
                    _ => None,
                }
            }),
            _ => None,
        };
        let (p, clave) = match de_la_fuente {
            Some((p, s, n)) => (p, format!("{s}.{n}")),
            None => {
                let (Some(Json::Str(p)), Some(Json::Str(s)), Some(Json::Str(n))) =
                    (it.get("paquete"), it.get("schema"), it.get("name"))
                else {
                    continue;
                };
                (p.clone(), format!("{s}.{n}"))
            }
        };
        let cat = catalogos
            .entry(p.clone())
            .or_insert_with(|| del_catalogo(raiz, &p));
        if let Some(e) = cat.as_ref().and_then(|c| c.get(&clave)) {
            it.insert("estadisticas".into(), e.clone());
        }
    }
}

/// `{commit, cuando, sujeto}` del último commit que tocó un fichero.
fn version_de(raiz: &Path, ruta: &str) -> Option<Json> {
    let s = crate::documentos::git(
        raiz,
        &["log", "-1", "--format=%h%x1f%aI%x1f%ae", "--", ruta],
    )?;
    version_de_cabecera(&s)
}

/// **Las versiones de muchas rutas en UNA pasada** (F1): `git log --name-only`
/// sobre toda la historia, del commit más reciente al más viejo, y cada ruta se
/// queda con el primero que la toca —a ella o a algo debajo: un repositorio es
/// una carpeta—. Es lo mismo que `git log -1 -- <ruta>` para cada una (medido:
/// 599 documentos y 144 carpetas de victor, ninguna distinta), en un proceso.
/// Salida de `-z`: `\x01<cabecera>\0\n<ruta>\0<ruta>\0…`.
fn versiones_de(
    raiz: &Path,
    rutas: &std::collections::BTreeSet<String>,
) -> std::collections::HashMap<String, Json> {
    let mut fuera = std::collections::HashMap::new();
    if rutas.is_empty() {
        return fuera;
    }
    let Some(s) = crate::documentos::git(
        raiz,
        &["log", "-z", "--format=%x01%h%x1f%aI%x1f%ae", "--name-only"],
    ) else {
        return fuera;
    };
    let mut faltan: std::collections::BTreeSet<&str> = rutas.iter().map(String::as_str).collect();
    for reg in s.split('\u{1}').skip(1) {
        let (cab, resto) = reg.split_once('\0').unwrap_or((reg, ""));
        let mut version: Option<Option<Json>> = None;
        for f in resto.trim_start_matches('\n').split('\0') {
            let mut p = f;
            while !p.is_empty() {
                if faltan.remove(p)
                    && let Some(v) = version.get_or_insert_with(|| version_de_cabecera(cab))
                {
                    fuera.insert(p.to_string(), v.clone());
                }
                p = p.rfind('/').map(|i| &p[..i]).unwrap_or("");
            }
        }
        if faltan.is_empty() {
            break;
        }
    }
    fuera
}

/// `{commit, cuando, sujeto}` de una cabecera `%h%x1f%aI%x1f%ae`.
fn version_de_cabecera(s: &str) -> Option<Json> {
    let mut p = s.split('\u{1f}');
    let commit = p.next()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    Some(Json::obj([
        ("commit", Json::s(commit)),
        ("cuando", Json::s(p.next().unwrap_or_default().trim())),
        (
            "sujeto",
            Json::s(
                p.next()
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches("@sujeto.invalid"),
            ),
        ),
    ]))
}

fn ahora() -> String {
    rfc3339(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(),
    )
}

/// Segundos desde la época → `2026-10-07T08:05:00Z` (RFC 3339, UTC).
pub(crate) fn rfc3339(s: u64) -> String {
    // ISO 8601 sin dependencias: lo suficiente para saber cuándo.
    let (d, h) = (s / 86400, s % 86400);
    let (a, m, dia) = civil(d as i64);
    format!(
        "{a:04}-{m:02}-{dia:02}T{:02}:{:02}:{:02}Z",
        h / 3600,
        (h % 3600) / 60,
        h % 60
    )
}

/// Días desde la época → (año, mes, día). Howard Hinnant, `civil_from_days`.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Lo que da [`Servidor::indice`]: de un directorio, el árbol ya anotado; de
/// la forja, el guardado (F7a) y si salió de la caché.
pub(crate) enum Indice {
    Arbol(Json),
    Guardado(Arc<Guardado>, bool),
}

impl Servidor {
    /// `GET /health…` (0063 S6c): del mismo índice, de memoria por cabeza, y
    /// con la hora de esta respuesta. Ver [`salud_de`].
    pub(crate) fn health(&self, rama: Option<&str>, camino: &[&str]) -> Respuesta {
        let r = match self.indice(rama, None) {
            Err(r) => return r,
            Ok(Indice::Arbol(j)) => salud_de(&j, camino),
            Ok(Indice::Guardado(g, _)) => salud_de(&g.salud_anotada(crate::salud::ahora()), camino),
        };
        match r {
            Ok(j) => Respuesta::ok(j),
            Err(e) => e,
        }
    }

    /// **0063 S6d · La salud de un activo, para que viaje con lo que se lee**
    /// (v1alpha30 §3.1): la del índice anotado, con el linaje (S6a), por su
    /// nombre cualificado. De lo que el índice tiene con ese nombre, la copia
    /// antes que la vista y que la tabla. `None`: no está, o es en vivo sin
    /// nada sabido (quien lee en vivo pone la de su lectura).
    ///
    /// ⭐ F7a: se pide en CADA lectura de un activo; antes clonaba el índice
    ///   entero para mirar un ítem. Ahora lee la mini-copia anotada.
    pub(crate) fn salud_del_activo(&self, rama: Option<&str>, qn: &str) -> Option<Json> {
        let corto = ore_core::normalize::a_corto(qn);
        const KINDS: [&str; 5] = ["dataset", "collection", "view", "table", "objecttable"];
        let j = match self.indice(rama, None).ok()? {
            Indice::Arbol(j) => j,
            Indice::Guardado(g, _) => {
                let a = g.reciente(crate::salud::ahora());
                return KINDS
                    .iter()
                    .find_map(|k| g.salud_de_item(&a, &format!("{k}:{corto}")));
            }
        };
        let Json::Obj(m) = &j else { return None };
        let Some(Json::Obj(items)) = m.get("items") else {
            return None;
        };
        KINDS
            .iter()
            .find_map(|k| match items.get(&format!("{k}:{corto}")) {
                Some(Json::Obj(it)) => it.get("salud").cloned(),
                _ => None,
            })
    }

    /// Pone `salud` en una respuesta 200 que es un objeto, si se sabe.
    pub(crate) fn con_salud(&self, mut r: Respuesta, rama: Option<&str>, qn: &str) -> Respuesta {
        if r.codigo / 100 == 2
            && let Some(s) = self.salud_del_activo(rama, qn)
            && let Json::Obj(m) = &mut r.cuerpo
        {
            m.insert("salud".into(), s);
        }
        r
    }

    /// `GET /assets` (la rama, por la cabecera) y `GET /assets/{commit}`.
    pub(crate) fn assets(&self, rama: Option<&str>, commit: Option<&str>) -> Respuesta {
        match self.indice(rama, commit) {
            Err(r) => r,
            Ok(Indice::Arbol(j)) => Respuesta::ok(j),
            // El texto, tal cual: `Crudo` se escribe sin tocarlo.
            Ok(Indice::Guardado(g, desde_cache)) => Respuesta::ok(Json::Crudo(
                g.cuerpo(desde_cache, &g.reciente(crate::salud::ahora())),
            )),
        }
    }

    /// El índice de una rama o de un commit: del directorio, hecho y anotado;
    /// de la forja, el guardado.
    pub(crate) fn indice(
        &self,
        rama: Option<&str>,
        commit: Option<&str>,
    ) -> Result<Indice, Respuesta> {
        let rama = rama.map(str::to_string);
        let commit = commit.map(str::to_string);
        if let Some(r) = &rama
            && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
        {
            return Err(Respuesta::error(422, m));
        }
        if let Some(c) = &commit
            && (c.len() < 4 || !c.chars().all(|x| x.is_ascii_hexdigit()))
        {
            return Err(Respuesta::error(422, format!("`{c}` no es un commit")));
        }
        match &self.arbol {
            // Un directorio: sin caché, sin ramas.
            Arbol::Directorio(d) => {
                if rama.is_some() || commit.is_some() {
                    return Err(Respuesta::error(
                        422,
                        "este árbol es un directorio, no una forja: no hay rama ni commit que leer",
                    ));
                }
                let cabeza = Cabeza {
                    cabeza: crate::documentos::cabeza_de(d),
                    rama: None,
                    generado: Some(ahora()),
                };
                let mut j = indice_con(d, cabeza, self.puestos.ejecuciones.encendido());
                if let Json::Obj(m) = &mut j {
                    m.insert("desde_cache".into(), Json::Bool(false));
                }
                anotar_salud(&mut j, crate::salud::ahora());
                Ok(Indice::Arbol(j))
            }
            Arbol::Forja(forja) => {
                let rama_nombre = rama.clone().unwrap_or_else(|| "main".to_string());
                let (g, desde_cache) = match commit {
                    None => self.assets_al_dia(forja, rama.as_deref(), rama_nombre),
                    Some(c) => self.assets_de_commit(forja, rama.as_deref(), rama_nombre, c),
                }?;
                Ok(Indice::Guardado(g, desde_cache))
            }
        }
    }

    /// El índice de lo último de una rama (o de `main`): del espejo, con la
    /// clave del árbol que se leería; de memoria si ya se hizo, y si no, hecho
    /// una sola vez sobre el árbol de lectura compartido (F3b, F4), y de uno en
    /// uno (F7a).
    fn assets_al_dia(
        &self,
        forja: &crate::git::Forja,
        rama: Option<&str>,
        rama_nombre: String,
    ) -> Result<(Arc<Guardado>, bool), Respuesta> {
        let t = std::time::Instant::now();
        // Sin espejo (`Err` que no es «no hay rama»), sin caché: se lee igual.
        // ⛔ 0079 ·5c: la clave es el commit de los documentos (del espejo, sin montar el
        //   árbol ni otro `git`: el índice de `/assets` es el camino caliente) más la
        //   `version()` de la ref en el catálogo (·5c·3, `lugar::version_de_la_ref`: una
        //   consulta, 0,6 ms p50).
        let version = match crate::lugar::version_de_la_ref(rama) {
            Ok(v) => v,
            Err(f) => return Err(Respuesta::error(f.codigo(), f.to_string())),
        };
        let con_version = |c: String| format!("{c}|{version}");
        let clave = match forja.clave_para_leer(rama).map(con_version) {
            Ok(c) => Some((rama_nombre.clone(), c)),
            Err(crate::git::Fallo::SinRama(r)) => {
                return Err(Respuesta::error(404, format!("no hay ninguna rama `{r}`")));
            }
            Err(_) => None,
        };
        let turno = clave.as_ref().map(|k| self.assets_cache.turno(k));
        if let Some(Turno::Hecho(g)) = &turno {
            crate::fases::indice(true, t.elapsed());
            return Ok((g.clone(), true));
        }
        let _obra = self.assets_cache.obra();
        // ⭐ 0044 C.2 ③: el catálogo de una rama lee de `main` al día lo que
        //   ella no tocó. ⭐ F3b: del árbol de LECTURA compartido del espejo —el
        //   mismo que `/fuentes` o `/arbol`—, no de un clon para escribir.
        let (arbol, clave_real) = match forja.para_leer_con_clave(rama, clave.is_none()) {
            Err(crate::git::Fallo::SinRama(r)) => {
                return Err(Respuesta::error(404, format!("no hay ninguna rama `{r}`")));
            }
            Err(e) => return Err(Respuesta::error(502, e.to_string())),
            Ok(x) => x,
        };
        let t = std::time::Instant::now();
        // 0079 ·5c·3: los punteros, de la ref que se lee (no pasa por la costura).
        let en = crate::lugar::EnRef::poner(rama);
        let g = Arc::new(Guardado::de(indice_con(
            arbol.ruta(),
            Cabeza {
                cabeza: crate::documentos::cabeza_de(arbol.ruta()),
                rama: Some(rama_nombre.clone()),
                generado: Some(ahora()),
            },
            self.puestos.ejecuciones.encendido(),
        )));
        drop(en);
        crate::fases::indice(false, t.elapsed());
        if let Some(c) = clave_real.map(con_version) {
            self.assets_cache.guarda((rama_nombre, c), g.clone());
        }
        drop(turno);
        Ok((g, false))
    }

    /// `GET /assets/{commit}`: el catálogo como era —sin `main` superpuesto: eso
    /// es la historia—, en un clon que se mueve a ese commit.
    ///
    /// ⭐ 0079 ·5c P5: **sólo los documentos**, sin punteros. La ruta no tiene
    ///   clientes; un commit de git sólo dice los documentos cuando los punteros
    ///   viven en el catálogo, y lo mismo con ficheros, para que la respuesta no
    ///   dependa del interruptor. `cabeza` sigue siendo la de git.
    fn assets_de_commit(
        &self,
        forja: &crate::git::Forja,
        rama: Option<&str>,
        rama_nombre: String,
        commit: String,
    ) -> Result<(Arc<Guardado>, bool), Respuesta> {
        let clave = (rama_nombre.clone(), commit.clone());
        let t = std::time::Instant::now();
        if let Some(g) = self.assets_cache.lee(&clave) {
            crate::fases::indice(true, t.elapsed());
            return Ok((g, true));
        }
        let _obra = self.assets_cache.obra();
        let prestado = match forja.clonar_rama(rama) {
            Err(crate::git::Fallo::SinRama(r)) => {
                return Err(Respuesta::error(404, format!("no hay ninguna rama `{r}`")));
            }
            Err(e) => return Err(Respuesta::error(502, e.to_string())),
            Ok(p) => p,
        };
        if crate::documentos::git(prestado.ruta(), &["checkout", "-q", &commit]).is_none() {
            return Err(Respuesta::error(
                404,
                format!("no hay ningún commit `{commit}` en `{rama_nombre}`"),
            ));
        }
        let t = std::time::Instant::now();
        let g = Arc::new(Guardado::de(indice_entero(
            prestado.ruta(),
            Cabeza {
                cabeza: crate::documentos::cabeza_de(prestado.ruta()).or(Some(commit)),
                rama: Some(rama_nombre),
                generado: Some(ahora()),
            },
            self.puestos.ejecuciones.encendido(),
            false,
        )));
        crate::fases::indice(false, t.elapsed());
        self.assets_cache.guarda(clave, g.clone());
        Ok((g, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F1: una pasada de `git log` da, para cada ruta —fichero o carpeta—, lo
    /// mismo que `git log -1 -- <ruta>`; con una ruta tocada varias veces, la
    /// última; y con una que la historia no conoce, nada.
    #[test]
    fn una_pasada_da_la_misma_version_que_una_por_ruta() {
        let d = std::env::temp_dir().join(format!("ore-f1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("packages/p/repo")).unwrap();
        let git = |a: &[&str]| {
            let s = std::process::Command::new("git")
                .current_dir(&d)
                .args(["-c", "user.name=ana", "-c", "user.email=ana@sujeto.invalid"])
                .args(a)
                .output()
                .unwrap();
            assert!(
                s.status.success(),
                "{a:?}: {}",
                String::from_utf8_lossy(&s.stderr)
            );
        };
        git(&["init", "-q"]);
        for (f, t) in [
            ("packages/p/a.yaml", "1"),
            ("packages/p/b.yaml", "1"),
            ("packages/p/a.yaml", "2"),
            ("packages/p/repo/x.py", "1"),
            ("otra.txt", "1"),
        ] {
            std::fs::write(d.join(f), t).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-qm", f]);
        }
        let rutas: std::collections::BTreeSet<String> = [
            "packages/p/a.yaml",
            "packages/p/b.yaml",
            "packages/p/repo",
            "packages/p",
            "packages/p/nadie.yaml",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let v = versiones_de(&d, &rutas);
        for r in &rutas {
            assert_eq!(
                v.get(r).map(|j| j.jcs()),
                version_de(&d, r).map(|j| j.jcs()),
                "{r}"
            );
        }
        assert!(!v.contains_key("packages/p/nadie.yaml"));
        assert_eq!(v.get("packages/p/repo"), v.get("packages/p"));
        assert_ne!(v.get("packages/p/a.yaml"), v.get("packages/p/b.yaml"));
        let _ = std::fs::remove_dir_all(&d);
    }

    fn arbol() -> std::path::PathBuf {
        arbol_en("indice")
    }

    /// Un árbol por prueba: dos pruebas en paralelo sobre el mismo directorio
    /// se lo borraban la una a la otra.
    fn arbol_en(nombre: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("ore-assets-serve-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["packages/v/tables", "packages/v/datasets", "datasets"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: v, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/tables/origen.yaml"),
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: origen, namespace: v }\nspec:\n  datasource: pg\n  object: public.origen\n  columns: { id: { type: Integer } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/datasets/pedidos.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n",
        )
        .unwrap();
        crate::lugar::de_la_prueba::poner_puntero(
            &d,
            "v.pedidos",
            "{\"estado\": \"copiada\", \"filas\": 2, \"dataset\": \"datasets/v_pedidos\"}\n",
        );
        d
    }

    /// docs/ejecuciones.md X2 · Con `--executions`, `system` › `default` ›
    /// `executions` en el índice, como una base con su dataset y sus columnas.
    #[test]
    fn con_executions_el_indice_trae_la_base_system() {
        // su propio árbol: con `arbol()` compartía `indice` con la prueba de abajo y,
        // en paralelo, una le borraba el directorio a la otra (el índice salía vacío)
        let d = arbol_en("system");
        let sin = indice_de(&d, Cabeza::default());
        assert!(!sin.jcs().contains("system.executions"));
        let j = indice_con(&d, Cabeza::default(), true);
        let Json::Obj(m) = &j else { panic!() };
        let Json::Arr(paquetes) = &m["paquetes"] else {
            panic!()
        };
        let system = paquetes
            .iter()
            .find(|p| matches!(p, Json::Obj(x) if x.get("name") == Some(&Json::s("system"))))
            .expect("la base system");
        let Json::Obj(system) = system else { panic!() };
        assert_eq!(system.get("type"), Some(&Json::s("standard")));
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        let it = items
            .iter()
            .find(|(k, _)| k.ends_with("system.executions"))
            .map(|(_, v)| v.jcs())
            .unwrap_or_else(|| panic!("{:?}", items.keys().collect::<Vec<_>>()));
        assert!(it.contains("queued_at"), "{it}");
        assert!(it.contains("\"system\":true"), "{it}");
        assert_eq!(system.get("system"), Some(&Json::Bool(true)));
    }

    #[test]
    fn el_indice_de_un_directorio_lleva_el_puntero_y_sin_historia_no_lleva_version() {
        let d = arbol();
        let j = indice_de(&d, Cabeza::default());
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        assert_eq!(items.len(), 2, "{:?}", items.keys().collect::<Vec<_>>());
        let Json::Obj(ds) = &items["dataset:v.pedidos"] else {
            panic!()
        };
        let Json::Obj(p) = &ds["puntero"] else {
            panic!()
        };
        assert_eq!(p["estado"], Json::s("copiada"));
        assert_eq!(ds["version"], Json::Crudo("null".into()));
        let Json::Obj(t) = &items["table:v.origen"] else {
            panic!()
        };
        let Json::Arr(r) = &t["relaciones"] else {
            panic!()
        };
        assert_eq!(r.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0063 S2 · Cada dataset lleva su salud, calculada al servir: este
    /// puntero (de antes de S2, sin `datos_de`) no sabe de cuándo es, y no se
    /// inventa la hora del commit.
    #[test]
    fn servido_es_lo_mismo_que_el_arbol_anotado() {
        // F7a: el texto cosido es, byte a byte, el árbol anotado de siempre —con
        // salud en lo que la tiene, sin ella en lo que no (la tabla en vivo sin
        // nada sabido), y las sumas de cada base—, a dos horas distintas, y desde
        // la caché o no.
        for (fixture, puntero) in [
            ("f7a-a", None),
            (
                "f7a-b",
                Some(
                    r#"{"estado":"error","metadata_location":"m","datos_de":"2026-10-13T12:00:00Z",
            "dataset":"datasets/v_pedidos","motivo":"falta `cliente`",
            "intento":{"cuando":"2026-10-13T13:00:00Z","resultado":"error"},
            "esquema_origen":{"added":["comprador"],"removed":["cliente"],"visto":"2026-10-13T13:00:00Z"}}"#,
                ),
            ),
        ] {
            let d = arbol_en(fixture);
            if let Some(p) = puntero {
                crate::lugar::de_la_prueba::poner_puntero(&d, "v.pedidos", p);
            }
            let j = indice_de(&d, Cabeza::default());
            let g = Guardado::de(j.clone());
            for (ahora, dc) in [(1_791_900_000, false), (1_760_400_000, true)] {
                let mut esperado = j.clone();
                if let Json::Obj(m) = &mut esperado {
                    m.insert("desde_cache".into(), Json::Bool(dc));
                }
                anotar_salud(&mut esperado, ahora);
                assert_eq!(
                    g.cuerpo(dc, &g.anotado(ahora)),
                    esperado.jcs(),
                    "{fixture} {ahora}"
                );
                // Y la salud por ámbito, de la mini-copia, la misma; y la de un
                // activo (la que viaja con cada lectura).
                let anotada = g.salud_anotada(ahora);
                for camino in [
                    &[][..],
                    &["v"][..],
                    &["dataset:v.pedidos"][..],
                    &["table:v.origen"][..],
                ] {
                    assert_eq!(
                        salud_de(&anotada, camino).map(|x| x.jcs()).ok(),
                        salud_de(&esperado, camino).map(|x| x.jcs()).ok(),
                        "{fixture} {camino:?}"
                    );
                }
                let Json::Obj(m) = &esperado else { panic!() };
                let Json::Obj(items) = &m["items"] else {
                    panic!()
                };
                for r in ["dataset:v.pedidos", "table:v.origen"] {
                    let Json::Obj(it) = &items[r] else { panic!() };
                    assert_eq!(
                        g.salud_de_item(&g.anotado(ahora), r).map(|x| x.jcs()),
                        it.get("salud").map(|x| x.jcs()),
                        "{fixture} {r}"
                    );
                }
            }
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    /// La salud para servir: la primera vez, calculada; en otro cubo, la de
    /// antes enseguida y la nueva cuando acaba el hilo que la rehace —la misma
    /// que calculada a esa hora—.
    #[test]
    fn la_salud_vieja_se_sirve_mientras_se_rehace() {
        let d = arbol_en("swr");
        let j = indice_de(&d, Cabeza::default());
        let g = Arc::new(Guardado::de(j.clone()));
        let (t0, t1) = (1_791_900_000, 1_791_900_000 + 7 * 86_400);
        let a0 = g.reciente(t0);
        assert!(Arc::ptr_eq(&a0, &g.reciente(t0)), "el mismo cubo, la misma");
        let otra = g.reciente(t1);
        assert!(Arc::ptr_eq(&a0, &otra), "en otro cubo se sirve la de antes");
        let t = std::time::Instant::now();
        let nueva = loop {
            let x = g.reciente(t1);
            if !Arc::ptr_eq(&x, &a0) {
                break x;
            }
            assert!(
                t.elapsed() < std::time::Duration::from_secs(20),
                "no se rehízo"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let fresca = Guardado::de(j).anotado(t1);
        assert_eq!(g.cuerpo(true, &nueva), g.cuerpo(true, &fresca));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0068 C4·4 · lo escrito por un build con una versión de su entrada más
    /// vieja que la de hoy (`leido_de` de su puntero) dice `input-refreshed`.
    #[test]
    fn lo_construido_con_una_entrada_vieja_lo_dice() {
        let mut j = ore_core::json::Json::Obj(Default::default());
        let indice = r#"{"items":{
          "dataset:ref.capa":{"kind":"Dataset","puntero":{"estado":"al-dia","metadata_location":"m",
            "datos_de":"2026-10-20T10:00:00Z","comprobado":"2026-10-20T11:00:00Z"}},
          "dataset:ventas.enriquecido":{"kind":"Dataset","puntero":{"estado":"al-dia","metadata_location":"m",
            "datos_de":"2026-10-19T10:00:00Z","leido_de":{"ref.capa":"2026-10-19T10:00:00Z"}},
            "relaciones":[{"tipo":"sale_de","ref":"dataset:ref.capa"}]}}}"#;
        if let Json::Obj(m) = &mut j {
            let n = ore_core::parse::parse(indice).unwrap();
            m.insert(
                "items".into(),
                Json::de_node_fiel(n.get("items").unwrap().1),
            );
        }
        anotar_salud(&mut j, 1_792_000_000);
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        let Json::Obj(ds) = &items["dataset:ventas.enriquecido"] else {
            panic!()
        };
        let t = ds["salud"].jcs();
        assert!(
            t.contains(r#""asset":"ref.capa","code":"input-refreshed""#),
            "{t}"
        );
    }

    #[test]
    fn cada_dataset_lleva_su_salud_al_servir() {
        let d = arbol_en("salud");
        let mut j = indice_de(&d, Cabeza::default());
        anotar_salud(&mut j, 1_791_900_000);
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        let Json::Obj(ds) = &items["dataset:v.pedidos"] else {
            panic!()
        };
        let t = ds["salud"].jcs();
        assert!(t.contains("\"state\":\"unknown\""), "{t}");
        assert!(t.contains("\"dataAsOf\":null"), "{t}");
        // Una tabla no es una copia: su salud llega con la capa 1.
        let Json::Obj(tb) = &items["table:v.origen"] else {
            panic!()
        };
        assert!(!tb.contains_key("salud"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0063 S6c · Lo que una copia vio de su tabla (el esquema cambió) lo dice
    /// la tabla en vivo; la base suma a sus activos, y `GET /health` lo da por
    /// ámbito y por activo.
    #[test]
    fn la_tabla_dice_lo_que_vio_su_copia_y_la_base_suma() {
        let d = arbol_en("salud-s6");
        crate::lugar::de_la_prueba::poner_puntero(
            &d,
            "v.pedidos",
            r#"{"estado":"error","metadata_location":"m","datos_de":"2026-10-13T12:00:00Z",
            "dataset":"datasets/v_pedidos","motivo":"falta `cliente`",
            "intento":{"cuando":"2026-10-13T13:00:00Z","resultado":"error"},
            "esquema_origen":{"added":["comprador"],"removed":["cliente"],"visto":"2026-10-13T13:00:00Z"}}"#,
        );
        let mut j = indice_de(&d, Cabeza::default());
        anotar_salud(&mut j, 1_791_900_000);
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        let Json::Obj(tb) = &items["table:v.origen"] else {
            panic!()
        };
        let t = tb["salud"].jcs();
        assert!(t.contains(r#""code":"source-schema-changed""#), "{t}");
        assert!(t.contains(r#""state":"degraded""#), "{t}");
        // La copia no lo recibe de vuelta como `upstream-degraded`: ya lo dice.
        let Json::Obj(ds) = &items["dataset:v.pedidos"] else {
            panic!()
        };
        assert!(!ds["salud"].jcs().contains("upstream-degraded"));

        let todo = salud_de(&j, &[]).ok().unwrap().jcs();
        assert!(todo.contains(r#""degraded":2"#), "{todo}");
        let base = salud_de(&j, &["v"]).ok().unwrap().jcs();
        assert!(base.contains(r#""needsAttention":2"#), "{base}");
        assert!(base.contains("table:v.origen") && base.contains("dataset:v.pedidos"));
        let uno = salud_de(&j, &["table:v.origen"]).ok().unwrap().jcs();
        assert!(uno.contains("source-schema-changed"), "{uno}");
        assert_eq!(salud_de(&j, &["nadie"]).err().map(|r| r.codigo), Some(404));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0062 · Un puntero a un origen lleva lo que su catálogo contó o estimó,
    /// diciendo cuál de las dos cosas es; un conjunto de objetos, cuántos y
    /// cuánto pesan.
    #[test]
    fn un_puntero_lleva_las_estadisticas_del_catalogo_de_su_fuente() {
        let d = std::env::temp_dir().join(format!("ore-assets-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("packages/lago/tables")).unwrap();
        std::fs::create_dir_all(d.join("packages/lago/objects")).unwrap();
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha28\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: lago, type: azure, connectionEnv: LAGO_URL }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/lago/package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: lago, version: 0.1.0, status: active, domain: lago }\nspec: { owner: team:d }\n",
        )
        .unwrap();
        for (n, o) in [("viajes", "part-0.parquet"), ("train", "train.csv")] {
            std::fs::write(
                d.join(format!("packages/lago/tables/{n}.yaml")),
                format!("apiVersion: oos.dev/v1alpha16\nkind: Table\nmetadata: {{ name: {n}, namespace: lago }}\nspec:\n  datasource: lago\n  object: \"{o}\"\n  format: {{ type: csv }}\n  columns: {{ id: {{ type: Integer }} }}\n  reads: {{ fullScan: cheap }}\n  changes: {{ mode: retract, witness: listing }}\n"),
            )
            .unwrap();
        }
        std::fs::write(
            d.join("packages/lago/objects/raiz_gz.yaml"),
            "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: { name: raiz_gz, namespace: lago }\nspec:\n  datasource: lago\n  prefix: \"\"\n  match: \"*.gz\"\n  media: archive\n  reads: { fullScan: cheap }\n  changes: { mode: retract, witness: listing }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/lago/discover.catalog.json"),
            r#"{ "source": "lago",
                 "tables": [ { "name": "default.viajes", "columns": [], "rows": 630889 },
                             { "name": "default.train", "columns": [], "rowsEstimate": 59800 } ],
                 "objects": [ { "name": "default.raiz_gz", "prefix": "", "media": "archive", "count": 4, "bytes": 11594722 } ] }"#,
        )
        .unwrap();
        let j = indice_de(&d, Cabeza::default());
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        let est = |r: &str| -> Json {
            let Json::Obj(it) = &items[r] else {
                panic!("{r}")
            };
            it.get("estadisticas")
                .cloned()
                .unwrap_or_else(|| panic!("{r} sin estadisticas: {it:?}"))
        };
        assert_eq!(
            est("table:lago.viajes"),
            Json::obj([
                ("filas", Json::Int(630889)),
                ("filas_son", Json::s("contadas"))
            ])
        );
        assert_eq!(
            est("table:lago.train"),
            Json::obj([
                ("filas", Json::Int(59800)),
                ("filas_son", Json::s("estimadas"))
            ])
        );
        assert_eq!(
            est("objecttable:lago.raiz_gz"),
            Json::obj([("objetos", Json::Int(4)), ("bytes", Json::Int(11594722))])
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn un_indice_se_hace_una_vez_aunque_lo_pidan_a_la_vez() {
        let c = Arc::new(Cache::<Json>::default());
        let hechos = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let k: Clave = ("r".into(), "c+m".into());
        let hilos: Vec<_> = (0..8)
            .map(|_| {
                let (c, hechos, k) = (c.clone(), hechos.clone(), k.clone());
                std::thread::spawn(move || match c.turno(&k) {
                    Turno::Hecho(j) => (*j).clone(),
                    Turno::Mio { _turno: _t } => {
                        hechos.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        let j = Arc::new(Json::Int(7));
                        c.guarda(k.clone(), j.clone());
                        (*j).clone()
                    }
                })
            })
            .collect();
        for h in hilos {
            assert_eq!(h.join().unwrap(), Json::Int(7));
        }
        assert_eq!(hechos.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn la_cache_no_pasa_de_su_peso_pero_guarda_el_ultimo() {
        let c = Cache::<Json>::default();
        let grande = |n: usize| Arc::new(Json::s("x".repeat(n)));
        c.guarda(("a".into(), "1".into()), grande(PESO / 2));
        c.guarda(("b".into(), "1".into()), grande(PESO / 2));
        assert!(c.lee(&("a".into(), "1".into())).is_none(), "pasó del peso");
        assert!(c.lee(&("b".into(), "1".into())).is_some());
        c.guarda(("c".into(), "1".into()), grande(PESO * 2));
        assert!(
            c.lee(&("c".into(), "1".into())).is_some(),
            "el último se queda"
        );
        assert_eq!(c.entradas.lock().unwrap().lista.len(), 1);
    }

    #[test]
    fn la_cache_recuerda_las_ultimas_cabezas_y_olvida_la_mas_vieja() {
        let c = Cache::default();
        for i in 0..(CABEZAS + 1) {
            c.guarda(
                ("main".into(), format!("c{i}")),
                Arc::new(Json::Int(i as i64)),
            );
        }
        assert!(c.lee(&("main".into(), "c0".into())).is_none());
        assert_eq!(
            *c.lee(&("main".into(), format!("c{CABEZAS}"))).unwrap(),
            Json::Int(CABEZAS as i64)
        );
        // la misma clave otra vez no duplica
        c.guarda(("main".into(), "c1".into()), Arc::new(Json::Int(99)));
        assert_eq!(c.entradas.lock().unwrap().lista.len(), CABEZAS);
    }

    #[test]
    fn la_fecha_civil_cuadra() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20717), (2026, 9, 21));
    }
}
