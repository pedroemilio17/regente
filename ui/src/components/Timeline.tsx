import { memo } from 'react';
import type { AgentEvent } from '../api/types';
import { formatTime } from '../lib/format';
import { eventTitle, kindStyle } from '../lib/labels';

const Entry = memo(function Entry({ event }: { event: AgentEvent }) {
  const kind = kindStyle(event.kind);
  const title = eventTitle(event);
  const showSummary = event.summary !== '' && event.summary !== title.text;
  return (
    <li data-kind={event.kind}>
      <span className="time">{formatTime(event.ts)}</span>
      <span className={`kind tag-${kind.tone}`}>{kind.label}</span>
      <span className="what">
        <span className="head">
          <span className={title.mono ? 'mono' : undefined}>{title.text}</span>
          {(event.lines_added ?? 0) > 0 && <span className="plus">+{event.lines_added}</span>}
          {(event.lines_removed ?? 0) > 0 && <span className="minus">−{event.lines_removed}</span>}
        </span>
        {showSummary && <span className="sub">{event.summary}</span>}
      </span>
    </li>
  );
});

interface TimelineProps {
  /** Newest first. */
  events: AgentEvent[];
  hasMore: boolean;
  loading: boolean;
  onLoadMore: () => void;
}

/** Event list of one agent; older pages are loaded on demand. */
export function Timeline({ events, hasMore, loading, onLoadMore }: TimelineProps) {
  if (events.length === 0) {
    return <p className="empty">{loading ? 'Carregando…' : 'Nenhum evento ainda.'}</p>;
  }
  return (
    <>
      <ol className="timeline" data-testid="timeline">
        {events.map((event) => (
          <Entry key={event.id} event={event} />
        ))}
      </ol>
      {hasMore && (
        <button type="button" className="btn small" onClick={onLoadMore} disabled={loading} style={{ alignSelf: 'center' }}>
          {loading ? 'Carregando…' : 'Carregar anteriores'}
        </button>
      )}
    </>
  );
}
