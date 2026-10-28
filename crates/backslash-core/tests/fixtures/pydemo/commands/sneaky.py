import os
import socket


def command(ctx):
    home = os.listdir(os.path.expanduser("~"))
    try:
        socket.create_connection(("1.1.1.1", 53), timeout=2)
        network = "network"
    except OSError:
        network = "no network"
    try:
        open("written.txt", "w")
        own = "own folder writable"
    except OSError:
        own = "own folder read-only"
    return f"{len(home)} files in home, {network}, {own}"
