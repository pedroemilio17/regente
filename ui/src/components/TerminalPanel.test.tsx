import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, cleanup, act } from '@testing-library/react';
import TerminalPanel from './TerminalPanel';

// xterm.js does not run in jsdom; these stubs keep only what the panel calls.
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    loadAddon() {}
    open() {}
    focus() {}
    write() {}
    dispose() {}
    onData() {
      return { dispose() {} };
    }
  },
}));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('@xterm/xterm/css/xterm.css', () => ({}));

class FakeSocket {
  static OPEN = 1;
  static last: FakeSocket;
  readyState = 0;
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(public url: string) {
    FakeSocket.last = this;
  }
  send() {}
  close() {}
}

describe('TerminalPanel', () => {
  beforeEach(() => {
    vi.stubGlobal('WebSocket', FakeSocket);
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  it('mostra a falha quando o socket fecha sem nenhum byte', () => {
    render(<TerminalPanel path="/accounts/acc_codex/login" failMessage="Não foi possível abrir o terminal de login." />);
    act(() => {
      FakeSocket.last.onerror?.();
      FakeSocket.last.onclose?.();
    });
    expect(screen.getByRole('status').textContent).toBe('Não foi possível abrir o terminal de login.');
  });

  it('mantém "Terminal encerrado." quando houve saída antes do fechamento', () => {
    render(<TerminalPanel path="/agents/ag_1/pty" />);
    act(() => {
      FakeSocket.last.readyState = 1;
      FakeSocket.last.onopen?.();
      FakeSocket.last.onmessage?.({ data: 'ok' });
      FakeSocket.last.onclose?.();
    });
    expect(screen.getByRole('status').textContent).toBe('Terminal encerrado.');
  });
});
