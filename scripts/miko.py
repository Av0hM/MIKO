#!/usr/bin/python3
"""Talk to the local MIKO socket; no cloud requests or shell interpolation."""
import argparse
import json
from pathlib import Path
import socket
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--conversation',default='terminal',help='Reuse an ID for conversation context')
commands=parser.add_subparsers(dest='command',required=True)
chat=commands.add_parser('chat');chat.add_argument('message')
confirm=commands.add_parser('confirm');confirm.add_argument('id')
deny=commands.add_parser('deny');deny.add_argument('id')
commands.add_parser('voice')
args=parser.parse_args()
if args.command=='chat':request={'type':'Chat','message':args.message}
elif args.command=='voice':request={'type':'Voice'}
else:request={'type':'ConfirmTool','tool_call_id':args.id,'confirmed':args.command=='confirm'}
request['conversation_id']=args.conversation
try:
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(180)
        stream.connect(str(Path.home()/'.local/state/samos/ai.sock'))
        stream.sendall((json.dumps(request)+'\n').encode())
        with stream.makefile('rb') as reader:response=json.loads(reader.readline(1024*1024))
    print(json.dumps(response,indent=2,ensure_ascii=False))
    if response.get('type')=='Error':raise SystemExit(1)
except (OSError,ValueError) as error:
    parser.exit(1,f'MIKO request failed: {error}\n')
