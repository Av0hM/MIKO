import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
spec=importlib.util.spec_from_file_location('installer',Path(__file__).resolve().parents[1]/'install/install.py')
installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
class Installation(unittest.TestCase):
    def test_preserve_and_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            existing=root/'.config/eww/eww.yuck';existing.parent.mkdir(parents=True);existing.write_text('keep original desktop')
            config=root/'.config/samos/config.toml';config.parent.mkdir(parents=True);config.write_text('theme="hacker"\nrefresh_ms=500\n')
            managed=root/'.config/eww-samos/eww.yuck';managed.parent.mkdir(parents=True);managed.write_text('old SamOS')
            backup=installer.install(root)
            self.assertEqual(existing.read_text(),'keep original desktop')
            self.assertIn('refresh_ms=500',config.read_text())
            subprocess.run(['/usr/bin/python3',str(installer.ROOT/'install/restore.py'),str(backup),'--prefix',str(root)],check=True)
            self.assertEqual(managed.read_text(),'old SamOS')
            self.assertFalse((root/'.local/bin/samosctl').exists())
if __name__=='__main__':unittest.main()
