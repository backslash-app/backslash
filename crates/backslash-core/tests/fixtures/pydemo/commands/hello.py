def command(ctx):
    return f"Hello {ctx.args.get('name', 'world')}"
