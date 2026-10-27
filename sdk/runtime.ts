// Bootstraps one command: `deno run [permissions] runtime.ts <entry.ts>`.
//
// Speaks JSON-RPC 2.0 with the launcher, one message per line on stdin/stdout
// (see docs/PROTOCOL.md). Turns the declarative values built with mod.ts into
// wire views and keeps the callbacks they reference.

import { type Json, KIND, type Rpc, setRpc } from "./internal.ts";
import type { Action, Command, Context, Effect, Item, ListSpec, Outcome, View } from "./mod.ts";

// stdout belongs to the protocol: everything printed goes to the log instead.
for (const level of ["log", "info", "debug", "warn"] as const) {
  console[level] = console.error;
}

const encoder = new TextEncoder();

function write(message: Record<string, unknown>) {
  const bytes = encoder.encode(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
  let offset = 0;
  while (offset < bytes.length) offset += Deno.stdout.writeSync(bytes.subarray(offset));
}

// --- Requests to the launcher ------------------------------------------------

let nextId = 1;
const waiting = new Map<number, { resolve: (v: Json) => void; reject: (e: Error) => void }>();

const host: Rpc = {
  request(method, params = null) {
    const id = nextId++;
    write({ id, method, params });
    return new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
  },
  notify(method, params = null) {
    write({ method, params });
  },
};
setRpc(host);

// --- State -------------------------------------------------------------------

interface Session {
  args: Record<string, unknown>;
  preferences: Record<string, unknown>;
  query: string;
}

let session: Session = { args: {}, preferences: {}, query: "" };
const lifetime = new AbortController();
let viewCounter = 0;
let actionCounter = 0;
const views = new Map<string, View>();
const callbacks = new Map<string, { viewId: string; run: (ctx: Context, values?: unknown) => unknown }>();
const inflight = new Map<number, AbortController>();

function context(signal: AbortSignal = lifetime.signal): Context {
  return { ...session, signal };
}

// --- Serialization -----------------------------------------------------------

function serializeActions(actions: Action[] | undefined, viewId: string): Json[] {
  return (actions ?? []).map((a) => {
    const base: Record<string, Json> = { id: `${viewId}:a${++actionCounter}`, title: a.title };
    if (a.icon) base.icon = a.icon;
    if (a.shortcut) base.shortcut = a.shortcut;
    if (a.destructive) base.destructive = true;
    if (a.confirm) base.confirm = a.confirm;
    switch (a[KIND]) {
      case "open":
        return { ...base, kind: "open", target: a.target };
      case "copy":
        return { ...base, kind: "copy", text: a.text };
      case "callback":
        callbacks.set(base.id as string, { viewId, run: a.run });
        return { ...base, kind: "callback" };
    }
  });
}

function serializeItems(items: Item[], viewId: string): Json[] {
  const seen = new Map<string, number>();
  return items.map((item) => {
    let id = item.id ?? item.title;
    const n = seen.get(id) ?? 0;
    seen.set(id, n + 1);
    if (n > 0) id = `${id}#${n}`;
    return {
      id,
      title: item.title,
      subtitle: item.subtitle ?? null,
      icon: item.icon ?? null,
      section: item.section ?? null,
      keywords: item.keywords ?? [],
      accessories: [
        ...(item.accessories ?? []).map((a) => ({ ...a }) as Json),
        ...(item.tags ?? []).map((tag) => ({ tag })),
      ],
      actions: serializeActions(item.actions, viewId),
    };
  });
}

function isAsyncIterable(v: unknown): v is AsyncIterable<Item[]> {
  return typeof v === "object" && v !== null && Symbol.asyncIterator in v;
}

function listJson(spec: ListSpec, id: string, items: Item[], loading: boolean): Json {
  return {
    type: "list",
    id,
    title: spec.title ?? null,
    placeholder: spec.placeholder ?? null,
    filtering: spec.search ? "extension" : "host",
    loading,
    items: serializeItems(items, id),
    empty: spec.empty ? { ...spec.empty } as Json : null,
    actions: serializeActions(spec.actions, id),
    layout: spec.layout ?? "list",
    columns: spec.columns ?? null,
  };
}

/** Renders a view to its wire form. Async generators keep updating it afterwards. */
async function render(view: View, signal: AbortSignal = lifetime.signal): Promise<Json> {
  const id = `v${++viewCounter}`;
  views.set(id, view);
  const ctx = context(signal);
  switch (view[KIND]) {
    case "list": {
      if (view.search) {
        return listJson(view, id, await view.search(session.query, ctx), false);
      }
      const source = typeof view.items === "function" ? view.items(ctx) : view.items ?? [];
      if (isAsyncIterable(source)) {
        stream(view, id, source);
        return listJson(view, id, [], true);
      }
      return listJson(view, id, await source, false);
    }
    case "detail": {
      const markdown = typeof view.markdown === "function" ? await view.markdown(ctx) : view.markdown;
      return {
        type: "detail",
        id,
        title: view.title ?? null,
        markdown,
        metadata: Object.entries(view.metadata ?? {}).map(([label, value]) => ({ label, value: String(value) })),
        actions: serializeActions(view.actions, id),
      };
    }
    case "form": {
      callbacks.set(`${id}:submit`, {
        viewId: id,
        run: (ctx, values) => view.submit((values ?? {}) as Record<string, string | boolean>, ctx),
      });
      return {
        type: "form",
        id,
        title: view.title ?? null,
        submitTitle: view.submitTitle ?? null,
        fields: view.fields.map((f) => ({ ...f }) as Json),
      };
    }
  }
}

async function stream(spec: ListSpec, id: string, source: AsyncIterable<Item[]>) {
  let last: Item[] = [];
  try {
    for await (const items of source) {
      last = items;
      host.notify("view/update", { view: listJson(spec, id, items, true) });
    }
  } catch (err) {
    console.error(err);
    host.request("toast", { title: "Error", message: String(err), style: "failure" }).catch(() => {});
  }
  host.notify("view/update", { view: listJson(spec, id, last, false) });
}

function isView(v: unknown): v is View {
  const kind = (v as { [KIND]?: string } | null)?.[KIND];
  return kind === "list" || kind === "detail" || kind === "form";
}

async function toEffect(outcome: Outcome, viewId: string): Promise<Json> {
  if (outcome === undefined || outcome === null) return { type: "none" };
  if (isView(outcome)) return { type: "push", view: await render(outcome) };
  const effect = outcome as Effect;
  switch (effect.type) {
    case "close":
      return { type: "close", message: effect.message ?? null };
    case "pop":
      return { type: "pop" };
    case "replace":
      return { type: "replace", view: await render(effect.view) };
    case "refresh": {
      const view = views.get(viewId);
      return view ? { type: "replace", view: await render(view) } : { type: "none" };
    }
  }
  throw new Error(`Unexpected action result: ${JSON.stringify(outcome)}`);
}

// --- Requests from the launcher ----------------------------------------------

const entry = Deno.args[0];
if (!entry) throw new Error("usage: runtime.ts <command entry>");

async function loadCommand(): Promise<Command> {
  const url = entry.startsWith("file:") ? entry : new URL(`file://${entry}`).href;
  const mod = await import(url);
  if (!mod.default) throw new Error(`${entry} has no default export`);
  return mod.default as Command;
}

const handlers: Record<string, (params: any, signal: AbortSignal) => Promise<Json>> = {
  async start(params): Promise<Json> {
    session = {
      args: params.arguments ?? {},
      preferences: params.preferences ?? {},
      query: params.query ?? "",
    };
    const command = await loadCommand();
    if (typeof command === "function") {
      return { type: "push", view: await render(await command(context())) };
    }
    if (command[KIND] === "run") {
      const message = await command.run(context());
      return { type: "close", message: message ?? null };
    }
    return { type: "push", view: await render(command) };
  },

  async search({ view: viewId, query }, signal) {
    const view = views.get(viewId);
    if (!view || view[KIND] !== "list" || !view.search) throw new Error(`view ${viewId} is not searchable`);
    session.query = query;
    const items = await view.search(query, context(signal));
    return listJson(view, viewId, items, false);
  },

  async action({ action, values }, signal) {
    const cb = callbacks.get(action);
    if (!cb) throw new Error(`unknown action ${action}`);
    return await toEffect((await cb.run(context(signal), values)) as Outcome, cb.viewId);
  },
};

async function handle(
  msg: { id?: number | string; method?: string; params?: any; result?: Json; error?: { message: string } },
) {
  // Response to one of our requests.
  if (msg.method === undefined) {
    const w = waiting.get(msg.id as number);
    if (!w) return;
    waiting.delete(msg.id as number);
    if (msg.error) w.reject(new Error(msg.error.message));
    else w.resolve(msg.result ?? null);
    return;
  }
  if (msg.method === "$/cancel") {
    inflight.get(msg.params?.id)?.abort();
    return;
  }
  const handler = handlers[msg.method];
  if (msg.id === undefined) return;
  if (!handler) {
    write({ id: msg.id, error: { code: -32601, message: `unknown method ${msg.method}` } });
    return;
  }
  const controller = new AbortController();
  inflight.set(msg.id as number, controller);
  try {
    write({ id: msg.id, result: await handler(msg.params ?? {}, controller.signal) });
  } catch (err) {
    const cancelled = controller.signal.aborted;
    if (!cancelled) console.error(err);
    write({
      id: msg.id,
      error: { code: cancelled ? -32800 : -32000, message: err instanceof Error ? err.message : String(err) },
    });
  } finally {
    inflight.delete(msg.id as number);
  }
}

let buffer = "";
for await (const chunk of Deno.stdin.readable.pipeThrough(new TextDecoderStream())) {
  buffer += chunk;
  let newline;
  while ((newline = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, newline).trim();
    buffer = buffer.slice(newline + 1);
    if (!line) continue;
    try {
      // Not awaited: a slow search must not block a cancellation behind it.
      handle(JSON.parse(line));
    } catch (err) {
      console.error("invalid message from launcher:", err);
    }
  }
}
lifetime.abort();
Deno.exit(0);
