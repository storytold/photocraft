import base64, json, os, secrets, socket, subprocess, sys, tempfile, time
from pathlib import Path

binary, output = Path(sys.argv[1]), Path(sys.argv[2])
endpoint = sys.argv[3] if len(sys.argv) > 3 else None
prompt_model = sys.argv[4] if len(sys.argv) > 4 else None
prompt_endpoint = sys.argv[5] if len(sys.argv) > 5 else None
if prompt_model and (not endpoint or not prompt_endpoint):
    raise ValueError('Automatic prompt smoke test needs explicit image and prompt endpoints')
with tempfile.TemporaryDirectory(prefix='photocraft-gui-smoke-') as temporary:
    work = Path(temporary)
    token = secrets.token_hex(32)
    token_file = work / 'control.token'
    token_file.write_text(token)
    token_file.chmod(0o600)
    runtime = work / 'runtime'
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, XDG_CONFIG_HOME=str(work / 'config'), XDG_CACHE_HOME=str(work / 'cache'),
               XDG_DATA_HOME=str(work / 'data'), XDG_RUNTIME_DIR=str(runtime),
               PHOTOCRAFT_NO_DESKTOP_INTEGRATION='1', RUST_LOG='warn')
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    with (work / 'app.log').open('w') as log:
        process = subprocess.Popen([str(binary), '--safe-gpu', '--control', str(port),
                                    '--control-token-file', str(token_file), '--automation-read-root', str(work), '--automation-write-root', str(work)], env=env, stdout=log, stderr=log)
        try:
            connection = None
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError('Application exited: ' + (work / 'app.log').read_text())
                try:
                    connection = socket.create_connection(('127.0.0.1', port), timeout=1)
                    break
                except OSError:
                    time.sleep(0.2)
            if connection is None:
                raise RuntimeError('No control server: ' + (work / 'app.log').read_text())
            connection.settimeout(30)
            stream = connection.makefile('rwb')
            count = 0
            def call(method, params):
                global count
                count += 1
                stream.write((json.dumps({'id':count,'method':method,'params':params})+'\n').encode())
                stream.flush()
                response = json.loads(stream.readline())
                if not response.get('ok'):
                    raise RuntimeError(method + ': ' + str(response))
                return response.get('result')
            call('auth', {'token':token})
            # --safe-gpu deliberately starts in CPU mode; acknowledge its startup notice
            # before inspecting dialogs so it does not cover the context-menu screenshot.
            initial = call('ui.inspect', {})
            if initial.get('gpuFallbackNotice'):
                width, height = initial['window']['width'], initial['window']['height']
                for offset in [95,75,55,115]:
                    call('ui.click', {'x':width/2-110,'y':height/2+offset})
                    if not call('ui.inspect', {}).get('gpuFallbackNotice'):
                        break
            call('engine.execute', {'id':'prefs.set','params':{'path':'interface.language','value':'de'}})
            call('engine.execute', {'id':'file.new','params':{'width':512,'height':512,'background':'white','depth':16}})
            result = call('engine.execute', {'id':'ai.result.list','params':{}})
            call('ui.menu.invoke', {'id':'edit.preferences.integrations'})
            inspected = call('ui.inspect', {})
            dialog = inspected['dialogs'][-1]
            assert dialog['fields']['section'] == 'integrations', dialog
            screenshot = call('ui.screenshot', {'focus':False})
            output.write_bytes(base64.b64decode(screenshot['base64']))
            assert screenshot['width'] >= 800 and screenshot['height'] >= 500
            call('ui.dialog.cancel', {'dialog':dialog['id']})
            forms = []
            call('engine.execute', {'command':'select.all','params':{}})
            call('ui.pointer', {'tool':'RectMarquee','button':'secondary',
                                'events':[{'kind':'down','x':64,'y':64},{'kind':'up','x':64,'y':64}]})
            entries = call('ui.inspect', {})['canvasToolMenu']['entries']
            assert all(any(e.get('id') == command and e.get('enabled') for e in entries)
                       for command in ['edit.generativeFill','edit.generativeRemove']), entries
            context_image = call('ui.screenshot', {'focus':False})
            output.with_name('linux-ai-context-menu.png').write_bytes(base64.b64decode(context_image['base64']))
            context_dialog = call('ui.context.choose', {'id':'edit.generativeFill'})
            call('ui.dialog.cancel', {'dialog':context_dialog['dialog']})
            for command in ['edit.generateImage','edit.aiEdit','edit.generativeFill','edit.generativeRemove',
                            'edit.aiBackground','layer.aiRemoveBackground','image.generativeExpand','image.aiUpscale']:
                opened = call('ui.menu.invoke', {'id':command})
                call('ui.dialog.cancel', {'dialog':opened['dialog']})
                forms.append(command)
            call('engine.execute', {'command':'select.deselect','params':{}})
            if endpoint:
                call('engine.execute', {'command':'prefs.set','params':{'path':'integrations.comfyuiEndpoint','value':endpoint}})
            if prompt_model:
                call('engine.execute', {'command':'prefs.set','params':{'path':'integrations.openaiPromptEndpoint','value':prompt_endpoint}})
                call('engine.execute', {'command':'prefs.set','params':{'path':'integrations.openaiPromptApiKeyEnv','value':'PHOTOCRAFT_SMOKE_PROMPT_KEY'}})
            opened = call('ui.menu.invoke', {'id':'edit.generateImage'})
            call('ui.dialog.set', {'dialog':opened['dialog'],'field':'__sizePreset','value':'4k'})
            call('ui.dialog.set', {'dialog':opened['dialog'],'field':'resolutionMode','value':'native'})
            call('ui.dialog.set', {'dialog':opened['dialog'],'field':'prompt',
                                   'value':'Eine blaue Keramiktasse vor einem beigen Hintergrund, weiches Studiolicht, kein Text.'})
            if prompt_model:
                call('ui.dialog.set', {'dialog':opened['dialog'],'field':'preparePrompt','value':True})
                call('ui.dialog.set', {'dialog':opened['dialog'],'field':'textModel','value':prompt_model})
            form_image = call('ui.screenshot', {'focus':False})
            output.with_name('linux-ai-form.png').write_bytes(base64.b64decode(form_image['base64']))
            configured = next(d for d in call('ui.inspect', {})['dialogs'] if d['id'] == opened['dialog'])
            assert configured['fields']['outputSize'] == {'width':3840,'height':2160}, configured
            assert configured['fields']['detailUpscale'] is False, configured
            if prompt_model:
                assert configured['fields']['textModel'] == prompt_model, configured
                assert configured['fields']['preparePrompt'] is True, configured
            live = 'not requested'
            if endpoint:
                started = call('ui.dialog.confirm', {'dialog':opened['dialog'],'wait':False})
                assert started.get('pending') and started.get('job'), started
                deadline = time.monotonic() + 3600
                while time.monotonic() < deadline:
                    jobs = call('jobs.list', {})
                    job = next((j for j in jobs['jobs'] if j['id'] == started['job']), None)
                    if job and job['state'] == 'done':
                        break
                    if job and job['state'] in ('failed', 'cancelled'):
                        raise RuntimeError('AI GUI job: ' + str(job))
                    time.sleep(0.5)
                else:
                    call('jobs.cancel', {'job':started['job']})
                    raise RuntimeError('AI GUI job timed out')
                rows = call('engine.execute', {'command':'ai.result.list','params':{}})['results']
                assert len(rows) == 1 and not rows[0]['stale'], rows
                assert rows[0]['outputSize'] == {'width':3840,'height':2160}, rows
                assert rows[0]['workingSize'] == rows[0]['outputSize'], rows
                assert rows[0]['detailUpscaler'] is None and not rows[0]['resampled'], rows
                call('app.save', {'path':'renamed-source.pcraft'})
                assert not call('engine.execute', {'command':'ai.result.list','params':{}})['results'][0]['stale']
                inspected = call('ui.inspect', {})
                assert any(d['fields'].get('__ai') == 'results' for d in inspected['dialogs']), inspected['dialogs']
                preview = call('ui.screenshot', {'focus':False})
                output.with_name('linux-ai-preview.png').write_bytes(base64.b64decode(preview['base64']))
                call('engine.execute', {'command':'ai.result.apply','params':{'result':rows[0]['result']}})
                call('engine.execute', {'command':'edit.undo','params':{}})
                call('engine.execute', {'command':'edit.redo','params':{}})
                # Separate Upscale starts from the applied, native 4K image.
                call('app.save', {'path':'direct-four-k.pcraft'})
                print(json.dumps({'nativeGeneration':rows[0],'saveAsIdentity':'passed','applyUndoRedo':'passed'}),flush=True)
                for dialog in call('ui.inspect', {})['dialogs']:
                    if dialog['fields'].get('__ai') == 'results':
                        call('ui.dialog.cancel', {'dialog':dialog['id']})
                upscale = call('ui.menu.invoke', {'id':'image.aiUpscale'})
                call('ui.dialog.set', {'dialog':upscale['dialog'],'field':'__upscalePreset','value':'2'})
                upscale_image = call('ui.screenshot', {'focus':False})
                output.with_name('linux-ai-upscale-form.png').write_bytes(base64.b64decode(upscale_image['base64']))
                started_upscale = call('ui.dialog.confirm', {'dialog':upscale['dialog'],'wait':False})
                assert started_upscale.get('pending') and started_upscale.get('job'), started_upscale
                deadline = time.monotonic() + 1800
                while time.monotonic() < deadline:
                    jobs = call('jobs.list', {})
                    job = next((j for j in jobs['jobs'] if j['id'] == started_upscale['job']), None)
                    if job and job['state'] == 'done':
                        break
                    if job and job['state'] in ('failed','cancelled'):
                        raise RuntimeError('Upscale GUI job: '+str(job))
                    time.sleep(0.5)
                else:
                    call('jobs.cancel', {'job':started_upscale['job']})
                    raise RuntimeError('Upscale GUI job timed out')
                up_rows = call('engine.execute', {'command':'ai.result.list','params':{}})['results']
                up = next(row for row in up_rows if row['operation'] == 'upscale')
                assert up['workingSize'] == {'width':3840,'height':2160}, up
                assert up['outputSize'] == {'width':7680,'height':4320}, up
                assert up['detailUpscaler'] == 'realesrgan-x4plus' and not up['stale'], up
                up_preview = call('ui.screenshot', {'focus':False})
                output.with_name('linux-ai-upscale-preview.png').write_bytes(base64.b64decode(up_preview['base64']))
                call('engine.execute', {'command':'ai.result.apply','params':{'result':up['result'],'hideOriginals':True}})
                call('engine.execute', {'command':'edit.undo','params':{}})
                call('engine.execute', {'command':'edit.redo','params':{}})
                call('app.save', {'path':'upscaled-eight-k.pcraft'})
                # A real edit protects the target; the captured result remains recoverable.
                call('engine.execute', {'command':'layer.new.layer','params':{'name':'Later edit'}})
                stale = next(row for row in call('engine.execute', {'command':'ai.result.list','params':{}})['results'] if row['result'] == up['result'])
                assert stale['stale'], stale
                stale_preview = call('ui.screenshot', {'focus':False})
                output.with_name('linux-ai-stale-recovery.png').write_bytes(base64.b64decode(stale_preview['base64']))
                recovered = call('engine.execute', {'command':'ai.result.openDocument','params':{'result':up['result'],'name':'Recovered Upscale'}})
                assert recovered.get('document') and recovered['document'] != up['document'], recovered
                call('app.save', {'path':'recovered-eight-k.pcraft'})
                call('app.open', {'path':'recovered-eight-k.pcraft'})
                live = 'passed: native 4K generation (no upscaler), Save As identity, separate 4K-to-8K Upscale, previews, apply, Undo/Redo, native save/reopen, stale recovery'
                print(json.dumps({'nativeGeneration':rows[0],'standaloneUpscale':up,'recovery':recovered}),flush=True)
            else:
                call('ui.dialog.cancel', {'dialog':opened['dialog']})
            call('app.quit', {})
            process.wait(timeout=15)
            print(json.dumps({'guiLaunch':'passed','newDocument':'passed','aiResultCommand':'passed',
                              'apiSettingsDialog':'passed','aiForms':forms,'liveWorkflow':live,'screenshot':str(output),
                              'automaticPromptModel':prompt_model,
                              'dimensions':[screenshot['width'],screenshot['height']]}))
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
