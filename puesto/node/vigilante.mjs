// EL VIGILANTE DEL KERNEL DE NODE — Elastic Sessions N2 (docs/products/0073-elastic-sessions.md §2.3,
// §2.6): el mismo que el de Python (`Vigilante`, `puesto/python/agente.py`), en el kernel.
//
// Mientras corre una celda, mira al motor cada `CADA_MS` desde una SEGUNDA conexión a la misma
// instancia de DuckDB (N0: contesta en ~1 ms con otra consulta en marcha). Si la memoria del motor
// pasa del `UMBRAL` de su `memory_limit`, o si empieza a derramar, pide el siguiente escalón a
// `ore-serve` (`POST /puestos/{id}/scale`, con la credencial de la sesión) y, cuando el pod ya mide
// lo nuevo, sube el `memory_limit` y los hilos desde esa conexión, con la consulta en marcha (2 ms).
//
// Con `previstoMb` (del sobre de la celda: lo que necesitó la otra vez), ANTES de correr pide eso de
// un golpe, con el motivo del sobre (`forecast`). Luego vigila igual.
//
// Si no se puede crecer (`quota`, `infeasible`, `deferred`, el techo) no es un error: la consulta
// sigue derramando y termina, y en esa celda no se vuelve a pedir. Fuera del session pool, nunca más.
//
// ⚠️ Lo que crece es lo nativo (DuckDB): el heap de V8 se fija al nacer el kernel. Y el vigilante es
//   un temporizador del mismo hilo que la celda: mira mientras la celda espera a DuckDB (lo que corre
//   en sus hilos), no durante un bucle de JavaScript que no suelta el hilo.
import * as ore from "./ore/index.mjs";
import { motor, aplicado, tropoMb, podMb as medirElPod, ajustar } from "./ore/reparto.mjs";

export const CADA_MS = 200;
export const UMBRAL = 0.8;
/** Tras crecer, lo que derramaba antes sigue en disco un momento. */
export const CALMA_MS = 3000;
/** El motivo con que se pide lo previsto (`growth[].reason` en executions). */
export const PREVISION = "forecast";

const log = (...a) => process.stdout.write("agente · " + a.join(" ") + "\n");
const dormir = (ms) => new Promise((ok) => setTimeout(ok, ms));
const MEDIR = "select (select coalesce(sum(memory_usage_bytes), 0) from duckdb_memory()) uso, "
  + "(select coalesce(sum(size), 0) from duckdb_temporary_files()) derramado";

export class Vigilante {
  /** `ore-serve` dijo que esta sesión no crece (no está en el session pool). */
  static sinPool = false;

  /** `d`, para las pruebas: `pedir`, `podMb`, `cada`, `esperaCrecer`. */
  constructor(texto = "", { previstoMb = 0, previstoMotivo = null } = {}, d = {}) {
    this.texto = texto;
    this.previstoMb = Number(previstoMb) || 0;
    this.previstoMotivo = previstoMotivo || PREVISION;
    this.pedidos = [];
    this.pedir = d.pedir ?? ((...a) => ore.session.pedir(...a));
    this.podMb = d.podMb ?? (() => medirElPod());
    this.cada = d.cada ?? CADA_MS;
    this.esperaCrecer = d.esperaCrecer ?? 200;
    this.otra = null;
    this.reloj = null;
    this.mirando = false;
    this.ocupado = false;
    this.calmaHasta = 0;
    this.derramado = 0;
  }

  /** Antes de la celda: lo previsto, y a mirar. */
  async empezar() {
    if (Vigilante.sinPool || !ore.session.id) return this;
    if (this.previstoMb > this.podMb()) {
      await this.crecer(this.previstoMotivo, { memoriaMb: this.previstoMb, deUnGolpe: true });
      if (Vigilante.sinPool) return this;
    }
    this.mirando = true;
    this.reloj = setInterval(() => void this.mirar(), this.cada);
    return this;
  }

  /** Una mirada: sin motor abierto todavía (la celda aún no ha llamado a `sql()`), nada. */
  async mirar() {
    if (this.ocupado || !this.mirando || !motor.instancia) return;
    this.ocupado = true;
    try {
      if (!this.otra) this.otra = await motor.instancia.connect();
      const [[uso, derramado]] = (await this.otra.runAndReadAll(MEDIR)).getRows();
      const usoMb = Number(uso) / 2 ** 20;
      const derrama = Number(derramado) > this.derramado;
      this.derramado = Number(derramado);
      if (Date.now() < this.calmaHasta || !this.mirando) return;
      const limite = aplicado.mb ?? tropoMb();
      let motivo = null;
      if (derrama) motivo = "spilling";
      else if (usoMb >= UMBRAL * limite) motivo = `memory at ${Math.floor((100 * usoMb) / Math.max(1, limite))}%`;
      if (!motivo) return;
      if (!(await this.crecer(motivo))) { this.mirando = false; clearInterval(this.reloj); return; }
      this.calmaHasta = Date.now() + CALMA_MS;
    } catch { /* una lectura que falla no para la celda */ } finally { this.ocupado = false; }
  }

  /** Pide el siguiente escalón (o `memoriaMb` de un golpe) y lo aplica. `false` si ya no hay que
   *  pedir más en esta celda. */
  async crecer(motivo, { memoriaMb = null, deUnGolpe = false } = {}) {
    const pod = this.podMb();
    const cuerpo = { memoria_mb: memoriaMb || pod * 2, motivo };
    if (deUnGolpe) cuerpo.de_un_golpe = true;
    let codigo, r;
    try {
      [codigo, r] = await this.pedir("POST", `/puestos/${ore.session.id}/scale`, cuerpo, 60_000);
    } catch (e) {
      log(`vigilante · no pude pedir crecer: ${e.message ?? e}`);
      return false;
    }
    r = r && typeof r === "object" ? r : {};
    this.pedidos.push({ motivo, codigo, estado: r.estado ?? null });
    if (codigo === 409 && String(r.error ?? "").includes("session pool")) { Vigilante.sinPool = true; return false; }
    if (codigo !== 200 || r.estado !== "hecho") {
      log(`vigilante · ${motivo}: no crece (${codigo} ${r.estado ?? JSON.stringify(r)})`);
      return false;
    }
    // ore-redimensionar espera a que el pod diga el tamaño nuevo; por si acaso, un momento.
    for (let i = 0; i < 25 && !(this.podMb() > pod); i++) await dormir(this.esperaCrecer);
    let mb = null;
    if (motor.instancia) {
      if (!this.otra) this.otra = await motor.instancia.connect();
      mb = await ajustar(this.otra, this.texto);
    }
    log(`vigilante · ${motivo}: el pod pasa de ${pod} a ${this.podMb()} MB${mb ? `; el motor, a ${mb} MB` : ""}`);
    return true;
  }

  /** Tras la celda. */
  parar() {
    this.mirando = false;
    clearInterval(this.reloj);
    try { this.otra?.closeSync?.(); } catch { /* ya cerrada */ }
    this.otra = null;
  }
}

/** Antes de bajar el pod: que el motor suelte lo que tiene (S0·c: con `memory_limit` a 256 MB,
 *  DuckDB suelta en < 5 s). Kubernetes no deja bajar un pod por debajo de lo que usa. */
export async function soltarElMotor(esperaMs = 3000) {
  if (!motor.conexion) return { motor_mb: 0 };
  await motor.conexion.run("set memory_limit = '256MB'");
  aplicado.mb = 256;
  await dormir(esperaMs);
  const [[uso]] = (await motor.conexion.runAndReadAll("select coalesce(sum(memory_usage_bytes), 0) from duckdb_memory()")).getRows();
  return { motor_mb: Math.floor(Number(uso) / 2 ** 20) };
}
