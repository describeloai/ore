// `ore` · el SDK del puesto para TS/JS (0031 W3.4). El mismo contrato que
// `puesto/python/ore`:
//
//   over("<base>.<view>")       → the rows of that view's copy (objects)
//   sql("select … from p.v")    → the rows of the result (DuckDB in the session)
//   person()                    → who opened the session (`persona:…`)
//
// La API pública está en inglés (S2); los nombres de antes en español siguen
// como alias del MISMO objeto (`persona`, `tabla`, `jsonDe`, `mediaDe`, …), y
// las opciones y las claves de lo devuelto en español también se leen.
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
// devuelven hasta `limit` filas (100 000 por defecto) y lo dicen:
// `rows.total` (las que hay, si se sabe), `rows.truncated`, `rows.types`
// (columna → tipo de Arrow). Con `{ strict: true }` una respuesta que no cabe
// en el límite falla en vez de recortarse; lo masivo se agrega en SQL
// (60–140 ms para 10 M de filas) o se hace en Python. `{ as: "columns" }`
// da `{ names, types, columns }` —arrays por columna, sin objeto por fila—
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
// Iceberg en el lago, el `Dataset` escrito en el árbol, el puntero— desde lo que `over()` o
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

// ── Los alias en español (S2) ──────────────────────────────────────────────
// Un solo interruptor: con `false` un alias ES la función inglesa (el mismo
// objeto); con `true`, una envoltura que avisa una vez por nombre.
const AVISAR_ALIAS = false;
const avisados = new Set();
function alias(viejo, nuevo, f) {
  if (!AVISAR_ALIAS) return f;
  return function (...a) {
    if (!avisados.has(viejo)) { avisados.add(viejo); process.emitWarning(`ore.${viejo}() is deprecated: use ore.${nuevo}()`, "DeprecationWarning"); }
    return f.apply(this, a);
  };
}

/** Las claves en español de lo que el SDK devuelve → las inglesas. */
const ES_EN = {
  tabla: "table", filas: "rows", operacion: "operation", repetida: "repeated",
  nombre: "name", fichero: "file", nueva: "new",
  nombres: "names", tipos: "types", columnas: "columns", truncada: "truncated",
};

/** Las claves en español, legibles pero no enumerables: JSON y consola ven sólo las inglesas. */
function conAlias(obj) {
  for (const [es, en] of Object.entries(ES_EN)) {
    if (en in obj && !Object.prototype.hasOwnProperty.call(obj, es)) {
      Object.defineProperty(obj, es, { get() { return this[en]; }, enumerable: false, configurable: true });
    }
  }
  return obj;
}

/** Las opciones en español → las inglesas (si vienen las dos, gana la inglesa). */
const OPCIONES_ES_EN = { limite: "limit", estricto: "strict", como: "as", modo: "mode", clave: "key" };
function opcionesEn(o) {
  const dadas = o ?? {};
  const out = {};
  for (const [k, v] of Object.entries(dadas)) {
    const en = OPCIONES_ES_EN[k];
    if (!en) out[k] = v;
    else if (!(en in dadas)) out[en] = v;
  }
  return out;
}

/** The session: what the agent knows about itself. `puesto` is the old name (same object). */
export const session = {
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
// Los campos con nombre inglés: el mismo dato (no enumerables).
Object.defineProperties(session, {
  server: { get() { return this.servidor; }, enumerable: false },
  store: { get() { return this.almacen; }, enumerable: false },
  person: { get() { return this.persona; }, set(v) { this.persona = v; }, enumerable: false },
  request: { get() { return this.pedir; }, enumerable: false },
});
/** @deprecated use {@link session} (the same object). */
export const puesto = session;

/** `[kind, namespace, name]` de un documento YAML, sin analizador. */
function cabeza(texto) {
  const k = /^kind:\s*([A-Za-z]+)\s*$/m.exec(texto);
  if (!k) throw new Error("declare(): the document has no `kind:`");
  const m = /^metadata:[ \t]*(.*)$/m.exec(texto);
  if (!m) throw new Error("declare(): the document has no `metadata:`");
  const campos = {};
  const resto = m[1].trim();
  const par = (s) => { const i = s.indexOf(":"); if (i > 0) campos[s.slice(0, i).trim()] = s.slice(i + 1).trim().replace(/^["']|["']$/g, ""); };
  if (resto.startsWith("{")) resto.replace(/^\{|\}$/g, "").split(",").forEach(par);
  else for (const l of texto.slice(m.index + m[0].length).split("\n").slice(1)) { if (!/^[ \t]/.test(l)) break; par(l); }
  if (!campos.name) throw new Error("declare(): `metadata.name` is missing");
  return [k[1], campos.namespace ?? "", campos.name, campos.schema || "default"];
}

/**
 * Declares an ontology document from the cell (0031 §9, W3.7 ①): the YAML
 * (string) or an object `{kind, metadata, spec}` → `PUT /documentos/{kind}/{ns}/{n}`
 * (Forge's gate: it compiles before pushing). Signed by whoever opened the
 * session, on their branch. Returns `{kind, name, file, commit, new}` (the old
 * keys `nombre`, `fichero`, `nueva` still read); a 422 throws with the diagnostics.
 * @param {string | object} document the YAML text or `{kind, metadata, spec}`
 */
export async function declare(document) {
  const documento = document;
  let kind, ns, nombre, schema, cuerpo;
  if (typeof documento === "string") { [kind, ns, nombre, schema] = cabeza(documento); cuerpo = { yaml: documento }; }
  else if (documento && typeof documento === "object") {
    kind = documento.kind; ns = documento.metadata?.namespace ?? ""; nombre = documento.metadata?.name ?? ""; cuerpo = documento;
    schema = documento.metadata?.schema || "default";
    if (!kind || !nombre) throw new Error("declare(): the document needs `kind` and `metadata.name`");
  } else throw new Error(`declare() takes the document's YAML or an object, not ${typeof documento}`);
  if (!ns) throw new Error("declare(): `metadata.namespace` is missing: a document lives in a package");
  // 0038: en su schema, `/documentos/{kind}/{base}/{schema}/{n}`; la de dos tramos es `default`.
  const ruta = schema === "default" ? `/documentos/${kind}/${ns}/${nombre}` : `/documentos/${kind}/${ns}/${schema}/${nombre}`;
  const [c, r] = await puesto.pedir("PUT", ruta, cuerpo, 120_000);
  const s = r?.schema ?? schema;
  if (c === 200 || c === 201) return conAlias({ kind: r?.kind ?? kind, name: s === "default" ? `${r?.namespace ?? ns}.${r?.name ?? nombre}` : `${r?.namespace ?? ns}.${s}.${r?.name ?? nombre}`, file: r?.fichero ?? "", commit: r?.commit ?? "", new: Boolean(r?.nueva ?? c === 201) });
  if (r?.diagnosticos?.length) throw new Error(`declare(${ns}.${nombre}): ${r.diagnosticos.map((d) => `${d.codigo ?? "?"}: ${d.mensaje ?? ""}`).join("; ")}`);
  throw new Error(`declare(${ns}.${nombre}): ${r?.error ?? "?"} (${c})`);
}

/** Who opened the session (`persona:…`). The agent knows it once it claims the session. */
export function person() {
  if (!puesto.persona) throw new Error("person(): the agent does not know yet who opened the session");
  return puesto.persona;
}
/** @deprecated use {@link person}. */
export const persona = alias("persona", "person", person);

// ── el almacén ─────────────────────────────────────────────────────────────
let tokenDelPod = { valor: "", caduca: 0 };

async function tokenDeGoogle() {
  if (Date.now() < tokenDelPod.caduca - 60_000) return tokenDelPod.valor;
  const r = await fetch("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token", {
    headers: { "Metadata-Flavor": "Google" },
    signal: AbortSignal.timeout(10_000),
  });
  if (!r.ok) throw new Error(`the metadata server answered ${r.status}`);
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
    if (!r.ok) throw new Error(`GCS answered ${r.status} for ${clave}: ${(await r.text()).slice(0, 200)}`);
    return Buffer.from(await r.arrayBuffer());
  }
  throw new Error(`ORE_ALMACEN=${JSON.stringify(puesto.almacen)} is not a store: it is \`gcs\` or \`dir:<path>\``);
}

function desenvolver(crudo) {
  if (crudo.subarray(0, 8).toString("latin1") !== MAGIA) throw new Error("the artifact is not an ORE copy (no `ORECOPY1`)");
  const n = crudo.readUInt32LE(8);
  return { cabecera: JSON.parse(crudo.subarray(12, 12 + n).toString("utf8")), carga: crudo.subarray(12 + n) };
}

// Lo que la sesión leyó (por nombre), y el transform activo si lo hay.
const leidas = [];
let transformActivo = null;

const DEFAULT = "default";

/** `base.nombre` o `base.schema.nombre` (0038) → la forma corta, la clave del
 *  árbol: `base.nombre` en `default`, `base.schema.nombre` en otro schema. */
function corto(nombre, que = "a tree name") {
  const p = typeof nombre === "string" ? nombre.split(".") : [];
  if (![2, 3].includes(p.length) || !p.every((x) => x)) throw new Error(`${que} is \`<base>.<schema>.<name>\` (or \`<base>.<name>\`, in \`default\`), not ${JSON.stringify(nombre)}`);
  return p.length === 3 && p[1] === DEFAULT ? `${p[0]}.${p[2]}` : p.join(".");
}

/** La forma corta → [base, schema, nombre]. */
function partes(c) {
  const p = c.split(".");
  return p.length === 2 ? [p[0], DEFAULT, p[1]] : p;
}

/** La ruta de `/v1` de una tabla, como Unity (0038 P4): la base es el `prefix`. */
function v1Tabla(c) {
  const [b, s, n] = partes(c);
  return `/v1/${b}/namespaces/${s}/tables/${n}`;
}

const q = (x) => `"${x.replaceAll('"', '""')}"`;

/** El nombre del árbol como vista de DuckDB con sus tres niveles (0038): un
 *  catálogo por base y un schema por schema; lo de `default`, con su alias en
 *  `main` (donde DuckDB busca un nombre de DOS partes). */
async function registra(con, c, fuente) {
  const [b, s, n] = partes(c);
  await con.run(`attach if not exists ':memory:' as ${q(b)}`);
  await con.run(`create schema if not exists ${q(b)}.${q(s)}`);
  await con.run(`create or replace view ${q(b)}.${q(s)}.${q(n)} as select * from ${fuente}`);
  if (s === DEFAULT) await con.run(`create or replace view ${q(b)}.main.${q(n)} as select * from ${q(b)}.${q(s)}.${q(n)}`);
}

/**
 * `transform({ inputs, output }, fn)` (0031 §9, W3.7 ③): returns a function that
 * runs `fn` with what was declared as the only thing it may read (`over`, `sql`)
 * and write (`write`); anything else throws. What it writes carries
 * `procedencia: {inputs, transform, …}`.
 */
export function transform({ inputs, output }, fn) {
  if (!Array.isArray(inputs)) throw new Error("transform(): `inputs` is a list of `<base>.<schema>.<name>`");
  inputs = inputs.map((i) => corto(i, "transform(): each input"));
  output = corto(output, "transform(): `output`");
  if (inputs.includes(output)) throw new Error(`transform(): \`${output}\` cannot be both input and output`);
  if (typeof fn !== "function") throw new Error("transform(): takes a function");
  const nombre = fn.name || "transform";
  const corre = async (...a) => {
    if (transformActivo) throw new Error(`transform(): \`${transformActivo.nombre}\` is already running; a transform does not call another`);
    transformActivo = { nombre, inputs: [...inputs], output };
    await decirTransform(transformActivo);
    try { return await fn(...a); } finally { transformActivo = null; await decirTransform(null); }
  };
  corre.inputs = [...inputs]; corre.output = output;
  return corre;
}

function lee(vista) {
  vista = corto(vista);
  if (transformActivo && !transformActivo.inputs.includes(vista)) throw new Error(`\`${vista}\` is not in the inputs of \`${transformActivo.nombre}\` (${transformActivo.inputs.join(", ")}): a transform only reads what it declares`);
  if (!leidas.includes(vista)) leidas.push(vista);
}

/** Lo declarado, dicho al servidor (W3.7 gobierno ⑤): mientras corre, resuelve
 *  sólo `inputs` y deja escribir sólo `output`. Un ore-serve viejo no contesta
 *  y el SDK sigue acotando por su cuenta. */
async function decirTransform(t) {
  try {
    if (t) await puesto.pedir("POST", `/puestos/${puesto.id}/transform`, { nombre: t.nombre, inputs: t.inputs, output: t.output });
    else await puesto.pedir("DELETE", `/puestos/${puesto.id}/transform`);
  } catch { /* el servidor no lo sabe: el SDK sigue acotando */ }
}

function procedencia(nombre) {
  const p = { puesto: puesto.id };
  if (transformActivo) { p.inputs = [...transformActivo.inputs].sort(); p.transform = transformActivo.nombre; }
  // Fuera de un transform, lo que la sesión leyó SIN lo que se está escribiendo
  // (W3.7 gobierno ③: un dataset no sale de sí mismo).
  else p.leidas = leidas.filter((l) => l !== nombre).sort();
  if (process.env.ORE_CODIGO) p.codigo = process.env.ORE_CODIGO;
  return p;
}

async function resolver(vista) {
  vista = corto(vista);
  lee(vista);
  const [codigo, r] = await puesto.pedir("GET", `/puestos/${puesto.id}/datos/${vista}`);
  return oElError(codigo, r, vista);
}

/** Lo que ore-serve contestó por un nombre, o el error de siempre: el mismo para
 *  `over()` (GET datos) que para `sql()` (POST sql). */
function oElError(codigo, r, vista) {
  if (codigo === 409) throw new Error(`the copy of \`${vista}\` is not ready: ${r?.error ?? ""}`);
  if (codigo === 404) throw new Error(`there is no \`View\` or \`Dataset\` \`${vista}\` in the tree`);
  // El conducto de la lectura (0031 W3.7 gobierno ②): lo que el dataset lleva
  // no cabe por `contextSurface.workspace`. Se dice tal cual.
  if (codigo === 403) throw new Error(r?.error ?? `ore-serve does not allow reading \`${vista}\` from a session`);
  if (codigo !== 200) throw new Error(`ore-serve answered ${codigo} for \`${vista}\`: ${r?.error ?? JSON.stringify(r)}`);
  return r;
}

// ── 0046 E9·4 · Los ficheros de una colección ─────────────────────────────
// Una propiedad `Media<c>` de una Entity guarda la HUELLA de un ítem de `c`.
// Servirlo es pedir a ore-serve una URL firmada que vive unos minutos y se abre
// sin credencial. Lo mismo que el SDK de Python: `mediaUrl`, `mediaUrls`,
// `mediaColumns` (antes `media`, `medias`, `mediaDe`, que siguen como alias).
// Lo que devuelven es lo que ore-serve contesta por cada ítem, tal cual.

function rutaDeColeccion(coleccion) {
  const [b, s, n] = partes(corto(coleccion, "mediaUrl(): the collection"));
  return `/colecciones/${b}/${s}/${n}/items`;
}

function servido(codigo, r, que) {
  if (codigo === 200) return r;
  const e = new Error(`${que}: ${r?.error ?? codigo}`);
  e.codigo = codigo;
  throw e;
}

/** Which collection each `Media<c>` column of `view` holds fingerprints of: `{column: collection}`. */
export async function mediaColumns(view) {
  return { ...((await resolver(view))?.media ?? {}) };
}
/** @deprecated use {@link mediaColumns}. */
export const mediaDe = alias("mediaDe", "mediaColumns", mediaColumns);

/** The signed URLs of several fingerprints, a hundred at a time: `{fingerprint: {url, tipo, …}}`
 *  (each item as ore-serve serves it). `o.ttl`: seconds the URLs live. */
export async function mediaUrls(collection, fingerprints, o = {}) {
  const coleccion = collection, huellas = fingerprints;
  const ruta = rutaDeColeccion(coleccion) + "/resolver";
  const unicas = [...new Set(huellas.filter(Boolean))];
  const out = {};
  for (let i = 0; i < unicas.length; i += 100) {
    const cuerpo = { huellas: unicas.slice(i, i + 100) };
    if (o.ttl !== undefined) cuerpo.ttl = String(Math.trunc(o.ttl));
    const [codigo, r] = await puesto.pedir("POST", ruta, cuerpo);
    const d = servido(codigo, r, `mediaUrls(${coleccion})`);
    for (const it of d.items ?? []) out[it.huella] = { segundos: d.segundos, caduca_ms: d.caduca_ms, ...it };
  }
  return out;
}
/** @deprecated use {@link mediaUrls}. */
export const medias = alias("medias", "mediaUrls", mediaUrls);

/** The item of `collection` with this `fingerprint`, served: `{url, tipo, disposicion, …}`
 *  (as ore-serve serves it). The `url` opens without a credential for `o.ttl` s (300; 30 to 3600). */
export async function mediaUrl(collection, fingerprint, o = {}) {
  const coleccion = collection, huella = fingerprint;
  if (o.ttl === undefined) {
    const [codigo, r] = await puesto.pedir("GET", `${rutaDeColeccion(coleccion)}/${encodeURIComponent(huella)}`);
    return servido(codigo, r, `mediaUrl(${coleccion}, ${huella})`);
  }
  const it = (await mediaUrls(coleccion, [huella], o))[huella];
  if (!it) throw Object.assign(new Error(`mediaUrl(${coleccion}, ${huella}): no item has that fingerprint`), { codigo: 404 });
  return it;
}
/** @deprecated use {@link mediaUrl}. */
export const media = alias("media", "mediaUrl", mediaUrl);

function copiasPorDefecto() {
  if (process.env.ORE_COPIAS) return process.env.ORE_COPIAS;
  try { accessSync("/trabajo", constants.W_OK); return "/trabajo/copias"; } catch { return join(tmpdir(), "ore-copias"); }
}

/** La copia de la vista como Parquet local, bajado UNA vez por sesión (la
 *  clave es el digest del artefacto: una clave nueva es otra copia). */
async function parquetDe(vista, resuelto) {
  const r = resuelto ?? (await resolver(vista));
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
  return fuenteDeRespuesta(vista, await resolver(vista));
}

// Donde se pone la vista de DuckDB de cada dataset que una View lee: aparte de
// los nombres del árbol, porque una View y su dataset pueden llamarse igual.
const ESQUEMA_DE_DATASETS = "__ore_dataset";

/** Lo que `datos` contestó, como fragmento SQL. Una View llega como su pregunta
 *  (`consulta`, SQL sobre `"__ore_dataset"."<p>.<n>"`) con sus datasets ya
 *  resueltos por el servidor: cada uno se pone como vista de DuckDB por el
 *  camino de siempre, y la View es la consulta encima. Medido: con `select *`
 *  sobre la raíz, una View con `where` y `fields` daba 20 000 filas y 4
 *  columnas donde dice 5 000 y 2 (`medida-la-vista-con-filtro.py`). */
async function fuenteDeRespuesta(vista, r) {
  if (r.consulta) {
    const con = await duckdb();
    // En `memory`, y nombradas con él: dentro de una vista de un catálogo
    // adjunto (una base, 0038) un schema sin cualificar se busca en ESE catálogo.
    await con.run(`create schema if not exists memory."${ESQUEMA_DE_DATASETS}"`);
    for (const [d, rd] of Object.entries(r.datasets ?? {})) {
      const [fuente] = await fuenteDeRespuesta(d, rd);
      await con.run(`create or replace view memory."${ESQUEMA_DE_DATASETS}"."${d.replaceAll('"', '""')}" as select * from ${fuente}`);
    }
    return [`(${r.consulta.replaceAll(`"${ESQUEMA_DE_DATASETS}".`, `memory."${ESQUEMA_DE_DATASETS}".`)})`, r];
  }
  if (r.metadata_location) {
    if (r.metadata_location.startsWith("s3://") && !s3) {
      // La credencial de lectura que `datos` presta (W3.7 gobierno ②b).
      if (r.credencial?.["s3.access-key-id"]) s3 = r.credencial;
      else {
        const [c, l] = await puesto.pedir("GET", v1Tabla(corto(vista)), undefined, 30_000, DELEGAR);
        if (c === 200 && l?.config?.["s3.access-key-id"]) s3 = l.config;
      }
    }
    return [await iceberg(r.metadata_location, r.credencial?.["gcs.oauth2.token"]), r];
  }
  const [f] = await parquetDe(vista, r);
  return [`read_parquet('${f.replaceAll("'", "''").replaceAll("\\", "/")}')`, r];
}

/** `iceberg_scan` sobre la raíz de la tabla y la versión del puntero (con
 *  `allow_moved_paths` la raíz es lo que se le pasa, y así no lista nada). En el
 *  bucket, la API XML de GCS por https con el token del pod como bearer. */
async function iceberg(metadataLocation, prestada) {
  const i = metadataLocation.lastIndexOf("/metadata/");
  let raiz = metadataLocation.slice(0, i);
  const version = metadataLocation.slice(i + "/metadata/".length).replace(/\.metadata\.json$/, "");
  const con = await duckdb();
  // `iceberg` arrastra `avro` (y usa `json` e `icu`); con el autoinstalado apagado
  // hay que cargarlas por su nombre, en orden. `httpfs` sólo para el bucket.
  for (const e of LAGO) await cargar(con, e);
  if (raiz.startsWith("gs://")) {
    await cargar(con, "httpfs");
    raiz = "https://storage.googleapis.com/" + raiz.slice(5);
    // Con la credencial prestada para este dataset (②b): un secreto por raíz,
    // con `scope`; sin ella, el token del pod (lo de antes).
    if (prestada) {
      const { createHash } = await import("node:crypto");
      const n = createHash("sha1").update(raiz).digest("hex").slice(0, 12);
      await con.run(`create or replace secret ore_gcs_${n} (type http, bearer_token '${prestada.replaceAll("'", "''")}', scope '${raiz.replaceAll("'", "''")}')`);
    } else {
      await con.run(`create or replace secret ore_gcs (type http, bearer_token '${(await tokenDeGoogle()).replaceAll("'", "''")}')`);
    }
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

/** Where the image leaves DuckDB's preinstalled extensions. */
export const EXTENSIONS = "/opt/ore/duckdb";
/** @deprecated use {@link EXTENSIONS}. */
export const EXTENSIONES = EXTENSIONS;
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
    if (existsSync(EXTENSIONES)) throw new Error(`the image does not ship DuckDB's \`${extension}\` extension: it must be preinstalled in ${EXTENSIONES}`);
    await con.run(`install ${extension}`); await con.run(`load ${extension}`);
  }
}


/** How many rows `over()`/`sql()` materialize unless told otherwise. */
/** The version of this SDK's interface (S3): 2 is the English names. Code that ORE generates checks it. */
export const API = 2;

export const LIMIT = 100_000;
/** @deprecated use {@link LIMIT}. */
export const LIMITE = LIMIT;

/** A DuckDB type, with the name Arrow (and `pyarrow`) gives it: what the console
 *  shows, and the same the Python and Java agents say. */
export function arrowName(type) {
  const t = String(type);
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
  if (t.endsWith("[]")) return `list<item: ${arrowName(t.slice(0, -2))}>`;
  if (t.startsWith("STRUCT(")) return "struct";
  if (t.startsWith("MAP(")) return "map";
  if (t.startsWith("ENUM(")) return "dictionary<values=string, indices=int32, ordered=0>";
  return t.toLowerCase();
}
/** @deprecated use {@link arrowName}. */
export const nombreArrow = alias("nombreArrow", "arrowName", arrowName);

/** Corre `texto` y materializa hasta `limite` filas (+1 para saber si había más). */
async function leerHasta(con, texto, limite) {
  const r = await con.runAndReadUntil(texto, limite + 1);
  const truncada = r.currentRowCount > limite;
  return { r, truncada };
}

/** El resultado en la forma pedida, con lo que hay que saber de él colgado. */
function entregar(r, truncada, limite, total, como) {
  const nombres = r.columnNames();
  const tipos = Object.fromEntries(nombres.map((n, i) => [n, arrowName(r.columnTypes()[i])]));
  if (como === "columns") {
    const columnas = r.getColumns().map((c) => (truncada ? c.slice(0, limite) : c));
    return conAlias({ names: nombres, types: tipos, columns: columnas, total, truncated: truncada });
  }
  if (como !== "rows") throw new Error(`as: ${JSON.stringify(como)} is not a shape: it is "rows" or "columns"`);
  const filas = r.getRowObjects();
  if (truncada) filas.length = limite;
  // Propiedades, no elementos: `rows.length`, `rows.map` y `for…of` ven sólo filas.
  Object.defineProperties(filas, {
    types: { value: tipos, enumerable: false },
    total: { value: total, enumerable: false },
    truncated: { value: truncada, enumerable: false },
  });
  return conAlias(filas);
}

/** Las formas de `as` (y las de `como`, en español). */
const FORMAS = { rows: "rows", columns: "columns", filas: "rows", columnas: "columns" };

function opciones(o) {
  const { limit = LIMIT, strict = false, as = "rows" } = opcionesEn(o);
  if (!Number.isInteger(limit) || limit < 1) throw new Error("limit takes an integer ≥ 1");
  return { limite: limit, estricto: strict, como: FORMAS[as] ?? as };
}

/**
 * The copy of `<base>.<view>`: rows (objects with typed values) up to `limit`,
 * with `.types`, `.total` and `.truncated` (non-enumerable); or, with
 * `{ as: "columns" }`, `{ names, types, columns, total, truncated }`.
 * Options: `limit` (default {@link LIMIT}), `strict` (throw instead of truncating),
 * `as` (`"rows"` | `"columns"`). The old keys `limite`, `estricto`, `como`
 * (`"filas"` | `"columnas"`) still work, and so do the old result keys
 * (`tipos`, `truncada`, `nombres`, `columnas`).
 * @param {string} view
 * @param {{limit?: number, strict?: boolean, as?: "rows" | "columns"}} [options]
 */
export async function over(view, options) {
  const vista = view;
  const { limite, estricto, como } = opciones(options);
  const [fuente] = await fuenteDe(vista);
  const con = await duckdb();
  const total = Number((await con.runAndReadAll(`select count(*) from ${fuente}`)).getColumns()[0][0]);
  if (estricto && total > limite) throw new Error(`over(${JSON.stringify(vista)}): the copy has ${total} rows and the limit is ${limite}; raise limit, aggregate in sql() or drop strict`);
  const { r, truncada } = await leerHasta(con, `select * from ${fuente}`, limite);
  return entregar(r, truncada, limite, total, como);
}

/**
 * SQL (DuckDB) over the copies: each `base.view` after FROM/JOIN is resolved,
 * fetched once and registered as a DuckDB view. Returns the same as `over()`
 * (same options); `total` is only known if the result fits in the limit.
 * @param {string} text
 * @param {{limit?: number, strict?: boolean, as?: "rows" | "columns"}} [options]
 */
export async function sql(text, options) {
  const texto = text;
  if (typeof texto !== "string" || !texto.trim()) throw new Error("sql() takes a query");
  const { limite, estricto, como } = opciones(options);
  const con = await duckdb();
  // El texto entero a ore-serve (`POST /puestos/{id}/sql`): él dice qué nombres
  // del árbol lee —tokenizador y árbol como filtro, sin regex— y los resuelve
  // como `over()`, en una ida y vuelta.
  // Sin puesto no hay árbol, y sin un punto no hay `a.b`: el motor solo.
  const [codigo, resp] = puesto.id && texto.includes(".")
    ? await puesto.pedir("POST", `/puestos/${puesto.id}/sql`, { texto })
    : [200, {}];
  if (codigo !== 200) oElError(codigo, resp, resp?.nombre ?? "?");
  for (const [v, rd] of Object.entries(resp?.fuentes ?? {}).sort(([a], [b]) => (a < b ? -1 : 1))) {
    lee(v);
    const [fuente] = await fuenteDeRespuesta(v, rd);
    await registra(con, v, fuente);
  }
  const { r, truncada } = await leerHasta(con, texto, limite);
  if (estricto && truncada) throw new Error(`sql(): the result exceeds ${limite} rows; raise limit, aggregate more or drop strict`);
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

/** El tipo de DuckDB de un tipo de Arrow (la vuelta de `arrowName`). */
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
  if (t === "uint64") throw new Error(`write(): column \`${columna}\` is uint64, which does not fit in int64 without lying (0032); convert it first`);
  if (t === "null") throw new Error(`write(): column \`${columna}\` has no type (all null): give it one first (0032)`);
  throw new Error(`write(): column \`${columna}\` is \`${t}\`, which the type contract (0032) does not have`);
}

/** `{ names, types, columns }` (o `{ nombres, tipos, columnas }`, en español) → sus partes, o null. */
function porColumnas(v) {
  if (!v || typeof v !== "object") return null;
  const nombres = v.names ?? v.nombres, columnas = v.columns ?? v.columnas;
  if (!Array.isArray(nombres) || !Array.isArray(columnas)) return null;
  return { nombres, tipos: v.types ?? v.tipos, columnas, total: v.total };
}

/** Lo que se escribe, como `{ nombres, tipos, columnas }` con tipos de Arrow. */
function columnasDe(datos) {
  const pc = Array.isArray(datos) ? null : porColumnas(datos);
  if (pc) return { nombres: pc.nombres, tipos: pc.tipos ?? {}, columnas: pc.columnas };
  if (!Array.isArray(datos)) throw new TypeError("write() takes rows (objects) or { names, types, columns }");
  if (datos.length === 0) throw new Error("write(): the table has no rows");
  const nombres = [...new Set(datos.flatMap((f) => Object.keys(f ?? {})))];
  const tipos = { ...(datos.types ?? datos.tipos ?? {}) };
  for (const n of nombres) {
    if (!tipos[n]) {
      const v = datos.find((f) => f?.[n] !== undefined && f?.[n] !== null)?.[n];
      tipos[n] = inferredType(v);
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
    if (!d) throw new Error(`write(): column \`${n}\` is \`${tipos[n]}\`, which the type contract (0032) does not have`);
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
    if (!env.ORE_GCS_TOKEN) throw new Error(`write(): the catalog vended no credential for \`${ubicacion}\``);
  } else if (ubicacion.startsWith("s3://")) {
    nombre = "ore-store-r2";
    env.ORE_R2_BUCKET = ubicacion.slice(5).split("/")[0];
    env.ORE_R2_S3_ENDPOINT = config["s3.endpoint"] ?? "";
    env.ORE_R2_ACCESS_KEY_ID = config["s3.access-key-id"] ?? "";
    env.ORE_R2_SECRET_ACCESS_KEY = config["s3.secret-access-key"] ?? "";
    env.ORE_R2_REGION = config["s3.region"] ?? "auto";
  } else {
    throw new Error(`write(): the table lives in \`${ubicacion}\`, which is not a lake this SDK can write`);
  }
  const dirs = [process.env.ORE_STORE_DIR, ...(process.env.PATH ?? "").split(delimiter)].filter(Boolean);
  for (const d of dirs) {
    for (const ext of ["", ".exe"]) {
      const b = join(d, nombre + ext);
      if (existsSync(b)) return { binario: b, env };
    }
  }
  throw new Error(`write(): \`${nombre}\` is not on the PATH (the session image ships it; elsewhere, ORE_STORE_DIR)`);
}

function mensajeDe(r) {
  const e = r?.error;
  return typeof e === "object" && e ? (e.message ?? JSON.stringify(e)) : String(e ?? JSON.stringify(r));
}

/** `mode` (inglés o español) → el valor que viaja a `ore-store` y entra en la
 *  semilla de la operación: el de siempre, en español (cambiarlo cambiaría la
 *  clave de operación y una escritura repetida dejaría de serlo). */
const MODO_AL_CABLE = { overwrite: "sobrescribir", append: "anexar", upsert: "upsert", sobrescribir: "sobrescribir", anexar: "anexar" };

/**
 * Writes `data` as the lake dataset `<base>.<table>` (see above): rows (objects),
 * what `over()`/`sql()` returned, or `{ names, types, columns }`.
 * Options: `mode` — `"overwrite"` (default), `"append"` or `"upsert"` (the old
 * `"sobrescribir"`/`"anexar"` still work); `key` — the columns that identify a
 * row, only with `mode: "upsert"`. The old option keys `modo`/`clave` still work.
 * Returns `{ table, rows, snapshot, metadata_location, operation, repeated }`
 * (the old keys `tabla`, `filas`, `operacion`, `repetida` still read).
 * @param {string} name
 * @param {object[] | {names: string[], types?: object, columns: any[][]}} data
 * @param {{mode?: "overwrite" | "append" | "upsert", key?: string[]}} [options]
 */
export async function write(name, data, options) {
  let nombre = name;
  const datos = data;
  const { mode = "overwrite", key: clave } = opcionesEn(options);
  nombre = corto(nombre, "write(): the name");
  const modo = MODO_AL_CABLE[mode];
  if (!modo) throw new Error(`mode: ${JSON.stringify(mode)}: it is "overwrite", "append" or "upsert"`);
  if (clave !== undefined && (!Array.isArray(clave) || !clave.every((c) => typeof c === "string"))) throw new Error(`key: ${JSON.stringify(clave)}: a list of column names`);
  if (clave !== undefined && modo !== "upsert") throw new Error('`key` goes with mode: "upsert"');
  const [bd, ns, t] = partes(nombre); // el namespace de /v1 es el schema (0038 P4)
  const { nombres, tipos, columnas } = columnasDe(datos);
  if (nombres.length === 0 || (columnas[0]?.length ?? 0) === 0) throw new Error("write(): the table has no rows");
  const esquema = { type: "struct", "schema-id": 0, fields: nombres.map((n, i) => ({ id: i + 1, name: n, type: tipoIceberg(n, tipos[n]), required: false })) };
  const parquet = await parquetDe_(nombres, tipos, columnas);
  const dataset = `catalogo/${bd}/${ns}/${t}`; // una etiqueta: la ubicación la da el catálogo
  if (transformActivo && nombre !== transformActivo.output) throw new Error(`\`${nombre}\` is not the output of \`${transformActivo.nombre}\` (${transformActivo.output}): a transform only writes what it declares`);
  const semilla = `${nombre}|${modo}` + (clave?.length ? `|${clave.join(",")}` : "");
  const cargar = async () => {
    const [c, r] = await puesto.pedir("GET", v1Tabla(nombre), undefined, 30_000, DELEGAR);
    // Prestado sólo para leer (lo de otra persona, un mantenido): el porqué,
    // antes de escribir un fichero con una credencial que no escribe.
    if (c === 200 && r.config?.["ore.solo-lectura"]) throw new Error(`write(${nombre}): ${r.config["ore.solo-lectura"]}`);
    if (c === 200) return { base: r["metadata-location"], esbozo: null, config: r.config ?? {}, ubicacion: r.metadata.location };
    if (c === 404) {
      const [c2, r2] = await puesto.pedir("POST", `/v1/${bd}/namespaces/${ns}/tables`, { name: t, "stage-create": true, schema: esquema, properties: {} }, 30_000, DELEGAR);
      if (c2 !== 200) throw new Error(`write(${nombre}): ${mensajeDe(r2)}`);
      return { base: null, esbozo: r2.metadata, config: r2.config ?? {}, ubicacion: r2.metadata.location };
    }
    throw new Error(`write(${nombre}): ore-serve answered ${c}: ${mensajeDe(r)}`);
  };
  let claveOperacion = "";
  let escrito = null;
  for (let intento = 0; intento < 4; intento++) {
    const { base, esbozo, config, ubicacion } = await cargar();
    if (config["s3.access-key-id"]) s3 = config;
    const { binario, env } = escritor(config, ubicacion);
    const peticion = { dataset, modo, formato: "parquet", operacion: "contenido", semilla, procedencia: procedencia(nombre) };
    if (clave?.length) peticion.clave = clave;
    if (base) peticion.base = base; else peticion.esbozo = esbozo;
    const p = spawnSync(binario, ["escribir"], { input: Buffer.concat([Buffer.from(JSON.stringify(peticion) + "\n"), parquet]), env, maxBuffer: 1 << 26 });
    if (p.status !== 0) throw new Error(`write(): ${(p.stderr?.toString("utf8") ?? "").trim().replace(/^error: /, "") || "the writer failed"}`);
    escrito = JSON.parse(p.stdout.toString("utf8"));
    claveOperacion = escrito.operacion || claveOperacion;
    const [c, r] = await puesto.pedir("POST", v1Tabla(nombre), { identifier: { namespace: [ns], name: t }, requirements: escrito.requirements, updates: escrito.updates }, 120_000);
    if (c === 200) {
      const snap = r?.metadata?.["current-snapshot-id"];
      // repetida: el catálogo contestó con lo que ya había (el mismo puntero)
      // — los ids de snapshot no se comparan: en JS un int64 pierde precisión
      return conAlias({ table: nombre, rows: escrito.filas, snapshot: String(snap ?? ""), metadata_location: r?.["metadata-location"] ?? "", operation: claveOperacion, repeated: base !== null && r?.["metadata-location"] === base });
    }
    if (c === 409) continue; // alguien escribió mientras tanto: otra vez sobre lo que hay
    if (c >= 500) {
      // el commit pudo entrar: se MIRA antes de darlo por perdido
      const [c2, r2] = await puesto.pedir("GET", v1Tabla(nombre));
      if (c2 === 200) {
        const md = r2.metadata;
        const vigente = (md.snapshots ?? []).find((x) => x["snapshot-id"] === md["current-snapshot-id"]);
        if (vigente?.summary?.["ore.operacion"] === claveOperacion) {
          return conAlias({ table: nombre, rows: escrito.filas, snapshot: String(md["current-snapshot-id"]), metadata_location: r2["metadata-location"], operation: claveOperacion, repeated: false });
        }
      }
      throw new Error(`write(${nombre}): the catalog answered ${c} and the commit is not there: ${mensajeDe(r)}`);
    }
    throw new Error(`write(${nombre}): ${mensajeDe(r)}`);
  }
  throw new Error(`write(${nombre}): someone else wrote first four times; try again`);
}

// ── El JSON de la consola (0032 §1) ───────────────────────────────────────
/**
 * What `over()` and `sql()` return —rows (objects with typed values) or
 * `{ names, types, columns }`— → the console's `tabla` output of the contract
 * (0032 §1, «console JSON»): the SAME JSON the Python and Java agents emit.
 * Any array of objects works too (without declared types they are inferred
 * from the first value).
 *
 * ⚠ What it returns IS the wire format the agent posts to ore-serve
 * (`{columnas, filas, total, limite}`), so its keys stay as they are.
 * @param {any} value
 * @param {number} [limit=200]
 */
export function table(value, limit = 200) {
  const valor = value, limite = limit;
  const pc = Array.isArray(valor) ? null : porColumnas(valor);
  if (pc) {
    const n = pc.columnas[0]?.length ?? 0;
    const filas = [];
    for (let i = 0; i < Math.min(n, limite); i++) filas.push(pc.columnas.map((c) => toJson(c[i])));
    return {
      columnas: pc.nombres.map((c) => ({ name: c, type: pc.tipos?.[c] ?? "null" })),
      filas,
      total: pc.total ?? n,
      limite,
    };
  }
  if (!Array.isArray(valor) || valor.length === 0) return null;
  if (!valor.every((f) => f && typeof f === "object" && !Array.isArray(f))) return null;
  const cabeza = valor.slice(0, limite);
  const tiposDados = valor.types ?? valor.tipos;
  const columnas = tiposDados ? Object.keys(tiposDados) : [...new Set(cabeza.flatMap((f) => Object.keys(f)))];
  const tipoDe = (c) => tiposDados?.[c] ?? inferredType(valor.find((f) => f[c] !== null && f[c] !== undefined)?.[c]);
  return {
    columnas: columnas.map((c) => ({ name: c, type: tipoDe(c) })),
    filas: cabeza.map((f) => columnas.map((c) => toJson(f[c]))),
    total: valor.total ?? valor.length,
    limite,
  };
}
/** @deprecated use {@link table}. */
export const tabla = alias("tabla", "table", table);

/** El tipo de Arrow de un valor suelto, para lo que no viene de `over()`/`sql()`. */
function inferredType(v) {
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

/** A value (DuckDB-typed, or plain) → the contract's JSON (0032 §1):
 *  integer → number if |x| ≤ 2⁵³, else string · decimal → always a string
 *  (except scale 0, which is an integer and goes as one) ·
 *  float → number, with `NaN`/`Infinity`/`-Infinity` as strings · date
 *  `YYYY-MM-DD` · time `HH:MM:SS[.ffffff]` · zone-less datetime in ISO with `T`
 *  · UTC instant with `Z` · bytes in base64 · list → array · struct →
 *  object · map → `[{key, value}]`. Nothing is silently degraded. */
export function toJson(v) {
  const jsonDe = toJson;
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

/** @deprecated use {@link toJson}. */
export const jsonDe = alias("jsonDe", "toJson", toJson);

export default {
  API,
  over, sql, write, declare, transform, person, session, arrowName, LIMIT, EXTENSIONS, table, toJson, mediaUrl, mediaUrls, mediaColumns,
  // los nombres de antes (alias, el mismo objeto)
  persona, puesto, nombreArrow, LIMITE, EXTENSIONES, tabla, jsonDe, media, medias, mediaDe,
};
