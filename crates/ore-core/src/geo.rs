//! v1alpha32 (ORE 0067 G1·2) · **El valor de una geometría**: WKB, y cómo
//! viaja en texto (`01-la-geometria` §3, §5).
//!
//! El contrato entre capas de 0067 dice que por el protocolo de texto de un
//! driver una geometría es **su WKB en hexadecimal**, y que en Arrow es
//! `Binary` con la extensión `geoarrow.wkb`. Este módulo es lo que hay entre
//! las dos: leer el hexadecimal, **validar** el WKB entero (no solo la
//! cabecera: un recuento que miente o un trozo que falta se dicen aquí, no en
//! el lector de quien consulta), y dejarlo en la forma única que guarda el lago.
//!
//! # Por qué se normaliza
//!
//! El mismo punto llega de tres maneras: WKB ISO de un GeoParquet, EWKB de
//! PostGIS (con el SRID dentro y las dimensiones en banderas) y en orden de
//! bytes grande o pequeño. El lago guarda **una**: WKB ISO, *little-endian*, sin
//! SRID (el CRS es del tipo de la columna, §3). Así el mismo valor tiene los
//! mismos bytes venga de donde venga, que es lo que el digest y las
//! estadísticas necesitan.
//!
//! # Por qué aquí
//!
//! Porque lo usan la copia (`ore-store`), los drivers y las pruebas. WKB son
//! siete tipos y una regla de anidamiento: se lee aquí, sin dependencias. WKT y
//! GeoJSON (G3·1, [`a_wkb`]) los leen los *crates* `wkt` y `geojson` (medidos en
//! G3·0: 0,7 y 1,2 M de geometrías/s), y se escriben a WKB aquí, para que lo
//! valide el mismo [`leer`].

/// El tipo de geometría de un WKB (los siete de OGC Simple Features).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Point = 1,
    LineString = 2,
    Polygon = 3,
    MultiPoint = 4,
    MultiLineString = 5,
    MultiPolygon = 6,
    GeometryCollection = 7,
}

impl Base {
    fn de(n: u32) -> Option<Base> {
        Some(match n {
            1 => Base::Point,
            2 => Base::LineString,
            3 => Base::Polygon,
            4 => Base::MultiPoint,
            5 => Base::MultiLineString,
            6 => Base::MultiPolygon,
            7 => Base::GeometryCollection,
            _ => return None,
        })
    }

    /// El nombre de OOS (`01-la-geometria` §1).
    pub fn nombre(self) -> &'static str {
        match self {
            Base::Point => "Point",
            Base::LineString => "LineString",
            Base::Polygon => "Polygon",
            Base::MultiPoint => "MultiPoint",
            Base::MultiLineString => "MultiLineString",
            Base::MultiPolygon => "MultiPolygon",
            Base::GeometryCollection => "GeometryCollection",
        }
    }

    /// Lo que puede ir dentro de un `Multi*`; `None` en una colección (cualquiera).
    fn hijo(self) -> Option<Base> {
        match self {
            Base::MultiPoint => Some(Base::Point),
            Base::MultiLineString => Some(Base::LineString),
            Base::MultiPolygon => Some(Base::Polygon),
            _ => None,
        }
    }
}

/// Las dimensiones de un WKB: `XY`, `Z`, `M` o `ZM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dims {
    pub z: bool,
    pub m: bool,
}

impl Dims {
    fn n(self) -> usize {
        2 + self.z as usize + self.m as usize
    }

    /// El sufijo de OOS y de WKT: `""`, `" Z"`, `" M"` o `" ZM"`.
    pub fn sufijo(self) -> &'static str {
        match (self.z, self.m) {
            (false, false) => "",
            (true, false) => " Z",
            (false, true) => " M",
            (true, true) => " ZM",
        }
    }
}

/// Un WKB leído y validado.
#[derive(Debug, Clone, PartialEq)]
pub struct Wkb {
    /// El mismo valor en la forma que guarda el lago: WKB ISO, *little-endian*,
    /// sin SRID.
    pub iso: Vec<u8>,
    pub base: Base,
    pub dims: Dims,
    /// Una geometría vacía (`POINT EMPTY`: coordenadas NaN; las demás, sin
    /// partes). Es un valor, no un nulo (§3).
    pub vacia: bool,
    /// El SRID de un EWKB, si lo traía.
    pub srid: Option<u32>,
    /// `[xmin, ymin, xmax, ymax]` de las coordenadas que no son NaN; `None`
    /// si no hay ninguna.
    pub bbox: Option<[f64; 4]>,
}

impl Wkb {
    /// El subtipo de OOS de este valor: `Point`, `Polygon Z`…
    pub fn subtipo(&self) -> String {
        format!("{}{}", self.base.nombre(), self.dims.sufijo())
    }
}

/// Las banderas de EWKB (PostGIS) en el entero del tipo.
const EWKB_Z: u32 = 0x8000_0000;
const EWKB_M: u32 = 0x4000_0000;
const EWKB_SRID: u32 = 0x2000_0000;

/// Lo que más se anida: un WKB más profundo es un error, no una pila agotada.
const PROFUNDIDAD: usize = 32;

struct Lector<'a> {
    b: &'a [u8],
    i: usize,
    salida: Vec<u8>,
    bbox: Option<[f64; 4]>,
}

impl Lector<'_> {
    fn bytes(&mut self, n: usize) -> Result<&[u8], String> {
        let fin = self
            .i
            .checked_add(n)
            .filter(|f| *f <= self.b.len())
            .ok_or_else(|| {
                format!(
                    "el WKB se acaba en el byte {} y faltaban {n} más",
                    self.b.len()
                )
            })?;
        let r = &self.b[self.i..fin];
        self.i = fin;
        Ok(r)
    }

    fn u32(&mut self, le: bool) -> Result<u32, String> {
        let b: [u8; 4] = self.bytes(4)?.try_into().expect("cuatro bytes");
        Ok(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn f64(&mut self, le: bool) -> Result<f64, String> {
        let b: [u8; 8] = self.bytes(8)?.try_into().expect("ocho bytes");
        Ok(if le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        })
    }

    /// Un recuento, comprobado contra lo que queda: un recuento que miente no
    /// reserva memoria ni da vueltas en vano.
    fn cuenta(&mut self, le: bool, minimo_por_elemento: usize) -> Result<u32, String> {
        let n = self.u32(le)?;
        let queda = self.b.len() - self.i;
        if (n as usize).saturating_mul(minimo_por_elemento) > queda {
            return Err(format!(
                "el WKB dice {n} elementos y solo quedan {queda} bytes"
            ));
        }
        self.salida.extend_from_slice(&n.to_le_bytes());
        Ok(n)
    }

    /// `n` coordenadas de `dims`; devuelve si todas eran NaN.
    fn coordenadas(&mut self, le: bool, puntos: u32, dims: Dims) -> Result<bool, String> {
        let mut todo_nan = true;
        for _ in 0..puntos {
            let mut xy = [0f64; 2];
            for k in 0..dims.n() {
                let v = self.f64(le)?;
                if let Some(c) = xy.get_mut(k) {
                    *c = v;
                }
                if !v.is_nan() {
                    todo_nan = false;
                }
                self.salida.extend_from_slice(&v.to_le_bytes());
            }
            if !xy[0].is_nan() && !xy[1].is_nan() {
                let c = self.bbox.get_or_insert([xy[0], xy[1], xy[0], xy[1]]);
                c[0] = c[0].min(xy[0]);
                c[1] = c[1].min(xy[1]);
                c[2] = c[2].max(xy[0]);
                c[3] = c[3].max(xy[1]);
            }
        }
        Ok(todo_nan)
    }

    /// Una geometría entera (cabecera y cuerpo). Devuelve su tipo, sus
    /// dimensiones, si está vacía y el SRID si lo traía.
    fn geometria(&mut self, profundidad: usize) -> Result<(Base, Dims, bool, Option<u32>), String> {
        if profundidad > PROFUNDIDAD {
            return Err(format!("el WKB anida más de {PROFUNDIDAD} niveles"));
        }
        let le = match self.bytes(1)?[0] {
            0 => false,
            1 => true,
            o => return Err(format!("`{o}` no es un orden de bytes de WKB (0 o 1)")),
        };
        let crudo = self.u32(le)?;
        let srid = if crudo & EWKB_SRID != 0 {
            Some(self.u32(le)?)
        } else {
            None
        };
        let codigo = crudo & !(EWKB_Z | EWKB_M | EWKB_SRID);
        let (iso_dims, base_n) = (codigo / 1000, codigo % 1000);
        if iso_dims > 3 || (iso_dims > 0 && crudo & (EWKB_Z | EWKB_M) != 0) {
            return Err(format!("`{crudo:#x}` no es un tipo de WKB"));
        }
        let dims = Dims {
            z: crudo & EWKB_Z != 0 || iso_dims == 1 || iso_dims == 3,
            m: crudo & EWKB_M != 0 || iso_dims == 2 || iso_dims == 3,
        };
        let base = Base::de(base_n)
            .ok_or_else(|| format!("`{base_n}` no es un tipo de geometría de WKB (1–7)"))?;
        // La cabecera, en ISO y little-endian.
        self.salida.push(1);
        let iso = base as u32 + 1000 * (dims.z as u32 + 2 * dims.m as u32);
        self.salida.extend_from_slice(&iso.to_le_bytes());
        let coord = 8 * dims.n();
        let vacia = match base {
            Base::Point => self.coordenadas(le, 1, dims)?,
            Base::LineString => {
                let n = self.cuenta(le, coord)?;
                self.coordenadas(le, n, dims)?;
                n == 0
            }
            Base::Polygon => {
                let anillos = self.cuenta(le, 4)?;
                for _ in 0..anillos {
                    let n = self.cuenta(le, coord)?;
                    self.coordenadas(le, n, dims)?;
                }
                anillos == 0
            }
            _ => {
                let partes = self.cuenta(le, 5)?;
                for _ in 0..partes {
                    let (b, d, _, _) = self.geometria(profundidad + 1)?;
                    if let Some(h) = base.hijo()
                        && b != h
                    {
                        return Err(format!(
                            "un `{}` dentro de un `{}`: solo van `{}`",
                            b.nombre(),
                            base.nombre(),
                            h.nombre()
                        ));
                    }
                    if d != dims {
                        return Err(format!(
                            "una parte `{}{}` dentro de un `{}{}`: las dimensiones no casan",
                            b.nombre(),
                            d.sufijo(),
                            base.nombre(),
                            dims.sufijo()
                        ));
                    }
                }
                partes == 0
            }
        };
        Ok((base, dims, vacia, srid))
    }
}

/// **Lee y valida un WKB** (ISO o EWKB, en cualquier orden de bytes), entero.
/// Lo que sobra al final también es un error: un WKB es exactamente una
/// geometría.
pub fn leer(b: &[u8]) -> Result<Wkb, String> {
    if b.is_empty() {
        return Err("un WKB vacío no es una geometría (la geometría vacía es `POINT EMPTY`, y la ausencia, un nulo)".into());
    }
    let mut l = Lector {
        b,
        i: 0,
        salida: Vec::with_capacity(b.len()),
        bbox: None,
    };
    let (base, dims, vacia, srid) = l.geometria(0)?;
    if l.i != b.len() {
        return Err(format!(
            "al WKB le sobran {} bytes tras la geometría",
            b.len() - l.i
        ));
    }
    Ok(Wkb {
        iso: l.salida,
        base,
        dims,
        vacia,
        srid,
        bbox: l.bbox,
    })
}

/// El hexadecimal de un WKB, como lo da PostGIS (mayúsculas).
pub fn a_hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(D[(x >> 4) as usize] as char);
        s.push(D[(x & 15) as usize] as char);
    }
    s
}

/// Del hexadecimal (mayúsculas o minúsculas, con `\x` delante si viene de un
/// `bytea` de PostgreSQL) a los bytes.
pub fn de_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    let s = s.strip_prefix("\\x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        return Err(format!(
            "`{}…` no es hexadecimal: tiene un número impar de cifras",
            corto(s)
        ));
    }
    let cifra = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    s.as_bytes()
        .chunks(2)
        .map(|p| match (cifra(p[0]), cifra(p[1])) {
            (Some(a), Some(b)) => Ok(a << 4 | b),
            _ => Err(format!(
                "`{}…` no es WKB en hexadecimal (¿un WKT o un GeoJSON? por el texto, una geometría viaja como su WKB en hexadecimal)",
                corto(s)
            )),
        })
        .collect()
}

fn corto(s: &str) -> &str {
    let mut fin = s.len().min(24);
    while !s.is_char_boundary(fin) {
        fin -= 1;
    }
    &s[..fin]
}

/// ¿Casa el SRID de un EWKB con el CRS de la columna? `0` es «sin SRID» en
/// PostGIS, y no dice nada. `4326` casa con `EPSG:4326` y con `OGC:CRS84` (el
/// mismo datum, con los ejes x/y que el valor ya lleva, §1).
pub fn srid_casa(srid: u32, crs: &str) -> bool {
    srid == 0 || crs == format!("EPSG:{srid}") || (srid == 4326 && crs == crate::types::CRS_DEFECTO)
}

/// **El valor de una columna geo, desde el texto del protocolo**: el
/// hexadecimal, leído, validado contra el tipo de la columna (subtipo,
/// dimensiones, SRID y, en una `Geography`, el rango) y normalizado a WKB ISO.
/// El error dice qué falla, para que quien escribe lo diga con la columna y la
/// fila (§3).
pub fn valor_de_texto(
    hex: &str,
    subtipo: &str,
    crs: &str,
    esferica: bool,
) -> Result<Vec<u8>, String> {
    // ORE 0067 G3·1: por el códec, que además de hex WKB entiende WKT y GeoJSON.
    a_wkb(Valor::Texto(hex), subtipo, crs, esferica)
}

/// Lo de [`valor_de_texto`] sobre un WKB ya leído.
pub fn comprobar(w: &Wkb, subtipo: &str, crs: &str, esferica: bool) -> Result<(), String> {
    if subtipo != crate::types::GEO_SUBTIPO_DEFECTO && w.subtipo() != subtipo {
        return Err(format!("un `{}` en una columna `{subtipo}`", w.subtipo()));
    }
    if let Some(s) = w.srid
        && !srid_casa(s, crs)
    {
        return Err(format!(
            "el valor dice SRID {s} y la columna está en `{crs}`: cambiar de CRS es reproyectar, y eso no lo hace quien escribe"
        ));
    }
    if esferica
        && let Some([x0, y0, x1, y1]) = w.bbox
        && (x0 < -180.0 || x1 > 180.0 || y0 < -90.0 || y1 > 90.0)
    {
        return Err(format!(
            "una `Geography` va de −180 a 180 de longitud y de −90 a 90 de latitud, y este valor llega a [{x0}, {y0}, {x1}, {y1}]"
        ));
    }
    Ok(())
}

/// v1alpha32 (ORE 0067 G1·4, aquí desde G3·1) · **El tipo geo nativo de un origen**, desde el
/// tipo que el origen dice (`physicalType`): `geometry(MultiPolygon,25830)` de
/// PostGIS es `Geometry<MultiPolygon, EPSG:25830>`; `geography(Point,4326)`,
/// `Geography<Point>`; `GEOGRAPHY` de BigQuery, `Geography`. Una `geometry`
/// sin SRID no dice su CRS y no se adivina (`None`: queda la pregunta de tipo).
pub fn tipo_nativo(origen: &str) -> Option<String> {
    let o = origen.trim().to_ascii_lowercase();
    if o == "geography" {
        return Some("Geography".into());
    }
    let (ctor, args) = if let Some(r) = o.strip_prefix("geometry(") {
        ("Geometry", r)
    } else {
        ("Geography", o.strip_prefix("geography(")?)
    };
    let args = args.strip_suffix(')')?;
    let mut partes = args.split(',').map(str::trim);
    let crudo = partes.next()?;
    let srid: u32 = partes.next()?.parse().ok()?;
    // PostGIS: `PointZ`, `MultiPolygonM`, `GeometryZM`… → subtipo y dimensión de OOS.
    let (base, dim) = ["zm", "z", "m"]
        .iter()
        .find_map(|d| {
            crudo
                .strip_suffix(d)
                .filter(|b| SUBTIPO_PG.iter().any(|(pg, _)| pg == b))
                .map(|b| (b, *d))
        })
        .unwrap_or((crudo, ""));
    let (_, subtipo) = SUBTIPO_PG.iter().find(|(pg, _)| *pg == base)?;
    let subtipo = match dim {
        "" => subtipo.to_string(),
        d => format!("{subtipo} {}", d.to_ascii_uppercase()),
    };
    let t = if ctor == "Geography" {
        if srid != 4326 {
            return None;
        }
        format!("Geography<{subtipo}>")
    } else if srid == 0 {
        return None;
    } else {
        format!("Geometry<{subtipo}, EPSG:{srid}>")
    };
    // La forma canónica (sin lo que vale por defecto), por el mismo analizador;
    // lo que no analiza (`Any Z`) no se propone.
    crate::types::parse_type(&t).ok().map(|x| x.to_string())
}

/// Los subtipos de PostGIS (en minúsculas) y su nombre en OOS.
const SUBTIPO_PG: &[(&str, &str)] = &[
    ("geometry", "Any"),
    ("point", "Point"),
    ("linestring", "LineString"),
    ("polygon", "Polygon"),
    ("multipoint", "MultiPoint"),
    ("multilinestring", "MultiLineString"),
    ("multipolygon", "MultiPolygon"),
    ("geometrycollection", "GeometryCollection"),
];

// ── ORE 0067 G3·1 · El códec de entrada ─────────────────────────────────────

/// Una geometría como llega de un origen: texto (lo que da un driver por el
/// protocolo de texto, o una columna `Utf8` de Arrow) o bytes (`Binary`).
#[derive(Debug, Clone, Copy)]
pub enum Valor<'a> {
    Texto(&'a str),
    Bytes(&'a [u8]),
}

/// **ORE 0067 G3·1 · El códec de entrada del lago.** Toda geometría que entra
/// pasa por aquí, por texto o por Arrow: WKB o EWKB (en binario o en hex, con
/// `\x` delante o sin él), WKT o EWKT (`SRID=4326;POINT(…)`) y GeoJSON (la
/// geometría, no un `Feature`). Devuelve el WKB ISO validado contra el tipo de
/// la columna, como [`valor_de_texto`]. Un origen nuevo que dé otra forma se
/// arregla aquí, con su caso en el corpus, y no en su driver.
pub fn a_wkb(v: Valor, subtipo: &str, crs: &str, esferica: bool) -> Result<Vec<u8>, String> {
    let w = match v {
        Valor::Bytes(b) if matches!(b.first(), Some(0 | 1)) => leer(b)?,
        Valor::Bytes(b) => match std::str::from_utf8(b) {
            Ok(t) => de_texto(t)?,
            Err(_) => {
                return Err(
                    "ni WKB ni texto: el primer byte no es un orden de bytes de WKB".into(),
                );
            }
        },
        Valor::Texto(t) => de_texto(t)?,
    };
    comprobar(&w, subtipo, crs, esferica)?;
    Ok(w.iso)
}

fn de_texto(t: &str) -> Result<Wkb, String> {
    let s = t.trim();
    let s = s.strip_prefix("\\x").unwrap_or(s);
    if s.starts_with('{') {
        return de_geojson(s);
    }
    if !s.is_empty() && s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return leer(&de_hex(s)?);
    }
    let (srid, wkt) = match s.get(..5) {
        Some(p) if p.eq_ignore_ascii_case("SRID=") => {
            let (n, resto) = s[5..]
                .split_once(';')
                .ok_or_else(|| format!("`{}…` es EWKT sin `;` tras el SRID", corto(s)))?;
            let n: u32 = n
                .trim()
                .parse()
                .map_err(|_| format!("`{}…`: el SRID del EWKT no es un número", corto(s)))?;
            (Some(n), resto)
        }
        _ => (None, s),
    };
    let mut w = de_wkt(wkt)?;
    w.srid = srid;
    Ok(w)
}

/// Escribe WKB ISO (*little-endian*) y lo pasa por [`leer`], que es quien
/// valida: el códec no tiene reglas propias.
struct Escritor(Vec<u8>);

impl Escritor {
    fn cabecera(&mut self, base: Base, dims: Dims, n: Option<usize>) {
        self.0.push(1);
        let t = base as u32 + if dims.z { 1000 } else { 0 } + if dims.m { 2000 } else { 0 };
        self.0.extend_from_slice(&t.to_le_bytes());
        if let Some(n) = n {
            self.0.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
    fn coord(&mut self, c: &[f64]) {
        for x in c {
            self.0.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn fin(self) -> Result<Wkb, String> {
        leer(&self.0)
    }
}

fn wkt_dims(d: wkt::types::Dimension) -> Dims {
    use wkt::types::Dimension as D;
    Dims {
        z: matches!(d, D::XYZ | D::XYZM),
        m: matches!(d, D::XYM | D::XYZM),
    }
}

fn wkt_coord(c: &wkt::types::Coord<f64>, d: Dims) -> Vec<f64> {
    let mut v = vec![c.x, c.y];
    if d.z {
        v.push(c.z.unwrap_or(f64::NAN));
    }
    if d.m {
        v.push(c.m.unwrap_or(f64::NAN));
    }
    v
}

fn de_wkt(s: &str) -> Result<Wkb, String> {
    let g: wkt::Wkt<f64> = s.parse().map_err(|e| {
        format!(
            "`{}…` no es una geometría en WKB, WKT ni GeoJSON (WKT: {e})",
            corto(s)
        )
    })?;
    let mut e = Escritor(Vec::new());
    escribir_wkt(&mut e, &g);
    e.fin()
}

fn escribir_wkt(e: &mut Escritor, g: &wkt::Wkt<f64>) {
    use wkt::Wkt as W;
    let linea = |e: &mut Escritor, l: &wkt::types::LineString<f64>, d: Dims| {
        e.0.extend_from_slice(&(l.coords().len() as u32).to_le_bytes());
        for c in l.coords() {
            e.coord(&wkt_coord(c, d));
        }
    };
    let poligono = |e: &mut Escritor, p: &wkt::types::Polygon<f64>, d: Dims| {
        e.0.extend_from_slice(&(p.rings().len() as u32).to_le_bytes());
        for r in p.rings() {
            linea(e, r, d);
        }
    };
    let punto = |e: &mut Escritor, p: &wkt::types::Point<f64>, d: Dims| {
        e.cabecera(Base::Point, d, None);
        match p.coord() {
            Some(c) => e.coord(&wkt_coord(c, d)),
            None => e.coord(&vec![f64::NAN; d.n()]),
        }
    };
    match g {
        W::Point(p) => punto(e, p, wkt_dims(p.dimension())),
        W::LineString(l) => {
            let d = wkt_dims(l.dimension());
            e.cabecera(Base::LineString, d, None);
            linea(e, l, d);
        }
        W::Polygon(p) => {
            let d = wkt_dims(p.dimension());
            e.cabecera(Base::Polygon, d, None);
            poligono(e, p, d);
        }
        W::MultiPoint(m) => {
            let d = wkt_dims(m.dimension());
            e.cabecera(Base::MultiPoint, d, Some(m.points().len()));
            for p in m.points() {
                punto(e, p, d);
            }
        }
        W::MultiLineString(m) => {
            let d = wkt_dims(m.dimension());
            e.cabecera(Base::MultiLineString, d, Some(m.line_strings().len()));
            for l in m.line_strings() {
                e.cabecera(Base::LineString, d, None);
                linea(e, l, d);
            }
        }
        W::MultiPolygon(m) => {
            let d = wkt_dims(m.dimension());
            e.cabecera(Base::MultiPolygon, d, Some(m.polygons().len()));
            for p in m.polygons() {
                e.cabecera(Base::Polygon, d, None);
                poligono(e, p, d);
            }
        }
        W::GeometryCollection(c) => {
            let d = wkt_dims(c.dimension());
            e.cabecera(Base::GeometryCollection, d, Some(c.geometries().len()));
            for h in c.geometries() {
                escribir_wkt(e, h);
            }
        }
    }
}

fn de_geojson(s: &str) -> Result<Wkb, String> {
    let g: geojson::Geometry = s
        .parse()
        .map_err(|e| format!("`{}…` no es una geometría GeoJSON: {e}", corto(s)))?;
    let mut e = Escritor(Vec::new());
    escribir_geojson(&mut e, &g.value)?;
    e.fin()
}

/// Las dimensiones de un GeoJSON: XY, o Z si las posiciones traen tres números
/// (RFC 7946 no tiene M).
fn gj_dims(v: &geojson::GeometryValue) -> Dims {
    use geojson::GeometryValue as G;
    let n = match v {
        G::Point { coordinates } => coordinates.as_slice().len(),
        G::MultiPoint { coordinates } | G::LineString { coordinates } => {
            coordinates.first().map_or(2, |p| p.as_slice().len())
        }
        G::MultiLineString { coordinates } | G::Polygon { coordinates } => coordinates
            .iter()
            .flatten()
            .next()
            .map_or(2, |p| p.as_slice().len()),
        G::MultiPolygon { coordinates } => coordinates
            .iter()
            .flatten()
            .flatten()
            .next()
            .map_or(2, |p| p.as_slice().len()),
        G::GeometryCollection { .. } => 2,
    };
    Dims {
        z: n >= 3,
        m: false,
    }
}

fn escribir_geojson(e: &mut Escritor, v: &geojson::GeometryValue) -> Result<(), String> {
    use geojson::GeometryValue as G;
    let d = gj_dims(v);
    let pos = |e: &mut Escritor, p: &geojson::Position| -> Result<(), String> {
        let c = p.as_slice();
        if c.len() < 2 {
            return Err("una posición GeoJSON con menos de dos números".into());
        }
        e.coord(&c[..d.n().min(c.len())]);
        if c.len() < d.n() {
            e.coord(&[f64::NAN]);
        }
        Ok(())
    };
    let linea = |e: &mut Escritor, l: &[geojson::Position]| -> Result<(), String> {
        e.0.extend_from_slice(&(l.len() as u32).to_le_bytes());
        l.iter().try_for_each(|p| pos(e, p))
    };
    let poligono = |e: &mut Escritor, p: &[Vec<geojson::Position>]| -> Result<(), String> {
        e.0.extend_from_slice(&(p.len() as u32).to_le_bytes());
        p.iter().try_for_each(|r| linea(e, r))
    };
    match v {
        G::Point { coordinates } => {
            e.cabecera(Base::Point, d, None);
            pos(e, coordinates)?;
        }
        G::LineString { coordinates } => {
            e.cabecera(Base::LineString, d, None);
            linea(e, coordinates)?;
        }
        G::Polygon { coordinates } => {
            e.cabecera(Base::Polygon, d, None);
            poligono(e, coordinates)?;
        }
        G::MultiPoint { coordinates } => {
            e.cabecera(Base::MultiPoint, d, Some(coordinates.len()));
            for p in coordinates {
                e.cabecera(Base::Point, d, None);
                pos(e, p)?;
            }
        }
        G::MultiLineString { coordinates } => {
            e.cabecera(Base::MultiLineString, d, Some(coordinates.len()));
            for l in coordinates {
                e.cabecera(Base::LineString, d, None);
                linea(e, l)?;
            }
        }
        G::MultiPolygon { coordinates } => {
            e.cabecera(Base::MultiPolygon, d, Some(coordinates.len()));
            for p in coordinates {
                e.cabecera(Base::Polygon, d, None);
                poligono(e, p)?;
            }
        }
        G::GeometryCollection { geometries } => {
            e.cabecera(Base::GeometryCollection, d, Some(geometries.len()));
            for h in geometries {
                escribir_geojson(e, &h.value)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ORE 0067 G3·1 · El corpus del códec (`tests/geo-corpus.json`, con shapely
    /// como oráculo: `tests/geo-corpus.py`): cada geometría, en sus cinco formas
    /// y como bytes y como texto, da el mismo WKB ISO.
    #[test]
    fn el_codec_da_el_mismo_wkb_en_todas_las_formas() {
        let c: serde_json::Value =
            serde_json::from_str(include_str!("../tests/geo-corpus.json")).unwrap();
        let mut n = 0;
        for caso in c.as_array().unwrap() {
            let iso = caso["iso"].as_str().unwrap();
            let sub = caso["subtipo"].as_str().unwrap();
            for (forma, v) in caso["formas"].as_object().unwrap() {
                let v = v.as_str().unwrap();
                let w = a_wkb(Valor::Texto(v), sub, "EPSG:4326", false)
                    .unwrap_or_else(|e| panic!("{sub} · {forma}: {e}"));
                assert_eq!(a_hex(&w), iso, "{sub} · {forma} · texto");
                // Lo mismo como bytes: el binario de WKB, y el texto en un `Binary`.
                let b = if forma.ends_with("_hex") {
                    de_hex(v).unwrap()
                } else {
                    v.as_bytes().to_vec()
                };
                let w = a_wkb(Valor::Bytes(&b), sub, "EPSG:4326", false)
                    .unwrap_or_else(|e| panic!("{sub} · {forma} · bytes: {e}"));
                assert_eq!(a_hex(&w), iso, "{sub} · {forma} · bytes");
                n += 1;
            }
        }
        assert!(n >= 55, "{n}");
    }

    #[test]
    fn el_codec_dice_por_que_no() {
        let e = a_wkb(Valor::Texto("hola"), "Any", "OGC:CRS84", false).unwrap_err();
        assert!(e.contains("WKB, WKT ni GeoJSON"), "{e}");
        // El SRID de un EWKT que no casa con la columna.
        let e = a_wkb(
            Valor::Texto("SRID=25830;POINT(1 2)"),
            "Any",
            "OGC:CRS84",
            false,
        )
        .unwrap_err();
        assert!(e.contains("reproyectar"), "{e}");
        // El subtipo.
        let e = a_wkb(Valor::Texto("POINT(1 2)"), "Polygon", "OGC:CRS84", false).unwrap_err();
        assert!(e.contains("Point"), "{e}");
        // Una `Geography` fuera de rango.
        let e = a_wkb(
            Valor::Texto(r#"{"type":"Point","coordinates":[440000,4474000]}"#),
            "Any",
            "OGC:CRS84",
            true,
        )
        .unwrap_err();
        assert!(e.contains("Geography"), "{e}");
        // `\x` delante (lo que da `bytea` de Postgres por texto).
        assert!(
            a_wkb(
                Valor::Texto("\\x0101000000000000000000F03F0000000000000040"),
                "Point",
                "OGC:CRS84",
                false
            )
            .is_ok()
        );
    }

    /// WKB ISO little-endian a mano: cabecera y dobles.
    fn wkb(tipo: u32, cuerpo: &[u8]) -> Vec<u8> {
        let mut v = vec![1];
        v.extend_from_slice(&tipo.to_le_bytes());
        v.extend_from_slice(cuerpo);
        v
    }
    fn f(xs: &[f64]) -> Vec<u8> {
        xs.iter().flat_map(|x| x.to_le_bytes()).collect()
    }
    fn u(n: u32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }

    /// El punto de PostGIS que midió la vara (EWKB con SRID 4326) sale como
    /// WKB ISO sin SRID, y vuelve igual por el hexadecimal.
    #[test]
    fn el_ewkb_de_postgis_se_normaliza_a_iso() {
        let pg = "0101000020E6100000406A1327F7BB0DC053E8BCC62E454440";
        let w = leer(&de_hex(pg).unwrap()).unwrap();
        assert_eq!(
            (w.base, w.dims, w.vacia, w.srid),
            (Base::Point, Dims { z: false, m: false }, false, Some(4326))
        );
        assert_eq!(a_hex(&w.iso), "0101000000406A1327F7BB0DC053E8BCC62E454440");
        // La forma ISO es un punto fijo: leerla da los mismos bytes.
        assert_eq!(leer(&w.iso).unwrap().iso, w.iso);
        let [x, y, _, _] = w.bbox.unwrap();
        assert!(
            (x + 3.717).abs() < 0.01 && (y - 40.54).abs() < 0.01,
            "{x} {y}"
        );
        // Casa con OGC:CRS84 y con EPSG:4326; no con EPSG:25830.
        assert!(valor_de_texto(pg, "Point", "OGC:CRS84", false).is_ok());
        assert!(valor_de_texto(pg, "Any", "EPSG:4326", true).is_ok());
        let e = valor_de_texto(pg, "Point", "EPSG:25830", false).unwrap_err();
        assert!(e.contains("reproyectar"), "{e}");
        // Minúsculas y el `\x` de un bytea también valen.
        assert!(
            valor_de_texto(
                &format!("\\x{}", pg.to_lowercase()),
                "Point",
                "OGC:CRS84",
                false
            )
            .is_ok()
        );
    }

    /// Big-endian y dimensiones: el mismo `Point Z` en ISO big-endian y en
    /// EWKB con la bandera Z da los mismos bytes normalizados.
    #[test]
    fn el_orden_de_bytes_y_las_dimensiones_convergen() {
        let mut be = vec![0];
        be.extend_from_slice(&1001u32.to_be_bytes());
        for x in [1.5f64, 2.5, 30.0] {
            be.extend_from_slice(&x.to_be_bytes());
        }
        let ewkb = wkb(EWKB_Z | 1, &f(&[1.5, 2.5, 30.0]));
        let a = leer(&be).unwrap();
        let b = leer(&ewkb).unwrap();
        assert_eq!(a.iso, b.iso);
        assert_eq!(a.subtipo(), "Point Z");
        assert!(comprobar(&a, "Point Z", "EPSG:4979", false).is_ok());
        assert_eq!(
            comprobar(&a, "Point", "EPSG:4979", false).unwrap_err(),
            "un `Point Z` en una columna `Point`"
        );
        let zm = leer(&wkb(3003, &[u(1), u(4), f(&[0.0; 16])].concat())).unwrap();
        assert_eq!(zm.subtipo(), "Polygon ZM");
    }

    /// Nula ≠ vacía (§3): `POINT EMPTY` (NaN) y un polígono sin anillos son
    /// valores vacíos; un WKB sin bytes no es nada.
    #[test]
    fn la_vacia_es_un_valor() {
        let p = leer(&de_hex("0101000000000000000000F87F000000000000F87F").unwrap()).unwrap();
        assert!(p.vacia && p.bbox.is_none());
        let pol = leer(&wkb(3, &u(0))).unwrap();
        assert!(pol.vacia);
        assert!(comprobar(&pol, "Polygon", "OGC:CRS84", true).is_ok());
        assert!(leer(&[]).unwrap_err().contains("nulo"));
        assert!(de_hex("").unwrap().is_empty());
    }

    /// Lo compuesto: un multipolígono con dos partes vale; un punto dentro de
    /// un `MultiPolygon` o una parte con otras dimensiones, no.
    #[test]
    fn lo_compuesto_se_valida_entero() {
        let anillo = [u(1), u(4), f(&[0., 0., 1., 0., 1., 1., 0., 0.])].concat();
        let pol = wkb(3, &anillo);
        let mp = wkb(6, &[u(2), pol.clone(), pol.clone()].concat());
        let w = leer(&mp).unwrap();
        assert_eq!(
            (w.subtipo(), w.vacia, w.bbox),
            ("MultiPolygon".into(), false, Some([0., 0., 1., 1.]))
        );
        let mal = wkb(6, &[u(1), wkb(1, &f(&[0., 0.]))].concat());
        assert!(
            leer(&mal)
                .unwrap_err()
                .contains("un `Point` dentro de un `MultiPolygon`")
        );
        let dims = wkb(
            7,
            &[u(2), wkb(1, &f(&[0., 0.])), wkb(1001, &f(&[0., 0., 0.]))].concat(),
        );
        assert!(leer(&dims).unwrap_err().contains("no casan"));
        // Una colección admite cualquier cosa con sus mismas dimensiones.
        assert!(leer(&wkb(7, &[u(2), wkb(1, &f(&[0., 0.])), pol].concat())).is_ok());
    }

    /// Lo roto se dice, y nunca reserva lo que un recuento mentiroso pide.
    #[test]
    fn un_wkb_roto_dice_por_que() {
        let casos: &[(&[u8], &str)] = &[
            (&[1, 1, 0, 0], "se acaba"),
            (&[2, 1, 0, 0, 0], "orden de bytes"),
            (&wkb(9, &[]), "no es un tipo de geometría"),
            (&wkb(4001, &[]), "no es un tipo de WKB"),
            (&wkb(2, &u(u32::MAX)), "solo quedan"),
            (&[wkb(1, &f(&[0., 0.])), vec![0]].concat(), "le sobran 1"),
        ];
        for (b, dice) in casos {
            let e = leer(b).unwrap_err();
            assert!(e.contains(dice), "{e} · esperaba «{dice}»");
        }
        // Texto que no es hexadecimal: un WKT, con su pista.
        assert!(de_hex("POINT(1 2)").unwrap_err().contains("WKT"));
        assert!(de_hex("ABC").unwrap_err().contains("impar"));
        // Anidado sin fin: se corta.
        let mut b = wkb(1, &f(&[0., 0.]));
        for _ in 0..40 {
            b = wkb(7, &[u(1), b].concat());
        }
        assert!(leer(&b).unwrap_err().contains("anida"));
    }

    /// El subtipo y el rango de una `Geography` se cumplen al escribir.
    #[test]
    fn el_tipo_de_la_columna_se_cumple() {
        let p = a_hex(&wkb(1, &f(&[200., 10.])));
        assert!(valor_de_texto(&p, "Point", "EPSG:3857", false).is_ok());
        assert!(
            valor_de_texto(&p, "Point", "OGC:CRS84", true)
                .unwrap_err()
                .contains("−180 a 180")
        );
        let l = a_hex(&wkb(2, &[u(2), f(&[0., 0., 1., 1.])].concat()));
        assert_eq!(
            valor_de_texto(&l, "Point", "OGC:CRS84", false).unwrap_err(),
            "un `LineString` en una columna `Point`"
        );
        assert!(valor_de_texto(&l, "Any", "OGC:CRS84", false).is_ok());
    }
}
