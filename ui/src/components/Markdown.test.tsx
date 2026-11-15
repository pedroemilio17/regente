import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { AgentMarkdown, isWebUrl } from './Markdown';

afterEach(cleanup);

function renderMarkdown(text: string) {
  const { container } = render(<AgentMarkdown text={text} />);
  return container;
}

describe('AgentMarkdown', () => {
  it('renders headings, lists, emphasis, inline and block code', () => {
    const root = renderMarkdown(
      ['## Título', '', '- um', '- dois', '', '**forte** e *itálico* com `inline`', '', '```ts', 'const x = 1;', '```'].join('\n'),
    );
    expect(root.querySelector('h2')?.textContent).toBe('Título');
    expect([...root.querySelectorAll('li')].map((li) => li.textContent)).toEqual(['um', 'dois']);
    expect(root.querySelector('strong')?.textContent).toBe('forte');
    expect(root.querySelector('em')?.textContent).toBe('itálico');
    expect(root.querySelector('p code')?.textContent).toBe('inline');
    const block = root.querySelector('pre code');
    expect(block?.textContent?.trim()).toBe('const x = 1;');
    expect(block?.className).toContain('language-ts');
  });

  it('renders GFM tables', () => {
    const root = renderMarkdown(['| Nome | Valor |', '| --- | --- |', '| a | 1 |', '| b | 2 |'].join('\n'));
    const headers = [...root.querySelectorAll('th')].map((cell) => cell.textContent);
    const cells = [...root.querySelectorAll('tbody td')].map((cell) => cell.textContent);
    expect(headers).toEqual(['Nome', 'Valor']);
    expect(cells).toEqual(['a', '1', 'b', '2']);
  });

  it('opens http and https links in a new tab without opener access', () => {
    const root = renderMarkdown('[site](https://exemplo.com/a) e [antigo](http://exemplo.com)');
    const links = [...root.querySelectorAll('a')];
    expect(links.map((link) => link.getAttribute('href'))).toEqual(['https://exemplo.com/a', 'http://exemplo.com']);
    for (const link of links) {
      expect(link.getAttribute('target')).toBe('_blank');
      expect(link.getAttribute('rel')).toBe('noopener noreferrer');
    }
  });

  it('keeps links with other schemes as plain text', () => {
    const root = renderMarkdown(
      ['[js](javascript:alert(1))', '[dados](data:text/html,<b>x</b>)', '[arquivo](file:///etc/passwd)', '[relativo](/rota)'].join('\n\n'),
    );
    expect(root.querySelectorAll('a')).toHaveLength(0);
    expect(root.textContent).toContain('js');
    expect(root.textContent).toContain('arquivo');
    expect(root.textContent).toContain('relativo');
  });

  it('does not render raw HTML from agent text', () => {
    const root = renderMarkdown('Antes <script>alert(1)</script> <img src="x" onerror="alert(1)"> <b onclick="x()">oi</b> depois');
    expect(root.querySelector('script')).toBeNull();
    expect(root.querySelector('img')).toBeNull();
    expect(root.querySelector('b')).toBeNull();
    expect(root.querySelector('[onclick]')).toBeNull();
    expect(root.querySelector('[onerror]')).toBeNull();
  });

  it('shows images as their description instead of fetching them', () => {
    const root = renderMarkdown('![logo da marca](https://exemplo.com/logo.png)');
    expect(root.querySelector('img')).toBeNull();
    expect(root.textContent).toContain('logo da marca');
  });
});

describe('isWebUrl', () => {
  it('accepts only http and https absolute URLs', () => {
    expect(isWebUrl('https://a.b/c')).toBe(true);
    expect(isWebUrl('http://a.b')).toBe(true);
    expect(isWebUrl('javascript:alert(1)')).toBe(false);
    expect(isWebUrl('/relativo')).toBe(false);
    expect(isWebUrl('')).toBe(false);
    expect(isWebUrl(undefined)).toBe(false);
  });
});
