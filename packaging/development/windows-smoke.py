import json, os, subprocess, sys, tempfile, time
from pathlib import Path

installer = Path(sys.argv[1])
report = Path(sys.argv[2])
def windows_path(path):
    return 'Z:' + str(path).replace('/', '\\')

with tempfile.TemporaryDirectory(prefix='photocraft-windows-smoke-') as temporary:
    work = Path(temporary)
    installed = work / 'installed'
    env = dict(os.environ, WINEPREFIX=str(work / 'wine'), WINEARCH='win64', WINEDEBUG='err+all',
               WINEDLLOVERRIDES='winemenubuilder.exe=d;mscoree,mshtml=',
               XDG_CONFIG_HOME=str(work / 'config'), XDG_DATA_HOME=str(work / 'data'),
               XDG_CACHE_HOME=str(work / 'cache'))
    def run(*args, timeout=90):
        with tempfile.TemporaryFile(mode='w+t') as stdout, tempfile.TemporaryFile(mode='w+t') as stderr:
            try:
                result = subprocess.run(args, env=env, stdout=stdout, stderr=stderr, timeout=timeout)
            except subprocess.TimeoutExpired as error:
                stdout.seek(0)
                stderr.seek(0)
                raise RuntimeError(str(args) + ' timed out: ' + stdout.read()[-4000:] + stderr.read()[-4000:]) from error
            stdout.seek(0)
            stderr.seek(0)
            output, errors = stdout.read(), stderr.read()
        if result.returncode:
            raise RuntimeError(str(args) + ': ' + output + errors)
        return output
    try:
        wine_version = run('wine', '--version').strip()
        print('Preparing ' + wine_version, flush=True)
        run('wineboot', '-u')
        run('wine', 'reg', 'add', 'HKCU\\Software\\Wine', '/v', 'Version', '/d', 'win10', '/f')
        run('wine', str(installer), '/S', '/D=' + windows_path(installed))
        print('Installer completed', flush=True)
        for name in ['photocraft.exe', 'photocraft-cli.exe', 'LICENSES.txt', 'BUILD-INFO.json', 'uninstall.exe']:
            assert (installed / name).is_file(), name
        assert not (installed / 'portable.txt').exists()
        cli = str(installed / 'photocraft-cli.exe')
        version = run('wine', cli, '--version').strip()
        gui_version = run('wine', str(installed / 'photocraft.exe'), '--version').strip()
        assert version.split()[1] in gui_version, gui_version
        commands = json.loads(run('wine', cli, 'commands', '--json', '--filter', 'ai.result'))
        assert {command['id'] for command in commands} == {'ai.result.apply','ai.result.list','ai.result.discard','ai.result.openDocument'}
        all_commands = {command['id'] for command in json.loads(run('wine', cli, 'commands', '--json'))}
        assert {'edit.generateImage','edit.aiEdit','edit.generativeFill','edit.generativeRemove',
                'edit.aiBackground','layer.aiRemoveBackground','image.generativeExpand','image.aiUpscale',
                'edit.preferences.integrations'} <= all_commands
        document = work / 'installer-test.pcraft'
        run('wine', cli, 'run', '--new', '{"width":16,"height":16,"background":"white"}',
            '--cmd', 'layer.new.layer', '--params', '{"name":"Installer-Test"}', '--out', windows_path(document))
        assert document.stat().st_size > 0
        info = json.loads(run('wine', cli, 'info', windows_path(document), '--compact'))
        assert 'Installer-Test' in json.dumps(info), info
        registry = run('wine', 'reg', 'query', 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PhotoCraft-Development')
        assert 'DisplayVersion' in registry and 'UninstallString' in registry
        assert '"' + windows_path(installed) + '\\uninstall.exe"' in registry, registry
        note = installed / 'user-file.txt'
        note.write_text('Preserve this user file during uninstall.\n')
        run('wine', str(installed / 'uninstall.exe'), '/S')
        deadline = time.monotonic() + 20
        removed_files = ['photocraft.exe', 'photocraft-cli.exe', 'LICENSES.txt', 'BUILD-INFO.json', 'uninstall.exe']
        while any((installed / name).exists() for name in removed_files) and time.monotonic() < deadline:
            time.sleep(0.1)
        for name in removed_files:
            assert not (installed / name).exists(), name
        assert note.read_text() == 'Preserve this user file during uninstall.\n'
        absent = subprocess.run(['wine','reg','query','HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PhotoCraft-Development'],env=env,capture_output=True,timeout=15)
        assert absent.returncode != 0
        data = {'wine':wine_version,'application':version,'installer':'passed','uninstall':'passed',
                'userFilePreserved':'passed','registry':'passed','aiCommands':'passed','nativeFormatRoundtrip':'passed',
                'guiExecutableLoads':'passed', 'guiVersion':gui_version,
                'nativeWindowsTesting':'not performed'}
        report.write_text(json.dumps(data,indent=2)+'\n')
        print(json.dumps(data))
    finally:
        subprocess.run(['wineserver','-k'],env=env,capture_output=True,timeout=10)
