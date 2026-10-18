// ORE 0068 C1·2 · Que los tipos geo del SDK de Node compilan y se usan como en una Function.
import type { Geometry, Geography } from "../../../puesto/node/ore/index.d.ts";
export default function zona(g: Geometry<"Point", "EPSG:25830">, h: Geography<"Point">, a: Geometry): Geometry<"Polygon Z", "EPSG:25830"> {
  const w: Uint8Array = g;
  return w.length > 0 ? h : a;
}
