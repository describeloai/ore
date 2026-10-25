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
// En SU PROCESO (Elastic Sessions N1): `kernel.mjs`. Si se pasa de memoria muere
// él y la sesión sigue; la celda lo dice y la siguiente corre en un kernel nuevo.
//
// Quién es y cuándo muere: como el de Python (client credentials contra el
// IdP dentro del clúster; `ORE_SUJETO` en las pruebas; TTL de inactividad;
// 410 = cerrado).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import * as ore from "./ore/index.mjs";
import { Correa } from "./correa.mjs";
import { Kernel, KernelAparte, comoTabla, log } from "./kernel.mjs";

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
    // ⭐ R1 · El token de ESTE pod, si lo trae: el servidor sabe qué puesto habla.
    //   Se lee cada vez: el kubelet lo renueva en el sitio. Con la credencial del
    //   agente también en el pod, van las dos (un `ore-serve` sin pods, la suya).
    let pod = null;
    try { pod = readFileSync(process.env.ORE_TOKEN_DEL_POD ?? "/var/run/ore/pod/token", "utf8").trim() || null; } catch { /* sin token de pod */ }
    if (pod && !(this.cliente && this.secreto)) return { "x-ore-pod": pod };
    const h = await this.delAgente();
    return pod ? { ...h, "x-ore-pod": pod } : h;
  }
  async delAgente() {
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

// ── El bucle ───────────────────────────────────────────────────────────────
/** La correa del servidor de lenguaje (0050 L3·3, `correa.mjs`): escucha desde
 *  el principio, y el servidor no arranca hasta el primer mensaje del editor. */
let correa = null;

async function main() {
  const p = ore.session;
  if (!p.id) { log("sin PUESTO en el entorno"); return 2; }
  const ttl = Number(process.env.TTL ?? "1800");
  // Un trabajo (W3.7 ④): `TRABAJO=<ruta>@<commit>` → una sola celda (el fichero)
  // y fuera, con el resultado como código de salida.
  const trabajo = (process.env.TRABAJO ?? "").trim();
  if (trabajo) process.env.ORE_CODIGO = trabajo;
  const testigo = new Testigo();
  const kernel = new KernelAparte();
  // La última actividad: una celda, o (0050 sesión automática) un mensaje del
  // editor por la correa —escribir con tipos, `tsc`, las pruebas—. Sin ninguna
  // de las dos durante el TTL, el puesto se va.
  let ultimo = Date.now();
  if (!trabajo) {
    correa = new Correa(p, testigo, { alActividad: () => { ultimo = Date.now(); } });
    void correa.escuchar();
  }
  log(`puesto ${p.id} · ore-serve ${p.servidor} · TTL ${ttl}s · almacén ${p.almacen} · node ${process.version}`);
  const espera = (s) => new Promise((ok) => setTimeout(ok, s * 1000));
  while (true) {
    if (Date.now() - ultimo > ttl * 1000) {
      log(`sin actividad (celdas ni editor) durante ${ttl}s: cierro`);
      // Fuera de la cola: si no, Flux recrea el Job.
      try { p._cabeceras = await testigo.cabeceras(); await p.pedir("POST", `/puestos/${p.id}/cierre`); } catch (e) { log(`no se pudo notificar el cierre: ${e}`); }
      kernel.cerrar();
      return 0;
    }
    let codigo, r;
    try {
      p._cabeceras = await testigo.cabeceras();
      [codigo, r] = await p.pedir("GET", `/puestos/${p.id}/pendiente`, undefined, 40_000);
    } catch (e) { log(`ore-serve no contesta (${e.message}): reintento en 5 s`); await espera(5); continue; }
    if (codigo === 410) { log("el puesto está cerrado: adiós"); kernel.cerrar(); return 0; }
    if (codigo !== 200) { log(`pendiente contestó ${codigo}: ${JSON.stringify(r)} · reintento en 5 s`); await espera(5); continue; }
    if (!p.persona) {
      const [c2, ficha] = await p.pedir("GET", `/puestos/${p.id}`);
      if (c2 === 200 && ficha?.persona) { p.persona = ficha.persona; log(`el puesto es de ${p.persona}`); }
    }
    if (!r?.pendiente) continue;
    // Un plan (Query A1): Node no lo hace todavía; se dice ya, y la admisión decide sin él
    //   (sin esto, el plan esperaría su plazo y correría como una celda).
    if (r.plan != null) {
      try { await p.pedir("POST", `/puestos/${p.id}/planes/${r.plan}`, { error: "a Node session does not plan yet" }); } catch (e) { log(`no pude contestar el plan ${r.plan}: ${e.message}`); }
      continue;
    }
    const { celda: n, texto = "", lenguaje = "typescript" } = r;
    log(`celda ${n} · ${lenguaje} · ${Buffer.byteLength(texto)} bytes`);
    p._cabeceras = await testigo.cabeceras();
    const salida = await kernel.correr(texto, lenguaje, { persona: p.persona, cabeceras: p._cabeceras });
    ultimo = Date.now();
    try {
      p._cabeceras = await testigo.cabeceras();
      const [c3, r3] = await p.pedir("POST", `/puestos/${p.id}/celdas/${n}/salida`, salida);
      if (c3 !== 200 && c3 !== 201) log(`la salida de la celda ${n} no se aceptó: ${c3} ${JSON.stringify(r3)}`);
    } catch (e) { log(`no pude entregar la salida de la celda ${n}: ${e.message}`); }
    log(`celda ${n} · ${salida.tipo} · ${salida.ms} ms`);
    if (trabajo) { log(`trabajo ${trabajo}: ${salida.tipo === "error" ? "error" : "hecho"}`); kernel.cerrar(); return salida.tipo === "error" ? 1 : 0; }
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
  if (f[0].n !== 42n || f.types.d !== "decimal128(4, 2)" || f.tipos !== f.types || t.filas[0][0] !== 42 || t.filas[0][1] !== "1.50" || !/Z$/.test(t.filas[0][2])) { log(`el sdk no cumple el contrato de tipos: ${JSON.stringify(t)}`); process.exit(1); }
  // Y el kernel en su proceso (N1): contesta, y guarda lo de una celda para la siguiente.
  const aparte = new KernelAparte();
  await aparte.correr("const y = 21", "typescript");
  const r2 = await aparte.correr("y * 2", "typescript");
  aparte.cerrar();
  if (r2.tipo !== "texto" || r2.texto !== "42") { log(`el kernel aparte no contesta 42: ${JSON.stringify(r2)}`); process.exit(1); }
  log(`agente y sdk listos · ${process.version} · ${Object.keys(m).join(" ")} · tipos ok · kernel aparte ok`);
  k.repl.close();
  process.exit(0);
}
process.exitCode = await main();
// El flujo del editor y el servidor de lenguaje no son razón para seguir vivo.
correa?.parar();
process.exit(process.exitCode);
