/**
 * Backslash extension API.
 *
 * A command is a module whose default export describes what to show:
 *
 * ```ts
 * import { list, open, copy } from "@backslash/api";
 *
 * export default list({
 *   items: async () => [
 *     { title: "Backslash", actions: [open("https://github.com/backslash-app")] },
 *   ],
 * });
 * ```
 *
 * Views are plain data. The launcher draws them natively, filters them,
 * handles the keyboard, and only calls back into your code when it must.
 *
 * @module
 */

import { type Json, KIND, rpc } from "./internal.ts";

// ---------------------------------------------------------------------------
// Context

/** Everything a command receives when it starts or handles an action. */
export interface Context<Args = Record<string, unknown>, Prefs = Record<string, unknown>> {
  /** Values of the command's `[[commands.arguments]]`, already validated. */
  args: Args;
  /** Values of the extension's `[[preferences]]`, defaults applied. */
  preferences: Prefs;
  /** Text typed in the search bar when the command was opened. */
  query: string;
  /** Aborted when the user types again, goes back or closes the window. */
  signal: AbortSignal;
}

type Awaitable<T> = T | Promise<T>;

// ---------------------------------------------------------------------------
// Views

/** Icon name from the user's icon theme (`folder`, `web-browser`), an absolute path, or one emoji. */
export type Icon = string;

export type Color = "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "gray";

export interface Accessory {
  text?: string;
  /** Rendered as a pill. */
  tag?: string;
  icon?: Icon;
  color?: Color;
}

export interface Item {
  /** Stable id; defaults to the title. Keeps the selection when the list refreshes. */
  id?: string;
  title: string;
  subtitle?: string;
  icon?: Icon;
  /** Items sharing a section are grouped under a header. */
  section?: string;
  /** Extra words that match when searching. */
  keywords?: string[];
  /** Shorthand for `accessories: [{ tag }]`. */
  tags?: string[];
  accessories?: Accessory[];
  /** The first action runs on Enter, the second on Ctrl+Enter, all are in Ctrl+K. */
  actions?: Action[];
}

export type ItemSource =
  | Item[]
  | AsyncIterable<Item[]>
  | ((ctx: Context) => Awaitable<Item[]> | AsyncIterable<Item[]>);

export interface ListOptions {
  title?: string;
  placeholder?: string;
  /**
   * Loaded once, then filtered by the launcher as the user types (fast, fuzzy).
   * An async generator can yield several times to show results progressively.
   */
  items?: ItemSource;
  /**
   * Called on every keystroke (debounced, previous calls aborted) when the
   * results depend on the query, e.g. a web search. Use either this or `items`.
   */
  search?: (query: string, ctx: Context) => Awaitable<Item[]>;
  empty?: { title: string; description?: string; icon?: Icon };
  /** Actions available even with no item selected. */
  actions?: Action[];
  /** `"grid"` shows tiles with a big icon and the title below (emoji, images). */
  layout?: "list" | "grid";
  /** Tiles per line in a grid (default 6). */
  columns?: number;
}

export interface DetailOptions {
  title?: string;
  /**
   * Markdown body, or a function computing it. A function may also be an
   * async generator yielding the whole text so far, to stream an answer.
   */
  markdown: string | ((ctx: Context) => Awaitable<string> | AsyncIterable<string>);
  /** Key/value pairs shown in a side panel. */
  metadata?: Record<string, string | number>;
  actions?: Action[];
}

export type FormValues = Record<string, string | boolean>;

export interface FormOptions {
  title?: string;
  fields: Field[];
  submitTitle?: string;
  submit: (values: FormValues, ctx: Context) => Awaitable<Outcome>;
}

export interface ListSpec extends ListOptions {
  readonly [KIND]: "list";
}
export interface DetailSpec extends DetailOptions {
  readonly [KIND]: "detail";
}
export interface FormSpec extends FormOptions {
  readonly [KIND]: "form";
}
export type View = ListSpec | DetailSpec | FormSpec;

/** A searchable list of items. */
export function list(options: ListOptions): ListSpec {
  if (options.items && options.search) {
    throw new Error("list(): use either `items` or `search`, not both");
  }
  return { ...options, [KIND]: "list" };
}

/** A markdown document. */
export function detail(options: DetailOptions | string): DetailSpec {
  return typeof options === "string" ? { markdown: options, [KIND]: "detail" } : { ...options, [KIND]: "detail" };
}

/** A form; `submit` receives the values keyed by field id. */
export function form(options: FormOptions): FormSpec {
  return { ...options, [KIND]: "form" };
}

// ---------------------------------------------------------------------------
// Commands without a view

export interface RunSpec {
  readonly [KIND]: "run";
  run: (ctx: Context) => Awaitable<string | void>;
}

/**
 * A command that does something and closes the launcher. Return a string to
 * flash it as a confirmation; throw to show the error.
 */
export function run<Args = Record<string, unknown>>(fn: (ctx: Context<Args>) => Awaitable<string | void>): RunSpec {
  return { [KIND]: "run", run: fn as RunSpec["run"] };
}

/** What a command module may export by default. */
export type Command = View | RunSpec | ((ctx: Context) => Awaitable<View>);

// ---------------------------------------------------------------------------
// Actions

interface ActionOptions {
  icon?: Icon;
  /** GTK accelerator, e.g. `<Ctrl>o`, `<Ctrl><Shift>c`. */
  shortcut?: string;
  /** Rendered in red. */
  destructive?: boolean;
  /** Ask "<confirm>" before running. */
  confirm?: string;
}

export type Action =
  & ActionOptions
  & { title: string }
  & (
    | { readonly [KIND]: "open"; target: string }
    | { readonly [KIND]: "copy"; text: string }
    | { readonly [KIND]: "callback"; run: (ctx: Context) => Awaitable<Outcome> }
  );

/** Opens a URL, file or folder with the default application. */
export function open(target: string, title = "Open", options: ActionOptions = {}): Action {
  return { icon: "web-browser", ...options, title, target, [KIND]: "open" };
}

/** Copies text to the clipboard and closes the launcher. */
export function copy(text: string, title = "Copy to Clipboard", options: ActionOptions = {}): Action {
  return { icon: "edit-copy", ...options, title, text, [KIND]: "copy" };
}

/**
 * Runs your code. Return a view to navigate to it, or an effect
 * (`close()`, `pop()`, `refresh()`, `replace(view)`).
 */
export function action(
  title: string,
  run: (ctx: Context) => Awaitable<Outcome>,
  options: ActionOptions = {},
): Action {
  return { ...options, title, run, [KIND]: "callback" };
}

// ---------------------------------------------------------------------------
// Effects

export type Effect =
  | { readonly [KIND]: "effect"; type: "close"; message?: string }
  | { readonly [KIND]: "effect"; type: "pop" }
  | { readonly [KIND]: "effect"; type: "refresh" }
  | { readonly [KIND]: "effect"; type: "replace"; view: View };

/** What an action may return. Nothing means "stay here". */
export type Outcome = void | undefined | View | Effect;

/** Hide the launcher, optionally flashing a short message. */
export const close = (message?: string): Effect => ({ [KIND]: "effect", type: "close", message });
/** Go back to the previous view. */
export const pop = (): Effect => ({ [KIND]: "effect", type: "pop" });
/** Reload the current view (re-runs `items` or `search`). */
export const refresh = (): Effect => ({ [KIND]: "effect", type: "refresh" });
/** Swap the current view for another one. */
export const replace = (view: View): Effect => ({ [KIND]: "effect", type: "replace", view });

// ---------------------------------------------------------------------------
// Form fields

interface FieldOptions {
  placeholder?: string;
  required?: boolean;
  value?: string | boolean;
}

export type Field =
  & FieldOptions
  & { id: string; title: string }
  & (
    | { kind: "text" | "password" | "textArea" | "checkbox" }
    | { kind: "dropdown"; options: { value: string; title: string }[] }
  );

export const field = {
  text: (id: string, title: string, o: FieldOptions = {}): Field => ({ ...o, id, title, kind: "text" }),
  password: (id: string, title: string, o: FieldOptions = {}): Field => ({ ...o, id, title, kind: "password" }),
  textArea: (id: string, title: string, o: FieldOptions = {}): Field => ({ ...o, id, title, kind: "textArea" }),
  checkbox: (id: string, title: string, o: FieldOptions = {}): Field => ({ ...o, id, title, kind: "checkbox" }),
  dropdown: (
    id: string,
    title: string,
    options: (string | { value: string; title: string })[],
    o: FieldOptions = {},
  ): Field => ({
    ...o,
    id,
    title,
    kind: "dropdown",
    options: options.map((opt) => typeof opt === "string" ? { value: opt, title: opt } : opt),
  }),
};

// ---------------------------------------------------------------------------
// Launcher services

export const clipboard = {
  write: async (text: string): Promise<void> => {
    await rpc().request("clipboard/write", { text });
  },
  /** Requires `clipboard-read = true` in `[permissions]`. */
  read: async (): Promise<string> => String(await rpc().request("clipboard/read") ?? ""),
};

/** Opens a URL, file or folder with the default application. */
export async function openExternal(target: string): Promise<void> {
  await rpc().request("open", { target });
}

/** Shows a small message at the bottom of the launcher. */
export async function toast(
  title: string,
  options: { style?: "info" | "success" | "failure"; message?: string } = {},
): Promise<void> {
  await rpc().request("toast", { title, style: options.style ?? "info", message: options.message ?? null });
}

/** Persistent key/value storage private to your extension. No permission needed. */
export const storage = {
  get: async <T extends Json = Json>(key: string): Promise<T | undefined> =>
    (await rpc().request("storage/get", { key }) ?? undefined) as T | undefined,
  set: async (key: string, value: Json): Promise<void> => {
    await rpc().request("storage/set", { key, value });
  },
  delete: async (key: string): Promise<void> => {
    await rpc().request("storage/set", { key, value: null });
  },
  keys: async (): Promise<string[]> => (await rpc().request("storage/keys")) as string[],
};

/**
 * Runs a program and returns its trimmed stdout. Throws with stderr on a
 * non-zero exit. The program must be listed in `[permissions] run`.
 */
export async function exec(
  program: string,
  args: string[] = [],
  options: { signal?: AbortSignal; stdin?: string } = {},
): Promise<string> {
  const child = new Deno.Command(program, {
    args,
    stdin: options.stdin === undefined ? "null" : "piped",
    stdout: "piped",
    stderr: "piped",
    signal: options.signal,
  }).spawn();
  if (options.stdin !== undefined) {
    const writer = child.stdin.getWriter();
    await writer.write(new TextEncoder().encode(options.stdin));
    await writer.close();
  }
  const { code, stdout, stderr } = await child.output();
  const decode = (b: Uint8Array) => new TextDecoder().decode(b).trim();
  if (code !== 0) {
    throw new Error(`${program} exited with ${code}: ${decode(stderr) || decode(stdout)}`);
  }
  return decode(stdout);
}
