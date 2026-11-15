import { useId, useState, type FormEvent } from 'react';
import { api } from '../api/client';
import type { Project } from '../api/types';
import { Modal } from '../components/Modal';
import { navigate } from '../lib/route';
import { useStore } from '../store';

/** Creates a project, or edits name, folder and description when `project` is given. */
export function NewProject({ project, onClose }: { project?: Project; onClose: () => void }) {
  const [name, setName] = useState(project?.name ?? '');
  const [root, setRoot] = useState(project?.root ?? '');
  const [context, setContext] = useState(project?.context ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [picking, setPicking] = useState(false);
  const rootId = useId();
  const title = project ? 'Configurações do projeto' : 'Novo projeto';
  const pickFolder = async () => {
    setPicking(true); setError('');
    try {
      const { path } = await api.pickFolder();
      if (path) {
        setRoot(path);
        const folder = path.replace(/\/+$/, '').split('/').pop() ?? '';
        setName((current) => current.trim() ? current : folder);
      }
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Não foi possível abrir o seletor de pastas; digite o caminho.'); }
    finally { setPicking(false); }
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!name.trim() || !root.trim().startsWith('/')) { setError('Informe o nome e o caminho absoluto da pasta do projeto.'); return; }
    setBusy(true); setError('');
    try {
      const fields = { name: name.trim(), root: root.trim(), context: context.trim() };
      if (project) {
        await api.patchProject(project.id, fields);
        await useStore.getState().refresh();
        onClose();
        return;
      }
      const result = await api.createProject(fields);
      await useStore.getState().refresh();
      onClose();
      navigate({ screen: 'overview', projectId: result.project.id });
    } catch (cause) { setError(cause instanceof Error ? cause.message : project ? 'Não foi possível salvar o projeto.' : 'Não foi possível criar o projeto.'); setBusy(false); }
  };
  return <Modal title={title} onClose={onClose}><form onSubmit={submit} aria-label={title} className="modal-form">
    <div className="grid-2">
      <label className="field"><span>Nome</span><input className="input" value={name} onChange={(e) => setName(e.target.value)} required /></label>
      <div className="field"><label htmlFor={rootId}>Pasta (caminho absoluto)</label><div className="input-row">
        <input id={rootId} className="input mono" value={root} onChange={(e) => setRoot(e.target.value)} placeholder="/home/voce/projeto" required />
        <button type="button" className="btn" onClick={pickFolder} disabled={picking} aria-label="Escolher pasta no explorador de arquivos">{picking ? 'Abrindo…' : 'Escolher…'}</button>
      </div>{project && <span className="muted">Só é possível trocar a pasta sem tarefas ativas.</span>}</div>
    </div>
    <label className="field"><span>Descrição (opcional)</span><textarea className="textarea" rows={4} value={context} onChange={(e) => setContext(e.target.value)} /></label>
    {error && <span className="error-text" role="alert">{error}</span>}
    <div className="form-actions"><button type="button" className="btn" onClick={onClose}>Cancelar</button><button className="btn primary" type="submit" disabled={busy}>{project ? 'Salvar' : 'Criar projeto'}</button></div>
  </form></Modal>;
}
