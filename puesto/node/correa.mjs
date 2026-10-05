// LA CORREA DE NODE (0050 L3·3): entre el editor y el servidor de TypeScript.
//
// La gemela de la `Correa` de `puesto/python/agente.py` —el mismo canal, los
// mismos verbos—, con el servidor de su lenguaje: `typescript-language-server`
// sobre el `tsserver` de la imagen (`/opt/ore/tipos`, L3·2).
//
//   GET  /puestos/{id}/lsp/agente   → flujo de eventos: lo que el editor manda
//   POST /puestos/{id}/lsp/salida   ← lo que el servidor contesta, en lotes
//
// ── Lo que esta correa SÍ entiende, y la de Python no ──────────────────────
//
// Medido en L3·0 con un cliente LSP de verdad: con el documento sólo en la
// memoria del servidor, `ore` y los paquetes de npm se resuelven, pero sin el
// `tsconfig.json` en disco el servidor tipa con sus opciones por defecto y
// deja pasar un `enum` (TS1294), que Node no puede correr. Con el repositorio
// en disco, dice lo mismo que `tsc`. Así que, antes de arrancarlo:
//
//   1. EL ÁRBOL DE TIPOS: `/trabajo/packages/node_modules`, enlaces —lo de la
//      imagen primero (manda el contenedor), luego lo que sólo tipa de la
//      imagen, la capa y su caja de tipos—. Un fichero de
//      `/trabajo/packages/<p>/<repo>/…` lo encuentra subiendo; lo que corre
//      (`/trabajo/celdas` → `/trabajo/node_modules`) no lo ve nunca: lo de
//      desarrollo no llega a ejecución.
//   2. EL REPOSITORIO EN DISCO: su código, su `tsconfig.json` y su
//      `package.json`, leídos de `ore-serve` en la rama del puesto
//      (`GET /arbol` acotado a su carpeta, y cada fichero).
//   3. EL ESPEJO: lo que el editor abre o cambia se escribe también en disco.
//      Si no, al cerrar una pestaña el servidor volvería a leer el fichero de
//      antes.
//
// Y en el `initialize` del editor, lo que la medida pidió: un solo `tsserver`
// (`useSyntaxServer: "never"`: ~255 MB y no ~385, los mismos diagnósticos) y
// SIN el instalador automático de tipos, que saldría a npm saltándose la capa.
//
// ⛔ No arranca solo: si nadie abre un fichero, la sesión no lo paga. El primer
//   mensaje del editor lo enciende.
//
// ── `ore/comprobar` (L4): `tsc` sobre el repositorio, al guardar ──────────
//
// El servidor de lenguaje dice lo de los ficheros abiertos, mientras se teclea.
// Lo que rompe un cambio en OTRO fichero —un `helpers.ts` que ya no exporta lo
// que otro importa— sólo lo dice `tsc` sobre el proyecto entero. La consola lo
// pide tras un commit con esta petición propia, que NO va al servidor de
// lenguaje ni espera en su cola: se trae el repositorio tal como quedó en la
// rama (lo guardado, no lo que hay en las pestañas), y `tsc --noEmit -p` con el
// mismo árbol de tipos. Contesta `{ diagnosticos, errores, ficheros, ms }`.
//
// ── `ore/probar` (L5): las pruebas del repositorio, desde Test ─────────────
//
// `node --test` sobre sus `*.test.*`/`*.spec.*` —lo guardado y, encima, lo que
// la consola manda sin guardar—, con el informe de `informe-de-pruebas.mjs`,
// en un proceso sin la identidad de la sesión y con su tope. Contesta
// `{ todos, ficheros, pruebas, resumen, ms }`: cada prueba con su estado, su
// `describe`, su línea y, si falla, el mensaje, lo esperado y lo obtenido.
import { execFile, spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const AQUI = dirname(fileURLToPath(import.meta.url));
const ENLACE = process.platform === "win32" ? "junction" : "dir";
const log = (...a) => process.stdout.write("correa · " + a.join(" ") + "\n");

/** Lo que la imagen trae para correr (`ore`, `@duckdb/node-api`). */
const IMAGEN = process.env.ORE_IMAGEN_NODE ?? join(AQUI, "node_modules");
/** Lo que sólo tipa en la imagen (L3·2), y el servidor de lenguaje. */
const TIPOS_IMAGEN = process.env.ORE_TIPOS_IMAGEN ?? "/opt/ore/tipos/node_modules";
/** Lo que el repositorio declara, resuelto: lo que corre y lo que sólo tipa. */
const CAPA = process.env.ORE_CAPA_NODE ?? "/capa/node_modules";
const TIPOS_CAPA = process.env.ORE_TIPOS_NODE ?? "/capa/tipos/node_modules";

/** Lo que se trae del repositorio al disco: lo que el servidor lee. */
const DE_CODIGO = /\.(ts|mts|cts|tsx|js|mjs|cjs|jsx|json)$/;
const FICHEROS_MAXIMOS = 2000;
const BYTES_MAXIMOS = 1 << 20;

/** Enlaza en `a` cada paquete de `desde` que `a` no tenga ya (los `@ámbito/…`,
 *  uno a uno dentro de su ámbito): el primero que llega, manda. */
export function enlazar(desde, a) {
  if (!existsSync(desde)) return 0;
  let n = 0;
  for (const e of readdirSync(desde)) {
    if (e.startsWith(".")) continue;
    const de = join(desde, e), destino = join(a, e);
    if (e.startsWith("@")) {
      if (!existsSync(destino)) mkdirSync(destino);
      for (const x of readdirSync(de)) {
        if (!existsSync(join(destino, x))) { symlinkSync(join(de, x), join(destino, x), ENLACE); n++; }
      }
    } else if (!existsSync(destino)) {
      symlinkSync(de, destino, ENLACE);
      n++;
    }
  }
  return n;
}

/** El fichero de disco de una uri del editor, si cae dentro de `trabajo`. */
export function enDisco(uri, trabajo) {
  if (typeof uri !== "string" || !uri.startsWith("file:")) return null;
  let f;
  try { f = resolve(fileURLToPath(uri)); } catch { return null; }
  const base = resolve(trabajo) + sep;
  return f.startsWith(base) && !f.slice(base.length).split(sep).includes("node_modules") ? f : null;
}

/**
 * Lo que `tsc --pretty false` dice, como diagnósticos del árbol: `ruta(l,c):
 * error TS1234: mensaje`, y las líneas sangradas que siguen, del mismo mensaje.
 * Las rutas, relativas al árbol (`tsc` las da desde su `cwd`, `/trabajo`).
 */
export function leerTsc(salida) {
  const diagnosticos = [];
  for (const linea of salida.split(/\r?\n/)) {
    const m = /^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.*)$/.exec(linea);
    const g = !m && /^(error|warning) (TS\d+): (.*)$/.exec(linea);
    if (m) diagnosticos.push({ fichero: m[1].replace(/\\/g, "/"), linea: Number(m[2]), columna: Number(m[3]), codigo: m[5], mensaje: m[6], severidad: m[4] === "error" ? "error" : "aviso" });
    else if (g) diagnosticos.push({ codigo: g[2], mensaje: g[3], severidad: g[1] === "error" ? "error" : "aviso" });
    else if (/^\s+\S/.test(linea) && diagnosticos.length) diagnosticos.at(-1).mensaje += "\n" + linea.trim();
  }
  const errores = diagnosticos.filter((d) => d.severidad === "error").length;
  const ficheros = new Set(diagnosticos.map((d) => d.fichero).filter(Boolean)).size;
  return { diagnosticos: diagnosticos.slice(0, 500), errores, ficheros };
}

/** Los ficheros de un directorio que cumplen `re`, relativos a él, sin `node_modules`. */
function ficherosQue(re, dir, base = "") {
  if (!existsSync(dir)) return [];
  const fuera = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "node_modules" || e.name.startsWith(".")) continue;
    const rel = base ? `${base}/${e.name}` : e.name;
    if (e.isDirectory()) fuera.push(...ficherosQue(re, join(dir, e.name), rel));
    else if (re.test(e.name)) fuera.push(rel);
  }
  return fuera.sort();
}

/** Los ficheros de prueba de un repositorio, relativos a él: `*.test.*` y `*.spec.*`. */
export const ficherosDePrueba = (dir) => ficherosQue(/\.(test|spec)\.(ts|mts|cts|js|mjs|cjs)$/, dir);
/** Lo que `materializar` trae, tal como está en disco. */
const codigoEnDisco = (dir) => ficherosQue(DE_CODIGO, dir);

/**
 * Lo que `informe-de-pruebas.mjs` escribió (una línea JSON por cosa), como
 * resultado de `ore/probar`: las pruebas y, por fichero, su estado, lo que
 * imprimió y —si ni cargó— por qué.
 */
export function leerPruebas(texto, pedidos) {
  const pruebas = [];
  const porFichero = new Map(pedidos.map((f) => [f, { ruta: f, estado: "ok", salida: "" }]));
  const fichero = (f) => porFichero.get(f) ?? porFichero.set(f, { ruta: f, estado: "ok", salida: "" }).get(f);
  for (const linea of texto.split("\n")) {
    if (!linea.trim()) continue;
    let x;
    try { x = JSON.parse(linea); } catch { continue; }
    if (x.e === "prueba") pruebas.push(x);
    else if (x.e === "salida" && x.fichero) fichero(x.fichero).salida += x.texto;
    else if (x.e === "fichero" && x.fichero) Object.assign(fichero(x.fichero), { estado: "error", mensaje: x.mensaje, traza: x.traza });
  }
  for (const f of porFichero.values()) {
    // Un fichero que no cargó dice POR QUÉ en lo que imprimió (el error de Node), no en el evento.
    if (f.estado === "error" && f.salida.trim()) f.mensaje = f.salida.replace(/\n\s+at .*$/gm, "").replace(/\nNode\.js v[\d.]+\s*$/, "").trim();
    f.salida = f.salida.slice(-20_000);
    if (f.estado !== "error" && pruebas.some((p) => p.fichero === f.ruta && (p.estado === "fallo" || p.estado === "cancelada"))) f.estado = "fallo";
  }
  return { ficheros: [...porFichero.values()], pruebas };
}

export class Correa {
  /**
   * @param p        la sesión (`ore.session`): `id`, `servidor`, `pedir`
   * @param testigo  quien da las cabeceras de identidad (`cabeceras()`)
   * @param alActividad  (0050 sesión automática) a cada mensaje del editor: el
   *   agente lo cuenta como actividad para su TTL. Con la sesión abierta sola
   *   al abrir un `.ts`, escribir —hover, diagnósticos, `tsc`, las pruebas— es
   *   trabajar, aunque no se corra ninguna celda.
   */
  constructor(p, testigo, { trabajo = process.env.TRABAJO_DIR ?? "/trabajo", orden, alActividad } = {}) {
    this.p = p;
    this.alActividad = alActividad ?? (() => {});
    this.testigo = testigo;
    this.trabajo = trabajo;
    this.orden = orden ?? (process.env.ORE_LSP
      ? process.env.ORE_LSP.split(" ").filter(Boolean)
      : [process.execPath, join(TIPOS_IMAGEN, "typescript-language-server", "lib", "cli.mjs"), "--stdio"]);
    this.proceso = null;
    this.salientes = [];
    this.cadena = Promise.resolve();
    this.vivo = false;
    this.reloj = null;
  }

  // ── antes de arrancar: los tipos y el repositorio en disco ───────────────
  arbolDeTipos() {
    const nm = join(this.trabajo, "packages", "node_modules");
    mkdirSync(nm, { recursive: true });
    const n = [IMAGEN, TIPOS_IMAGEN, CAPA, TIPOS_CAPA].map((d) => enlazar(d, nm));
    log(`árbol de tipos · ${n[0]} de la imagen, ${n[1]} que sólo tipan, ${n[2]} de la capa y ${n[3]} de sus tipos`);
  }

  async materializar() {
    const t0 = Date.now();
    const cab = await this.testigo.cabeceras();
    const [c, ficha] = await this.p.pedir("GET", `/puestos/${this.p.id}`, undefined, 30_000, cab);
    const repo = c === 200 ? ficha?.repositorio : null;
    if (!repo) { log("sin repositorio: el servidor sólo verá lo que el editor abra"); return null; }
    const rama = ficha?.rama ? { "x-ore-rama": ficha.rama } : {};
    const [ci, indice] = await this.p.pedir("GET", "/arbol", undefined, 60_000, { ...cab, ...rama, "x-ore-raiz": repo });
    if (ci !== 200) { log(`el índice de ${repo} contestó ${ci}: sin repositorio en disco`); return null; }
    const todos = (indice?.ficheros ?? []).filter((f) =>
      DE_CODIGO.test(f.ruta) && !f.ruta.includes("node_modules/") && !f.ruta.endsWith("package-lock.json") && (f.bytes ?? 0) <= BYTES_MAXIMOS);
    const lista = todos.slice(0, FICHEROS_MAXIMOS);
    if (todos.length > lista.length) log(`⚠️ ${todos.length} ficheros de código: se traen ${lista.length}`);
    let n = 0;
    const cola = [...lista];
    await Promise.all(Array.from({ length: 8 }, async () => {
      for (let f; (f = cola.shift()); ) {
        const ruta = f.ruta.split("/").map(encodeURIComponent).join("/");
        const [cf, r] = await this.p.pedir("GET", `/arbol/${ruta}`, undefined, 30_000, { ...cab, ...rama });
        const destino = resolve(this.trabajo, f.ruta);
        if (cf !== 200 || typeof r?.texto !== "string" || !destino.startsWith(resolve(this.trabajo) + sep)) continue;
        mkdirSync(dirname(destino), { recursive: true });
        writeFileSync(destino, r.texto);
        n++;
      }
    }));
    // Lo que la rama ya no tiene, tampoco el disco: una prueba borrada no corre.
    const enLaRama = new Set(lista.map((f) => f.ruta));
    let fuera = 0;
    for (const rel of codigoEnDisco(join(this.trabajo, repo))) {
      if (!enLaRama.has(`${repo}/${rel}`)) { rmSync(join(this.trabajo, repo, rel), { force: true }); fuera++; }
    }
    log(`repositorio ${repo}${ficha?.rama ? ` (${ficha.rama})` : ""} en disco · ${n} fichero(s)${fuera ? `, ${fuera} retirado(s)` : ""} en ${Date.now() - t0} ms`);
    return repo;
  }

  // ── `ore/comprobar` (L4) ─────────────────────────────────────────────────
  async comprobar(m) {
    const t0 = Date.now();
    let result;
    try {
      this.arbolDeTipos();
      const repo = await this.materializar();
      if (!repo) result = { error: "este puesto no tiene repositorio: no hay proyecto que comprobar" };
      else if (!existsSync(join(this.trabajo, repo, "tsconfig.json")))
        result = { repositorio: repo, error: "el repositorio no tiene tsconfig.json: la plantilla v4 lo trae (Actualizar)" };
      else result = { repositorio: repo, ...(await this.tsc(repo)) };
    } catch (e) {
      result = { error: String(e?.message ?? e) };
    }
    result.ms = Date.now() - t0;
    log(`comprobar · ${result.error ?? `${result.errores} error(es) en ${result.ficheros} fichero(s)`} · ${result.ms} ms`);
    this.salientes.push(JSON.stringify({ jsonrpc: "2.0", id: m.id, result }));
    this.entregas();
  }

  // ── `ore/probar` (L5) ────────────────────────────────────────────────────
  /**
   * Las pruebas del repositorio (`*.test.ts`, `*.spec.ts`, y sus `.js`), con el
   * corredor de Node, sobre LO QUE SE VE EN EL EDITOR: lo guardado en la rama y,
   * encima, los borradores que manda la consola. Pide `{ ficheros?, nombres?,
   * borradores? }`: sin `ficheros`, todas; con `nombres`, sólo esas pruebas.
   *
   * ⛔ Unitarias: corren en un proceso aparte, SIN la identidad de la sesión
   *   (sin `PUESTO` ni el token en su entorno): una prueba no lee datos. Con su
   *   tope (120 s) y, en Linux, matando su grupo de procesos entero: el
   *   corredor abre uno por fichero, y un bucle infinito no se para con
   *   `--test-timeout`.
   */
  async probar(m) {
    const t0 = Date.now();
    const q = m.params ?? {};
    let result;
    try {
      this.arbolDeTipos();
      const repo = await this.materializar();
      if (!repo) result = { error: "este puesto no tiene repositorio: no hay pruebas que correr" };
      else {
        // Los borradores de la consola, encima de lo guardado (sólo los del repositorio).
        for (const b of Array.isArray(q.borradores) ? q.borradores : []) {
          if (typeof b?.ruta !== "string" || typeof b?.texto !== "string" || !b.ruta.startsWith(`${repo}/`)) continue;
          const f = resolve(this.trabajo, b.ruta);
          if (!f.startsWith(resolve(this.trabajo, repo) + sep) || f.split(sep).includes("node_modules")) continue;
          mkdirSync(dirname(f), { recursive: true });
          writeFileSync(f, b.texto);
        }
        const todos = ficherosDePrueba(join(this.trabajo, repo)).map((f) => `${repo}/${f}`);
        const pedidos = Array.isArray(q.ficheros) && q.ficheros.length ? todos.filter((f) => q.ficheros.includes(f)) : todos;
        result = { repositorio: repo, todos, ...(pedidos.length ? await this.node(pedidos, Array.isArray(q.nombres) ? q.nombres : []) : { ficheros: [], pruebas: [] }) };
      }
    } catch (e) {
      result = { error: String(e?.message ?? e) };
    }
    result.ms = Date.now() - t0;
    if (result.pruebas) {
      const n = (e) => result.pruebas.filter((p) => p.estado === e).length;
      result.resumen = { total: result.pruebas.length, ok: n("ok"), fallo: n("fallo") + n("cancelada"), saltada: n("saltada") + n("pendiente"), ficherosConError: result.ficheros.filter((f) => f.estado === "error").length };
    }
    log(`probar · ${result.error ?? `${result.resumen.ok} bien, ${result.resumen.fallo} mal, ${result.resumen.saltada} saltada(s)`} · ${result.ms} ms`);
    this.salientes.push(JSON.stringify({ jsonrpc: "2.0", id: m.id, result }));
    this.entregas();
  }

  /** `node --test` con el informe de `informe-de-pruebas.mjs`, en su proceso. */
  node(ficheros, nombres) {
    const argumentos = [
      ...(Number(process.versions.node.split(".")[0]) < 23 ? ["--experimental-strip-types", "--no-warnings"] : []),
      "--test",
      `--test-reporter=${pathToFileURL(join(AQUI, "informe-de-pruebas.mjs")).href}`,
      "--test-timeout=30000",
      ...nombres.map((n) => `--test-name-pattern=^${String(n).replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")}$`),
      ...ficheros,
    ];
    // ⛔ Sin la identidad de la sesión: sólo lo que un proceso necesita.
    const entorno = { PATH: process.env.PATH ?? "", HOME: this.trabajo, NODE_ENV: "test", ...(process.platform === "win32" ? { SystemRoot: process.env.SystemRoot ?? "" } : {}) };
    const tope = Number(process.env.ORE_TOPE_PRUEBAS_MS ?? 120_000);
    return new Promise((ok) => {
      const hijo = spawn(process.execPath, argumentos, { cwd: this.trabajo, env: entorno, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"] });
      let salida = "", errores = "", vencido = false;
      hijo.stdout.on("data", (d) => (salida += d));
      hijo.stderr.on("data", (d) => (errores += d));
      const reloj = setTimeout(() => {
        vencido = true;
        try { process.platform !== "win32" ? process.kill(-hijo.pid, "SIGKILL") : hijo.kill("SIGKILL"); } catch { /* ya no estaba */ }
      }, tope);
      hijo.on("close", () => {
        clearTimeout(reloj);
        const r = leerPruebas(salida, ficheros);
        if (vencido) r.error = `las pruebas tardaron más de ${Math.round(tope / 1000)} s: se pararon`;
        else if (!r.pruebas.length && errores.trim()) r.error = errores.trim().slice(-2000);
        ok(r);
      });
    });
  }

  /** `tsc --noEmit -p <repositorio>`, y lo que dice, como diagnósticos del árbol. */
  tsc(repo) {
    const tsc = join(TIPOS_IMAGEN, "typescript", "bin", "tsc");
    return new Promise((ok) => {
      execFile(process.execPath, [tsc, "--noEmit", "-p", join(this.trabajo, repo), "--pretty", "false"],
        { cwd: this.trabajo, timeout: 120_000, maxBuffer: 16 << 20 }, (e, salida) => {
          if (e?.killed) return ok({ error: "tsc tardó más de 120 s" });
          ok(leerTsc(String(salida ?? "")));
        });
    });
  }

  async encender() {
    if (this.proceso !== null) return this.proceso !== false;
    try { this.arbolDeTipos(); } catch (e) { log(`sin árbol de tipos (${e.message})`); }
    try { await this.materializar(); } catch (e) { log(`sin repositorio en disco (${e.message})`); }
    if (!existsSync(this.orden[1] ?? this.orden[0]) && !process.env.ORE_LSP) {
      log(`no hay servidor de lenguaje (${this.orden.join(" ")}): el editor se queda sin ayuda`);
      this.proceso = false;
      return false;
    }
    const s = spawn(this.orden[0], this.orden.slice(1), { cwd: this.trabajo, stdio: ["pipe", "pipe", "ignore"] });
    s.on("error", (e) => { log(`el servidor de lenguaje no arranca (${e.message})`); this.proceso = false; });
    s.on("exit", (c) => { log(`el servidor de lenguaje terminó (${c})`); this.proceso = null; });
    // Escribirle a un servidor que acaba de irse es un EPIPE: se dice y se sigue.
    s.stdin.on("error", (e) => log(`el servidor de lenguaje no escucha (${e.code ?? e.message})`));
    this.proceso = s;
    this.leer(s.stdout);
    log(`servidor de lenguaje arrancado: ${this.orden.map((x) => x.replace(/.*[\\/]node_modules[\\/]/, "")).join(" ")}`);
    return true;
  }

  // ── lo que llega del editor ──────────────────────────────────────────────
  /** Un mensaje del editor, en orden: lo que llega mientras se prepara espera.
   *  Salvo `ore/comprobar`, que va por su cuenta: un `tsc` no detiene un hover. */
  escribir(crudo) {
    this.alActividad();
    if (crudo.includes('"ore/comprobar"') || crudo.includes('"ore/probar"')) {
      let m = null;
      try { m = JSON.parse(crudo); } catch { /* no era */ }
      if (m?.method === "ore/comprobar") return this.comprobar(m);
      if (m?.method === "ore/probar") return this.probar(m);
    }
    this.cadena = this.cadena.then(() => this.entregarAlServidor(crudo)).catch((e) => log(`un mensaje se perdió (${e.message})`));
    return this.cadena;
  }

  async entregarAlServidor(crudo) {
    let m = null;
    try { m = JSON.parse(crudo); } catch { /* lo que no es JSON va tal cual */ }
    if (m && typeof m === "object") {
      if (m.method === "initialize") {
        // ⭐ Lo que la medida pidió (L3·0), diga lo que diga el editor.
        const io = m.params?.initializationOptions ?? {};
        m.params = {
          ...(m.params ?? {}),
          initializationOptions: {
            ...io,
            disableAutomaticTypingAcquisition: true,
            tsserver: { ...(io.tsserver ?? {}), path: join(TIPOS_IMAGEN, "typescript", "lib"), useSyntaxServer: "never" },
          },
        };
        crudo = JSON.stringify(m);
      }
      this.espejo(m);
    }
    if (!(await this.encender()) || !this.proceso?.stdin.writable) return;
    const b = Buffer.from(crudo, "utf8");
    this.proceso.stdin.write(`Content-Length: ${b.length}\r\n\r\n`);
    this.proceso.stdin.write(b);
  }

  /** Lo que el editor abre o cambia, también en disco (ver arriba, 3). */
  espejo(m) {
    let uri, texto;
    if (m.method === "textDocument/didOpen") ({ uri, text: texto } = m.params?.textDocument ?? {});
    else if (m.method === "textDocument/didChange") {
      uri = m.params?.textDocument?.uri;
      const cambios = m.params?.contentChanges ?? [];
      // Sólo el texto entero (es lo que manda la consola); un trozo con rango no se aplica a ciegas.
      const entero = cambios.length && cambios.every((c) => !c.range) ? cambios.at(-1).text : undefined;
      texto = entero;
    }
    if (typeof texto !== "string") return;
    const f = enDisco(uri, this.trabajo);
    if (!f) return;
    try { mkdirSync(dirname(f), { recursive: true }); writeFileSync(f, texto); } catch (e) { log(`no pude reflejar ${f} (${e.message})`); }
  }

  // ── lo que contesta el servidor, en lotes ────────────────────────────────
  leer(salida) {
    let buf = Buffer.alloc(0);
    salida.on("data", (d) => {
      buf = Buffer.concat([buf, d]);
      for (;;) {
        const fin = buf.indexOf("\r\n\r\n");
        if (fin < 0) return;
        const largo = Number(/content-length:\s*(\d+)/i.exec(buf.subarray(0, fin).toString())?.[1]);
        if (!Number.isFinite(largo)) { buf = buf.subarray(fin + 4); continue; }
        if (buf.length < fin + 4 + largo) return;
        this.salientes.push(buf.subarray(fin + 4, fin + 4 + largo).toString("utf8"));
        buf = buf.subarray(fin + 4 + largo);
      }
    });
    this.entregas();
  }

  entregas() {
    if (!this.reloj) this.reloj = setInterval(() => void this.vaciar(), 20);
  }

  /** ⛔ Un mensaje por petición serían decenas de conexiones por segundo: se agrupan. */
  async vaciar() {
    if (!this.salientes.length || this.vaciando) return;
    this.vaciando = true;
    const lote = this.salientes;
    this.salientes = [];
    try {
      const [c, r] = await this.p.pedir("POST", `/puestos/${this.p.id}/lsp/salida`, { mensajes: lote }, 30_000, await this.testigo.cabeceras());
      if (![200, 201, 202].includes(c)) log(`la salida del servidor de lenguaje no se aceptó: ${c} ${JSON.stringify(r)}`);
    } catch (e) {
      log(`no pude entregar ${lote.length} mensaje(s) del servidor de lenguaje: ${e.message}`);
    } finally {
      this.vaciando = false;
    }
  }

  // ── el flujo de entrada ──────────────────────────────────────────────────
  /** El flujo de `ore-serve`, que se despide a los 240 s («vuelve»): se reconecta. */
  async escuchar() {
    this.vivo = true;
    while (this.vivo) {
      try {
        await this.unaVuelta();
      } catch (e) {
        if (!this.vivo) return;
        log(`el flujo del servidor de lenguaje se cortó (${e.message}): vuelvo en 3 s`);
        await new Promise((ok) => setTimeout(ok, 3000));
      }
    }
  }

  async unaVuelta() {
    const r = await fetch(`${this.p.servidor}/puestos/${this.p.id}/lsp/agente`, {
      headers: { accept: "text/event-stream", "x-ore-puesto": this.p.id, ...(await this.testigo.cabeceras()) },
      signal: AbortSignal.timeout(300_000),
    });
    if (!r.ok) throw new Error(`el flujo contestó ${r.status}`);
    const dec = new TextDecoder();
    let resto = "", evento = null;
    for await (const trozo of r.body) {
      resto += dec.decode(trozo, { stream: true });
      for (let i; (i = resto.indexOf("\n")) >= 0; ) {
        const linea = resto.slice(0, i).replace(/\r$/, "");
        resto = resto.slice(i + 1);
        if (linea.startsWith("event: ")) evento = linea.slice(7);
        else if (linea.startsWith("data: ")) {
          if (evento === "lsp") this.escribir(linea.slice(6));
          else if (evento === "fin") return;
        }
      }
    }
  }

  parar() {
    this.vivo = false;
    if (this.reloj) clearInterval(this.reloj);
    if (this.proceso) this.proceso.kill();
  }
}
