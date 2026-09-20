// `ore` · el SDK del puesto para TS/JS (0031 W3.4). El mismo contrato que
// `puesto/python/ore`:
//
//   over("<paquete>.<vista>")   → las filas de la copia de esa vista (objetos)
//   sql("select … from p.v")    → las filas del resultado (DuckDB en el puesto)
//   persona()                   → quién abrió el puesto (`persona:…`)
//
// Los valores son los TIPADOS de DuckDB, que cumplen el contrato de tipos
// (0032 §1) sin añadir nada a la imagen: `bigint` para un entero de 64 bits,
// `DuckDBDecimalValue` (exacto) para un decimal, `DuckDBDateValue`,
// `DuckDBTimeValue`, `DuckDBTimestampValue` (hora de pared, `.micros`),
// `DuckDBTimestampTZValue` (un instante, `.micros` desde la época UTC),
// `DuckDBBlobValue` (`.bytes`), `DuckDBListValue` (`.items`)… Un `count(*)` es
// `3n`, no `3`: el tipo dice lo que es.
//
// Y EN NODE NO SE MATERIALIZAN 10 M DE FILAS (medido en 0032 T4: JS crea un
// objeto por valor, 0,7 M filas/s por cualquier camino). `over()` y `sql()`
// devuelven hasta `limite` filas (100 000 por defecto) y lo dicen:
// `filas.total` (las que hay, si se sabe), `filas.truncada`, `filas.tipos`
// (columna → tipo de Arrow). Con `{ estricto: true }` una respuesta que no cabe
// en el límite falla en vez de recortarse; lo masivo se agrega en SQL
// (60–140 ms para 10 M de filas) o se hace en Python. `{ como: "columnas" }`
// da `{ nombres, tipos, columnas }` —arrays por columna, sin objeto por fila—
// para quien recorra muchas filas.
//
// El código nunca ve el bucket ni una credencial: pregunta a `ore-serve` QUÉ
// copia es (con la identidad del puesto, que la resuelve en nombre de la
// persona y con su potestad) y baja el artefacto con la identidad del pod
// (el token del servidor de metadatos; sin librería de Google: la API JSON de
// GCS a pelo, que es lo único que hace falta). El sobre `ORECOPY1` se
// desenvuelve aquí; la carga es Parquet, y DuckDB (`@duckdb/node-api`) la lee.
//
// Fuera del clúster (las pruebas de fuego) el almacén es un directorio:
// `ORE_ALMACEN=dir:/ruta` lee `ore/v1/<clave>` de ahí.
//
// TODO ES UN DATASET (0031 §10): `datos` contesta o `metadata_location` —el
// `metadata.json` vigente de una tabla Iceberg en el bucket, que DuckDB lee EN
// SITIO (`iceberg_scan` sobre la raíz y la versión, con el token del pod como
// bearer; medido en `medida-w3-lago.py`)— o `clave`, el sobre ORECOPY1 heredado,
// que se baja una vez. `over()` y `sql()` no distinguen.
// **Escribir** (0031 §11, W3.6c): `write("p.t", filas)` deja un dataset —una tabla
// Iceberg en el lago, la `Table` en el árbol, el puntero— desde lo que `over()` o
// `sql()` devolvieron (filas tipadas, o `{ nombres, tipos, columnas }`) o desde
// objetos JS cualquiera. Node no lleva Arrow: la tabla se arma en DuckDB (tipada:
// el `appendValue` con el tipo de cada columna) y sale como Parquet a `ore-store`,
// el escritor de Rust, que la lleva al físico del contrato (0032) y escribe los
// ficheros **con la credencial que el catálogo prestó**, acotada a esa tabla; el
// commit va al catálogo REST de Iceberg de `ore-serve` (`/v1/…`). Idempotente por
// la clave de operación (del contenido); un 409 se reintenta; un 5xx se mira.
import { mkdirSync, existsSync, readFileSync, writeFileSync, renameSync, accessSync, constants, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, delimiter } from "node:path";
import { spawnSync } from "node:child_process";

const MAGIA = "ORECOPY1";

export const puesto = {
  servidor: (process.env.ORE_SERVE ?? "http://127.0.0.1:8080").replace(/\/+$/, ""),
  id: process.env.PUESTO ?? "",
  bucket: process.env.BUCKET ?? "",
  almacen: process.env.ORE_ALMACEN ?? "gcs",
  // Quién abrió el puesto: lo pone el agente al arrancar (de la ficha).
  persona: "",
  // El token lo pone el agente (`agente.mjs`) y lo renueva; una celda no lo ve.
  _cabeceras: {},
  /** `[código, cuerpo]` de una petición a ore-serve; el cuerpo, JSON o `{error}`. */
  async pedir(metodo, ruta, cuerpo, plazoMs = 30_000, cabeceras = {}) {
    const cab = { accept: "application/json", ...this._cabeceras, ...cabeceras };
    if (cuerpo !== undefined) cab["content-type"] = "application/json";
    // Desde qué puesto: el catálogo escribe en nombre de quien lo abrió.
    if (this.id) cab["x-ore-puesto"] = this.id;
    const r = await fetch(this.servidor + ruta, {
      method: metodo,
      headers: cab,
      body: cuerpo === undefined ? undefined : JSON.stringify(cuerpo),
      signal: AbortSignal.timeout(plazoMs),
    });
    const texto = await r.text();
    if (!texto.trim()) return [r.status, null];
    try { return [r.status, JSON.parse(texto)]; } catch { return [r.status, { error: texto.trim() }]; }
  },
};

/** Quién abrió el puesto (`persona:…`). Lo sabe el agente desde que reclama. */
export function persona() {
  if (!puesto.persona) throw new Error("persona(): el agente aún no sabe quién abrió el puesto");
  return puesto.persona;
}

// ── el almacén ─────────────────────────────────────────────────────────────
let tokenDelPod = { valor: "", caduca: 0 };

async function tokenDeGoogle() {
  if (Date.now() < tokenDelPod.caduca - 60_000) return tokenDelPod.valor;
  const r = await fetch("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token", {
    headers: { "Metadata-Flavor": "Google" },
    signal: AbortSignal.timeout(10_000),
  });
  if (!r.ok) throw new Error(`el servidor de metadatos contestó ${r.status}`);
  const t = await r.json();
  tokenDelPod = { valor: t.access_token, caduca: Date.now() + (t.expires_in ?? 300) * 1000 };
  return tokenDelPod.valor;
}

async function bajar(bucket, clave) {
  if (puesto.almacen.startsWith("dir:")) {
    return readFileSync(join(puesto.almacen.slice(4).replace(/\/+$/, ""), clave));
  }
  if (puesto.almacen === "gcs") {
    const url = `https://storage.googleapis.com/storage/v1/b/${encodeURIComponent(bucket)}/o/${encodeURIComponent(clave)}?alt=media`;
    const r = await fetch(url, { headers: { authorization: "Bearer " + (await tokenDeGoogle()) }, signal: AbortSignal.timeout(600_000) });
    if (!r.ok) throw new Error(`GCS contestó ${r.status} por ${clave}: ${(await r.text()).slice(0, 200)}`);
    return Buffer.from(await r.arrayBuffer());
  }
  throw new Error(`ORE_ALMACEN=${JSON.stringify(puesto.almacen)} no es un almacén: vale \`gcs\` o \`dir:<ruta>\``);
}

function desenvolver(crudo) {
  if (crudo.subarray(0, 8).toString("latin1") !== MAGIA) throw new Error("el artefacto no es una copia de ORE (sin `ORECOPY1`)");
  const n = crudo.readUInt32LE(8);
  return { cabecera: JSON.parse(crudo.subarray(12, 12 + n).toString("utf8")), carga: crudo.subarray(12 + n) };
}

async function resolver(vista) {
  if (typeof vista !== "string" || vista.split(".").length !== 2) throw new Error(`se quiere \`<paquete>.<vista>\`, no ${JSON.stringify(vista)}`);
  const [codigo, r] = await puesto.pedir("GET", `/puestos/${puesto.id}/datos/${vista}`);
  if (codigo === 409) throw new Error(`la copia de \`${vista}\` no está hecha: ${r?.error ?? ""}`);
  if (codigo === 404) throw new Error(`no hay ninguna \`View\` \`${vista}\` en el árbol`);
  if (codigo !== 200) throw new Error(`ore-serve contestó ${codigo} por \`${vista}\`: ${r?.error ?? JSON.stringify(r)}`);
  return r;
}

function copiasPorDefecto() {
  if (process.env.ORE_COPIAS) return process.env.ORE_COPIAS;
  try { accessSync("/trabajo", constants.W_OK); return "/trabajo/copias"; } catch { return join(tmpdir(), "ore-copias"); }
}

/** La copia de la vista como Parquet local, bajado UNA vez por sesión (la
 *  clave es el digest del artefacto: una clave nueva es otra copia). */
async function parquetDe(vista) {
  const r = await resolver(vista);
  const d = copiasPorDefecto();
  mkdirSync(d, { recursive: true });
  const f = join(d, r.clave.replaceAll("/", "_") + ".parquet");
  if (!existsSync(f)) {
    const { carga } = desenvolver(await bajar(r.bucket || puesto.bucket, r.clave));
    writeFileSync(f + ".parte", carga);
    renameSync(f + ".parte", f);
  }
  return [f, r];
}

/** De qué se lee la vista, como fragmento SQL de DuckDB: `iceberg_scan(...)` si es un
 *  dataset Iceberg (el puntero trae `metadata_location`), `read_parquet('…')` si es un
 *  sobre heredado (trae `clave`, y se baja una vez). */
async function fuenteDe(vista) {
  const r = await resolver(vista);
  if (r.metadata_location) {
    if (r.metadata_location.startsWith("s3://") && !s3) {
      const [ns, t] = vista.split(".");
      const [c, l] = await puesto.pedir("GET", `/v1/namespaces/${ns}/tables/${t}`, undefined, 30_000, DELEGAR);
      if (c === 200 && l?.config?.["s3.access-key-id"]) s3 = l.config;
    }
    return [await iceberg(r.metadata_location), r];
  }
  const [f] = await parquetDe(vista);
  return [`read_parquet('${f.replaceAll("'", "''").replaceAll("\\", "/")}')`, r];
}

/** `iceberg_scan` sobre la raíz de la tabla y la versión del puntero (con
 *  `allow_moved_paths` la raíz es lo que se le pasa, y así no lista nada). En el
 *  bucket, la API XML de GCS por https con el token del pod como bearer. */
async function iceberg(metadataLocation) {
  const i = metadataLocation.lastIndexOf("/metadata/");
  let raiz = metadataLocation.slice(0, i);
  const version = metadataLocation.slice(i + "/metadata/".length).replace(/\.metadata\.json$/, "");
  const con = await duckdb();
  // `iceberg` arrastra `avro` (y usa `json` e `icu`); con el autoinstalado apagado
  // hay que cargarlas por su nombre, en orden. `httpfs` sólo para el bucket.
  for (const e of LAGO) await cargar(con, e);
  if (raiz.startsWith("gs://")) {
    await cargar(con, "httpfs");
    await con.run(`create or replace secret ore_gcs (type http, bearer_token '${(await tokenDeGoogle()).replaceAll("'", "''")}')`);
    raiz = "https://storage.googleapis.com/" + raiz.slice(5);
  } else if (raiz.startsWith("s3://") && s3) {
    // Un S3 (R2, o el de mentira de las pruebas): con la credencial que el
    // catálogo prestó al escribir, o la de la tabla que se pidió leer.
    await cargar(con, "httpfs");
    await secretoS3(con, s3);
  }
  return `iceberg_scan('${raiz.replaceAll("'", "''").replaceAll("\\", "/")}', version='${version.replaceAll("'", "''")}', allow_moved_paths=true)`;
}

// ── DuckDB ─────────────────────────────────────────────────────────────────
let conexion = null;

/** Donde la imagen deja las extensiones de DuckDB preinstaladas. */
export const EXTENSIONES = "/opt/ore/duckdb";
const LAGO = ["json", "icu", "avro", "iceberg"];

async function duckdb() {
  if (!conexion) {
    const { DuckDBInstance } = await import("@duckdb/node-api");
    const inst = await DuckDBInstance.create(":memory:");
    conexion = await inst.connect();
    if (process.env.ORE_HILOS) await conexion.run(`set threads to ${Number(process.env.ORE_HILOS)}`);
    // Nunca salir a por una extensión: sin red, DuckDB se rinde a los 120 s
    // (medido en el clúster). Lo que la imagen trae está en /opt/ore/duckdb.
    await conexion.run("set autoinstall_known_extensions = false");
    // Un instante es un instante: el contrato (0032 §1) lo quiere en UTC, y DuckDB
    // enseña un TIMESTAMPTZ en la zona de la sesión. Aquí la sesión ES UTC.
    await conexion.run("set TimeZone = 'UTC'");
    if (existsSync(EXTENSIONES)) await conexion.run(`set extension_directory = '${EXTENSIONES}'`);
  }
  return conexion;
}

/** `LOAD` de una extensión: en la imagen está preinstalada; fuera, si falta, se instala una vez. */
async function cargar(con, extension) {
  try { await con.run(`load ${extension}`); }
  catch (e) {
    if (existsSync(EXTENSIONES)) throw new Error(`la imagen no trae la extensión \`${extension}\` de DuckDB: hay que preinstalarla en ${EXTENSIONES}`);
    await con.run(`install ${extension}`); await con.run(`load ${extension}`);
  }
}

const VISTAS_EN_SQL = /\b(?:from|join)\s+([a-z_][a-z0-9_]*)\.([a-z_][a-z0-9_]*)\b/gi;

/** Cuántas filas materializa `over()`/`sql()` si no se dice otra cosa. */
export const LIMITE = 100_000;

/** El tipo de DuckDB, con el nombre que Arrow (y `pyarrow`) le da: es lo que la
 *  consola enseña y lo mismo que dicen los agentes de Python y de Java. */
export function nombreArrow(tipo) {
  const t = String(tipo);
  const simples = {
    BOOLEAN: "bool", TINYINT: "int8", SMALLINT: "int16", INTEGER: "int32", BIGINT: "int64", HUGEINT: "int128",
    UTINYINT: "uint8", USMALLINT: "uint16", UINTEGER: "uint32", UBIGINT: "uint64", UHUGEINT: "uint128",
    FLOAT: "float", DOUBLE: "double", VARCHAR: "string", BLOB: "binary", UUID: "string",
    DATE: "date32[day]", TIME: "time64[us]", TIMESTAMP: "timestamp[us]", "TIMESTAMP WITH TIME ZONE": "timestamp[us, tz=UTC]",
    TIMESTAMP_NS: "timestamp[ns]", TIMESTAMP_MS: "timestamp[ms]", TIMESTAMP_S: "timestamp[s]", "NULL": "null",
  };
  if (simples[t]) return simples[t];
  let m = /^DECIMAL\((\d+),\s*(\d+)\)$/.exec(t);
  if (m) return `decimal128(${m[1]}, ${m[2]})`;
  if (t.endsWith("[]")) return `list<item: ${nombreArrow(t.slice(0, -2))}>`;
  if (t.startsWith("STRUCT(")) return "struct";
  if (t.startsWith("MAP(")) return "map";
  if (t.startsWith("ENUM(")) return "dictionary<values=string, indices=int32, ordered=0>";
  return t.toLowerCase();
}

/** Corre `texto` y materializa hasta `limite` filas (+1 para saber si había más). */
async function leerHasta(con, texto, limite) {
  const r = await con.runAndReadUntil(texto, limite + 1);
  const truncada = r.currentRowCount > limite;
  return { r, truncada };
}

/** El resultado en la forma pedida, con lo que hay que saber de él colgado. */
function entregar(r, truncada, limite, total, como) {
  const nombres = r.columnNames();
  const tipos = Object.fromEntries(nombres.map((n, i) => [n, nombreArrow(r.columnTypes()[i])]));
  if (como === "columnas") {
    const columnas = r.getColumns().map((c) => (truncada ? c.slice(0, limite) : c));
    return { nombres, tipos, columnas, total, truncada };
  }
  if (como !== "filas") throw new Error(`como: ${JSON.stringify(como)} no es una forma: vale "filas" o "columnas"`);
  const filas = r.getRowObjects();
  if (truncada) filas.length = limite;
  // Propiedades, no elementos: `filas.length`, `filas.map` y `for…of` ven sólo filas.
  Object.defineProperties(filas, {
    tipos: { value: tipos, enumerable: false },
    total: { value: total, enumerable: false },
    truncada: { value: truncada, enumerable: false },
  });
  return filas;
}

function opciones(o) {
  const { limite = LIMITE, estricto = false, como = "filas" } = o ?? {};
  if (!Number.isInteger(limite) || limite < 1) throw new Error("limite quiere un entero ≥ 1");
  return { limite, estricto, como };
}

/** La copia de `<paquete>.<vista>`: filas (objetos con valores tipados) hasta
 *  `limite`, con `.tipos`, `.total` y `.truncada`; o `{ como: "columnas" }`. */
export async function over(vista, o) {
  const { limite, estricto, como } = opciones(o);
  const [fuente] = await fuenteDe(vista);
  const con = await duckdb();
  const total = Number((await con.runAndReadAll(`select count(*) from ${fuente}`)).getColumns()[0][0]);
  if (estricto && total > limite) throw new Error(`over(${JSON.stringify(vista)}): la copia tiene ${total} filas y el límite es ${limite}; sube limite, agrega en sql() o quita estricto`);
  const { r, truncada } = await leerHasta(con, `select * from ${fuente}`, limite);
  return entregar(r, truncada, limite, total, como);
}

/** SQL (DuckDB) sobre las copias: cada `paquete.vista` tras FROM/JOIN se
 *  resuelve, se baja una vez y queda como vista `paquete.vista`. Devuelve lo
 *  mismo que `over()`; `total` sólo se sabe si el resultado cabe en el límite. */
export async function sql(texto, o) {
  if (typeof texto !== "string" || !texto.trim()) throw new Error("sql() quiere una consulta");
  const { limite, estricto, como } = opciones(o);
  const con = await duckdb();
  const vistas = new Set([...texto.matchAll(VISTAS_EN_SQL)].map((m) => `${m[1]}.${m[2]}`));
  for (const v of [...vistas].sort()) {
    const [esquema, nombre] = v.split(".");
    const [fuente] = await fuenteDe(v);
    await con.run(`create schema if not exists "${esquema}"`);
    await con.run(`create or replace view "${esquema}"."${nombre}" as select * from ${fuente}`);
  }
  const { r, truncada } = await leerHasta(con, texto, limite);
  if (estricto && truncada) throw new Error(`sql(): el resultado pasa de ${limite} filas; sube limite, agrega más o quita estricto`);
  return entregar(r, truncada, limite, truncada ? undefined : r.currentRowCount, como);
}

// ── Escribir (0031 §11) ────────────────────────────────────────────────────
const DELEGAR = { "x-iceberg-access-delegation": "vended-credentials" };
let s3 = null;

async function secretoS3(con, cfg) {
  let ep = cfg["s3.endpoint"] ?? "";
  const ssl = ep.startsWith("https://") ? "true" : "false";
  ep = ep.replace(/^https?:\/\//, "").replace(/\/+$/, "");
  const q = (v) => String(v ?? "").replaceAll("'", "''");
  await con.run(`create or replace secret ore_s3 (type s3, key_id '${q(cfg["s3.access-key-id"])}', secret '${q(cfg["s3.secret-access-key"])}', endpoint '${q(ep)}', url_style 'path', use_ssl ${ssl}, region '${q(cfg["s3.region"] ?? "auto")}')`);
}

/** El tipo de DuckDB de un tipo de Arrow (la vuelta de `nombreArrow`). */
function tipoDuck(t) {
  const simples = {
    bool: "BOOLEAN", int8: "TINYINT", int16: "SMALLINT", int32: "INTEGER", int64: "BIGINT", uint8: "UTINYINT", uint16: "USMALLINT", uint32: "UINTEGER",
    float: "FLOAT", double: "DOUBLE", string: "VARCHAR", "date32[day]": "DATE", "time64[us]": "TIME", "timestamp[us]": "TIMESTAMP",
    "timestamp[us, tz=UTC]": "TIMESTAMPTZ", "timestamp[ms, tz=UTC]": "TIMESTAMPTZ", "timestamp[ms]": "TIMESTAMP", "timestamp[ns]": "TIMESTAMP", "timestamp[s]": "TIMESTAMP",
  };
  if (simples[t]) return simples[t];
  const m = /^decimal128\((\d+), (\d+)\)$/.exec(t);
  if (m) return `DECIMAL(${m[1]},${m[2]})`;
  return null;
}

/** El tipo de Iceberg con el que la tabla se esboza (lo mismo que `ore-store`
 *  hace al escribir, 0032): lo que el contrato no tiene se niega aquí, con el
 *  nombre de la columna, antes de mandar nada. */
function tipoIceberg(columna, t) {
  const simples = {
    bool: "boolean", int8: "long", int16: "long", int32: "long", int64: "long", uint8: "long", uint16: "long", uint32: "long",
    float: "double", double: "double", string: "string", "date32[day]": "date", "time64[us]": "time",
    "timestamp[us]": "timestamp", "timestamp[ms]": "timestamp", "timestamp[ns]": "timestamp", "timestamp[s]": "timestamp",
    "timestamp[us, tz=UTC]": "timestamptz", "timestamp[ms, tz=UTC]": "timestamptz",
  };
  if (simples[t]) return simples[t];
  const m = /^decimal128\((\d+), (\d+)\)$/.exec(t);
  if (m) return `decimal(${m[1]}, ${m[2]})`;
  if (t === "uint64") throw new Error(`write(): la columna \`${columna}\` es uint64, que no cabe en int64 sin mentir (0032); conviértela antes`);
  if (t === "null") throw new Error(`write(): la columna \`${columna}\` no tiene tipo (todo nulo): dale uno antes (0032)`);
  throw new Error(`write(): la columna \`${columna}\` es \`${t}\`, que el contrato de tipos (0032) no tiene`);
}

/** Lo que se escribe, como `{ nombres, tipos, columnas }` con tipos de Arrow. */
function columnasDe(datos) {
  if (datos && typeof datos === "object" && Array.isArray(datos.nombres) && Array.isArray(datos.columnas)) {
    return { nombres: datos.nombres, tipos: datos.tipos ?? {}, columnas: datos.columnas };
  }
  if (!Array.isArray(datos)) throw new TypeError("write() quiere filas (objetos) o { nombres, tipos, columnas }");
  if (datos.length === 0) throw new Error("write(): la tabla no tiene filas");
  const nombres = [...new Set(datos.flatMap((f) => Object.keys(f ?? {})))];
  const tipos = { ...(datos.tipos ?? {}) };
  for (const n of nombres) {
    if (!tipos[n]) {
      const v = datos.find((f) => f?.[n] !== undefined && f?.[n] !== null)?.[n];
      tipos[n] = tipoInferido(v);
    }
  }
  return { nombres, tipos, columnas: nombres.map((n) => datos.map((f) => f?.[n] ?? null)) };
}

/** La tabla, en DuckDB y de ahí a Parquet (bytes): tipada columna a columna. */
async function parquetDe_(nombres, tipos, columnas) {
  const { timestampTZValueFromDate, timestampValueFromDate, dateValueFromDate } = await import("@duckdb/node-api");
  const con = await duckdb();
  const t = `escritura_${Date.now()}_${Math.floor(Math.random() * 1e6)}`;
  const decl = nombres.map((n) => {
    const d = tipoDuck(tipos[n]);
    if (!d) throw new Error(`write(): la columna \`${n}\` es \`${tipos[n]}\`, que el contrato de tipos (0032) no tiene`);
    return `"${n.replaceAll('"', '""')}" ${d}`;
  });
  await con.run(`create temp table "${t}" (${decl.join(", ")})`);
  const ap = await con.createAppender(t);
  const n = columnas[0]?.length ?? 0;
  for (let i = 0; i < n; i++) {
    for (let c = 0; c < nombres.length; c++) {
      let v = columnas[c][i];
      const tipo = tipos[nombres[c]];
      if (v === undefined || v === null || (typeof v === "number" && Number.isNaN(v) && tipo !== "double")) { ap.appendNull(); continue; }
      if (v?.constructor?.name === "Date") {
        const d = new Date(v.getTime());
        v = tipo === "date32[day]" ? dateValueFromDate(d) : tipo.includes("tz") ? timestampTZValueFromDate(d) : timestampValueFromDate(d);
      } else if (typeof v === "number" && (tipo === "int64" || tipo === "int32")) {
        v = BigInt(Math.trunc(v));
      } else if (typeof v === "string" && tipo === "date32[day]") {
        v = dateValueFromDate(new Date(v + "T00:00:00Z"));
      }
      ap.appendValue(v);
    }
    ap.endRow();
  }
  ap.closeSync();
  const f = join(tmpdir(), `${t}.parquet`);
  await con.run(`copy "${t}" to '${f.replaceAll("'", "''").replaceAll("\\", "/")}' (format parquet)`);
  await con.run(`drop table "${t}"`);
  const bytes = readFileSync(f);
  try { unlinkSync(f); } catch {}
  return bytes;
}

/** El escritor y su entorno: `ore-store-gcs` con el token prestado si la tabla
 *  vive en `gs://`, `ore-store-r2` con las claves prestadas si en `s3://`. */
function escritor(config, ubicacion) {
  const env = { ...process.env };
  let nombre;
  if (ubicacion.startsWith("gs://")) {
    nombre = "ore-store-gcs";
    env.ORE_GCS_BUCKET = ubicacion.slice(5).split("/")[0];
    env.ORE_GCS_TOKEN = config["gcs.oauth2.token"] ?? "";
    if (!env.ORE_GCS_TOKEN) throw new Error(`write(): el catálogo no prestó credencial para \`${ubicacion}\``);
  } else if (ubicacion.startsWith("s3://")) {
    nombre = "ore-store-r2";
    env.ORE_R2_BUCKET = ubicacion.slice(5).split("/")[0];
    env.ORE_R2_S3_ENDPOINT = config["s3.endpoint"] ?? "";
    env.ORE_R2_ACCESS_KEY_ID = config["s3.access-key-id"] ?? "";
    env.ORE_R2_SECRET_ACCESS_KEY = config["s3.secret-access-key"] ?? "";
    env.ORE_R2_REGION = config["s3.region"] ?? "auto";
  } else {
    throw new Error(`write(): la tabla vive en \`${ubicacion}\`, que no es un lago que este SDK sepa escribir`);
  }
  const dirs = [process.env.ORE_STORE_DIR, ...(process.env.PATH ?? "").split(delimiter)].filter(Boolean);
  for (const d of dirs) {
    for (const ext of ["", ".exe"]) {
      const b = join(d, nombre + ext);
      if (existsSync(b)) return { binario: b, env };
    }
  }
  throw new Error(`write(): no está \`${nombre}\` en el PATH (la imagen del puesto lo lleva; fuera, ORE_STORE_DIR)`);
}

function mensajeDe(r) {
  const e = r?.error;
  return typeof e === "object" && e ? (e.message ?? JSON.stringify(e)) : String(e ?? JSON.stringify(r));
}

/** Escribe `datos` como el dataset `<paquete>.<tabla>` del lago (ver arriba).
 *  Devuelve `{ tabla, filas, snapshot, metadata_location, operacion, repetida }`. */
export async function write(nombre, datos, o) {
  const { modo = "sobrescribir" } = o ?? {};
  if (typeof nombre !== "string" || nombre.split(".").length !== 2) throw new Error(`write() quiere \`<paquete>.<tabla>\`, no ${JSON.stringify(nombre)}`);
  if (modo !== "sobrescribir" && modo !== "anexar") throw new Error(`modo: ${JSON.stringify(modo)}: vale "sobrescribir" o "anexar"`);
  const [ns, t] = nombre.split(".");
  const { nombres, tipos, columnas } = columnasDe(datos);
  if (nombres.length === 0 || (columnas[0]?.length ?? 0) === 0) throw new Error("write(): la tabla no tiene filas");
  const esquema = { type: "struct", "schema-id": 0, fields: nombres.map((n, i) => ({ id: i + 1, name: n, type: tipoIceberg(n, tipos[n]), required: false })) };
  const parquet = await parquetDe_(nombres, tipos, columnas);
  const dataset = `datasets/${ns}_${t}`;
  const semilla = `${nombre}|${modo}`;
  const cargar = async () => {
    const [c, r] = await puesto.pedir("GET", `/v1/namespaces/${ns}/tables/${t}`, undefined, 30_000, DELEGAR);
    if (c === 200) return { base: r["metadata-location"], esbozo: null, config: r.config ?? {}, ubicacion: r.metadata.location };
    if (c === 404) {
      const [c2, r2] = await puesto.pedir("POST", `/v1/namespaces/${ns}/tables`, { name: t, "stage-create": true, schema: esquema, properties: {} }, 30_000, DELEGAR);
      if (c2 !== 200) throw new Error(`write(${nombre}): ${mensajeDe(r2)}`);
      return { base: null, esbozo: r2.metadata, config: r2.config ?? {}, ubicacion: r2.metadata.location };
    }
    throw new Error(`write(${nombre}): ore-serve contestó ${c}: ${mensajeDe(r)}`);
  };
  let clave = "";
  let escrito = null;
  for (let intento = 0; intento < 4; intento++) {
    const { base, esbozo, config, ubicacion } = await cargar();
    if (config["s3.access-key-id"]) s3 = config;
    const { binario, env } = escritor(config, ubicacion);
    const peticion = { dataset, modo, formato: "parquet", operacion: "contenido", semilla };
    if (base) peticion.base = base; else peticion.esbozo = esbozo;
    const p = spawnSync(binario, ["escribir"], { input: Buffer.concat([Buffer.from(JSON.stringify(peticion) + "\n"), parquet]), env, maxBuffer: 1 << 26 });
    if (p.status !== 0) throw new Error(`write(): ${(p.stderr?.toString("utf8") ?? "").trim().replace(/^error: /, "") || "el escritor falló"}`);
    escrito = JSON.parse(p.stdout.toString("utf8"));
    clave = escrito.operacion || clave;
    const [c, r] = await puesto.pedir("POST", `/v1/namespaces/${ns}/tables/${t}`, { identifier: { namespace: [ns], name: t }, requirements: escrito.requirements, updates: escrito.updates }, 120_000);
    if (c === 200) {
      const snap = r?.metadata?.["current-snapshot-id"];
      // repetida: el catálogo contestó con lo que ya había (el mismo puntero)
      // — los ids de snapshot no se comparan: en JS un int64 pierde precisión
      return { tabla: nombre, filas: escrito.filas, snapshot: String(snap ?? ""), metadata_location: r?.["metadata-location"] ?? "", operacion: clave, repetida: base !== null && r?.["metadata-location"] === base };
    }
    if (c === 409) continue; // alguien escribió mientras tanto: otra vez sobre lo que hay
    if (c >= 500) {
      // el commit pudo entrar: se MIRA antes de darlo por perdido
      const [c2, r2] = await puesto.pedir("GET", `/v1/namespaces/${ns}/tables/${t}`);
      if (c2 === 200) {
        const md = r2.metadata;
        const vigente = (md.snapshots ?? []).find((x) => x["snapshot-id"] === md["current-snapshot-id"]);
        if (vigente?.summary?.["ore.operacion"] === clave) {
          return { tabla: nombre, filas: escrito.filas, snapshot: String(md["current-snapshot-id"]), metadata_location: r2["metadata-location"], operacion: clave, repetida: false };
        }
      }
      throw new Error(`write(${nombre}): el catálogo contestó ${c} y el commit no está: ${mensajeDe(r)}`);
    }
    throw new Error(`write(${nombre}): ${mensajeDe(r)}`);
  }
  throw new Error(`write(${nombre}): cuatro veces alguien escribió antes; vuelve a intentarlo`);
}

// ── El JSON de la consola (0032 §1) ───────────────────────────────────────
/** Lo que devuelven `over()` y `sql()` —filas (objetos con valores tipados) o
 *  `{ nombres, tipos, columnas }`— → la salida `tabla` del contrato (0032 §1,
 *  columna «JSON de la consola»): el MISMO JSON que emiten los agentes de
 *  Python y de Java. Un array de objetos cualquiera también vale (sin tipos
 *  declarados se infieren del primer valor). */
export function tabla(valor, limite = 200) {
  if (valor && typeof valor === "object" && Array.isArray(valor.nombres) && Array.isArray(valor.columnas)) {
    const n = valor.columnas[0]?.length ?? 0;
    const filas = [];
    for (let i = 0; i < Math.min(n, limite); i++) filas.push(valor.columnas.map((c) => jsonDe(c[i])));
    return {
      columnas: valor.nombres.map((c) => ({ name: c, type: valor.tipos?.[c] ?? "null" })),
      filas,
      total: valor.total ?? n,
      limite,
    };
  }
  if (!Array.isArray(valor) || valor.length === 0) return null;
  if (!valor.every((f) => f && typeof f === "object" && !Array.isArray(f))) return null;
  const cabeza = valor.slice(0, limite);
  const columnas = valor.tipos ? Object.keys(valor.tipos) : [...new Set(cabeza.flatMap((f) => Object.keys(f)))];
  const tipoDe = (c) => valor.tipos?.[c] ?? tipoInferido(valor.find((f) => f[c] !== null && f[c] !== undefined)?.[c]);
  return {
    columnas: columnas.map((c) => ({ name: c, type: tipoDe(c) })),
    filas: cabeza.map((f) => columnas.map((c) => jsonDe(f[c]))),
    total: valor.total ?? valor.length,
    limite,
  };
}

/** El tipo de Arrow de un valor suelto, para lo que no viene de `over()`/`sql()`. */
function tipoInferido(v) {
  if (v === undefined || v === null) return "null";
  if (typeof v === "bigint") return "int64";
  if (typeof v === "number") return Number.isInteger(v) ? "int64" : "double";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "string") return "string";
  // Por el nombre del constructor, no por `instanceof`: una celda corre en su
  // propio contexto de `vm`, con su propio `Date`.
  const clase = v?.constructor?.name ?? "";
  if (clase === "Date") return "timestamp[ms, tz=UTC]";
  if (clase === "DuckDBDecimalValue") return `decimal128(${v.width}, ${v.scale})`;
  if (clase === "DuckDBDateValue") return "date32[day]";
  if (clase === "DuckDBTimeValue") return "time64[us]";
  if (clase === "DuckDBTimestampValue") return "timestamp[us]";
  if (clase === "DuckDBTimestampTZValue") return "timestamp[us, tz=UTC]";
  if (clase === "DuckDBBlobValue") return "binary";
  if (clase === "DuckDBListValue") return "list";
  if (clase === "DuckDBStructValue") return "struct";
  if (clase === "DuckDBMapValue") return "map";
  return Array.isArray(v) ? "list" : "struct";
}

const ENTERO_EXACTO = 2n ** 53n;
const US_POR_DIA = 86_400_000_000n;

function dos(n) { return String(n).padStart(2, "0"); }

/** Fracción de segundo en microsegundos → `.ffffff` sin ceros de más, o nada. */
function fraccion(us) {
  if (us === 0n) return "";
  return "." + String(us).padStart(6, "0").replace(/0+$/, "");
}

/** Días desde 1970-01-01 → `YYYY-MM-DD` (calendario proléptico, como Arrow). */
function fechaDeDias(dias) {
  const d = new Date(Number(dias) * 86_400_000);
  const y = d.getUTCFullYear();
  return `${y < 0 ? "-" : ""}${String(Math.abs(y)).padStart(4, "0")}-${dos(d.getUTCMonth() + 1)}-${dos(d.getUTCDate())}`;
}

/** Microsegundos desde la medianoche → `HH:MM:SS[.ffffff]`. */
function horaDeMicros(us) {
  const s = us / 1_000_000n;
  return `${dos(s / 3600n)}:${dos((s / 60n) % 60n)}:${dos(s % 60n)}${fraccion(us % 1_000_000n)}`;
}

/** Microsegundos desde la época → ISO 8601 con `T`; `z` añade la `Z` de un instante en UTC. */
function isoDeMicros(us, z) {
  const dias = us >= 0n ? us / US_POR_DIA : -((-us + US_POR_DIA - 1n) / US_POR_DIA);
  const resto = us - dias * US_POR_DIA;
  return `${fechaDeDias(dias)}T${horaDeMicros(resto)}${z ? "Z" : ""}`;
}

/** Un valor (tipado de DuckDB, o suelto) → el JSON del contrato (0032 §1):
 *  entero → número si |x| ≤ 2⁵³, si no cadena · decimal → cadena siempre
 *  (salvo el de escala 0, que es un entero y va como tal) ·
 *  float → número, y `NaN`/`Infinity`/`-Infinity` como cadena · fecha
 *  `YYYY-MM-DD` · hora `HH:MM:SS[.ffffff]` · fecha-hora sin zona en ISO con `T`
 *  · instante en UTC con `Z` · bytes en base64 · lista → array · struct →
 *  objeto · map → `[{key, value}]`. Nada se degrada en silencio. */
export function jsonDe(v) {
  if (v === undefined || v === null) return null;
  if (typeof v === "bigint") return v >= -ENTERO_EXACTO && v <= ENTERO_EXACTO ? Number(v) : v.toString();
  if (typeof v === "number") return Number.isNaN(v) ? "NaN" : Number.isFinite(v) ? v : v > 0 ? "Infinity" : "-Infinity";
  if (typeof v === "string" || typeof v === "boolean") return v;
  if (v instanceof Date) return v.toISOString();
  if (v instanceof Uint8Array) return Buffer.from(v).toString("base64");
  if (Array.isArray(v)) return v.map(jsonDe);
  const clase = v?.constructor?.name ?? "";
  // Un decimal de escala 0 (un HUGEINT: `sum(1)`, `count`) es un entero y va como los enteros; con decimales, cadena siempre.
  if (clase === "DuckDBDecimalValue") return v.scale === 0 ? jsonDe(BigInt(v.value)) : v.toString();
  if (clase === "DuckDBDateValue") return fechaDeDias(BigInt(v.days));
  if (clase === "DuckDBTimeValue") return horaDeMicros(BigInt(v.micros));
  if (clase === "DuckDBTimestampValue") return isoDeMicros(BigInt(v.micros), false);
  if (clase === "DuckDBTimestampTZValue") return isoDeMicros(BigInt(v.micros), true);
  if (clase === "DuckDBTimestampNanosecondsValue") { const ns = BigInt(v.nanos); const s = ns >= 0n ? ns / 1_000_000_000n : -((-ns + 999_999_999n) / 1_000_000_000n); const f = String(ns - s * 1_000_000_000n).padStart(9, "0").replace(/0+$/, ""); return isoDeMicros(s * 1_000_000n, false) + (f ? "." + f : ""); }
  if (clase === "DuckDBBlobValue") return Buffer.from(v.bytes).toString("base64");
  if (clase === "DuckDBListValue" || clase === "DuckDBArrayValue") return v.items.map(jsonDe);
  if (clase === "DuckDBStructValue") return Object.fromEntries(Object.entries(v.entries).map(([k, x]) => [k, jsonDe(x)]));
  if (clase === "DuckDBMapValue") return v.entries.map((e) => ({ key: jsonDe(e.key), value: jsonDe(e.value) }));
  if (clase === "DuckDBUUIDValue" || clase === "DuckDBIntervalValue" || clase === "DuckDBTimeTZValue" || clase === "DuckDBBitValue") return v.toString();
  if (typeof v === "object") {
    try { return JSON.parse(JSON.stringify(v, (k, x) => (typeof x === "bigint" ? jsonDe(x) : x))); } catch { return String(v); }
  }
  return String(v);
}

/** Un valor suelto (el resultado de una celda que no es tabla) → JSON. */

export default { over, sql, write, persona, puesto, nombreArrow, LIMITE, tabla, jsonDe };
