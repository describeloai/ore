#!/usr/bin/env node
// EL AGENTE DEL PUESTO — Node (0031 W3.4): lo que corre dentro de la sesión TS/JS.
//
// El mismo agente que `puesto/python/agente.py`, en su lenguaje: pide trabajo a
// `ore-serve` por HTTP (polling largo: el puesto no acepta conexiones, «sin
// entrada»), ejecuta cada celda en un espacio que dura toda la sesión, y
// devuelve la salida TIPADA —tabla, texto, error, vacía— al mismo sitio.
//
//   GET  /puestos/{id}/pendiente            → 200 {celda, lenguaje, texto}
//   POST /puestos/{id}/celdas/{n}/salida    ← {tipo, ms, …}
//
// ── El kernel ──────────────────────────────────────────────────────────────
//
// Medido antes de escribirlo (`medida-w3-ts-jvm.py`, en victor sobre node
// 24): Node quita los tipos de TypeScript por sí mismo (`node:module`
// stripTypeScriptTypes, 74 ms la primera vez, <2 ms después) y el evaluador
// del REPL (`repl.start().eval`) da lo que tiene el kernel Python: un
// contexto que dura toda la sesión, `await` arriba (`const x = await …` queda
// en el contexto), y el valor de la última expresión. Una celda corre en
// 0,2–11 ms.
//
// Una celda-MÓDULO (con `import`/`export` arriba: un fichero `.ts` del árbol,
// por ejemplo) no cabe en el REPL: se escribe en /trabajo/celdas y se importa
// tal cual (Node 24 lee `.ts`; 4 ms medidos); sus exports pasan al contexto,
// así la siguiente celda puede llamar a la función.
//
// Quién es y cuándo muere: como el de Python (client credentials contra el
// IdP dentro del clúster; `ORE_SUJETO` en las pruebas; TTL de inactividad;
// 410 = cerrado).
import { stripTypeScriptTypes } from "node:module";
import repl from "node:repl";
import { PassThrough } from "node:stream";
import { inspect } from "node:util";
import { mkdirSync, writeFileSync, symlinkSync, existsSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ore from "./ore/index.mjs";

const FILAS_MAXIMAS = 200;
const AQUI = dirname(fileURLToPath(import.meta.url));

const log = (...a) => process.stdout.write("agente · " + a.join(" ") + "\n");
const ms = (t0) => Math.round(performance.now() - t0);

// ── Quién soy: el token ────────────────────────────────────────────────────
class Testigo {
  constructor() {
    this.sujeto = process.env.ORE_SUJETO;
    this.direccion = (process.env.DIRECCION ?? "").replace(/\/+$/, "");
    this.realm = process.env.REALM ?? "rubix";
    this.cliente = Testigo.fichero("agente-cliente");
    this.secreto = Testigo.fichero("agente-secreto");
    this.token = null;
    this.caduca = 0;
  }
  static fichero(nombre) {
    try { return readFileSync(join(process.env.PUESTO_DIR ?? "/puesto", nombre), "utf8").trim(); } catch { return null; }
  }
  async cabeceras() {
    if (this.sujeto) return { "x-ore-sujeto": this.sujeto };
    if (!(this.cliente && this.secreto && this.direccion)) {
      log("sin identidad: ni ORE_SUJETO ni /puesto/agente-{cliente,secreto} con DIRECCION");
      process.exit(2);
    }
    if (Date.now() > this.caduca - 60_000) {
      const r = await fetch(`${this.direccion}/realms/${this.realm}/protocol/openid-connect/token`, {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({ grant_type: "client_credentials", client_id: this.cliente, client_secret: this.secreto }),
        signal: AbortSignal.timeout(20_000),
      });
      if (!r.ok) throw new Error(`el emisor contestó ${r.status}`);
      const t = await r.json();
      this.token = t.access_token;
      this.caduca = Date.now() + (t.expires_in ?? 300) * 1000;
      log(`token del agente renovado · caduca en ${t.expires_in ?? 300}s`);
    }
    return { authorization: "Bearer " + this.token };
  }
}

// ── El kernel: un contexto para toda la sesión ─────────────────────────────
class Kernel {
  constructor() {
    this.salida = new PassThrough();
    this.capturado = "";
    this.salida.on("data", (d) => (this.capturado += d));
    this.repl = repl.start({ input: new PassThrough(), output: this.salida, terminal: false, useGlobal: false, prompt: "", ignoreUndefined: true });
    // Un error SÍNCRONO de la celda no llega al callback del eval: va al dominio
    // del REPL (que lo escribiría por output y seguiría). Se le quita ese oído y
    // se le pone el nuestro: la celda pendiente acaba con el error. Medido:
    // sin esto, una celda con `noExiste + 1` no contesta nunca.
    this.pendiente = null;
    this.repl._domain.removeAllListeners("error");
    this.repl._domain.on("error", (e) => { const p = this.pendiente; this.pendiente = null; if (p) p({ e }); });
    Object.assign(this.repl.context, { ore, over: ore.over, sql: ore.sql, write: ore.write, declare: ore.declare, persona: ore.persona });
    // Las celdas-módulo viven en /trabajo/celdas y resuelven `ore` por este enlace.
    this.celdas = null;
    try {
      const base = process.env.ORE_CELDAS ?? "/trabajo";
      mkdirSync(join(base, "celdas"), { recursive: true });
      // (`junction` en Windows: un enlace de directorio sin privilegios; en el pod, un symlink.)
      if (!existsSync(join(base, "node_modules"))) symlinkSync(join(AQUI, "node_modules"), join(base, "node_modules"), process.platform === "win32" ? "junction" : "dir");
      this.celdas = join(base, "celdas");
    } catch (e) { log(`sin sitio para celdas-módulo (${e.message}): una celda con import/export no correrá`); }
    this.n = 0;
  }

  evaluar(codigo) {
    return new Promise((ok) => {
      this.pendiente = ok;
      this.repl.eval(codigo, this.repl.context, "celda", (e, v) => { this.pendiente = null; ok({ e, v }); });
    });
  }

  /** Una celda-módulo: a fichero (ya sin tipos: `stripTypeScriptTypes` deja
   *  las posiciones como estaban, así que un error señala la línea de verdad),
   *  importada, y sus exports al contexto. */
  async importar(texto, ts) {
    if (!this.celdas) throw new Error("este puesto no tiene dónde escribir una celda-módulo");
    const f = join(this.celdas, `celda-${++this.n}-${Date.now()}.mjs`);
    writeFileSync(f, ts ? stripTypeScriptTypes(texto, { mode: "strip" }) : texto);
    const m = await import(pathToFileURL(f).href);
    const exportados = Object.keys(m).filter((k) => k !== "default");
    Object.assign(this.repl.context, Object.fromEntries(exportados.map((k) => [k, m[k]])));
    if ("default" in m) return m.default;
    return exportados.length ? `exports: ${exportados.join(", ")}` : undefined;
  }

  async correr(texto, lenguaje) {
    const t0 = performance.now();
    this.capturado = "";
    const antes = { log: console.log, error: console.error, warn: console.warn, info: console.info };
    const escribe = (...a) => { this.capturado += a.map((x) => (typeof x === "string" ? x : inspect(x))).join(" ") + "\n"; };
    Object.assign(console, { log: escribe, error: escribe, warn: escribe, info: escribe });
    try {
      let valor;
      if (lenguaje === "sql") {
        valor = await ore.sql(texto);
      } else {
        const ts = lenguaje !== "javascript";
        if (/^\s*(import|export)\b/m.test(texto)) {
          valor = await this.importar(texto, ts);
        } else {
          const js = ts ? stripTypeScriptTypes(texto, { mode: "strip" }) : texto;
          const { e, v } = await this.evaluar(js);
          if (e) throw e;
          valor = v;
        }
      }
      return this.salidaDe(valor, this.capturado, t0);
    } catch (e) {
      return { tipo: "error", nombre: e?.name ?? "Error", mensaje: String(e?.message ?? e), traza: String(e?.stack ?? e), texto: this.capturado, ms: ms(t0) };
    } finally {
      Object.assign(console, antes);
    }
  }

  salidaDe(valor, texto, t0) {
    const tabla = comoTabla(valor);
    if (tabla) return { ...tabla, tipo: "tabla", texto, ms: ms(t0) };
    if (valor === undefined) return texto.trim() ? { tipo: "texto", texto, ms: ms(t0) } : { tipo: "vacia", ms: ms(t0) };
    return { tipo: "texto", texto: texto + (typeof valor === "string" ? valor : inspect(valor, { depth: 3, maxArrayLength: 50 })), ms: ms(t0) };
  }
}

/** La salida `tabla` del contrato: la hace el SDK (`ore.tabla`), que es lo que
 *  una celda también puede pedir; aquí sólo se le pone el límite de la consola. */
function comoTabla(valor) { return ore.tabla(valor, FILAS_MAXIMAS); }

/** Un valor suelto (el resultado de una celda que no es tabla) → JSON. */
function llano(v) { return ore.jsonDe(v); }

// ── El bucle ───────────────────────────────────────────────────────────────
async function main() {
  const p = ore.puesto;
  if (!p.id) { log("sin PUESTO en el entorno"); return 2; }
  const ttl = Number(process.env.TTL ?? "1800");
  const testigo = new Testigo();
  const kernel = new Kernel();
  log(`puesto ${p.id} · ore-serve ${p.servidor} · TTL ${ttl}s · almacén ${p.almacen} · node ${process.version}`);
  let ultimo = Date.now();
  const espera = (s) => new Promise((ok) => setTimeout(ok, s * 1000));
  while (true) {
    if (Date.now() - ultimo > ttl * 1000) { log(`sin celdas durante ${ttl}s: cierro`); return 0; }
    let codigo, r;
    try {
      p._cabeceras = await testigo.cabeceras();
      [codigo, r] = await p.pedir("GET", `/puestos/${p.id}/pendiente`, undefined, 40_000);
    } catch (e) { log(`ore-serve no contesta (${e.message}): reintento en 5 s`); await espera(5); continue; }
    if (codigo === 410) { log("el puesto está cerrado: adiós"); return 0; }
    if (codigo !== 200) { log(`pendiente contestó ${codigo}: ${JSON.stringify(r)} · reintento en 5 s`); await espera(5); continue; }
    if (!p.persona) {
      const [c2, ficha] = await p.pedir("GET", `/puestos/${p.id}`);
      if (c2 === 200 && ficha?.persona) { p.persona = ficha.persona; log(`el puesto es de ${p.persona}`); }
    }
    if (!r?.pendiente) continue;
    const { celda: n, texto = "", lenguaje = "typescript" } = r;
    log(`celda ${n} · ${lenguaje} · ${Buffer.byteLength(texto)} bytes`);
    const salida = await kernel.correr(texto, lenguaje);
    ultimo = Date.now();
    try {
      p._cabeceras = await testigo.cabeceras();
      const [c3, r3] = await p.pedir("POST", `/puestos/${p.id}/celdas/${n}/salida`, salida);
      if (c3 !== 200 && c3 !== 201) log(`la salida de la celda ${n} no se aceptó: ${c3} ${JSON.stringify(r3)}`);
    } catch (e) { log(`no pude entregar la salida de la celda ${n}: ${e.message}`); }
    log(`celda ${n} · ${salida.tipo} · ${salida.ms} ms`);
  }
}

// `--comprobar` (la imagen, al construirse): importa, crea el kernel y corre una
// celda. Lo que `node --check` no ve —un import que no resuelve— se ve aquí.
if (process.argv.includes("--comprobar")) {
  const k = new Kernel();
  const r = await k.correr("const xs: number[] = [1, 2]; xs.length * 21", "typescript");
  if (r.tipo !== "texto" || r.texto !== "42") { log(`el kernel no contesta 42: ${JSON.stringify(r)}`); process.exit(1); }
  const m = await import("ore");
  // Y el contrato de tipos por DuckDB (0032 T3): un bigint es 42n, un decimal es exacto, y el JSON de la tabla es el del contrato.
  const f = await m.sql("select 42::bigint n, 1.50::decimal(4,2) d, timestamp '2024-06-01 12:00:00'::timestamptz t");
  const t = comoTabla(f);
  if (f[0].n !== 42n || f.tipos.d !== "decimal128(4, 2)" || t.filas[0][0] !== 42 || t.filas[0][1] !== "1.50" || !/Z$/.test(t.filas[0][2])) { log(`el sdk no cumple el contrato de tipos: ${JSON.stringify(t)}`); process.exit(1); }
  log(`agente y sdk listos · ${process.version} · ${Object.keys(m).join(" ")} · tipos ok`);
  k.repl.close();
  process.exit(0);
}
process.exitCode = await main();
