#!/usr/bin/env python3
"""Test packaged Tauri startup and independent daemon persistence on X11."""
import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request

spec = importlib.util.spec_from_file_location('native_performance', Path(__file__).with_name('native-performance.py'))
perf = importlib.util.module_from_spec(spec)
spec.loader.exec_module(perf)


class ClientData(ctypes.Union):
    _fields_ = [('b', ctypes.c_char * 20), ('s', ctypes.c_short * 10), ('l', ctypes.c_long * 5)]


class ClientMessage(ctypes.Structure):
    _fields_ = [('type', ctypes.c_int), ('serial', ctypes.c_ulong), ('send_event', ctypes.c_int), ('display', ctypes.c_void_p), ('window', ctypes.c_ulong), ('message_type', ctypes.c_ulong), ('format', ctypes.c_int), ('data', ClientData)]


class XEvent(ctypes.Union):
    _fields_ = [('client', ClientMessage), ('pad', ctypes.c_long * 24)]


def close_window(window):
    x = ctypes.CDLL('libX11.so.6')
    x.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x.XOpenDisplay.restype = ctypes.c_void_p
    x.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    x.XInternAtom.restype = ctypes.c_ulong
    x.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.POINTER(XEvent)]
    x.XFlush.argtypes = [ctypes.c_void_p]
    x.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = x.XOpenDisplay(None)
    if not display:
        raise RuntimeError('Cannot open X display')
    try:
        event = XEvent()
        event.client.type = 33
        event.client.display = display
        event.client.window = int(window, 16)
        event.client.message_type = x.XInternAtom(display, b'WM_PROTOCOLS', 0)
        event.client.format = 32
        event.client.data.l[0] = x.XInternAtom(display, b'WM_DELETE_WINDOW', 0)
        event.client.data.l[1] = 0
        if not x.XSendEvent(display, event.client.window, 0, 0, ctypes.byref(event)):
            raise RuntimeError('XSendEvent failed')
        x.XFlush(display)
    finally:
        x.XCloseDisplay(display)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--app', type=Path, required=True)
    args = parser.parse_args()
    if not os.environ.get('DISPLAY'):
        raise RuntimeError('DISPLAY is required')
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    app = None
    daemon_pid = None
    with tempfile.TemporaryDirectory(prefix='regente-native-autostart-') as temp:
        env = os.environ.copy()
        env.update({'REGENTE_HOME': temp, 'REGENTE_PORT': str(port), 'GDK_BACKEND': 'x11'})
        env.pop('REGENTE_UI_DIR', None)
        env.pop('REGENTE_FIXTURES', None)
        try:
            app = subprocess.Popen([str(args.app.resolve())], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            perf.wait_until(lambda: perf.api(base, '/health').get('ok'), 15, 'packaged daemon health')
            with urllib.request.urlopen(base + '/', timeout=3) as response:
                if response.status != 200 or 'text/html' not in response.headers.get('Content-Type', ''):
                    raise RuntimeError('Packaged UI not served')
            window = perf.wait_until(lambda: perf.window_for(app.pid), 15, 'window with app PID')
            def find_daemon():
                procs = perf.describe_processes(perf.descendants(app.pid))
                matches = [int(pid) for pid, name in procs.items() if name == 'regented']
                return matches[0] if len(matches) == 1 else None
            daemon_pid = perf.wait_until(find_daemon, 10, 'single daemon child')
            close_window(window)
            code = app.wait(timeout=10)
            if code != 0:
                raise RuntimeError(f'App exited with status {code}')
            if not perf.api(base, '/health').get('ok'):
                raise RuntimeError('Daemon did not remain healthy')
            perf.stat(daemon_pid)
            print(json.dumps({'app_exit_code': code, 'window': window, 'daemon_pid': daemon_pid, 'health_after_window_close': True, 'packaged_ui_http_ok': True}, indent=2))
        finally:
            if daemon_pid is None and app is not None and app.poll() is None:
                children = perf.describe_processes(perf.descendants(app.pid))
                matches = [int(pid) for pid, name in children.items() if name == 'regented']
                if len(matches) == 1:
                    daemon_pid = matches[0]
            perf.terminate(app)
            if daemon_pid is not None:
                try:
                    os.kill(daemon_pid, 15)
                    for _ in range(50):
                        try:
                            # A zombie is already terminated and cannot hold the port.
                            state = Path(f'/proc/{daemon_pid}/stat').read_text().split(') ', 1)[1][0]
                            if state == 'Z':
                                break
                        except FileNotFoundError:
                            break
                        time.sleep(.1)
                except ProcessLookupError:
                    pass


if __name__ == '__main__':
    main()
