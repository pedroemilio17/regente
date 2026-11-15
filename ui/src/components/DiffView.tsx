import { memo } from 'react';

function lineClass(line: string): string | undefined {
  if (line.startsWith('@@')) {
    return 'hunk';
  }
  if (line.startsWith('+') && !line.startsWith('+++')) {
    return 'add';
  }
  if (line.startsWith('-') && !line.startsWith('---')) {
    return 'del';
  }
  return undefined;
}

const MAX_LINES = 400;

/** Unified diff with added and removed lines highlighted. */
export const DiffView = memo(function DiffView({ diff }: { diff: string }) {
  const lines = diff.replace(/\n$/, '').split('\n');
  const shown = lines.slice(0, MAX_LINES);
  return (
    <pre className="diff" data-testid="diff">
      {shown.map((line, index) => (
        <div key={index} className={lineClass(line)}>
          {line || ' '}
        </div>
      ))}
      {lines.length > MAX_LINES && <div>… mais {lines.length - MAX_LINES} linhas</div>}
    </pre>
  );
});
