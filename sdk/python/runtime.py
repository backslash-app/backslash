"""Bootstraps one Python command: ``python3 -I -u runtime.py <entry.py>``.

Speaks JSON-RPC 2.0 with the launcher, one message per line on stdin/stdout
(see docs/PROTOCOL.md). Turns the values built with ``backslash`` into wire
views and keeps the callbacks they reference. Each request runs in its own
thread, so a slow search never delays a cancellation or another request.
"""

import importlib.util
import itertools
import json
import os
import sys
import threading
import time
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# The extension's own modules, next to its commands.
sys.path.insert(1, os.getcwd())

import backslash as bs  # noqa: E402

# stdout belongs to the protocol: print() and child processes go to the log.
_protocol = os.fdopen(os.dup(1), "w", encoding="utf-8")
os.dup2(2, 1)
sys.stdout = sys.stderr


# --- Requests to the launcher ------------------------------------------------


class Rpc:
    def __init__(self):
        self.lock = threading.Lock()
        self.ids = itertools.count(1)
        self.waiting = {}

    def send(self, message):
        line = json.dumps({"jsonrpc": "2.0", **message}, default=_json_default)
        with self.lock:
            _protocol.write(line + "\n")
            _protocol.flush()

    def request(self, method, params=None):
        id = next(self.ids)
        slot = {"done": threading.Event()}
        self.waiting[id] = slot
        self.send({"id": id, "method": method, "params": params})
        slot["done"].wait()
        if "error" in slot:
            raise bs.BackslashError(slot["error"].get("message", "launcher error"))
        return slot.get("result")

    def notify(self, method, params=None):
        self.send({"method": method, "params": params})

    def answered(self, message):
        slot = self.waiting.pop(message.get("id"), None)
        if slot is None:
            return
        if "error" in message:
            slot["error"] = message["error"]
        else:
            slot["result"] = message.get("result")
        slot["done"].set()


def _json_default(value):
    raise TypeError(f"{type(value).__name__} is not something the launcher can show")


rpc = Rpc()
bs._set_rpc(rpc)

# --- State -------------------------------------------------------------------

session = {"command": "", "args": {}, "preferences": {}, "query": "", "launch_type": "userInitiated"}
lifetime = threading.Event()
view_ids = itertools.count(1)
action_ids = itertools.count(1)
views = {}
callbacks = {}
submenus = {}
filters = {}
#: Set when a view is replaced, to stop its streams.
view_stops = {}


def context(cancelled=None, filter=None):
    return bs.Context(session["args"], session["preferences"], session["query"], cancelled or lifetime, filter,
                      session["launch_type"])


def call(fn, *args):
    return bs._resolve(fn(*args))


# --- Serialization -----------------------------------------------------------


def actions_json(actions, view_id):
    out = []
    for a in actions or []:
        if not isinstance(a, bs._Action):
            raise TypeError(f"not an action: {a!r} (use Open, Copy, Paste, Action or Submenu)")
        base = {"id": f"{view_id}:a{next(action_ids)}", "title": a.title}
        for key in ("icon", "shortcut", "confirm", "section"):
            if a.options.get(key):
                base[key] = a.options[key]
        if a.options.get("destructive"):
            base["destructive"] = True
        concealed = bool(a.options.get("concealed"))
        if a.kind == "submenu":
            inner = a.fields["actions"]
            if callable(inner):
                submenus[base["id"]] = (view_id, inner)
                out.append({**base, "kind": "submenu", "actions": [], "lazy": True})
            else:
                out.append({**base, "kind": "submenu", "actions": actions_json(inner, view_id)})
        elif a.kind == "open":
            out.append({**base, "kind": "open", "target": a.fields["target"]})
        elif a.kind in ("copy", "paste"):
            out.append({**base, "kind": a.kind, "text": a.fields["text"], "concealed": concealed})
        else:
            callbacks[base["id"]] = (view_id, a.fields["run"])
            out.append({**base, "kind": "callback"})
    return out


def icon_json(icon):
    if icon is None:
        return None, None
    if isinstance(icon, str):
        return icon, None
    return icon.get("source"), icon.get("tint")


def metadata_json(metadata):
    if not metadata:
        return []
    if isinstance(metadata, dict):
        return [{"label": k, "value": str(v)} for k, v in metadata.items()]
    out = []
    for m in metadata:
        icon, tint = icon_json(m.get("icon"))
        out.append({
            "label": m.get("label", ""),
            "value": m.get("value") or "",
            "icon": icon,
            "color": m.get("color") or tint,
            "tags": [{"text": t["text"], "color": t.get("color")} for t in m.get("tags") or []],
            "link": m.get("link"),
            "avatar": m.get("avatar"),
            "progress": m.get("progress"),
            "separator": bool(m.get("separator")),
        })
    return out


def stats_json(stats):
    return [{"label": s["label"], "value": str(s["value"]), "color": s.get("color")} for s in stats or []]


def items_json(items, view_id):
    seen = {}
    out = []
    for item in items:
        id = item.get("id") or item["title"]
        n = seen.get(id, 0)
        seen[id] = n + 1
        if n:
            id = f"{id}#{n}"
        icon, tint = icon_json(item.get("icon"))
        accessories = []
        for a in item.get("accessories") or []:
            a_icon, a_tint = icon_json(a.get("icon"))
            accessories.append({**a, "icon": a_icon, "color": a.get("color") or a_tint})
        accessories += [{"tag": t} for t in item.get("tags") or []]
        detail = item.get("detail")
        out.append({
            "id": id,
            "title": item["title"],
            "subtitle": item.get("subtitle"),
            "icon": icon,
            "iconTint": tint,
            "section": item.get("section"),
            "keywords": item.get("keywords") or [],
            "accessories": accessories,
            "actions": actions_json(item.get("actions"), view_id),
            "detail": {
                "image": detail.get("image"),
                "stats": stats_json(detail.get("stats")),
                "markdown": detail.get("markdown"),
                "metadata": metadata_json(detail.get("metadata")),
            } if detail else None,
        })
    return out


def menu_items_json(items, view_id):
    seen = {}
    out = []
    for item in items:
        id = item.get("id") or item["title"]
        n = seen.get(id, 0)
        seen[id] = n + 1
        if n:
            id = f"{id}#{n}"
        icon, tint = icon_json(item.get("icon"))
        action = item.get("action")
        out.append({
            "id": id,
            "title": item["title"],
            "subtitle": item.get("subtitle"),
            "icon": icon,
            "iconTint": tint,
            "section": item.get("section"),
            "enabled": item.get("enabled", True),
            "action": actions_json([action], view_id)[0] if action else None,
            "items": menu_items_json(item.get("items") or [], view_id),
        })
    return out


def menu_json(spec, id, ctx):
    items = call(spec.items, ctx) if callable(spec.items) else spec.items
    icon, tint = icon_json(spec.icon)
    return {"type": "menu", "id": id, "title": spec.title, "icon": icon, "iconTint": tint,
            "tooltip": spec.tooltip, "items": menu_items_json(items or [], id)}


def filter_options(spec):
    return [bs._option(o) for o in spec.filter or []]


def list_json(spec, id, items, loading):
    items = list(items or [])
    status = spec.status(items) if callable(spec.status) else spec.status
    return {
        "type": "list",
        "id": id,
        "title": spec.title,
        "placeholder": spec.placeholder,
        "filtering": "extension" if spec.search else "host",
        "loading": loading,
        "items": items_json(items, id),
        "empty": dict(spec.empty) if spec.empty else None,
        "actions": actions_json(spec.actions, id),
        "layout": spec.layout,
        "columns": spec.columns,
        "filter": {"options": filter_options(spec), "value": filters.get(id, "")} if spec.filter else None,
        "status": status,
    }


def detail_json(spec, id, markdown, loading):
    return {
        "type": "detail",
        "id": id,
        "title": spec.title,
        "markdown": markdown,
        "metadata": metadata_json(spec.metadata),
        "stats": stats_json(spec.stats),
        "status": spec.status,
        "actions": actions_json(spec.actions, id),
        "loading": loading,
    }


def filter_key(spec):
    return f"$filter:{session['command']}:{spec.title or spec.placeholder or ''}"


def initial_filter(spec):
    values = [o["value"] for o in filter_options(spec)]
    try:
        saved = rpc.request("storage/get", {"key": filter_key(spec)})
    except bs.BackslashError:
        saved = None
    if isinstance(saved, str) and saved in values:
        return saved
    return spec.filter_default or (values[0] if values else "")


def retire(view_id):
    stop = view_stops.pop(view_id, None)
    if stop:
        stop.set()
    views.pop(view_id, None)
    filters.pop(view_id, None)


def render(view, filter=None):
    if not isinstance(view, bs.View):
        raise TypeError(f"not a view: {view!r} (use List, Detail or Form)")
    id = f"v{next(view_ids)}"
    views[id] = view
    stop = threading.Event()
    view_stops[id] = stop
    if isinstance(view, bs.List) and view.filter:
        filters[id] = filter if filter is not None else initial_filter(view)
    ctx = context(stop, filters.get(id))
    if isinstance(view, bs.MenuBar):
        return menu_json(view, id, ctx)
    if isinstance(view, bs.List):
        if view.search:
            return list_json(view, id, call(view.search, session["query"], ctx), False)
        source = view.items(ctx) if callable(view.items) else view.items
        if bs._is_stream(source):
            threading.Thread(target=stream, args=(view, id, source, stop), daemon=True).start()
            return list_json(view, id, [], True)
        return list_json(view, id, bs._resolve(source), False)
    if isinstance(view, bs.Detail):
        markdown = view.markdown(ctx) if callable(view.markdown) else view.markdown
        if bs._is_stream(markdown):
            threading.Thread(target=stream_detail, args=(view, id, markdown, stop), daemon=True).start()
            return detail_json(view, id, "", True)
        return detail_json(view, id, bs._resolve(markdown), False)
    callbacks[f"{id}:submit"] = (id, lambda ctx, values: view.submit(values or {}, ctx))
    return {
        "type": "form",
        "id": id,
        "title": view.title,
        "submitTitle": view.submit_title,
        "fields": [dict(f) for f in view.fields],
    }


def stream(spec, id, source, stop):
    """Sends each batch of items; later yields are refreshes."""
    last = []
    try:
        for items in bs._iterate(source):
            if stop.is_set() or lifetime.is_set():
                return
            last = list(items)
            rpc.notify("view/update", {"view": list_json(spec, id, last, False)})
    except Exception as err:  # noqa: BLE001 - shown to the user
        if stop.is_set():
            return
        traceback.print_exc()
        try:
            rpc.request("toast", {"title": "Error", "message": str(err), "style": "failure"})
        except bs.BackslashError:
            pass
    if not stop.is_set():
        rpc.notify("view/update", {"view": list_json(spec, id, last, False)})


def stream_detail(spec, id, source, stop):
    """Sends the text as it grows, at most every 80 ms."""
    text, sent = "", 0.0
    try:
        for chunk in bs._iterate(source):
            if stop.is_set():
                return
            text = chunk
            if time.monotonic() - sent > 0.08:
                sent = time.monotonic()
                rpc.notify("view/update", {"view": detail_json(spec, id, text, True)})
    except Exception as err:  # noqa: BLE001 - shown in the document
        traceback.print_exc()
        text += ("\n\n" if text else "") + f"**Error:** {err}"
    rpc.notify("view/update", {"view": detail_json(spec, id, text, False)})


def to_effect(outcome, view_id):
    if outcome is None:
        return {"type": "none"}
    if isinstance(outcome, bs.MenuBar):
        raise TypeError("an action cannot open a tray menu")
    if isinstance(outcome, bs.View):
        return {"type": "push", "view": render(outcome)}
    if not isinstance(outcome, bs.Effect):
        raise TypeError(f"unexpected action result: {outcome!r}")
    if outcome.type == "close":
        return {"type": "close", "message": outcome.fields.get("message")}
    if outcome.type == "pop":
        return {"type": "pop"}
    if outcome.type == "replace":
        retire(view_id)
        return {"type": "replace", "view": render(outcome.fields["view"])}
    view = views.get(view_id)
    if view is None:
        return {"type": "none"}
    filter = filters.get(view_id)
    retire(view_id)
    return {"type": "replace", "view": render(view, filter)}


# --- Requests from the launcher ----------------------------------------------

entry = sys.argv[1] if len(sys.argv) > 1 else sys.exit("usage: runtime.py <command entry>")


def load_command():
    spec = importlib.util.spec_from_file_location("backslash_command", entry)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if not hasattr(module, "command"):
        raise AttributeError(f"{entry} defines no `command`")
    return module.command


def start(params, cancelled):
    session.update(
        command=params.get("command") or "",
        args=params.get("arguments") or {},
        preferences=params.get("preferences") or {},
        query=params.get("query") or "",
        launch_type="background" if params.get("launchType") == "background" else "userInitiated",
    )
    command = load_command()
    if isinstance(command, bs.View):
        return {"type": "push", "view": render(command)}
    result = call(command, context())
    if isinstance(result, bs.View):
        return {"type": "push", "view": render(result)}
    return {"type": "close", "message": None if result is None else str(result)}


def search(params, cancelled):
    view_id = params.get("view")
    view = views.get(view_id)
    if not isinstance(view, bs.List) or not view.search:
        raise ValueError(f"view {view_id} is not searchable")
    session["query"] = params.get("query") or ""
    items = call(view.search, session["query"], context(cancelled, filters.get(view_id)))
    return list_json(view, view_id, items, False)


def filter_(params, cancelled):
    view_id = params.get("view")
    view = views.get(view_id)
    if not isinstance(view, bs.List):
        raise ValueError(f"view {view_id} has no filter")
    value = str(params.get("value"))
    try:
        rpc.request("storage/set", {"key": filter_key(view), "value": value})
    except bs.BackslashError:
        pass
    retire(view_id)
    return render(view, value)


def submenu(params, cancelled):
    view_id, load = submenus[params["action"]]
    return actions_json(call(load, context(cancelled)), view_id)


def action(params, cancelled):
    found = callbacks.get(params.get("action"))
    if found is None:
        raise KeyError(f"unknown action {params.get('action')}")
    view_id, run = found
    ctx = context(cancelled)
    values = params.get("values")
    outcome = call(run, ctx, values) if params["action"].endswith(":submit") else call(run, ctx)
    return to_effect(outcome, view_id)


handlers = {"start": start, "search": search, "filter": filter_, "submenu": submenu, "action": action}
inflight = {}


def handle(message):
    id = message["id"]
    cancelled = threading.Event()
    inflight[id] = cancelled
    try:
        result = handlers[message["method"]](message.get("params") or {}, cancelled)
        rpc.send({"id": id, "result": result})
    except Exception as err:  # noqa: BLE001 - every error goes back to the launcher
        if not cancelled.is_set():
            traceback.print_exc()
        code = -32800 if cancelled.is_set() else -32000
        rpc.send({"id": id, "error": {"code": code, "message": str(err) or type(err).__name__}})
    finally:
        inflight.pop(id, None)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            print("invalid message from launcher:", line, file=sys.stderr)
            continue
        method = message.get("method")
        if method is None:
            rpc.answered(message)
        elif method == "$/cancel":
            cancelled = inflight.get((message.get("params") or {}).get("id"))
            if cancelled:
                cancelled.set()
        elif "id" in message:
            if method not in handlers:
                rpc.send({"id": message["id"], "error": {"code": -32601, "message": f"unknown method {method}"}})
            else:
                threading.Thread(target=handle, args=(message,), daemon=True).start()
    lifetime.set()
    os._exit(0)


main()
