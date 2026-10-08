"""Local native transport fixture: real gated child, delayed visible readiness."""
import json, os, socket, subprocess, sys, time
if sys.argv[1:] == ['--version']:
    print('herdr 0.9.1')
    sys.exit()
path = os.environ['HERDR_SOCKET_PATH']
if sys.argv[1:] == ['remote-api-bridge']:
    client = socket.socket(socket.AF_UNIX)
    client.connect(path)
    client.sendall(sys.stdin.buffer.readline())
    print(client.makefile().readline(), end='')
    sys.exit()
server = socket.socket(socket.AF_UNIX)
try:
    server.bind(path)
except OSError as error:
    with open(os.path.join(os.path.dirname(__file__), 'socket-error'), 'w') as diagnostic:
        diagnostic.write('Unix socket bind: ' + error.strerror)
    raise
server.listen()
s = {}
while True:
    client, _ = server.accept()
    f = client.makefile('rw')
    request = json.loads(f.readline())
    method, params = request['method'], request.get('params', {})
    pane = dict(pane_id='w:p', workspace_id='w', tab_id='w:t', terminal_id='term', cwd=s.get('cwd'), agent='codex')
    agent = dict(pane, interactive_ready=True, agent_status='idle')
    result = {'type': 'ok'}
    if method == 'ping':
        result = dict(type='pong', version='0.9.1', capabilities=dict(workspace_create_command=True))
    elif method == 'workspace.create_command':
        child = subprocess.Popen(params['command'], cwd=params['cwd'], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=open(os.path.join(os.path.dirname(__file__), 'child-stderr'), 'wb'), start_new_session=True, env={'PATH': '/usr/bin:/bin'})
        s.update(child=child, argv=params['command'], cwd=params['cwd'])
        result = dict(type='workspace_created', workspace=dict(workspace_id='w', pane_count=1), root_pane=dict(pane_id='w:p'))
    elif method == 'pane.get':
        result = dict(pane=pane)
    elif method == 'pane.process_info':
        result = dict(process_info=dict(pane_id='w:p', foreground_processes=[dict(pid=s['child'].pid, argv=s['argv'])]))
    elif method == 'pane.send_input':
        s['child'].stdin.write(params['text'].encode())
        s['child'].stdin.flush()
        if open(os.path.join(os.path.dirname(__file__), 'setup-delay')).read() == 'true':
            deadline = time.monotonic() + 8  # inside the probe's 10 s request timeout
            while not os.path.exists(os.path.join(s['cwd'], 'setup-pending')):
                if time.monotonic() >= deadline:
                    here = os.path.dirname(__file__)
                    def read(name):
                        try:
                            return open(name, 'rb').read()[-2000:].decode(errors='replace')
                        except OSError as error:
                            return 'unreadable: ' + error.strerror
                    with open(os.path.join(here, 'socket-error'), 'w') as diagnostic:
                        diagnostic.write('setup wrapper did not start within 8 s; child exit=%r; mode=%r; cwd=%r; stderr=%r' % (
                            s['child'].poll(), read(os.path.join(s['cwd'], '..', 'mode')),
                            sorted(os.listdir(s['cwd'])) if os.path.isdir(s['cwd']) else 'missing', read(os.path.join(here, 'child-stderr'))))
                    raise RuntimeError('setup wrapper did not start')
                time.sleep(0.01)
        s['released'] = time.monotonic()
    elif method == 'agent.list':
        result = dict(type='agent_list', agents=[agent])
    elif method == 'agent.explain':
        open(os.path.join(s['cwd'], 'readiness-started'), 'a').close()
        delay = float(open(os.path.join(os.path.dirname(__file__), 'delay')).read())
        s.setdefault('readiness_started', time.monotonic())
        ready = time.monotonic() - s['readiness_started'] >= delay
        result = dict(type='agent_explain', explain=dict(agent='codex', state='idle', manifest_source='bundled', manifest_version='2026.09.14.1', matched_rule=dict(id='prompt', state='idle'), visible_idle=ready, visible_blocker=not ready, visible_working=False, screen_detection_skipped=False, skip_state_update=False, local_override_shadowing_remote=False, fallback_reason=None, warning=None))
    elif method == 'pane.read':
        result = dict(type='pane_read', text='Welcome. Trust. Press enter. Network. PRIVATE_SCREEN_TOKEN')
    elif method == 'agent.prompt':
        result = dict(type='agent_prompted', agent=agent)
    f.write(json.dumps(dict(id=request['id'], result=result)) + '\n')
    f.flush()
    client.close()
