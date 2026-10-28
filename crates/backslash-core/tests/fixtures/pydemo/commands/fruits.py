from backslash import Action, Detail, Item, List, refresh, storage


def count():
    storage.set("clicks", storage.get("clicks", 0) + 1)
    return refresh()


def items(ctx):
    clicks = storage.get("clicks", 0)
    return [
        Item("Apple", tags=[f"clicked {clicks}"], actions=[Action("Count", lambda ctx: count())]),
        Item("Banana", actions=[Action("Details", lambda ctx: Detail("# Banana"))]),
    ]


command = List(items)
