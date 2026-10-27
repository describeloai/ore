// EL KERNEL DEL PUESTO DE NODE: un contexto para toda la sesión, en SU PROCESO.
//
// Elastic Sessions N1 (docs/products/0073-elastic-sessions.md §2.2, §8): como en Python, el
// agente no ejecuta celdas. Lanza el kernel como proceso hijo (`child_process.fork`, con IPC) y
// le pasa cada celda; el agente se queda con el bucle, el latido, la correa y el cierre. Si el
// kernel se pasa de memoria muere él, no la sesión: la celda contesta `OutOfMemoryError`, se abre
// un kernel nuevo y la siguiente celda corre en la misma sesión. Nada se reintenta solo.
//
// Dos maneras de quedarse sin memoria, y las dos se dicen:
//   - el pod entero (el cgroup): el sistema mata el proceso que se pasa (`oom_kill` en
//     `memory.events`, con `singleProcessOOMKill` en el session pool);
//   - el heap de V8 (`--max-old-space-size`, el 30 % del pod al nacer el kernel: `reparto.mjs`):
//     V8 aborta con «Reached heap limit».
//
// Medido antes de escribirlo (`medida-w3-ts-jvm.py`, en victor sobre node 24): Node quita los
// tipos de TypeScript por sí mismo (`node:module` stripTypeScriptTypes, 74 ms la primera vez, <2 ms
// después) y el evaluador del REPL (`repl.start().eval`) da lo que tiene el kernel Python: un
// contexto que dura toda la sesión, `await` arriba (`const x = await …` queda en el contexto), y el
// valor de la última expresión. Una celda corre en 0,2–11 ms.
//
// Una celda-MÓDULO (con `import`/`export` arriba: un fichero `.ts` del árbol, por ejemplo) no cabe
// en el REPL: se escribe en /trabajo/celdas y se importa tal cual (Node 24 lee `.ts`; 4 ms
// medidos); sus exports pasan al contexto, así la siguiente celda puede llamar a la función.
import { stripTypeScriptTypes } from "node:module";
import repl from "node:repl";
import { PassThrough } from "node:stream";
import { inspect } from "node:util";
import { fork } from "node:child_process";
import { mkdirSync, writeFileSync, symlinkSync, existsSync, readdirSync } from "node:fs";
import { pathToFileURL, fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import * as ore from "./ore/index.mjs";
import { delCgroup, heapMb, podMb } from "./ore/reparto.mjs";
import { Vigilante, soltarElMotor } from "./vigilante.mjs";

const FILAS_MAXIMAS = 200;
const AQUI = dirname(fileURLToPath(import.meta.url));

export const log = (...a) => process.stdout.write("agente · " + a.join(" ") + "\n");
const ENLACE = process.platform === "win32" ? "junction" : "dir";

/** Enlaza en `a` cada paquete de `desde` que `a` no tenga ya (los `@ámbito/…`,
 *  uno a uno dentro de su ámbito). Devuelve cuántos enlazó. */
function enlazar(desde, a) {
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
const ms = (t0) => Math.round(performance.now() - t0);

// ── El kernel: un contexto para toda la sesión ─────────────────────────────
export class Kernel {
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
    Object.assign(this.repl.context, { ore, over: ore.over, sql: ore.sql, explain: ore.explain, OriginReadError: ore.OriginReadError, write: ore.write, declare: ore.declare, transform: ore.transform, person: ore.person, persona: ore.persona });
    // Las celdas-módulo viven en /trabajo/celdas y resuelven `ore` por este enlace.
    this.celdas = null;
    try {
      const base = process.env.ORE_CELDAS ?? "/trabajo";
      mkdirSync(join(base, "celdas"), { recursive: true });
      // (`junction` en Windows: un enlace de directorio sin privilegios; en el pod, un symlink.)
      // ⭐ 0050 R3 T5b: con capa —lo que el repositorio declara en `package.json`,
      //   en `/capa/node_modules`—, `node_modules` es un directorio de enlaces: lo
      //   de la imagen PRIMERO (manda el contenedor, como en la JVM: el SDK está
      //   hecho contra él) y lo de la capa detrás, sin pisar nada. Sin capa, un
      //   enlace a lo de la imagen, como siempre.
      const nm = join(base, "node_modules");
      const capa = process.env.ORE_CAPA_NODE ?? "/capa/node_modules";
      if (!existsSync(nm)) {
        if (!existsSync(capa)) symlinkSync(join(AQUI, "node_modules"), nm, ENLACE);
        else {
          mkdirSync(nm);
          enlazar(join(AQUI, "node_modules"), nm);
          log(`capa · ${enlazar(capa, nm)} paquete(s) de /capa, detrás de los de la imagen`);
        }
      }
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

  /** `previsto`: lo que el sobre de la celda dice que necesitará (`previstoMb`, `previstoMotivo`). */
  async correr(texto, lenguaje, previsto = {}) {
    const t0 = performance.now();
    this.capturado = "";
    // N2 · el vigilante: antes, lo previsto; mientras corre, crecer si hace falta.
    const vigilante = await new Vigilante(lenguaje === "sql" ? texto : "", previsto).empezar();
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
      vigilante.parar();
    }
  }

  salidaDe(valor, texto, t0) {
    const tabla = comoTabla(valor);
    if (tabla) return { ...tabla, tipo: "tabla", texto, ms: ms(t0) };
    if (valor === undefined) return texto.trim() ? { tipo: "texto", texto, ms: ms(t0) } : { tipo: "vacia", ms: ms(t0) };
    return { tipo: "texto", texto: texto + (typeof valor === "string" ? valor : inspect(valor, { depth: 3, maxArrayLength: 50 })), ms: ms(t0) };
  }
}

/** La salida `tabla` del contrato: la hace el SDK (`ore.table`), que es lo que
 *  una celda también puede pedir; aquí sólo se le pone el límite de la consola. */
export function comoTabla(valor) { return ore.table(valor, FILAS_MAXIMAS); }

// ── El kernel en su proceso, visto desde el agente ─────────────────────────

/** Las veces que el sistema mató un proceso del pod por memoria (`memory.events` del cgroup),
 *  o `null` sin cgroup (un portátil). */
export function oomKills(leer = delCgroup) {
  const m = /^oom_kill\s+(\d+)$/m.exec(leer("memory.events") ?? "");
  return m ? Number(m[1]) : null;
}

/** **El kernel en su proceso**: `correr` como el de `Kernel`, por IPC. Si el proceso muere a mitad
 *  de una celda, la celda vuelve con un error que dice por qué (y si fue por memoria, cuánta
 *  había) y se abre otro kernel para la siguiente. */
export class KernelAparte {
  constructor() { this.arrancar(); }

  arrancar() {
    const heap = Number(process.env.ORE_HEAP_MB) || heapMb();
    this.heap = heap;
    this.proc = fork(fileURLToPath(import.meta.url), ["--kernel"], {
      // Sin el aviso de `stripTypeScriptTypes` (experimental en Node 24): sale en la primera celda
      //   de cada kernel y no es suyo (medido: se colaba en su salida).
      execArgv: ["--disable-warning=ExperimentalWarning", ...(heap ? [`--max-old-space-size=${heap}`] : [])],
      serialization: "advanced",
      stdio: ["ignore", "inherit", "pipe", "ipc"],
    });
    // Lo que el kernel dice por stderr, a la vista, y su cola para saber por qué murió.
    this.cola = "";
    this.proc.stderr.on("data", (d) => { process.stderr.write(d); this.cola = (this.cola + d).slice(-4000); });
    log(`kernel · en marcha (pid ${this.proc.pid}${heap ? `, heap ${heap} MB` : ""})`);
  }

  /** Corre la celda en el kernel. `persona` y `cabeceras` son las de la sesión: el kernel las
   *  necesita para hablar con `ore-serve` (el token lo renueva el agente). */
  correr(texto, lenguaje = "typescript", { persona = "", cabeceras = {}, previstoMb = 0, previstoMotivo = null } = {}) {
    if (!this.proc.connected) this.arrancar();
    const t0 = performance.now();
    const antes = oomKills();
    const proc = this.proc;
    return new Promise((ok) => {
      const fin = () => { proc.off("message", alMensaje); proc.off("exit", alSalir); };
      const alMensaje = (m) => { if (m && m.r) { fin(); ok(m.r); } };
      const alSalir = (codigo, senal) => {
        fin();
        ok(this.porQueMurio(codigo, senal, antes, t0));
        this.arrancar();
      };
      proc.on("message", alMensaje);
      proc.on("exit", alSalir);
      proc.send({ que: "celda", texto, lenguaje, persona, cabeceras, previstoMb, previstoMotivo });
    });
  }

  /** Que el motor suelte lo que tiene, antes de bajar el pod: `{motor_mb}`. */
  bajar() {
    if (!this.proc.connected) return Promise.resolve({ motor_mb: 0 });
    const proc = this.proc;
    return new Promise((ok) => {
      const fin = () => { proc.off("message", alMensaje); proc.off("exit", alSalir); };
      const alMensaje = (m) => { if (m && m.bajado) { fin(); ok(m.bajado); } };
      const alSalir = () => { fin(); ok({ motor_mb: 0 }); this.arrancar(); };
      proc.on("message", alMensaje);
      proc.on("exit", alSalir);
      proc.send({ que: "bajar" });
    });
  }

  porQueMurio(codigo, senal, antes, t0) {
    const despues = oomKills();
    const sistema = antes !== null && despues !== null ? despues > antes : senal === "SIGKILL";
    const heap = /heap limit|JavaScript heap out of memory/i.test(this.cola);
    const restart = "The session is still open, but its variables are gone: the kernel restarted";
    log(`kernel · murió (código ${codigo}, señal ${senal}${sistema ? ", sin memoria del pod" : heap ? ", sin heap" : ""}): abro otro`);
    if (sistema) {
      const gib = podMb() / 1024;
      return { tipo: "error", nombre: "OutOfMemoryError", ms: ms(t0),
        mensaje: `this cell ran out of memory${gib ? `: the session's ${gib.toFixed(0)} GiB were not enough` : ""} and the system stopped it. ${restart}` };
    }
    if (heap) {
      return { tipo: "error", nombre: "OutOfMemoryError", ms: ms(t0),
        mensaje: `this cell ran out of JavaScript memory: the kernel's heap${this.heap ? ` (${this.heap} MB)` : ""} was not enough. Keep less in JavaScript (let sql() do the work) or let the session grow. ${restart}` };
    }
    return { tipo: "error", nombre: "KernelDied", ms: ms(t0),
      mensaje: `the kernel stopped (exit ${codigo ?? senal}) while running this cell. ${restart}` };
  }

  cerrar() {
    if (this.proc.connected) this.proc.disconnect();
    this.proc.kill();
  }
}

// ── Bajar a la base ────────────────────────────────────────────────────────

/** Tras cuánto sin celdas una sesión que creció vuelve a su base (s). */
export const BAJAR_TRAS = Number(process.env.ORE_BAJAR_TRAS ?? "600");

/** El tamaño con que nace la sesión (`ORE_MEMORIA_MB`, el de la plantilla). */
const baseMb = () => Number(process.env.ORE_MEMORIA_MB) || 0;

/** N2 · Si la sesión creció y lleva `BAJAR_TRAS` segundos sin celdas: el motor suelta primero, y
 *  luego se pide la base. Devuelve lo que pasó, o `null` si no tocaba. La sesión cuesta lo que pesa
 *  y sólo mientras lo pesa. ⚠️ El heap de V8 del kernel es el de cuando nació: si nació grande,
 *  sigue así hasta el siguiente kernel. */
export async function bajarSiToca(kernel, puesto, desde, medirPod = () => podMb()) {
  const base = baseMb();
  if (!base || Date.now() - desde < BAJAR_TRAS * 1000 || medirPod() <= base) return null;
  const suelto = await kernel.bajar();
  let codigo, r;
  try {
    [codigo, r] = await puesto.pedir("POST", `/puestos/${puesto.id}/scale`, { memoria_mb: base, motivo: `idle ${BAJAR_TRAS} s` }, 60_000);
  } catch (e) {
    log(`no pude volver a la base: ${e.message ?? e}`);
    return { motor: suelto, error: String(e) };
  }
  log(`sin celdas ${BAJAR_TRAS} s: el motor suelta (${JSON.stringify(suelto)}) y el pod vuelve a ${base} MB (${codigo} ${r?.estado ?? ""})`);
  return { motor: suelto, codigo, respuesta: r };
}

// ── El proceso del kernel ──────────────────────────────────────────────────
if (process.argv.includes("--kernel") && process.send) {
  const kernel = new Kernel();
  process.on("message", async (m) => {
    if (m?.que === "bajar") {
      let bajado;
      try { bajado = await soltarElMotor(); } catch (e) { bajado = { error: String(e?.message ?? e) }; }
      process.send({ bajado });
      return;
    }
    if (m?.que !== "celda") return;
    if (m.persona) ore.session.persona = m.persona;
    ore.session._cabeceras = m.cabeceras ?? {};
    let r;
    try {
      r = await kernel.correr(m.texto, m.lenguaje, { previstoMb: m.previstoMb, previstoMotivo: m.previstoMotivo });
    } catch (e) {
      r = { tipo: "error", nombre: e?.name ?? "Error", mensaje: String(e?.message ?? e), ms: 0 };
    }
    process.send({ r });
  });
  // Sin el agente, el kernel no tiene a quién contestar.
  process.on("disconnect", () => process.exit(0));
}
