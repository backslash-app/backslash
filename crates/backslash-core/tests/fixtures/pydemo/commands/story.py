from backslash import Detail


def tell(ctx):
    text = ""
    for word in ["Once", "upon", "a", "time"]:
        text += word + " "
        yield text.strip()


command = Detail(tell, title="Story")
