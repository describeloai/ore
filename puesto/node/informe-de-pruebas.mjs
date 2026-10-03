// EL INFORME DE LAS PRUEBAS (0050 L5): el `--test-reporter` de `node --test`
// en el puesto. Convierte los eventos del corredor de Node en una línea JSON por
// cosa —una prueba, un fichero que no cargó, lo que un fichero imprimió—, que
// la correa (`correa.mjs`, `ore/probar`) lee y devuelve a la consola.
//
// Medido con Node 22 antes de escribirlo (`node:test`, aislamiento por proceso):
// - cada fichero llega TAMBIÉN como una prueba que lo envuelve (nombre = su
//   ruta, línea 1). Si falla por `subtestsFailed`, ya lo dicen sus pruebas; si
//   falla por otra cosa —un fichero que no carga—, ESE es el error del fichero;
// - los eventos `start` llegan antes que el `pass`/`fail` de sus hijas, pero
//   los `complete` no siguen ningún orden: se lee el `start` para saber en qué
//   `describe` está cada prueba y el `pass`/`fail` para su resultado;
// - lo que una prueba imprime llega por fichero (`test:stdout`), no por prueba;
// - en un fallo de `assert`, `details.error.cause` trae `expected` y `actual`.
import { relative, sep } from "node:path";
import { inspect } from "node:util";

const CWD = process.cwd();
const deArbol = (f) => (f ? relative(CWD, f).split(sep).join("/") : undefined);
const comoTexto = (v) => (typeof v === "string" ? v : inspect(v, { depth: 6, breakLength: 100, compact: 3 }));
/** La traza, sin el corredor de Node por medio. */
const traza = (s) =>
  String(s ?? "")
    .split("\n")
    .filter((l) => !/\(node:|node:internal|\bnode:test\b/.test(l))
    .join("\n")
    .trim();

export default async function* informe(fuente) {
  // Por fichero, en qué `describe` está cada nivel.
  const pilas = new Map();
  for await (const { type, data: d = {} } of fuente) {
    const fichero = deArbol(d.file);
    if (type === "test:start") {
      const p = pilas.get(fichero) ?? [];
      p.length = d.nesting ?? 0;
      p.push(d.name);
      pilas.set(fichero, p);
    } else if (type === "test:pass" || type === "test:fail") {
      const err = d.details?.error;
      // La prueba que envuelve un fichero: sólo cuenta si el fichero mismo falló.
      if (d.nesting === 0 && fichero && deArbol(d.name) === fichero) {
        if (type === "test:fail" && err?.failureType !== "subtestsFailed") {
          const c = err?.cause;
          yield JSON.stringify({ e: "fichero", fichero, estado: "error", mensaje: String((typeof c === "object" ? c?.message : c) ?? err?.message ?? "no se pudo cargar"), traza: traza(typeof c === "object" ? c?.stack : "") }) + "\n";
        }
        continue;
      }
      if (d.details?.type === "suite" && (type === "test:pass" || err?.failureType === "subtestsFailed")) continue;
      const pila = pilas.get(fichero) ?? [];
      // La causa es el error de la prueba; a veces sólo un texto (una cancelada).
      const causa = err?.cause && typeof err.cause === "object" ? err.cause : { message: err?.cause };
      const fallo = type === "test:fail";
      yield JSON.stringify({
        e: "prueba",
        fichero,
        ruta: pila.slice(0, d.nesting ?? 0),
        nombre: d.name,
        estado: d.skip ? "saltada" : d.todo ? "pendiente" : !fallo ? "ok" : err?.failureType === "cancelledByParent" || err?.failureType === "testTimeoutFailure" ? "cancelada" : "fallo",
        ms: Math.round((d.details?.duration_ms ?? 0) * 10) / 10,
        linea: d.line,
        columna: d.column,
        ...(fallo
          ? {
              mensaje: String(causa?.message ?? err?.message ?? "falló").trim(),
              ...("expected" in causa ? { esperado: comoTexto(causa.expected), obtenido: comoTexto(causa.actual) } : {}),
              traza: traza(causa?.stack),
            }
          : {}),
      }) + "\n";
    } else if (type === "test:stdout" || type === "test:stderr") {
      yield JSON.stringify({ e: "salida", fichero, texto: String(d.message ?? "") }) + "\n";
    }
  }
}
