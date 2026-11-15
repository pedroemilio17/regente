#!/usr/bin/env python3
"""Measure an isolated native Tauri window with four fake agents; Linux/X11 only."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def api(base, path, data=None):
    payload = None if data is None else json.dumps(data).encode()
    req = urllib.request.Request(base + '/api/v1' + path, data=payload, headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=3) as response:
        return json.load(response)


def wait_until(fn, seconds, description):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = fn()
            if value:
                return value
        except (OSError, ValueError, urllib.error.URLError, subprocess.CalledProcessError):
            pass
        time.sleep(.1)
    raise RuntimeError('Timeout: ' + description)


def stat(pid):
    text = Path(f'/proc/{pid}/stat').read_text()
    fields = text[text.rfind(')') + 2:].split()
    return int(fields[1]), int(fields[11]) + int(fields[12])


def descendants(root):
    table = {}
    for entry in Path('/proc').iterdir():
        if entry.name.isdigit():
            try:
                parent, ticks = stat(int(entry.name))
                table[int(entry.name)] = (parent, ticks)
            except (OSError, ValueError, IndexError):
                pass
    found = {root}
    while True:
        new = found | {pid for pid, (parent, _) in table.items() if parent in found}
        if new == found:
            break
        found = new
    return {pid: table[pid][1] for pid in found if pid in table}


def describe_processes(pids):
    result = {}
    for pid in pids:
        try:
            result[str(pid)] = Path(f'/proc/{pid}/comm').read_text().strip()
        except OSError:
            result[str(pid)] = '<exited>'
    return result


def window_for(pid):
    out = subprocess.check_output(['xprop', '-root', '_NET_CLIENT_LIST'], text=True)
    for window in re.findall(r'0x[0-9a-fA-F]+', out):
        props = subprocess.check_output(['xprop', '-id', window, '_NET_WM_PID'], text=True)
        if re.search(r'=\s*' + str(pid) + r'\s*$', props):
            return window
    return None


def hidden(window):
    props = subprocess.check_output(['xprop', '-id', window, '_NET_WM_STATE'], text=True)
    return '_NET_WM_STATE_HIDDEN' in props


def minimize(window):
    x11 = ctypes.CDLL('libX11.so.6')
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XDefaultScreen.argtypes = [ctypes.c_void_p]
    x11.XDefaultScreen.restype = ctypes.c_int
    x11.XIconifyWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError('XOpenDisplay failed')
    try:
        if not x11.XIconifyWindow(display, int(window, 16), x11.XDefaultScreen(display)):
            raise RuntimeError('XIconifyWindow failed')
        x11.XFlush(display)
    finally:
        x11.XCloseDisplay(display)


def terminate(proc):
    if proc and proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--app', required=True, type=Path, help='Extracted native application binary')
    parser.add_argument('--daemon', type=Path, default=ROOT / 'target/debug/regented')
    parser.add_argument('--seconds', type=float, default=30)
    parser.add_argument('--intel-rc6', type=Path, help='Optional Intel GPU rc6_residency_ms counter path')
    args = parser.parse_args()
    if args.seconds <= 0:
        parser.error('--seconds must be positive')
    if not os.environ.get('DISPLAY'):
        raise RuntimeError('DISPLAY is required; a window manager supporting _NET_WM_STATE_HIDDEN is required')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    daemon = app = None
    agent_ids = []
    with tempfile.TemporaryDirectory(prefix='regente-native-perf-') as temp:
        temp = Path(temp)
        repo = temp / 'repo'
        repo.mkdir()
        for cmd in (['git', 'init', '-b', 'main', str(repo)], ['git', '-C', str(repo), 'config', 'user.name', 'Regente Test'], ['git', '-C', str(repo), 'config', 'user.email', 'test@example.invalid']):
            subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
        (repo / 'README.md').write_text('# Native performance\n')
        subprocess.run(['git', '-C', str(repo), 'add', 'README.md'], check=True)
        subprocess.run(['git', '-C', str(repo), 'commit', '-m', 'initial'], check=True, stdout=subprocess.DEVNULL)
        env = os.environ.copy()
        env.update({'REGENTE_HOME': str(temp / 'home'), 'REGENTE_PORT': str(port), 'REGENTE_UI_DIR': str(ROOT / 'ui/dist'), 'REGENTE_FIXTURES': str(ROOT / 'fixtures'), 'REGENTE_FAKE_STEP_MS': '50', 'GDK_BACKEND': 'x11'})
        try:
            daemon = subprocess.Popen([str(args.daemon.resolve())], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            wait_until(lambda: api(base, '/health').get('ok'), 15, 'daemon health')
            with urllib.request.urlopen(base + '/', timeout=3) as response:
                if response.status != 200 or 'text/html' not in response.headers.get('Content-Type', ''):
                    raise RuntimeError('UI not served')
            agents = [{'name': f'Perf {n}', 'cli': 'fake', 'is_orchestrator': n == 1, 'settings': {'fake_scenario': 'performance'}} for n in range(1, 5)]
            project = api(base, '/projects', {'name': 'Performance', 'root': str(repo), 'base_branch': 'main', 'agents': agents})
            project_id = project['project']['id']
            agent_ids = [agent['id'] for agent in project['agents']]
            app = subprocess.Popen([str(args.app.resolve())], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            window = wait_until(lambda: window_for(app.pid), 20, 'Tauri window and matching PID')
            for agent_id in agent_ids:
                api(base, '/tasks', {'project_id': project_id, 'agent_id': agent_id, 'title': 'Performance', 'budget_usd': 1000})
            def active():
                snapshot = api(base, '/snapshot')
                return sum(a['id'] in agent_ids and a['live']['status'] in ('working', 'thinking') for a in snapshot['agents'])
            wait_until(lambda: active() == 4, 15, 'four active agents')
            minimize(window)
            wait_until(lambda: hidden(window), 5, 'window manager confirmation of minimization')
            # Allow WebKit children and rendering to settle before the baseline.
            time.sleep(5)
            if active() != 4 or not hidden(window):
                raise RuntimeError('Agents or window state changed during warmup')
            start_tree = descendants(app.pid)
            start_processes = describe_processes(start_tree)
            if not any(name.startswith('WebKitWebProces') for name in start_processes.values()):
                raise RuntimeError(f'WebKit web process absent from baseline: {start_processes}')
            def rc6():
                if args.intel_rc6 is None:
                    return None
                try:
                    return int(args.intel_rc6.read_text().strip())
                except (OSError, ValueError):
                    return None
            start_rc6 = rc6()
            start_daemon = stat(daemon.pid)[1]
            start = time.monotonic()
            samples = []
            all_hidden = True
            while time.monotonic() - start < args.seconds:
                samples.append(active())
                all_hidden = all_hidden and hidden(window)
                time.sleep(min(1, max(0, args.seconds - (time.monotonic() - start))))
            elapsed = time.monotonic() - start
            end_tree = descendants(app.pid)
            end_processes = describe_processes(end_tree)
            end_daemon = stat(daemon.pid)[1]
            end_rc6 = rc6()
            hz = os.sysconf('SC_CLK_TCK')
            ticks = sum(max(0, end_tree[pid] - start_tree[pid]) for pid in start_tree.keys() & end_tree.keys())
            print(json.dumps({'elapsed_seconds': elapsed, 'window': window, 'window_pid': app.pid, 'warmup_seconds': 5, 'processes_start': start_processes, 'processes_end': end_processes, 'new_processes': describe_processes(end_tree.keys() - start_tree.keys()), 'exited_processes': {str(pid): start_processes[str(pid)] for pid in start_tree.keys() - end_tree.keys()}, 'hidden_confirmed_all_samples': all_hidden, 'active_samples': samples, 'all_samples_four_active': all(n == 4 for n in samples), 'tauri_webkit_percent_one_core': ticks / hz / elapsed * 100, 'daemon_percent_one_core': (end_daemon - start_daemon) / hz / elapsed * 100, 'gpu_utilization': 'not measured', 'intel_gpu_rc6_idle_residency_ratio': (end_rc6 - start_rc6) / (elapsed * 1000) if start_rc6 is not None and end_rc6 is not None and end_rc6 >= start_rc6 else None, 'intel_rc6_counter_path': str(args.intel_rc6) if args.intel_rc6 else None, 'limitations': 'Only processes present at both samples count; subprocesses created or exited during measurement are excluded. CPU is percent of one core; GPU utilization was not measured. Intel RC6 is a global GPU idle residency counter and may include other applications; its complement is not GPU utilization.'}, indent=2))
        finally:
            for agent_id in agent_ids:
                try:
                    api(base, f'/agents/{agent_id}/pause', {'reason': 'Fim da medição'})
                except (OSError, urllib.error.URLError):
                    pass
            terminate(app)
            terminate(daemon)


if __name__ == '__main__':
    main()
