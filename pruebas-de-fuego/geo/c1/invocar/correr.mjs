// ORE 0068 · cierre de C1 · La celda DE VERDAD de una función de TypeScript que devuelve una geometría
// (la escribe `funciones.rs`), importada como el agente: sus filas (`export default`), en JSON.
// La spec (v1alpha32 01 §5): en texto, WKB en hexadecimal.
import { copyFileSync, mkdirSync, symlinkSync } from "node:fs";

mkdirSync("/tmp/n/node_modules", { recursive: true });
symlinkSync("/opt/ore/ore", "/tmp/n/node_modules/ore");
symlinkSync("/opt/ore/node_modules/@duckdb", "/tmp/n/node_modules/@duckdb");
copyFileSync("/celdas/celda.mjs", "/tmp/n/celda.mjs");
const ESPERA = "0101000000" + "0000000000da1a41" + "0000000004115141";
try {
  const { default: filas } = await import("/tmp/n/celda.mjs");
  const v = filas[0].valor;
  console.log(`${v === ESPERA ? "OK   " : "FALLA"} · node · ${JSON.stringify(filas[0])}`);
} catch (e) {
  console.log(`FALLA · node · ${e?.name}: ${String(e?.message).split("\n")[0].slice(0, 200)}`);
}
