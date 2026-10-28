from backslash import Item, List


def search(query, ctx):
    print(f"searching {query}")
    if query == "slow":
        ctx.cancelled.wait(30)
    return [Item(f"You typed {query}")]


command = List(search=search)
