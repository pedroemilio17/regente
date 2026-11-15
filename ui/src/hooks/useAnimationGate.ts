// Animation gate: CSS animations run only for elements on screen, and never
// while the window is hidden or without focus. Everything is done with DOM
// attributes, so pausing and resuming never re-renders React.

import { useCallback, type RefCallback } from 'react';

/** Set on <html> while the window is hidden or unfocused. */
export const PAUSED_CLASS = 'anim-paused';
/** Set on a gated element while it is outside the viewport. */
export const OFFSCREEN_ATTR = 'data-offscreen';

let observer: IntersectionObserver | null = null;

function sharedObserver(): IntersectionObserver | null {
  if (observer === null && typeof IntersectionObserver !== 'undefined') {
    observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        entry.target.toggleAttribute(OFFSCREEN_ATTR, !entry.isIntersecting);
      }
    });
  }
  return observer;
}

let uninstall: (() => void) | null = null;

/**
 * Pause every animation while the document is hidden or the window lost
 * focus. Idempotent; returns a function that removes the listeners.
 */
export function installWindowPause(): () => void {
  if (uninstall !== null) {
    return uninstall;
  }
  let focused = document.hasFocus();
  const sync = () => {
    document.documentElement.classList.toggle(PAUSED_CLASS, document.hidden || !focused);
  };
  const onFocus = () => {
    focused = true;
    sync();
  };
  const onBlur = () => {
    focused = false;
    sync();
  };
  window.addEventListener('focus', onFocus);
  window.addEventListener('blur', onBlur);
  document.addEventListener('visibilitychange', sync);
  sync();
  uninstall = () => {
    window.removeEventListener('focus', onFocus);
    window.removeEventListener('blur', onBlur);
    document.removeEventListener('visibilitychange', sync);
    document.documentElement.classList.remove(PAUSED_CLASS);
    observer?.disconnect();
    observer = null;
    uninstall = null;
  };
  return uninstall;
}

/**
 * Ref for an animated element: marks it with `data-offscreen` while it is
 * outside the viewport (the CSS stops its animation) and makes sure the
 * window-level pause is installed.
 */
export function useAnimationGate<T extends Element>(): RefCallback<T> {
  return useCallback((node: T | null) => {
    if (node === null) {
      return;
    }
    installWindowPause();
    const shared = sharedObserver();
    shared?.observe(node);
    return () => {
      shared?.unobserve(node);
    };
  }, []);
}
