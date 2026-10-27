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
  /** The chip picked in the list's `filter`, if it has one. */
  filter?: string;
}

type Awaitable<T> = T | Promise<T>;

// ---------------------------------------------------------------------------
// Views

/**
 * An icon: one of Backslash's own (`bs-…`, see {@link Icons}), a name from the
 * user's icon theme (`folder`), an absolute path, an image URL (its host
 * must be in your `net` permission), or one emoji. `{ source, tint }` colors
 * a symbolic icon.
 */
export type Icon = string | { source: string; tint?: Color };

export type Color = "red" | "orange" | "yellow" | "green" | "blue" | "purple" | "teal" | "pink" | "gray";

export interface Accessory {
  text?: string;
  /** Rendered as a pill. */
  tag?: string;
  icon?: Icon;
  color?: Color;
  /** A person: their name (shown as initials) or a picture URL. */
  avatar?: string;
  /** Shown on hover. */
  tooltip?: string;
}

/** A big number in a small card of a detail panel. */
export interface Stat {
  label: string;
  value: string | number;
  color?: Color;
}

/** One line of a detail panel; build them with {@link meta}. */
export interface Metadata {
  label: string;
  value?: string;
  icon?: Icon;
  color?: Color;
  tags?: { text: string; color?: Color }[];
  link?: string;
  avatar?: string;
  /** From 0 to 1. */
  progress?: number;
  separator?: boolean;
}

/** Builders for detail panel lines. */
export const meta = {
  text: (label: string, value: string | number, o: { icon?: Icon; color?: Color } = {}): Metadata => ({
    label,
    value: String(value),
    ...o,
  }),
  tags: (label: string, tags: (string | { text: string; color?: Color })[]): Metadata => ({
    label,
    tags: tags.map((t) => typeof t === "string" ? { text: t } : t),
  }),
  link: (label: string, text: string, url: string): Metadata => ({ label, value: text, link: url }),
  person: (label: string, name: string, avatar?: string): Metadata => ({ label, value: name, avatar: avatar ?? name }),
  progress: (label: string, ratio: number, text?: string): Metadata => ({
    label,
    progress: Math.max(0, Math.min(1, ratio)),
    value: text ?? `${Math.round(ratio * 100)} %`,
  }),
  separator: (): Metadata => ({ label: "", separator: true }),
};

/** What the side panel shows for a list item. */
export interface ItemDetail {
  /** A picture shown big (album cover, screenshot): a URL from your `net` hosts, or an absolute path. */
  image?: string;
  stats?: Stat[];
  /** Below the stats; a `detail` view's markdown subset. */
  markdown?: string;
  metadata?: Metadata[];
}

/**
 * Backslash's own icons: crisp, colored with `tint`, the same on every
 * desktop. Use them as `icon: Icons.inProgress` or `{ source: Icons.inProgress, tint: "yellow" }`.
 */
export const Icons = {
  // Statuses
  backlog: "bs-circle-dashed",
  todo: "bs-circle",
  inProgress: "bs-circle-half",
  inReview: "bs-circle-three-quarters",
  done: "bs-circle-check",
  canceled: "bs-circle-x",
  // Priorities
  urgent: "bs-priority-urgent",
  high: "bs-priority-high",
  medium: "bs-priority-medium",
  low: "bs-priority-low",
  noPriority: "bs-priority-none",
  // Code
  pullRequest: "bs-pr-open",
  pullRequestDraft: "bs-pr-draft",
  merged: "bs-pr-merged",
  pullRequestClosed: "bs-pr-closed",
  issue: "bs-issue-open",
  issueClosed: "bs-issue-closed",
  branch: "bs-branch",
  commit: "bs-commit",
  repository: "bs-repo",
  code: "bs-code",
  terminal: "bs-terminal",
  // Things
  check: "bs-check",
  cross: "bs-x",
  dot: "bs-dot",
  star: "bs-star",
  heart: "bs-heart",
  pin: "bs-pin",
  tag: "bs-tag",
  link: "bs-link",
  comment: "bs-comment",
  person: "bs-user",
  calendar: "bs-calendar",
  clock: "bs-clock",
  timer: "bs-timer",
  bell: "bs-bell",
  folder: "bs-folder",
  file: "bs-file",
  image: "bs-image",
  text: "bs-text",
  globe: "bs-globe",
  lock: "bs-lock",
  key: "bs-key",
  shield: "bs-shield",
  bolt: "bs-bolt",
  sparkle: "bs-sparkle",
  box: "bs-box",
  server: "bs-server",
  cpu: "bs-cpu",
  plug: "bs-plug",
  wifi: "bs-wifi",
  wifiOff: "bs-wifi-off",
  bluetooth: "bs-bluetooth",
  speaker: "bs-speaker",
  headphones: "bs-headphones",
  music: "bs-music",
  play: "bs-play",
  pause: "bs-pause",
  next: "bs-next",
  previous: "bs-previous",
  window: "bs-window",
  inbox: "bs-inbox",
  project: "bs-project",
  cycle: "bs-cycle",
  flag: "bs-flag",
  alert: "bs-alert",
  info: "bs-info",
  search: "bs-search",
  plus: "bs-plus",
  refresh: "bs-refresh",
  trash: "bs-trash",
  copy: "bs-copy",
  paste: "bs-paste",
  external: "bs-external",
  download: "bs-download",
  settings: "bs-settings",
  history: "bs-history",
  package: "bs-package",
  smile: "bs-smile",
  palette: "bs-palette",
  vpn: "bs-vpn",
  shuffle: "bs-shuffle",
  repeat: "bs-repeat",
  stop: "bs-stop",
  mic: "bs-mic",
  speakerOff: "bs-speaker-off",
  monitor: "bs-monitor",
  mail: "bs-mail",
  home: "bs-home",
  archive: "bs-archive",
  hash: "bs-hash",
  filter: "bs-filter",
  eye: "bs-eye",
  eyeOff: "bs-eye-off",
  cloud: "bs-cloud",
  keyboard: "bs-keyboard",
  apps: "bs-apps",
  calculator: "bs-calculator",
  sun: "bs-sun",
  moon: "bs-moon",
  power: "bs-power",
  starFilled: "bs-star-filled",
  heartFilled: "bs-heart-filled",
  upload: "bs-upload",
  minus: "bs-minus",
  question: "bs-question",
  arrowRight: "bs-arrow-right",
  arrowUp: "bs-arrow-up",
  arrowDown: "bs-arrow-down",
  chevronRight: "bs-chevron-right",
  square: "bs-square",
  squareCheck: "bs-square-check",
  dottedCircle: "bs-circle-dot",
} as const;

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
  /** The side panel: stats, markdown and metadata lines. */
  detail?: ItemDetail;
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
   * An async generator can yield several times to show results progressively,
   * or keep yielding to stay live (until `ctx.signal` aborts).
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
  /**
   * Chips under the search bar ("Assigned", "Created", "Subscribed"). The
   * picked value is `ctx.filter`; picking another one reloads the list. The
   * choice is remembered.
   */
  filter?: { options: (string | { value: string; title: string })[]; default?: string };
  /** A short line in the footer, or a function of the items shown. */
  status?: string | ((items: Item[]) => string);
}

export interface DetailOptions {
  title?: string;
  /**
   * Markdown body, or a function computing it. A function may also be an
   * async generator yielding the whole text so far, to stream an answer.
   */
  markdown: string | ((ctx: Context) => Awaitable<string> | AsyncIterable<string>);
  /** The side panel: key/value pairs, or richer lines built with {@link meta}. */
  metadata?: Record<string, string | number> | Metadata[];
  /** Big numbers above the text. */
  stats?: Stat[];
  /** A short line in the footer. */
  status?: string;
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
  icon?: string;
  /** GTK accelerator, e.g. `<Ctrl>o`, `<Ctrl><Shift>c`. */
  shortcut?: string;
  /** Rendered in red. */
  destructive?: boolean;
  /** Ask "<confirm>" before running. */
  confirm?: string;
  /** Groups actions under a header in the action panel (Ctrl+K). */
  section?: string;
}

export type Action =
  & ActionOptions
  & { title: string }
  & (
    | { readonly [KIND]: "open"; target: string }
    | { readonly [KIND]: "copy"; text: string }
    | { readonly [KIND]: "paste"; text: string }
    | { readonly [KIND]: "callback"; run: (ctx: Context) => Awaitable<Outcome> }
    | { readonly [KIND]: "submenu"; actions: Action[] | ((ctx: Context) => Awaitable<Action[]>) }
  );

/** Opens a URL, file or folder with the default application. */
export function open(target: string, title = "Open", options: ActionOptions = {}): Action {
  return { icon: "bs-external", ...options, title, target, [KIND]: "open" };
}

/** Copies text to the clipboard and closes the launcher. */
export function copy(text: string, title = "Copy to Clipboard", options: ActionOptions = {}): Action {
  return { icon: "bs-copy", ...options, title, text, [KIND]: "copy" };
}

/**
 * Pastes text into the app the user was in, then closes the launcher. Where
 * the desktop doesn't allow it, the text is copied instead.
 */
export function paste(text: string, title = "Paste", options: ActionOptions = {}): Action {
  return { icon: "bs-paste", ...options, title, text, [KIND]: "paste" };
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

/**
 * More actions behind one, opened in the action panel: "Change Status ›".
 * Pass a function to load them only when the user opens the submenu.
 */
export function submenu(
  title: string,
  actions: Action[] | ((ctx: Context) => Awaitable<Action[]>),
  options: ActionOptions = {},
): Action {
  return { ...options, title, actions, [KIND]: "submenu" };
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
  /** Pastes into the app the user was in and closes the launcher (copies where it can't). */
  paste: async (text: string): Promise<void> => {
    await rpc().request("clipboard/paste", { text });
  },
};

/** Opens a URL, file or folder with the default application. */
export async function openExternal(target: string): Promise<void> {
  await rpc().request("open", { target });
}

/**
 * Updates what the root search shows next to this command, e.g. "3 in
 * progress". Kept until the next update, so the user sees it before opening
 * the command again.
 */
export async function updateCommandMetadata(metadata: { subtitle?: string | null }): Promise<void> {
  await rpc().request("command/metadata", { subtitle: metadata.subtitle ?? null });
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

// ---------------------------------------------------------------------------
// OAuth

/**
 * An OAuth 2.0 service. The launcher signs in with the authorization code
 * flow and PKCE, the browser coming back to `http://127.0.0.1:<port>/callback`
 * (register that redirect with the service; `redirectPort` fixes the port
 * for services that need an exact match).
 *
 * The token URL's host must be in your `[permissions] net`.
 */
export interface OAuthProvider {
  /** Names the sign-in; one per provider and extension. */
  id: string;
  /** The service's name, shown while signing in. */
  title?: string;
  authorizeUrl: string;
  tokenUrl: string;
  clientId: string;
  /** For services that require one even with PKCE. */
  clientSecret?: string;
  scopes?: string[];
  /** Default `" "`; some services use `","`. */
  scopeSeparator?: string;
  /** Extra query parameters for the authorize URL. */
  params?: Record<string, string>;
  redirectPort?: number;
  /** Default `/callback`. */
  redirectPath?: string;
  /** `"basic"` sends the client credentials as HTTP Basic auth. */
  clientAuth?: "body" | "basic";
}

function signIn(provider: OAuthProvider, interactive: boolean): Promise<Json> {
  return rpc().request("oauth/token", { provider: provider as unknown as Json, interactive });
}

export const oauth = {
  /**
   * A valid access token: the saved one, refreshed when it expired, or a new
   * one after the user signs in through the browser. Tokens are kept in the
   * system keyring, never in your extension's storage.
   */
  token: async (provider: OAuthProvider): Promise<string> => String(await signIn(provider, true)),
  /** The saved token (refreshed if needed) without asking the user, if any. */
  savedToken: async (provider: OAuthProvider): Promise<string | undefined> =>
    (await signIn(provider, false) ?? undefined) as string | undefined,
  /** Forgets the tokens, e.g. for a "Sign Out" action. */
  signOut: async (provider: OAuthProvider): Promise<void> => {
    await rpc().request("oauth/signOut", { provider: provider as unknown as Json });
  },
};
