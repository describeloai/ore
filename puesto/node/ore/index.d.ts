// `ore` · the types of the session SDK for TypeScript (ORE 0031 W3.4, 0050 R3).
//
// The types a function's signature uses (OOS v1alpha23 `01` §6). They are
// ALIASES of what TypeScript already has —`Integer` is a `number`, a decimal
// and the `Local…` are `string`— so writing `const d: LocalDate = "2026-10-03"`
// needs no conversion. The signature is derived from the file by their NAME,
// without running it, and the contract checks the values at the boundary.
//
//   import type { Integer, Decimal, Money, LocalDate } from "ore";
//
//   export const config = { reads: ["sales.orders"], timeout: "30s" };
//
//   /** The total of an order. */
//   export default async function quoteOrder(orderId: string, discount: Decimal<5, 2> = "0"): Promise<Money<"EUR", 2>> { … }

// ── the types of a signature ─────────────────────────────────────────────────

/**
 * `Integer`: a `number` the contract requires to be an exact integer
 * (`Number.isSafeInteger`, ±2^53−1). For the full 64 bits, declare `bigint`.
 */
export type Integer = number;

/**
 * `Decimal` or `Decimal<p, s>`: exact, as the string of its digits (`"41.31"`),
 * never a `number`. At most `p` digits, `s` of them after the point
 * (`1 ≤ p ≤ 38`, `0 ≤ s ≤ p`). The arithmetic is the library you choose.
 */
export type Decimal<P extends number = number, S extends number = number> = string;

/** `Money<"EUR", 2>`: a decimal with its currency, as a string with at most `S` decimals. */
export type Money<U extends string, S extends number> = string;

/** `Quantity<"km", 1>`: a decimal with its unit, as a string with at most `S` decimals. */
export type Quantity<U extends string, S extends number> = string;

/** `Date` of OOS: a calendar date without a zone, `"2026-10-03"`. A JS `Date` is an instant (`DateTimeTz`). */
export type LocalDate = string;

/** `Time` of OOS: a time of day without a zone, `"08:30:15"`. */
export type LocalTime = string;

/** `DateTime` of OOS: a calendar date and time without a zone, `"2026-10-03T08:30:00"`. */
export type LocalDateTime = string;

/** A reference to an item of a collection (OOS `Media<c>`): where it is and which version, not its bytes. */
export interface MediaRef<C extends string = string> {
  uri?: string;
  collection: C;
  path: string;
  version?: string | null;
  digest?: string | null;
  size?: number | null;
  content_type?: string | null;
  content_type_detected?: string | null;
  checksum?: string | null;
  annotations?: Record<string, unknown> | null;
  modified?: string | null;
  state?: string | null;
}

/** `Media<"db.schema.collection">`: a reference to an item of that collection. */
export type Media<C extends string> = MediaRef<C>;

/** What `export const config = { … }` may say, all of it literal (OOS v1alpha23 `01` §4). */
export interface Config {
  /** The view whose rows the function works on; its first parameter is the row. */
  over?: string;
  /** The views it may read. */
  reads?: readonly string[];
  /** The models it may call. */
  models?: readonly string[];
  /** How long an invocation may take: `"30s"`, `"2m"`. */
  timeout?: string;
}

// ── the contract ─────────────────────────────────────────────────────────────

export { ContractError } from "./contract";
export * as contract from "./contract";

// ── the session SDK (loosely typed: its values are DuckDB's) ─────────────────

export interface Rows<T = Record<string, unknown>> extends Array<T> {
  types: Record<string, string>;
  total: number | null;
  truncated: boolean;
}

export interface Columns {
  names: string[];
  types: string[];
  columns: unknown[][];
  total?: number | null;
  truncated?: boolean;
}

export interface ReadOptions {
  limit?: number;
  strict?: boolean;
  as?: "rows" | "columns";
}

export const API: number;
export const LIMIT: number;
export const EXTENSIONS: string;
export function over(view: string, options?: ReadOptions): Promise<Rows>;
export function sql(text: string, options?: ReadOptions): Promise<Rows>;
export function write(name: string, data: unknown, options?: Record<string, unknown>): Promise<unknown>;
export function declare(document: string | Record<string, unknown>): Promise<unknown>;
export function transform<R>(declared: { inputs: string[]; output: string }, fn: () => R | Promise<R>): Promise<R>;
export function person(): string;
export const session: Record<string, unknown>;
export function arrowName(type: string): string;
export function table(value: unknown, limit?: number): unknown;
export function toJson(value: unknown): unknown;
export function mediaUrl(collection: string, fingerprint: string, options?: Record<string, unknown>): Promise<unknown>;
export function mediaUrls(collection: string, fingerprints: string[], options?: Record<string, unknown>): Promise<unknown>;
export function mediaColumns(view: string): Promise<unknown>;

declare const ore: Record<string, unknown>;
export default ore;
