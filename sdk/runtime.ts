// Bootstraps one command: `deno run [permissions] runtime.ts <entry.ts>`.
//
// Speaks JSON-RPC 2.0 with the launcher, one message per line on stdin/stdout
// (see docs/PROTOCOL.md). Turns the declarative values built with mod.ts into
// wire views and keeps the callbacks they reference.

import { type Json, KIND, type Rpc, setRpc } from "./internal.ts";
import type {
  Action,
  Command,
  Context,
  DetailSpec,
  Effect,
  Icon,
  Item,
  ListSpec,
  Metadata,
  Outcome,
  Stat,
  View,
} from "./mod.ts";

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
  command: string;
  args: Record<string, unknown>;
  preferences: Record<string, unknown>;
  query: string;
}

let session: Session = { command: "", args: {}, preferences: {}, query: "" };
const lifetime = new AbortController();
let viewCounter = 0;
let actionCounter = 0;
const views = new Map<string, View>();
const callbacks = new Map<string, { viewId: string; run: (ctx: Context, values?: unknown) => unknown }>();
const inflight = new Map<number, AbortController>();

/** The chip picked in each filtered list, by view id. */
const filters = new Map<string, string>();

function context(signal: AbortSignal = lifetime.signal, filter?: string): Context {
  const { args, preferences, query } = session;
  return { args, preferences, query, signal, filter };
}

// --- Serialization -----------------------------------------------------------

/** Lazy submenus, by action id. */
const submenus = new Map<string, { viewId: string; load: (ctx: Context) => unknown }>();

function serializeActions(actions: Action[] | undefined, viewId: string): Json[] {
  return (actions ?? []).map((a) => {
    const base: Record<string, Json> = { id: `${viewId}:a${++actionCounter}`, title: a.title };
    if (a.icon) base.icon = a.icon;
    if (a.shortcut) base.shortcut = a.shortcut;
    if (a.destructive) base.destructive = true;
    if (a.confirm) base.confirm = a.confirm;
    if (a.section) base.section = a.section;
    switch (a[KIND]) {
      case "submenu":
        if (typeof a.actions === "function") {
          submenus.set(base.id as string, { viewId, load: a.actions });
          return { ...base, kind: "submenu", actions: [], lazy: true };
        }
        return { ...base, kind: "submenu", actions: serializeActions(a.actions, viewId) };
      case "open":
        return { ...base, kind: "open", target: a.target };
      case "copy":
        return { ...base, kind: "copy", text: a.text };
      case "paste":
        return { ...base, kind: "paste", text: a.text };
      case "callback":
        callbacks.set(base.id as string, { viewId, run: a.run });
        return { ...base, kind: "callback" };
    }
  });
}

/** `icon` and `iconTint` from a plain or tinted icon. */
function iconJson(icon: Icon | undefined): { icon: Json; tint: Json } {
  if (icon === undefined || icon === null) return { icon: null, tint: null };
  if (typeof icon === "string") return { icon, tint: null };
  return { icon: icon.source, tint: icon.tint ?? null };
}

function metadataJson(metadata: Record<string, string | number> | Metadata[] | undefined): Json[] {
  if (!metadata) return [];
  if (!Array.isArray(metadata)) {
    return Object.entries(metadata).map(([label, value]) => ({ label, value: String(value) }));
  }
  return metadata.map((m) => {
    const { icon, tint } = iconJson(m.icon);
    return {
      label: m.label,
      value: m.value ?? "",
      icon,
      color: m.color ?? (tint as string | null) ?? null,
      tags: (m.tags ?? []).map((t) => ({ text: t.text, color: t.color ?? null })),
      link: m.link ?? null,
      avatar: m.avatar ?? null,
      progress: m.progress ?? null,
      separator: m.separator ?? false,
    };
  });
}

function statsJson(stats: Stat[] | undefined): Json[] {
  return (stats ?? []).map((s) => ({ label: s.label, value: String(s.value), color: s.color ?? null }));
}

function serializeItems(items: Item[], viewId: string): Json[] {
  const seen = new Map<string, number>();
  return items.map((item) => {
    let id = item.id ?? item.title;
    const n = seen.get(id) ?? 0;
    seen.set(id, n + 1);
    if (n > 0) id = `${id}#${n}`;
    const { icon, tint } = iconJson(item.icon);
    return {
      id,
      title: item.title,
      subtitle: item.subtitle ?? null,
      icon,
      iconTint: tint,
      section: item.section ?? null,
      keywords: item.keywords ?? [],
      accessories: [
        ...(item.accessories ?? []).map((a) => {
          const { icon, tint } = iconJson(a.icon);
          return { ...a, icon, color: a.color ?? tint } as Json;
        }),
        ...(item.tags ?? []).map((tag) => ({ tag })),
      ],
      actions: serializeActions(item.actions, viewId),
      detail: item.detail
        ? {
          image: item.detail.image ?? null,
          stats: statsJson(item.detail.stats),
          markdown: item.detail.markdown ?? null,
          metadata: metadataJson(item.detail.metadata),
        }
        : null,
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
    filter: spec.filter
      ? {
        options: spec.filter.options.map((o) => typeof o === "string" ? { value: o, title: o } : o),
        value: filters.get(id) ?? "",
      }
      : null,
    status: typeof spec.status === "function" ? spec.status(items) : spec.status ?? null,
  };
}

const filterKey = (spec: ListSpec) => `$filter:${session.command}:${spec.title ?? spec.placeholder ?? ""}`;

/** The filter value of a new list view: remembered, else the default, else the first option. */
async function initialFilter(spec: ListSpec): Promise<string | undefined> {
  if (!spec.filter) return undefined;
  const values = spec.filter.options.map((o) => typeof o === "string" ? o : o.value);
  const saved = await host.request("storage/get", { key: filterKey(spec) }).catch(() => null);
  if (typeof saved === "string" && values.includes(saved)) return saved;
  return spec.filter.default ?? values[0];
}

/** Renders a view to its wire form. Async generators keep updating it afterwards. */
/** Aborted when a view is replaced (refresh, replace), to stop its generators. */
const viewAborts = new Map<string, AbortController>();

function retire(viewId: string) {
  viewAborts.get(viewId)?.abort();
  viewAborts.delete(viewId);
  views.delete(viewId);
  filters.delete(viewId);
}

async function render(view: View, signal: AbortSignal = lifetime.signal, filter?: string): Promise<Json> {
  const id = `v${++viewCounter}`;
  views.set(id, view);
  const own = new AbortController();
  viewAborts.set(id, own);
  if (view[KIND] === "list" && view.filter) {
    filters.set(id, filter ?? await initialFilter(view) ?? "");
  }
  const ctx = context(AbortSignal.any([signal, own.signal]), filters.get(id));
  switch (view[KIND]) {
    case "list": {
      if (view.search) {
        return listJson(view, id, await view.search(session.query, ctx), false);
      }
      const source = typeof view.items === "function" ? view.items(ctx) : view.items ?? [];
      if (isAsyncIterable(source)) {
        stream(view, id, source, ctx.signal);
        return listJson(view, id, [], true);
      }
      return listJson(view, id, await source, false);
    }
    case "detail": {
      const markdown = typeof view.markdown === "function" ? view.markdown(ctx) : view.markdown;
      if (typeof markdown === "object" && markdown !== null && Symbol.asyncIterator in markdown) {
        streamDetail(view, id, markdown as AsyncIterable<string>);
        return detailJson(view, id, "", true);
      }
      return detailJson(view, id, await markdown, false);
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

function detailJson(spec: DetailSpec, id: string, markdown: string, loading: boolean): Json {
  return {
    type: "detail",
    id,
    title: spec.title ?? null,
    markdown,
    metadata: metadataJson(spec.metadata),
    stats: statsJson(spec.stats),
    status: spec.status ?? null,
    actions: serializeActions(spec.actions, id),
    loading,
  };
}

/** Sends the text as it grows, at most every 80 ms so the launcher keeps up. */
async function streamDetail(spec: DetailSpec, id: string, source: AsyncIterable<string>) {
  let text = "";
  let sent = 0;
  try {
    for await (const chunk of source) {
      text = chunk;
      if (Date.now() - sent > 80) {
        sent = Date.now();
        host.notify("view/update", { view: detailJson(spec, id, text, true) });
      }
    }
  } catch (err) {
    console.error(err);
    text += `${text ? "\n\n" : ""}**Error:** ${err instanceof Error ? err.message : String(err)}`;
  }
  host.notify("view/update", { view: detailJson(spec, id, text, false) });
}

/**
 * Sends each batch of items. The spinner shows until the first one: later
 * yields are refreshes (live views may never end). Stops when the view is
 * replaced.
 */
async function stream(spec: ListSpec, id: string, source: AsyncIterable<Item[]>, signal: AbortSignal) {
  let last: Item[] = [];
  try {
    for await (const items of source) {
      if (signal.aborted) break;
      last = items;
      host.notify("view/update", { view: listJson(spec, id, items, false) });
    }
  } catch (err) {
    if (signal.aborted) return;
    console.error(err);
    host.request("toast", { title: "Error", message: String(err), style: "failure" }).catch(() => {});
  }
  if (!signal.aborted) host.notify("view/update", { view: listJson(spec, id, last, false) });
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
      retire(viewId);
      return { type: "replace", view: await render(effect.view) };
    case "refresh": {
      const view = views.get(viewId);
      if (!view) return { type: "none" };
      const filter = filters.get(viewId);
      retire(viewId);
      return { type: "replace", view: await render(view, lifetime.signal, filter) };
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
      command: params.command ?? "",
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
    const items = await view.search(query, context(signal, filters.get(viewId)));
    return listJson(view, viewId, items, false);
  },

  /** Another chip was picked: the list again, as a new view. */
  async filter({ view: viewId, value }) {
    const view = views.get(viewId);
    if (!view || view[KIND] !== "list") throw new Error(`view ${viewId} has no filter`);
    host.request("storage/set", { key: filterKey(view), value }).catch(() => {});
    retire(viewId);
    return await render(view, lifetime.signal, String(value));
  },

  async submenu({ action }, signal) {
    const menu = submenus.get(action);
    if (!menu) throw new Error(`unknown submenu ${action}`);
    const actions = (await menu.load(context(signal))) as Action[];
    return serializeActions(actions, menu.viewId);
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
