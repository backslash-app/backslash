"""Backslash extension API for Python.

A command is a module whose ``command`` describes what to show::

    from backslash import List, Item, Copy

    command = List(items=[Item("Backslash", actions=[Copy("backslash")])])

``command`` can be a view (``List``, ``Detail``, ``Form``), or a function
receiving a ``Context`` and returning a view. A function returning anything
else (a string, ``None``) is a command without a view: the launcher flashes
the string and closes.

Views are plain data. The launcher draws them natively, filters them,
handles the keyboard, and only calls back into your code when it must.

This module ships with the launcher (``sdk/python``); the runtime puts it on
the module path, so extensions never install it. Standard library only.
"""

from __future__ import annotations

import asyncio
import inspect
import threading
from typing import Any, Callable, Iterable, Optional

__all__ = [
    "Context", "List", "Detail", "Form", "Item", "Icon", "MenuBar", "MenuItem",
    "Open", "Copy", "Paste", "Action", "Submenu",
    "close", "pop", "refresh", "replace",
    "field", "meta", "storage", "clipboard", "oauth",
    "toast", "open_external", "update_command_metadata", "BackslashError",
]


class BackslashError(Exception):
    """The launcher refused a request (a missing permission, a failed sign-in)."""


# ---------------------------------------------------------------------------
# Context


class Context:
    """Everything a command receives when it starts or handles an action."""

    def __init__(self, args: dict, preferences: dict, query: str, cancelled: threading.Event,
                 filter: Optional[str] = None, launch_type: str = "userInitiated"):
        #: Values of the command's ``[[commands.arguments]]``, already validated.
        self.args = args
        #: Values of the extension's ``[[preferences]]``, defaults applied.
        self.preferences = preferences
        #: Text typed in the search bar when the command was opened (or the
        #: current query in ``search``).
        self.query = query
        #: Set when the user types again, goes back or closes the window:
        #: long work should check it and stop.
        self.cancelled = cancelled
        #: The chip picked in the list's ``filter``, if it has one.
        self.filter = filter
        #: ``"background"`` when the command runs on its ``interval`` or
        #: refreshes its tray menu by itself, ``"userInitiated"`` otherwise.
        self.launch_type = launch_type


# ---------------------------------------------------------------------------
# Views


def Icon(source: str, tint: Optional[str] = None) -> dict:
    """A tinted icon: ``Icon("bs-in-progress", "yellow")``."""
    return {"source": source, "tint": tint}


def Item(title: str, *, id: Optional[str] = None, subtitle: Optional[str] = None, icon: Any = None,
         section: Optional[str] = None, keywords: Optional[list] = None, tags: Optional[list] = None,
         accessories: Optional[list] = None, actions: Optional[list] = None, detail: Optional[dict] = None) -> dict:
    """One row of a list. The first action runs on Enter, the second on
    Ctrl+Enter, all are in Ctrl+K. ``detail`` is the side panel:
    ``{"markdown": …, "stats": [...], "metadata": [meta.text(…)], "image": …}``.
    Plain dicts with the same keys work too."""
    return {k: v for k, v in dict(title=title, id=id, subtitle=subtitle, icon=icon, section=section,
                                  keywords=keywords, tags=tags, accessories=accessories, actions=actions,
                                  detail=detail).items() if v is not None}


class View:
    pass


class List(View):
    """A searchable list.

    ``items`` is loaded once and filtered by the launcher as the user types:
    a list, or a function of the context returning one. A generator yields
    several lists to show results progressively (or to stay live).
    ``search(query, ctx)`` is called on every keystroke instead, when results
    depend on the query. ``filter`` offers chips under the search bar
    (``["Assigned", "Created"]`` or ``[("mine", "Mine"), …]``); the picked one
    is ``ctx.filter``. ``status`` is a footer line, or a function of the items.
    """

    def __init__(self, items: Any = None, *, search: Optional[Callable] = None, title: Optional[str] = None,
                 placeholder: Optional[str] = None, empty: Optional[dict] = None, actions: Optional[list] = None,
                 layout: str = "list", columns: Optional[int] = None, filter: Optional[list] = None,
                 filter_default: Optional[str] = None, status: Any = None):
        if items is not None and search is not None:
            raise ValueError("List: use either items or search, not both")
        self.items, self.search, self.title, self.placeholder = items, search, title, placeholder
        self.empty, self.actions, self.layout, self.columns = empty, actions or [], layout, columns
        self.filter, self.filter_default, self.status = filter, filter_default, status


class Detail(View):
    """A markdown document. ``markdown`` may be a function of the context, or a
    generator yielding the whole text so far, to stream an answer."""

    def __init__(self, markdown: Any = "", *, title: Optional[str] = None, metadata: Any = None,
                 stats: Optional[list] = None, status: Optional[str] = None, actions: Optional[list] = None):
        self.markdown, self.title, self.metadata = markdown, title, metadata
        self.stats, self.status, self.actions = stats or [], status, actions or []


class Form(View):
    """A form; ``submit(values, ctx)`` receives the values keyed by field id and
    returns what an action returns."""

    def __init__(self, fields: list, submit: Callable, *, title: Optional[str] = None,
                 submit_title: Optional[str] = None):
        self.fields, self.submit, self.title, self.submit_title = fields, submit, title, submit_title


class MenuBar(View):
    """What a ``mode = "menu-bar"`` command shows: an icon in the system tray
    with a menu. ``items`` is a list of ``MenuItem``, or a function of the
    context returning one. Backslash runs the command when it starts, every
    ``interval``, and when the user opens it; clicking an item runs it again
    and calls the item's action."""

    def __init__(self, items: Any, *, title: Optional[str] = None, icon: Any = None,
                 tooltip: Optional[str] = None):
        self.items, self.title, self.icon, self.tooltip = items, title, icon, tooltip


def MenuItem(title: str, action: Any = None, *, id: Optional[str] = None, subtitle: Optional[str] = None,
             icon: Any = None, section: Optional[str] = None, enabled: bool = True,
             items: Optional[list] = None) -> dict:
    """A line of a tray menu: ``action`` (``Open``, ``Copy``, ``Action``) runs
    when it is clicked, ``items`` makes it a submenu. The id defaults to the
    title and must stay the same from one run to the next."""
    return {"title": title, "action": action, "id": id, "subtitle": subtitle, "icon": icon,
            "section": section, "enabled": enabled, "items": items or []}


# ---------------------------------------------------------------------------
# Actions

_OPTIONS = ("icon", "shortcut", "destructive", "confirm", "section", "concealed")


class _Action:
    def __init__(self, kind: str, title: str, options: dict, **fields):
        unknown = set(options) - set(_OPTIONS)
        if unknown:
            raise TypeError(f"unknown action option {', '.join(sorted(unknown))}")
        self.kind, self.title, self.options, self.fields = kind, title, options, fields


def Open(target: str, title: str = "Open", **options) -> _Action:
    """Opens a URL, file or folder with the default application."""
    return _Action("open", title, {"icon": "bs-external", **options}, target=target)


def Copy(text: str, title: str = "Copy to Clipboard", **options) -> _Action:
    """Copies text and closes the launcher. ``concealed=True`` keeps it out of
    clipboard histories (passwords)."""
    return _Action("copy", title, {"icon": "bs-copy", **options}, text=text)


def Paste(text: str, title: str = "Paste", **options) -> _Action:
    """Pastes text into the app the user was in (copies where the desktop
    doesn't allow it), then closes the launcher."""
    return _Action("paste", title, {"icon": "bs-paste", **options}, text=text)


def Action(title: str, run: Callable, **options) -> _Action:
    """Runs ``run(ctx)``. Return a view to navigate to it, or an effect
    (``close()``, ``pop()``, ``refresh()``, ``replace(view)``)."""
    return _Action("callback", title, options, run=run)


def Submenu(title: str, actions: Any, **options) -> _Action:
    """More actions behind one. Pass a function of the context to load them
    only when the submenu opens."""
    return _Action("submenu", title, options, actions=actions)


# Options for all actions: icon, shortcut ("<Ctrl>o"), destructive, confirm
# ("Really?"), section (a header in Ctrl+K).

# ---------------------------------------------------------------------------
# Effects


class Effect:
    def __init__(self, type: str, **fields):
        self.type, self.fields = type, fields


def close(message: Optional[str] = None) -> Effect:
    """Hide the launcher, optionally flashing a short message."""
    return Effect("close", message=message)


def pop() -> Effect:
    """Go back to the previous view."""
    return Effect("pop")


def refresh() -> Effect:
    """Reload the current view (runs ``items`` or ``search`` again)."""
    return Effect("refresh")


def replace(view: View) -> Effect:
    """Swap the current view for another one."""
    return Effect("replace", view=view)


# ---------------------------------------------------------------------------
# Form fields and detail lines


class field:
    """Form fields: ``field.text("title", "Title", required=True)``."""

    @staticmethod
    def _make(kind: str, id: str, title: str, placeholder=None, required=False, value=None, **extra) -> dict:
        out = {"id": id, "title": title, "kind": kind, "required": required, **extra}
        if placeholder is not None:
            out["placeholder"] = placeholder
        if value is not None:
            out["value"] = value
        return out

    @staticmethod
    def text(id: str, title: str, **o) -> dict:
        return field._make("text", id, title, **o)

    @staticmethod
    def password(id: str, title: str, **o) -> dict:
        return field._make("password", id, title, **o)

    @staticmethod
    def text_area(id: str, title: str, **o) -> dict:
        return field._make("textArea", id, title, **o)

    @staticmethod
    def checkbox(id: str, title: str, **o) -> dict:
        return field._make("checkbox", id, title, **o)

    @staticmethod
    def dropdown(id: str, title: str, options: Iterable, **o) -> dict:
        return field._make("dropdown", id, title, options=[_option(x) for x in options], **o)


def _option(o: Any) -> dict:
    if isinstance(o, str):
        return {"value": o, "title": o}
    if isinstance(o, (tuple, list)):
        return {"value": o[0], "title": o[1]}
    return dict(o)


class meta:
    """Lines of a detail panel: ``meta.text("Version", "1.0")``."""

    @staticmethod
    def text(label: str, value: Any, icon: Any = None, color: Optional[str] = None) -> dict:
        return {"label": label, "value": str(value), "icon": icon, "color": color}

    @staticmethod
    def tags(label: str, tags: Iterable) -> dict:
        return {"label": label, "tags": [{"text": t} if isinstance(t, str) else dict(t) for t in tags]}

    @staticmethod
    def link(label: str, text: str, url: str) -> dict:
        return {"label": label, "value": text, "link": url}

    @staticmethod
    def person(label: str, name: str, avatar: Optional[str] = None) -> dict:
        return {"label": label, "value": name, "avatar": avatar or name}

    @staticmethod
    def progress(label: str, ratio: float, text: Optional[str] = None) -> dict:
        return {"label": label, "progress": max(0.0, min(1.0, ratio)),
                "value": text if text is not None else f"{round(ratio * 100)} %"}

    @staticmethod
    def separator() -> dict:
        return {"label": "", "separator": True}


# ---------------------------------------------------------------------------
# Launcher services

_rpc = None


def _set_rpc(rpc) -> None:
    global _rpc
    _rpc = rpc


def _request(method: str, params: Optional[dict] = None) -> Any:
    if _rpc is None:
        raise BackslashError("the backslash module only works inside a running Backslash command")
    return _rpc.request(method, params or {})


class storage:
    """Persistent key/value storage private to your extension (JSON values)."""

    @staticmethod
    def get(key: str, default: Any = None) -> Any:
        value = _request("storage/get", {"key": key})
        return default if value is None else value

    @staticmethod
    def set(key: str, value: Any) -> None:
        _request("storage/set", {"key": key, "value": value})

    @staticmethod
    def delete(key: str) -> None:
        _request("storage/set", {"key": key, "value": None})

    @staticmethod
    def keys() -> list:
        return _request("storage/keys") or []


class clipboard:
    @staticmethod
    def write(text: str, concealed: bool = False) -> None:
        _request("clipboard/write", {"text": text, "concealed": concealed})

    @staticmethod
    def read() -> str:
        """Requires ``clipboard-read = true`` in ``[permissions]``."""
        return _request("clipboard/read") or ""

    @staticmethod
    def paste(text: str, concealed: bool = False) -> None:
        """Pastes into the app the user was in and closes the launcher."""
        _request("clipboard/paste", {"text": text, "concealed": concealed})


def toast(title: str, style: str = "info", message: Optional[str] = None) -> None:
    """A short message at the bottom of the launcher: ``info``, ``success`` or ``failure``."""
    _request("toast", {"title": title, "style": style, "message": message})


def open_external(target: str) -> None:
    """Opens a URL, file or folder with the default application."""
    _request("open", {"target": target})


def update_command_metadata(subtitle: Optional[str]) -> None:
    """What the root search shows under this command ("3 in progress"); ``None`` clears it."""
    _request("command/metadata", {"subtitle": subtitle})


class oauth:
    """OAuth 2.0 sign-in done by the launcher (PKCE, browser, refresh, tokens in
    the keyring). ``provider`` is a dict with the keys of the TypeScript SDK's
    ``OAuthProvider``: ``id``, ``authorizeUrl``, ``tokenUrl``, ``clientId``,
    and optionally ``title``, ``clientSecret``, ``scopes``, …"""

    @staticmethod
    def token(provider: dict) -> str:
        return str(_request("oauth/token", {"provider": provider, "interactive": True}))

    @staticmethod
    def saved_token(provider: dict) -> Optional[str]:
        return _request("oauth/token", {"provider": provider, "interactive": False})

    @staticmethod
    def sign_out(provider: dict) -> None:
        _request("oauth/signOut", {"provider": provider})


# ---------------------------------------------------------------------------
# Helpers for the runtime


def _resolve(value: Any) -> Any:
    """Waits for ``async def`` results, so both styles work."""
    if inspect.isawaitable(value):
        return asyncio.run(_await(value))
    return value


async def _await(value):
    return await value


def _iterate(source: Any):
    """Yields from a generator or an async generator."""
    if hasattr(source, "__aiter__"):
        loop = asyncio.new_event_loop()
        it = source.__aiter__()
        try:
            while True:
                try:
                    yield loop.run_until_complete(it.__anext__())
                except StopAsyncIteration:
                    return
        finally:
            loop.close()
    else:
        yield from source


def _is_stream(value: Any) -> bool:
    return inspect.isgenerator(value) or inspect.isasyncgen(value)
