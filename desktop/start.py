#!/usr/bin/python3
"""Start only SamOS windows on selected monitor names; no compositor configuration edits."""
import argparse
import json
from pathlib import Path
import subprocess
import time
import signal
import gi
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk

parser = argparse.ArgumentParser()
parser.add_argument('--config', type=Path, default=Path.home()/'.config/eww-samos')
parser.add_argument('--monitor', default='eDP-1')
parser.add_argument('--watch', action='store_true')
args = parser.parse_args()
signal.signal(signal.SIGTERM, lambda *_: exit(0))
base = ['eww', '--config', str(args.config)]
subprocess.run(base + ['daemon'], check=True)
old = None
try:
    while True:
        monitors = json.loads(subprocess.check_output(['hyprctl','-j','monitors'],text=True))
        display = Gdk.Display.get_default()
        selected = []
        for monitor in monitors:
            if args.monitor != 'all' and monitor['name'] != args.monitor:
                continue
            for index in range(display.get_n_monitors()):
                geometry = display.get_monitor(index).get_geometry()
                if (geometry.x, geometry.y) == (monitor['x'], monitor['y']):
                    selected.append((monitor['name'], index))
                    break
        current = tuple(selected)
        if current != old:
            subprocess.run(base + ['close-all'], check=True)
            for name, index in selected:
                for widget in ['identity','clock','player','miko','spectrum']:
                    subprocess.run(base + ['open', f'samos_{widget}', '--id', f'{widget}-{name}', '--arg', f'screen={index}'], check=True)
            old = current
        if not args.watch:
            break
        time.sleep(3)
finally:
    if args.watch:
        subprocess.run(base + ['kill'])
