// `ore/contract` · a function's contract in Node (ORE 0050 R3 T2). See `contract.mjs`.

/** A value that is not of the type the signature declares. */
export class ContractError extends TypeError {
  /** `input` or `output`: who did not keep the contract. */
  side?: "input" | "output";
  /** The parameter, when the input is at fault. */
  parameter?: string;
}

/** A parameter or field of a derived signature (`/funciones/firma`, or a `Function` document). */
export interface Field {
  name: string;
  /** The canonical OOS type, or its form (`BigInt` for an `Integer` declared `bigint`). */
  type: string;
  required?: boolean;
}

/** A derived signature: what the contract checks. */
export interface Signature {
  input?: Field[];
  output: { type: string } | { fields: Field[] };
  /** The function works on the rows of a view: its first parameter is the row. */
  over?: string | boolean;
}

export function parseType(text: string): unknown;
export function convert(what: string, value: unknown, type: string): unknown;
export function checkOutput(what: string, value: unknown, output: Signature["output"]): unknown;
export function call<R = unknown>(
  fn: (...args: any[]) => R | Promise<R>,
  signature: Signature,
  args?: Record<string, unknown>,
  options?: { row?: unknown; name?: string },
): Promise<unknown>;
export function toWire(value: unknown): unknown;
