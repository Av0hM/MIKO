#!/usr/bin/python3
"""Eww adapter: JSON polling, bounded actions, and real audio frames. Never uses a shell."""
import datetime
import fcntl
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_STATE = {'online': False, 'theme': 'hud', 'cpu': {'usage': 0}, 'memory': {'used_percent': 0}, 'battery': {'percent': 0}, 'temperature': {'celsius': 0}, 'system': {'hostname': 'Connecting…', 'kernel': 'Linux'}, 'workspace': {'workspaces': []}, 'control': {'wifi_enabled': False, 'wifi_ssid': '', 'bluetooth_enabled': False, 'power_profile': 'unknown'}, 'visualizer': {'now_playing_title': '', 'now_playing_artist': '', 'now_playing_status': 'Stopped'}}
STATE = Path.home() / '.local/state/samos'
CHAT = STATE / 'desktop-chat.json'
BASE = {'user': '', 'reply': 'Ask about your system, open an app, or save a reminder.', 'busy': False,
        'pending': '', 'action': '', 'conversation_id': 'desktop'}


def emit(value):
    print(json.dumps(value, ensure_ascii=False), flush=True)


def read_json(path, default):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def save_chat(data):
    STATE.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=STATE, delete=False) as file:
        json.dump(data, file)
        name = file.name
    os.replace(name, CHAT)


def command(args, timeout=8):
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=timeout).stdout.strip()


def ctl(*args):
    binary = Path.home() / '.local/bin/samosctl'
    return command([str(binary) if binary.exists() else 'samosctl', *args])


def request(data):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(180)
        stream.connect(str(STATE / 'ai.sock'))
        stream.sendall((json.dumps(data) + '\n').encode())
        with stream.makefile('rb') as reader:
            line = reader.readline(1024 * 1024)
        return json.loads(line)


def transact(data, user=None):
    STATE.mkdir(parents=True, exist_ok=True)
    with (STATE / 'desktop-chat.lock').open('w') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return
        state = BASE | read_json(CHAT, {})
        state.update(busy=True, pending='', action='')
        if user is not None:
            state.update(user=user, reply='Thinking…')
        save_chat(state)
        try:
            result = request(data)
            state['conversation_id'] = result.get('conversation_id', state['conversation_id'])
            kind = result.get('type')
            if kind == 'ToolCall':
                tool = result['tool_call']
                state.update(pending=tool['id'], action=tool['function']['name'] + '\n' + json.dumps(tool['function']['arguments'], ensure_ascii=False), reply='Please review this action before I continue.')
            elif kind == 'Error':
                state['reply'] = result.get('message', 'Request failed')
            else:
                state['reply'] = result.get('summary') or result.get('result') or 'Done.'
                if state['reply'].startswith('CONFIRMATION_REQUIRED:'):
                    state['reply'] = 'The running backend needs the confirmation-protocol update. No action was approved.'
        except (OSError, ValueError) as error:
            state['reply'] = 'MIKO is unavailable: ' + str(error)
        finally:
            state['busy'] = False
            save_chat(state)


def compose():
    # Eww substitutes input placeholders into shell commands without escaping.
    # GTK collects text directly, so user text never becomes executable shell syntax.
    import gi
    gi.require_version('Gtk', '3.0')
    from gi.repository import Gtk, Gdk
    dialog = Gtk.Dialog(title='Ask MIKO', flags=0)
    dialog.set_default_size(520, 150)
    dialog.set_position(Gtk.WindowPosition.CENTER)
    dialog.add_button('Cancel', Gtk.ResponseType.CANCEL)
    dialog.add_button('Send', Gtk.ResponseType.OK)
    dialog.set_default_response(Gtk.ResponseType.OK)
    entry = Gtk.Entry()
    entry.set_placeholder_text('What can I help you with?')
    entry.set_activates_default(True)
    entry.set_margin_start(20); entry.set_margin_end(20)
    entry.set_margin_top(20); entry.set_margin_bottom(20)
    dialog.get_content_area().add(entry)
    css = Gtk.CssProvider()
    css.load_from_data(b'window {background:#111b2b;color:#e9eff8;} entry {background:#1d2b40;color:#e9eff8;border-radius:12px;padding:14px;} button {background:#2e66ab;color:white;padding:10px;border-radius:9px;}')
    Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    dialog.show_all(); entry.grab_focus()
    response = dialog.run(); text = entry.get_text().strip(); dialog.destroy()
    while Gtk.events_pending():
        Gtk.main_iteration()
    if response == Gtk.ResponseType.OK and text:
        state = BASE | read_json(CHAT, {})
        transact({'type': 'Chat', 'message': text, 'conversation_id': state['conversation_id']}, text)


def spectrum():
    # Frontend-owned audio stream is independent of the daemon's 1s metric snapshot.
    with tempfile.TemporaryDirectory(prefix='samos-spectrum-') as temp:
        config = Path(temp) / 'cava.conf'
        config.write_text('[general]\nframerate=30\nbars=64\n[ input ]\nmethod=pulse\nsource=auto\n[output]\nmethod=raw\nraw_target=/dev/stdout\ndata_format=ascii\nascii_max_range=100\n'.replace('[ input ]', '[input]'))
        child = None
        def stop(*_):
            if child and child.poll() is None:
                child.terminate()
            raise SystemExit(0)
        signal.signal(signal.SIGTERM, stop)
        try:
            while True:
                emit([2] * 64)
                try:
                    child = subprocess.Popen(['cava', '-p', str(config)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
                    for line in child.stdout:
                        values = [min(100, max(0, int(v))) for v in line.strip().split(';') if v.isdigit()]
                        if len(values) == 64:
                            emit(values)
                    child.wait()
                except FileNotFoundError:
                    pass
                time.sleep(3)
        finally:
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    child.kill(); child.wait()


def main():
    action = sys.argv[1]
    if action == 'state':
        data = read_json(STATE / 'state.json', {})
        online = bool(data)
        data = DEFAULT_STATE | data
        data['online'] = online and time.time() - (STATE / 'state.json').stat().st_mtime < 10
        emit(data)
    elif action == 'clock':
        now = datetime.datetime.now()
        greeting = 'Good morning.' if now.hour < 12 else 'Good afternoon.' if now.hour < 18 else 'Good evening.'
        emit({'time': now.strftime('%H:%M'), 'seconds': now.strftime('%S'), 'date': now.strftime('%A, %d %B').upper(), 'greeting': greeting})
    elif action == 'chat-state':
        emit(BASE | read_json(CHAT, {}))
    elif action == 'spectrum':
        spectrum()
    elif action == 'compose':
        compose()
    elif action == 'confirm':
        state = BASE | read_json(CHAT, {})
        if state['pending']:
            transact({'type': 'ConfirmTool', 'tool_call_id': state['pending'], 'confirmed': sys.argv[2] == 'yes', 'conversation_id': state['conversation_id']})
    elif action == 'workspace':
        command(['hyprctl', 'dispatch', 'workspace', str(int(sys.argv[2]))])
    elif action == 'media' and sys.argv[2] in ('previous', 'play-pause', 'next'):
        command(['playerctl', sys.argv[2]])
    elif action == 'control' and sys.argv[2] in ('wifi-toggle', 'bluetooth-toggle'):
        ctl(sys.argv[2])
    elif action == 'profile':
        current = command(['powerprofilesctl', 'get'])
        names = ['balanced', 'power-saver', 'performance']
        ctl('power-profile', names[(names.index(current) + 1) % len(names)] if current in names else 'balanced')
    elif action == 'voice':
        state = BASE | read_json(CHAT, {})
        transact({'type': 'Voice', 'conversation_id': state['conversation_id']}, 'Listening for 5 seconds…')

if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        data = BASE | read_json(CHAT, {})
        data.update(reply=str(error), busy=False)
        save_chat(data)
        print(error, file=sys.stderr)
        sys.exit(1)
