import { useEffect, useState } from 'react';
import { api } from '../api/client';
import type { AccountModels, CliId } from '../api/types';
import { useStore } from '../store';

export const OTHER = '__other';
const DEFAULT_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];

/** Models listed by the account's own CLI (GET /accounts/{id}/models). */
function useModels(cli: string, accountId?: string | null): AccountModels | null {
  const id = useStore((state) => accountId ?? state.accounts.find((account) => account.provider === cli)?.id);
  const [result, setResult] = useState<AccountModels | null>(null);
  useEffect(() => {
    setResult(null);
    if (!id) return;
    let live = true;
    api.accountModels(id).then((value) => { if (live) setResult(value); }).catch(() => { if (live) setResult({ models: [], detail: 'Não foi possível listar os modelos.' }); });
    return () => { live = false; };
  }, [id]);
  return result;
}

interface Props {
  cli: CliId;
  accountId?: string | null;
  model: string;
  onModel: (model: string) => void;
  effort: string;
  onEffort: (effort: string) => void;
  idPrefix?: string;
}

/** Model (list, account default or free text) and optional reasoning level. */
export function ModelEffort({ cli, accountId, model, onModel, effort, onEffort, idPrefix = 'me' }: Props) {
  const list = useModels(cli, accountId);
  const models = list?.models ?? [];
  const [other, setOther] = useState(false);
  const isOther = other || (model !== '' && !models.some((item) => item.id === model));
  const current = models.find((item) => item.id === model);
  // agy puts the reasoning level in the model slug: no separate field.
  const efforts = cli === 'agy' ? [] : current?.efforts?.length ? current.efforts : DEFAULT_EFFORTS;
  return (
    <>
      <div className="grid-2">
        <label className="field" htmlFor={`${idPrefix}-model`}><span>Modelo</span>
          <select id={`${idPrefix}-model`} className="select" value={isOther ? OTHER : model} onChange={(e) => { const custom = e.target.value === OTHER; setOther(custom); onModel(custom ? '' : e.target.value); }}>
            <option value="">Padrão da conta</option>
            {models.map((item) => <option key={item.id} value={item.id}>{item.label && item.label !== item.id ? `${item.label} (${item.id})` : item.id}</option>)}
            <option value={OTHER}>Outro…</option>
          </select>
        </label>
        {efforts.length > 0 && <label className="field" htmlFor={`${idPrefix}-effort`}><span>Raciocínio</span>
          <select id={`${idPrefix}-effort`} className="select" value={efforts.includes(effort) ? effort : ''} onChange={(e) => onEffort(e.target.value)}>
            <option value="">Padrão</option>
            {efforts.map((level) => <option key={level} value={level}>{level}</option>)}
          </select>
        </label>}
      </div>
      {list?.detail && models.length === 0 && <small className="muted">{list.detail}</small>}
      {isOther && <label className="field"><span>Nome do modelo</span><input className="input" value={model} onChange={(e) => onModel(e.target.value)} placeholder="Nome aceito pelo CLI" /></label>}
    </>
  );
}
