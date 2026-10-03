// `ore/contract` · a function's contract in Node (ORE 0050 R3 T2, OOS v1alpha23 `01` §7).
//
//   import { contract } from "ore";
//   const total = await contract.call(quoteOrder, signature, { orderId: "P-7", discount: "5.00" });
//
// Each parameter arrives as the type its signature declares, and what the
// function returns is checked against it; a value that does not fit throws
// `ContractError` (a `TypeError`) saying which parameter, or the output.
//
// ---
//
// El contrato de una función de TypeScript: el de Python (`ore.contrato`), con
// una diferencia que viene del lenguaje. En Python el contrato lee las
// anotaciones del `def` al ejecutarse; en Node los tipos se BORRAN antes de
// ejecutar (Node 24, `--erasableSyntaxOnly`), así que no hay nada que leer en
// tiempo de ejecución. El contrato lee la FIRMA DERIVADA —la que `ore-code`
// saca del fichero sin ejecutarlo, la misma del documento `Function`—, y la
// trae quien llama: el arnés que invoca (T5) o la celda de Dry Run (T4).
//
// La firma viaja con los tipos en su forma canónica de OOS (`Decimal<12, 2>`,
// `Struct<id: Integer, …>`, `list<…>`), con una sola extensión: `BigInt`, un
// `Integer` que el código declaró `bigint`. El documento dice `Integer` en los
// dos casos; la frontera tiene que entregar un `number` exacto o un `bigint`
// según lo que el código escribió (`Tipo::forma` en `ore-code`).
//
// Convertir es de lo que viaja en JSON a lo que el código declaró, y nada más:
// `"41.31"` y `41.31` son un `Decimal<5, 2>` (llega como la cadena `"41.31"`,
// nunca como un `number`), `"2026-10-03"` es un `LocalDate`, pero `"3"` no es
// un `number`. Lo que ya es del tipo pasa tal cual. Sin dependencias.

/** A value that is not of the type the signature declares. */
export class ContractError extends TypeError {
  /**
   * @param {string} message
   * @param {{ side?: "input" | "output", parameter?: string }} [where]
   */
  constructor(message, where = {}) {
    super(message);
    this.name = "ContractError";
    /** `input` or `output`: who did not keep the contract. */
    this.side = where.side;
    /** The parameter, when the input is at fault. */
    this.parameter = where.parameter;
  }
}

// ── the canonical type, parsed ────────────────────────────────────────────────

/**
 * A type of the signature → a tree: `{ k: "Decimal", p, s }`, `{ k: "list", of }`,
 * `{ k: "Struct", fields: [[name, type]] }`, `{ k: "Money", unit, s }`, …
 * @param {string} text
 */
export function parseType(text) {
  let i = 0;
  const s = String(text);
  const ws = () => { while (s[i] === " ") i++; };
  const token = () => {
    ws();
    const j = i;
    while (i < s.length && !"<>,: ".includes(s[i])) i++;
    if (i === j) throw new Error(`a type expected at ${j} in \`${s}\``);
    return s.slice(j, i);
  };
  const expect = (c) => {
    ws();
    if (s[i] !== c) throw new Error(`\`${c}\` expected at ${i} in \`${s}\``);
    i++;
  };
  const num = () => {
    const t = token();
    if (!/^\d+$/.test(t)) throw new Error(`a number expected, got \`${t}\` in \`${s}\``);
    return Number(t);
  };
  const type = () => {
    const name = token();
    ws();
    if (s[i] !== "<") {
      if (["Struct", "list", "Money", "Quantity", "Media"].includes(name)) {
        throw new Error(`\`${name}\` without its arguments in \`${s}\``);
      }
      return { k: name };
    }
    i++;
    let r;
    if (name === "list") {
      r = { k: "list", of: type() };
    } else if (name === "Struct") {
      const fields = [];
      for (;;) {
        const n = token();
        expect(":");
        fields.push([n, type()]);
        ws();
        if (s[i] === ",") { i++; continue; }
        break;
      }
      r = { k: "Struct", fields };
    } else if (name === "Decimal") {
      const p = num();
      expect(",");
      r = { k: "Decimal", p, s: num() };
    } else if (name === "Money" || name === "Quantity") {
      const unit = token();
      expect(",");
      r = { k: name, unit, s: num() };
    } else if (name === "Media") {
      r = { k: "Media", collection: token() };
    } else {
      throw new Error(`\`${name}<…>\` is not a type of a signature`);
    }
    expect(">");
    return r;
  };
  const t = type();
  ws();
  if (i !== s.length) throw new Error(`\`${s.slice(i)}\` left over in \`${s}\``);
  return t;
}

const CACHE = new Map();
function tree(type) {
  if (typeof type !== "string") return type;
  let t = CACHE.get(type);
  if (!t) {
    t = parseType(type);
    CACHE.set(type, t);
  }
  return t;
}

/** The type as written, for messages. */
function shown(t) {
  switch (t.k) {
    case "list": return `list<${shown(t.of)}>`;
    case "Struct": return `Struct<${t.fields.map(([n, x]) => `${n}: ${shown(x)}`).join(", ")}>`;
    case "Decimal": return t.p === undefined ? "Decimal" : `Decimal<${t.p}, ${t.s}>`;
    case "Money": case "Quantity": return `${t.k}<${t.unit}, ${t.s}>`;
    case "Media": return `Media<${t.collection}>`;
    case "BigInt": return "Integer (bigint)";
    default: return t.k;
  }
}

function brief(v) {
  let r;
  try {
    r = typeof v === "bigint" ? `${v}n` : typeof v === "string" ? JSON.stringify(v) : v instanceof Date ? v.toISOString() : v instanceof Uint8Array ? `<${v.length} bytes>` : JSON.stringify(v);
  } catch {
    r = String(v);
  }
  if (r === undefined) r = String(v);
  return r.length <= 60 ? r : r.slice(0, 57) + "...";
}

// ── convert ──────────────────────────────────────────────────────────────────

const DECIMAL = /^[+-]?\d+(\.\d+)?$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;
const TIME = /^\d{2}:\d{2}(:\d{2}(\.\d{1,9})?)?$/;
const DATETIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d{1,9})?)?$/;
const INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d{1,9})?)?(Z|[+-]\d{2}:\d{2})$/i;
const BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;

/** (integer digits, decimals) of a decimal written as text. */
function digits(text) {
  const [ent, dec = ""] = text.replace(/^[+-]/, "").split(".");
  const e = ent.replace(/^0+/, "");
  return [e.length, dec.replace(/0+$/, "").length];
}

function validDate(y, m, d) {
  const x = new Date(Date.UTC(y, m - 1, d));
  return x.getUTCFullYear() === y && x.getUTCMonth() === m - 1 && x.getUTCDate() === d;
}

/**
 * `v` as the type `type` declares for `what` (a parameter, or a field).
 * `null` and `undefined` pass: whether they may is the caller's (`required`).
 * @param {string} what
 * @param {unknown} v
 * @param {string | object} type
 */
export function convert(what, v, type) {
  const t = tree(type);
  if (v === null || v === undefined) return v;
  const bad = (why = "") => new ContractError(`\`${what}\` is \`${shown(t)}\` and got ${brief(v)}${why}`);
  switch (t.k) {
    case "String":
      if (typeof v === "string") return v;
      throw bad();
    case "Boolean":
      if (typeof v === "boolean") return v;
      throw bad();
    case "Float":
      if (typeof v === "number") return v;
      if (typeof v === "bigint" && Number.isSafeInteger(Number(v))) return Number(v);
      throw bad();
    case "Integer": {
      // A `number` that is an integer and exact: beyond 2^53 a `number` has
      // already lost digits, and that is what this type exists to say.
      let n = v;
      if (typeof v === "bigint") n = Number(v);
      else if (typeof v === "string" && /^[+-]?\d+$/.test(v.trim())) n = Number(v.trim());
      if (typeof n !== "number") throw bad();
      if (!Number.isInteger(n)) throw bad(", which is not an integer");
      if (!Number.isSafeInteger(n) || (typeof v !== "number" && BigInt(n) !== BigInt(typeof v === "string" ? v.trim() : v))) {
        throw bad(", which a `number` cannot hold exactly (beyond ±2^53−1): declare it `bigint`");
      }
      return n;
    }
    case "BigInt":
      if (typeof v === "bigint") return v;
      if (typeof v === "number") {
        if (!Number.isInteger(v)) throw bad(", which is not an integer");
        if (!Number.isSafeInteger(v)) throw bad(": a JSON number beyond 2^53 has already lost digits; send it as a string");
        return BigInt(v);
      }
      if (typeof v === "string" && /^[+-]?\d+$/.test(v.trim())) return BigInt(v.trim());
      throw bad();
    case "Decimal":
    case "Money":
    case "Quantity": {
      // Exact: as the string of its digits, never as a `number`.
      let d;
      if (typeof v === "string" && DECIMAL.test(v.trim())) d = v.trim();
      else if (typeof v === "bigint") d = String(v);
      else if (typeof v === "number" && Number.isFinite(v)) {
        d = String(v);
        if (!DECIMAL.test(d)) throw bad(": write it as a string with its digits");
      } else throw bad(typeof v === "string" ? ", which is not a number" : "");
      const [ent, dec] = digits(d);
      if (t.k === "Decimal" && t.p !== undefined && (dec > t.s || ent > t.p - t.s)) {
        throw bad(", which does not fit");
      }
      if (t.k !== "Decimal" && dec > t.s) throw bad(`: it has more than ${t.s} decimals`);
      return d;
    }
    case "Date":
      if (typeof v === "string" && DATE.test(v) && validDate(+v.slice(0, 4), +v.slice(5, 7), +v.slice(8, 10))) return v;
      throw bad(", which is not a YYYY-MM-DD date");
    case "Time":
      if (typeof v === "string" && TIME.test(v) && +v.slice(0, 2) < 24 && +v.slice(3, 5) < 60) return v;
      throw bad(", which is not an ISO 8601 time");
    case "DateTime":
      if (typeof v === "string" && DATETIME.test(v) && validDate(+v.slice(0, 4), +v.slice(5, 7), +v.slice(8, 10))) return v;
      throw bad(", which is not an ISO 8601 date-time without a zone");
    case "DateTimeTz": {
      if (v instanceof Date) {
        if (Number.isNaN(v.getTime())) throw bad(", which is an invalid Date");
        return v;
      }
      if (typeof v === "string") {
        if (!INSTANT.test(v)) throw bad(", without a zone: an instant carries `Z` or `+02:00`");
        const x = new Date(v);
        if (Number.isNaN(x.getTime())) throw bad(", which is not an instant");
        return x;
      }
      throw bad();
    }
    case "Opaque":
      if (v instanceof Uint8Array) return v;
      if (typeof v === "string" && v.length % 4 === 0 && BASE64.test(v)) return new Uint8Array(Buffer.from(v, "base64"));
      throw bad(typeof v === "string" ? ", which is not base64" : "");
    case "list":
      if (!Array.isArray(v)) throw bad(", which is not a list");
      return v.map((x, i) => {
        if (x === null || x === undefined) throw new ContractError(`\`${what}[${i}]\` is missing: the items of a list do not`);
        return convert(`${what}[${i}]`, x, t.of);
      });
    case "Struct": {
      if (typeof v !== "object" || Array.isArray(v) || v instanceof Date || v instanceof Uint8Array) throw bad(", which is not an object");
      const names = new Set(t.fields.map(([n]) => n));
      const extra = Object.keys(v).filter((k) => !names.has(k));
      if (extra.length) throw bad(`: it does not declare ${extra.map((k) => `\`${k}\``).join(", ")}`);
      const out = {};
      for (const [n, ft] of t.fields) {
        if (n in v) out[n] = convert(`${what}.${n}`, v[n], ft);
      }
      return out;
    }
    case "Media": {
      if (typeof v !== "object" || typeof v.collection !== "string" || typeof v.path !== "string") {
        throw bad(", which is not a reference to an item (`collection`, `path`, …)");
      }
      if (v.collection !== t.collection) throw bad(`: an item of \`${v.collection}\``);
      return v;
    }
    default:
      throw new ContractError(`\`${what}\`: \`${shown(t)}\` is not a type of a signature`);
  }
}

// ── the output ───────────────────────────────────────────────────────────────

/** A parameter or field of a signature, as `/funciones/firma` or a document gives it. */
function fieldOf(c) {
  return { name: c.name ?? c.nombre, type: c.form ?? c.forma ?? c.type, required: c.required === true || c.requerido === true };
}

/**
 * What `what` returned, checked against its output: `{ type }`, a value, or
 * `{ fields }` (`campos`), an object. `null` and `undefined` are "no value"
 * (v1alpha23 `01` §7).
 */
export function checkOutput(what, v, output) {
  const fields = output?.fields ?? output?.campos;
  if (!fields) {
    try {
      return convert(what, v, output?.form ?? output?.forma ?? output?.type);
    } catch (e) {
      if (e instanceof ContractError) throw new ContractError(`\`${what}\` returned ${brief(v)} and declares \`${shown(tree(output?.form ?? output?.forma ?? output?.type))}\`: ${e.message}`, { side: "output" });
      throw e;
    }
  }
  if (v === null || v === undefined || typeof v !== "object" || Array.isArray(v)) {
    throw new ContractError(`\`${what}\` returned ${brief(v)} and declares an object`, { side: "output" });
  }
  const fs = fields.map(fieldOf);
  const names = new Set(fs.map((f) => f.name));
  const extra = Object.keys(v).filter((k) => !names.has(k) && v[k] !== undefined);
  if (extra.length) {
    throw new ContractError(`\`${what}\` returned an object with ${extra.map((k) => `\`${k}\``).join(", ")}, which its output does not declare`, { side: "output" });
  }
  const out = {};
  for (const f of fs) {
    const x = v[f.name];
    if (x === null || x === undefined) {
      if (f.required) throw new ContractError(`\`${what}\` returned an object without \`${f.name}\`, which is required`, { side: "output" });
      out[f.name] = null;
      continue;
    }
    try {
      out[f.name] = convert(f.name, x, f.type);
    } catch (e) {
      if (e instanceof ContractError) throw new ContractError(`\`${what}\` returned an object with ${e.message}`, { side: "output" });
      throw e;
    }
  }
  return out;
}

// ── the call ─────────────────────────────────────────────────────────────────

/**
 * Calls `fn` with its contract: `args` by name, converted to what `signature`
 * declares (`{ input: [{ name, type, required }], output, over? }`, the derived
 * signature), the row first if the function works `over` a view; then its
 * output, checked. Unknown or missing required arguments are the input's fault.
 * @param {Function} fn
 * @param {{ input?: object[], output: object, over?: string | boolean }} signature
 * @param {Record<string, unknown>} [args]
 * @param {{ row?: unknown, name?: string }} [o]
 */
export async function call(fn, signature, args = {}, o = {}) {
  const name = o.name ?? fn.name ?? "the function";
  const input = (signature.input ?? []).map(fieldOf);
  const known = new Set(input.map((p) => p.name));
  for (const k of Object.keys(args ?? {})) {
    if (!known.has(k)) {
      throw new ContractError(`\`${name}\` has no parameter \`${k}\`${input.length ? `: it takes ${input.map((p) => `\`${p.name}\``).join(", ")}` : ""}`, { side: "input", parameter: k });
    }
  }
  const positional = [];
  if (signature.over) positional.push(o.row);
  for (const p of input) {
    const v = args?.[p.name];
    if ((v === null || v === undefined) && p.required) {
      throw new ContractError(`\`${p.name}\` is required`, { side: "input", parameter: p.name });
    }
    try {
      positional.push(convert(p.name, v, p.type));
    } catch (e) {
      if (e instanceof ContractError) {
        e.side = "input";
        e.parameter = p.name;
      }
      throw e;
    }
  }
  const r = await fn(...positional);
  return checkOutput(name, r, signature.output);
}

/**
 * A value of a result, as JSON can carry it and the console reads it: a
 * `bigint` as the string of its digits, a `Date` in ISO (UTC), a
 * `Uint8Array` by its size.
 */
export function toWire(v) {
  if (v === null || v === undefined) return null;
  if (typeof v === "bigint") return String(v);
  if (v instanceof Date) return v.toISOString();
  if (v instanceof Uint8Array) return { bytes: v.length };
  if (Array.isArray(v)) return v.map(toWire);
  if (typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, toWire(x)]));
  if (typeof v === "number" && !Number.isFinite(v)) return String(v);
  return v;
}

export default { ContractError, parseType, convert, checkOutput, call, toWire };
