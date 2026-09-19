#!/usr/bin/python3
"""Send a local request to MIKO's Unix socket."""
import argparse
import json
from pathlib import Path
import socket

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--conversation", help="Conversation ID (voice: miko; other commands: terminal)")
commands = parser.add_subparsers(dest="command", required=True)
chat = commands.add_parser("chat"); chat.add_argument("message")
confirm = commands.add_parser("confirm"); confirm.add_argument("id")
deny = commands.add_parser("deny"); deny.add_argument("id")
voice = commands.add_parser("voice")
voice.add_argument("--speak", action="store_true", help="Play the reply through local Piper after this recording")
commands.add_parser("history")
forget = commands.add_parser("forget")
forget.add_argument("--yes", action="store_true", help="Confirm deletion of this conversation's history")
args = parser.parse_args()

if args.command == "chat":
    request = {"type": "Chat", "message": args.message}
elif args.command == "voice":
    request = {"type": "Voice", "speak_reply": args.speak}
elif args.command == "history":
    request = {"type": "History"}
elif args.command == "forget":
    if not args.yes:
        parser.error("Review with history, then use forget --yes to confirm deletion")
    request = {"type": "ForgetHistory", "confirmed": True}
else:
    request = {"type": "ConfirmTool", "tool_call_id": args.id, "confirmed": args.command == "confirm"}
request["conversation_id"] = args.conversation or ("miko" if args.command == "voice" else "terminal")

try:
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(180)
        stream.connect(str(Path.home() / ".local/state/samos/ai.sock"))
        stream.sendall((json.dumps(request) + "\n").encode())
        with stream.makefile("rb") as reader:
            response = json.loads(reader.readline(1024 * 1024))
    print(json.dumps(response, indent=2, ensure_ascii=False))
    if response.get("type") == "Error":
        raise SystemExit(1)
except (OSError, ValueError) as error:
    parser.exit(1, f"MIKO request failed: {error}\n")
