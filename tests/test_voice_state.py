"""Voice state compatibility across the frontend's two initialization paths."""
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('desktop', ROOT / 'desktop/eww/scripts/desktop.py')
desktop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop)


class VoiceState(unittest.TestCase):
    def test_yuck_and_python_initial_ai_state_agree(self):
        first_line = (ROOT / 'desktop/eww/eww.yuck').read_text().splitlines()[0]
        encoded = re.search(r':initial ("(?:\\.|[^"\\])*")', first_line).group(1)
        self.assertEqual(json.loads(json.loads(encoded))['ai'], desktop.DEFAULT_STATE['ai'])

    def test_old_exports_and_stale_audio_flags_are_safe(self):
        with tempfile.TemporaryDirectory() as tmp:
            state = Path(tmp)
            path = state / 'state.json'
            path.write_text(json.dumps({'ai': {'model': 'legacy', 'status': 'idle'}}))
            with patch.object(desktop, 'STATE', state), patch.object(desktop.sys, 'argv', ['desktop.py', 'state']), patch.object(desktop, 'emit') as emit:
                desktop.main()
                self.assertFalse(emit.call_args.args[0]['ai']['speaking'])
                self.assertEqual(emit.call_args.args[0]['ai']['model'], 'legacy')
                path.write_text(json.dumps({'ai': {'speaking': True}}))
                with patch.object(desktop.time, 'time', return_value=path.stat().st_mtime + 11):
                    desktop.main()
                self.assertFalse(emit.call_args.args[0]['online'])
                self.assertFalse(emit.call_args.args[0]['ai']['speaking'])


if __name__ == '__main__':
    unittest.main()
