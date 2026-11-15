// On-demand terminal. This module (and xterm.js with it) is loaded only when
// a terminal is opened (agent shell or account login); closing the panel disposes everything.

import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { useEffect, useRef, useState } from 'react';
import { wsUrl } from '../api/client';

/**
 * `path` is the PTY WebSocket route, such as `/agents/{id}/pty`. `failMessage`
 * is shown when the socket errors or closes before any byte arrived.
 */
export default function TerminalPanel({
  path,
  failMessage = 'Não foi possível abrir o terminal.',
}: {
  path: string;
  failMessage?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<'connecting' | 'open' | 'closed' | 'failed'>('connecting');

  useEffect(() => {
    const element = host.current;
    if (!element) {
      return;
    }
    const terminal = new Terminal({
      fontFamily: "'JetBrains Mono', ui-monospace, monospace",
      fontSize: 13,
      cursorBlink: false,
      theme: { background: '#1a1918', foreground: '#eceae3' },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(element);

    const socket = new WebSocket(wsUrl(path));
    socket.binaryType = 'arraybuffer';
    let received = false;
    const sendSize = () => {
      fit.fit();
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(JSON.stringify({ resize: [terminal.cols, terminal.rows] }));
      }
    };
    socket.onopen = () => {
      setState('open');
      sendSize();
      terminal.focus();
    };
    socket.onmessage = (event) => {
      received = true;
      terminal.write(typeof event.data === 'string' ? event.data : new Uint8Array(event.data as ArrayBuffer));
    };
    socket.onerror = () => {
      if (!received) {
        setState('failed');
      }
    };
    socket.onclose = () => setState(received ? 'closed' : 'failed');

    // Typed input goes as binary frames, so it can never be mistaken for the
    // `{"resize":[cols,rows]}` control message (a text frame).
    const encoder = new TextEncoder();
    const typed = terminal.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(encoder.encode(data));
      }
    });
    const resize = new ResizeObserver(sendSize);
    resize.observe(element);

    return () => {
      resize.disconnect();
      typed.dispose();
      socket.onclose = null;
      socket.onerror = null;
      socket.close();
      terminal.dispose();
    };
  }, [path]);

  return (
    <>
      {state !== 'open' && (
        <span className={state === 'failed' ? 'error-text' : 'muted'} style={{ fontSize: 12 }} role="status">
          {state === 'connecting'
            ? 'Conectando ao terminal…'
            : state === 'failed'
              ? failMessage
              : 'Terminal encerrado.'}
        </span>
      )}
      <div className="terminal-host" ref={host} data-testid="terminal" />
    </>
  );
}
