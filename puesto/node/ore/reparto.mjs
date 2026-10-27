// Elastic Sessions N1 (docs/decisions/0073-elastic-sessions.md §2.3, §8) · EL REPARTO, AL TAMAÑO
// DE AHORA: lo mismo que el SDK de Python (`_pod_mb`, `_tropo_mb`, `_ajustar`), en Node.
//
// Lo que mide el pod AHORA (su cgroup: cambia cuando la sesión crece) se reparte así:
//   DuckDB   60 %   (45 % si la sentencia lleva una operación reservada: §3 del ADR)
//   V8       30 %   el heap del kernel (`--max-old-space-size`), fijado al arrancarlo
//   el resto 10 %   Node, sus hilos, lo nativo que no es DuckDB
//
// Medido en N0 (2026-10-25): sin tope, DuckDB toma el 80 % del cgroup y V8 2.240 MB en un pod de
// 4 GiB: juntos pasan del pod. Con esto, una consulta grande derrama antes de llegar.
import { readFileSync } from "node:fs";

export const DUCKDB_POR_CIENTO = 60;
export const DUCKDB_RESERVADAS_POR_CIENTO = 45;
export const HEAP_POR_CIENTO = 30;

/** Las reservadas y peligrosas de §3, por su nombre seguido de `(`: como `_RESERVADAS` del SDK de Python. */
export const RESERVADAS = /\b(?:list|array_agg|string_agg|json_group_array|median|quantile(?:_cont|_disc)?|mode|histogram|approx_quantile)\s*\(/i;

/** Un fichero del cgroup del pod (cgroup v2), o `null`. */
export function delCgroup(nombre) {
  try { return readFileSync(`/sys/fs/cgroup/${nombre}`, "utf8").trim(); } catch { return null; }
}

/** Lo que mide el pod AHORA, en MB: el `memory.max` de su cgroup. Sin tope en el cgroup,
 *  `ORE_MEMORIA_MB`, que la plantilla iguala a `limits.memory` al nacer. 0 si nadie lo dice. */
export function podMb(leer = delCgroup) {
  const v = leer("memory.max");
  if (v && v !== "max" && /^\d+$/.test(v)) return Math.floor(Number(v) / 2 ** 20);
  return Number(process.env.ORE_MEMORIA_MB) || 0;
}

/** Los núcleos del pod (`cpu.max`: cuota / periodo, hacia arriba), o 0 si no tiene tope. */
export function podCpus(leer = delCgroup) {
  const v = leer("cpu.max");
  if (!v) return 0;
  const [cuota, periodo] = v.split(/\s+/);
  if (cuota === "max" || !Number(periodo)) return 0;
  return Math.max(1, Math.ceil(Number(cuota) / Number(periodo)));
}

/** Los MB que le tocan a DuckDB: el 60 % del pod, o el 45 % con una reservada. Sin cifra del
 *  pod —las pruebas, un portátil—, un suelo conservador: un tope equivocado da un error
 *  legible; ninguno da un proceso muerto. */
export function tropoMb(reservada = false, leer = delCgroup) {
  const pod = podMb(leer);
  const pct = reservada ? DUCKDB_RESERVADAS_POR_CIENTO : DUCKDB_POR_CIENTO;
  return pod > 0 ? Math.max(256, Math.floor((pod * pct) / 100)) : 512;
}

/** El heap de V8 para un kernel que nace ahora (`--max-old-space-size`, MB), o `null` sin
 *  cifra del pod (que V8 decida). */
export function heapMb(leer = delCgroup) {
  const pod = podMb(leer);
  return pod > 0 ? Math.max(128, Math.floor((pod * HEAP_POR_CIENTO) / 100)) : null;
}

/** N2 · El motor de la sesión, cuando el SDK lo abre (`duckdb()`): el vigilante abre sobre la misma
 *  instancia una SEGUNDA conexión para mirarlo y cambiar su `memory_limit` con una consulta en
 *  marcha (N0: contesta en 1 ms; el tope es de la instancia, vale para todas sus conexiones). */
export const motor = { instancia: null, conexion: null };

/** Lo que se le dijo al motor la última vez: sólo se vuelve a decir si cambia. */
export const aplicado = { mb: null, hilos: null };

/** **El reparto, al tamaño de ahora**: el `memory_limit` y los hilos de DuckDB según lo que mide
 *  el pod y lo que lleva la sentencia. Antes de cada consulta. Devuelve el `memory_limit` (MB). */
export async function ajustar(con, texto = "", leer = delCgroup) {
  const mb = tropoMb(RESERVADAS.test(texto || ""), leer);
  if (mb !== aplicado.mb) {
    await con.run(`set memory_limit = '${mb}MB'`);
    aplicado.mb = mb;
  }
  if (!process.env.ORE_HILOS) {
    const hilos = podCpus(leer);
    if (hilos && hilos !== aplicado.hilos) {
      await con.run(`set threads to ${hilos}`);
      aplicado.hilos = hilos;
    }
  }
  return mb;
}
