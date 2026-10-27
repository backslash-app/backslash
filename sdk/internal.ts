// Shared state between the public API (mod.ts) and the runtime (runtime.ts).
// Not part of the public API: extensions must not import this file.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface Rpc {
  request(method: string, params?: Json): Promise<Json>;
  notify(method: string, params?: Json): void;
}

let current: Rpc | undefined;

export function setRpc(rpc: Rpc) {
  current = rpc;
}

export function rpc(): Rpc {
  if (!current) {
    throw new Error("@backslash/api can only be used inside a running Backslash command");
  }
  return current;
}

/** Brand used to recognize values built by the SDK helpers. */
export const KIND = Symbol.for("backslash.kind");
