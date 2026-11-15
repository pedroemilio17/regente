import { useState, type FormEvent } from 'react';
import { api } from '../api/client';
import type { CliId, NewAgentBody } from '../api/types';
import { Modal } from '../components/Modal';
import { ModelEffort } from '../components/ModelEffort';
import { CLIS } from '../lib/labels';
import { usageBars } from '../lib/format';
import { navigate } from '../lib/route';
import { useStore } from '../store';

export function NewAgent({ projectId, onClose }: { projectId?: string; onClose: () => void }) {
  const project = useStore((state) => projectId ?? state.projects[0]?.id ?? '');
  const [cli, setCli] = useState<CliId>('claude');
  const quota = useStore((state) => {
    const account = state.accounts.find((item) => item.provider === cli);
    return state.usage?.accounts.find((entry) => entry.account_id === account?.id)?.windows;
  });
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState('');
  const [orchestrator, setOrchestrator] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!project) { setError('Crie um projeto antes de adicionar agentes.'); return; }
    const body: NewAgentBody = { project_id: project, cli, is_orchestrator: orchestrator };
    if (model.trim()) body.model = model.trim();
    const settings: Record<string, unknown> = cli === 'fake' ? { fake_scenario: 'basic' } : {};
    if (effort && cli !== 'agy') settings.effort = effort;
    if (Object.keys(settings).length) body.settings = settings;
    setBusy(true); setError('');
    try {
      await api.createAgent(body);
      await useStore.getState().refresh();
      onClose();
      navigate({ screen: 'overview', projectId: project });
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Não foi possível criar o agente.'); setBusy(false); }
  };
  return <Modal title="Novo agente" onClose={onClose}><form onSubmit={submit} aria-label="Novo agente" className="modal-form">
    <div className="grid-2">
      <label className="field"><span>Motor</span><select className="select" value={cli} onChange={(e) => { setCli(e.target.value as CliId); setModel(''); setEffort(''); }}>{CLIS.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}</select></label>
      <label className="field"><span>Função</span><select className="select" value={orchestrator ? 'orchestrator' : 'agent'} onChange={(e) => setOrchestrator(e.target.value === 'orchestrator')}><option value="orchestrator">Maestro</option><option value="agent">Agente do time</option></select></label>
    </div>
    {cli !== 'fake' && <small className="muted" data-testid="quota-hint">{usageBars(quota).map((bar) => `${bar.label}: ${bar.value}${bar.reset ? ` (${bar.reset})` : ''}`).join(' · ')}</small>}
    <ModelEffort cli={cli} model={model} onModel={setModel} effort={effort} onEffort={setEffort} idPrefix="new" />
    {error && <span className="error-text" role="alert">{error}</span>}
    <div className="form-actions"><button type="button" className="btn" onClick={onClose}>Cancelar</button><button className="btn primary" type="submit" disabled={busy}>Criar agente</button></div>
  </form></Modal>;
}
