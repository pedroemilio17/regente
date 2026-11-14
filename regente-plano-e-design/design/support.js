// Minimal standalone runtime for the Regente .dc.html design files.
// Renders {{holes}}, <sc-for>, <sc-if> and onClick handlers so the screens
// can be opened directly in a browser, outside the Claude design canvas.
(function () {
  class DCLogic {
    constructor(props) { this.props = props || {}; this.state = {}; }
    setState(patch) {
      this.state = Object.assign({}, this.state, typeof patch === 'function' ? patch(this.state) : patch);
      if (this.__render) this.__render();
    }
    forceUpdate() { if (this.__render) this.__render(); }
  }
  window.DCLogic = DCLogic;

  const HOLE = /\{\{\s*([^}]+?)\s*\}\}/g;
  const lookup = (scope, path) => {
    path = path.trim();
    if (path === 'true') return true;
    if (path === 'false') return false;
    if (/^-?\d+(\.\d+)?$/.test(path)) return Number(path);
    let v = scope;
    for (const k of path.split('.')) { if (v == null) return undefined; v = v[k]; }
    return v;
  };
  const interp = (str, scope) => str.replace(HOLE, (_, p) => { const v = lookup(scope, p); return v == null ? '' : String(v); });
  const whole = (str) => { const m = /^\s*\{\{\s*([^}]+?)\s*\}\}\s*$/.exec(str); return m ? m[1] : null; };

  function renderNode(node, scope, out) {
    if (node.nodeType === 3) { out.appendChild(document.createTextNode(interp(node.nodeValue, scope))); return; }
    if (node.nodeType !== 1) return;
    const tag = node.tagName.toLowerCase();
    if (tag === 'helmet') return;
    if (tag === 'sc-for') {
      const list = lookup(scope, whole(node.getAttribute('list')) || '') || [];
      const as = node.getAttribute('as') || 'item';
      list.forEach((item, i) => {
        const s = Object.create(scope); s[as] = item; s.$index = i;
        node.childNodes.forEach((c) => renderNode(c, s, out));
      });
      return;
    }
    if (tag === 'sc-if') {
      if (lookup(scope, whole(node.getAttribute('value')) || '')) node.childNodes.forEach((c) => renderNode(c, scope, out));
      return;
    }
    const el = node.namespaceURI === 'http://www.w3.org/2000/svg'
      ? document.createElementNS(node.namespaceURI, node.tagName)
      : document.createElement(node.tagName);
    for (const a of Array.from(node.attributes)) {
      if (a.name.startsWith('hint-')) continue;
      const w = whole(a.value);
      if (/^on[a-z]/i.test(a.name) && w) {
        const fn = lookup(scope, w);
        if (typeof fn === 'function') el.addEventListener(a.name.slice(2).toLowerCase(), (e) => { e.preventDefault(); fn(e); });
        continue;
      }
      el.setAttribute(a.name, interp(a.value, scope));
    }
    if (tag === 'input' && node.hasAttribute('checked')) el.checked = true;
    const src = tag === 'template' ? node.content : node;
    src.childNodes.forEach((c) => renderNode(c, scope, el));
    out.appendChild(el);
  }

  document.addEventListener('DOMContentLoaded', () => {
    const host = document.querySelector('x-dc');
    const script = document.querySelector('script[data-dc-script]');
    if (!host || !script) return;
    host.querySelectorAll('helmet > *').forEach((n) => document.head.appendChild(n.cloneNode(true)));
    const tpl = host.cloneNode(true);
    const Component = new Function('DCLogic', script.textContent + '\nreturn Component;')(DCLogic);
    const comp = new Component({});
    const mount = document.createElement('div');
    host.replaceWith(mount);
    comp.__render = () => {
      const frag = document.createDocumentFragment();
      const vals = comp.renderVals();
      tpl.childNodes.forEach((c) => renderNode(c, vals, frag));
      mount.replaceChildren(frag);
    };
    if (comp.componentDidMount) comp.componentDidMount();
    comp.__render();
  });
})();
