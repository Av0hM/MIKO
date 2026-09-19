#!/usr/bin/python3
"""Install explicit SamOS files with per-file backups; never mirror/delete config trees."""
import argparse
import datetime
import json
from pathlib import Path
import shutil
import subprocess

ROOT=Path(__file__).resolve().parents[1]

def files(root):
    result={}
    assets=ROOT/'desktop/eww'
    for source in assets.rglob('*'):
        if source.is_file() and '__pycache__' not in source.parts:
            result[root/'.config/eww-samos'/source.relative_to(assets)]=source
    result[root/'.local/share/samos/desktop/start.py']=ROOT/'desktop/start.py'
    for name in ['samosd','samosctl']:
        result[root/'.local/bin'/name]=ROOT/'target/release'/name
    return result

def install(root,activate=False,whisper_binary=None):
    mapping=files(root)
    if whisper_binary is not None:
        mapping[root/'.local/bin/whisper-cli'] = Path(whisper_binary)
    for path in mapping.values():
        if not path.is_file():
            raise RuntimeError(f'Missing installation asset: {path}. Build samosd and samosctl first.')
    backup=root/'.local/state/samos/backups'/datetime.datetime.now().strftime('%Y%m%d-%H%M%S-%f')
    backup.mkdir(parents=True)
    manifest={}
    def write(destination,content=None,source=None):
        relative=str(destination.relative_to(root))
        if destination.is_symlink():
            raise RuntimeError(f'Refusing to overwrite symlink: {destination}')
        manifest[relative]=destination.exists()
        if destination.exists():
            saved=backup/relative;saved.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(destination,saved)
        destination.parent.mkdir(parents=True,exist_ok=True)
        if source:
            if source.resolve()!=destination.resolve(): shutil.copy2(source,destination)
        else:
            destination.write_text(content)
        (backup/'manifest.json').write_text(json.dumps(manifest,indent=2))
    for destination,source in mapping.items():write(destination,source=source)
    write(root/'.config/systemd/user/samosd.service',f'''[Unit]
Description=SamOS system monitoring daemon
[Service]
ExecStart={ROOT}/target/release/samosd
WorkingDirectory={ROOT}
Environment=SAMOS_MODEL=qwen2.5:1.5b
Restart=on-failure
RestartSec=3
TimeoutStopSec=8
[Install]
WantedBy=default.target
''')
    write(root/'.config/systemd/user/samos-desktop.service','''[Unit]
Description=SamOS desktop corner widgets
After=graphical-session.target samosd.service
PartOf=graphical-session.target
[Service]
ExecStart=/usr/bin/python3 %h/.local/share/samos/desktop/start.py --watch --monitor eDP-1
ExecStop=/usr/bin/eww --config %h/.config/eww-samos kill
Restart=on-failure
RestartSec=3
TimeoutStopSec=8
[Install]
WantedBy=graphical-session.target
''')
    write(root/'.config/systemd/user/samos-ollama.service','''[Unit]
Description=Local Ollama server for MIKO
[Service]
ExecStart=/usr/bin/ollama serve
Environment=OLLAMA_HOST=127.0.0.1:11434
Environment=OLLAMA_KEEP_ALIVE=2m
Environment=OLLAMA_NUM_PARALLEL=1
Restart=on-failure
RestartSec=5
TimeoutStopSec=8
[Install]
WantedBy=default.target
''')
    config=root/'.config/samos/config.toml'
    if not config.exists():write(config,'theme="hud"\nmonitor="eDP-1"\nrefresh_ms=1000\n')
    for theme in ['hud','hacker','elegant','motivation','love','movie']:
        destination=root/'.config/samos/themes'/f'{theme}.toml'
        if not destination.exists():write(destination,f'name="{theme}"\n')
    print(f'Installed SamOS. Backup and restore manifest: {backup}')
    if activate:
        if root!=Path.home():raise RuntimeError('Activation only supports the current home directory')
        subprocess.run(['systemctl','--user','import-environment','WAYLAND_DISPLAY','DISPLAY','HYPRLAND_INSTANCE_SIGNATURE'],check=True)
        subprocess.run(['systemctl','--user','daemon-reload'],check=True)
        subprocess.run(['systemctl','--user','enable','--now','samosd.service','samos-desktop.service','samos-ollama.service'],check=True)
        subprocess.run(['systemctl','--user','restart','samos-desktop.service'],check=True)
    return backup

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prefix',type=Path,default=Path.home())
    parser.add_argument('--activate',action='store_true')
    parser.add_argument('--whisper-binary',type=Path)
    args=parser.parse_args()
    install(args.prefix.resolve(),args.activate,args.whisper_binary)
