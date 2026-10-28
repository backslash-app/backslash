from backslash import Action, MenuBar, MenuItem, close


def items(ctx):
    return [
        MenuItem(ctx.launch_type, id="launch", enabled=False),
        MenuItem("Inbox", section="Mail", items=[MenuItem("Archive", Action("Archive", lambda ctx: close("Archived")))]),
    ]


command = MenuBar(items, title="2", icon="bs-bell")
