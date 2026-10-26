import os, socket, subprocess, sys, tempfile, time
from pathlib import Path

with tempfile.TemporaryDirectory(prefix='photocraft-display-') as temporary:
    log_path = Path(temporary) / 'xvfb.log'
    with log_path.open('w') as log:
        display = subprocess.Popen(['Xvfb',':99','-screen','0','1440x1000x24','-nolisten','tcp','-ac'],stdout=log,stderr=log)
        try:
            deadline = time.monotonic() + 20
            ready = False
            while time.monotonic() < deadline:
                if display.poll() is not None:
                    raise RuntimeError('Virtual display exited: '+log_path.read_text())
                try:
                    with socket.socket(socket.AF_UNIX) as probe:
                        probe.connect('/tmp/.X11-unix/X99')
                    ready = True
                    break
                except OSError:
                    time.sleep(0.1)
            if not ready:
                raise RuntimeError('Virtual display did not start: '+log_path.read_text())
            env = dict(os.environ, DISPLAY=':99')
            result = subprocess.run(sys.argv[1:],env=env,timeout=int(env.get('PHOTOCRAFT_SMOKE_TIMEOUT', '180')))
        finally:
            display.terminate()
            display.wait(timeout=5)
    sys.exit(result.returncode)
