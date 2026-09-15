#!/usr/bin/python3
"""Restore a selected installation backup, touching only files listed in its manifest."""
import argparse
import json
from pathlib import Path
import shutil
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('backup',type=Path)
parser.add_argument('--prefix',type=Path,default=Path.home())
args=parser.parse_args()
root=args.prefix.resolve()
for relative,existed in json.loads((args.backup/'manifest.json').read_text()).items():
    target=root/relative
    if Path(relative).is_absolute() or '..' in Path(relative).parts:
        raise ValueError('Invalid backup manifest path')
    if target.is_symlink():raise ValueError(f'Refusing symlink: {target}')
    if existed:
        target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(args.backup/relative,target)
    else:target.unlink(missing_ok=True)
print('Restored managed files. Run systemctl --user daemon-reload, then restart the services you want.')
