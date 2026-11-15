import { navigate } from '../lib/route';
import { useStore, type Connection } from '../store';

const CONNECTION: Record<Connection, string> = {
  online: 'Conectado',
  reconnecting: 'Reconectando…',
  offline: 'Sem conexão',
  degraded: 'Instável',
};

interface TopbarProps {
  projectId?: string;
  chatOpen: boolean;
  onToggleChat: () => void;
  onSettings: () => void;
}

/** Project on the left, connection and chat toggle on the right. No metrics here. */
export function Topbar({ projectId, chatOpen, onToggleChat, onSettings }: TopbarProps) {
  const projects = useStore((state) => state.projects);
  const connection = useStore((state) => state.connection);
  const current = projects.find((project) => project.id === projectId);
  return (
    <header className="topbar">
      {projects.length > 1 ? (
        <select
          className="project-select"
          aria-label="Projeto"
          value={projectId}
          onChange={(event) => navigate({ screen: 'overview', projectId: event.target.value })}
        >
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
      ) : (
        <span className="project-name">{current?.name ?? 'Regente'}</span>
      )}
      {current && (
        <button type="button" className="btn small" onClick={onSettings}>
          Configurações do projeto
        </button>
      )}
      <span className="conn-badge" data-state={connection} role="status" data-testid="connection">
        {CONNECTION[connection]}
      </span>
      <button type="button" className="btn small" aria-pressed={chatOpen} onClick={onToggleChat}>
        {chatOpen ? 'Fechar chat' : 'Abrir chat'}
      </button>
    </header>
  );
}
