import type { ComponentPropsWithoutRef } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';

/** Only web links are clickable; any other scheme (javascript:, data:, relative paths…) stays plain text. */
export function isWebUrl(href: string | undefined): href is string {
  if (!href) return false;
  try {
    const url = new URL(href);
    return url.protocol === 'http:' || url.protocol === 'https:';
  } catch {
    return false;
  }
}

function SafeLink({ href, children }: ComponentPropsWithoutRef<'a'>) {
  if (!isWebUrl(href)) return <span>{children}</span>;
  return (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  );
}

// Images would make the client fetch arbitrary URLs from agent text; show their description instead.
function ImageAsText({ alt }: ComponentPropsWithoutRef<'img'>) {
  return <span>{alt ?? ''}</span>;
}

const components: Components = { a: SafeLink, img: ImageAsText };

/**
 * Markdown from an agent. Raw HTML is not parsed (no rehype-raw), so it is shown as text
 * or dropped; links are restricted to http(s).
 */
export function AgentMarkdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}
