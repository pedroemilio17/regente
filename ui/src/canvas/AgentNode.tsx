import { memo } from 'react';
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react';
import type { AgentView } from '../api/types';
import { Pet } from '../components/Pet';
import { formatAgo } from '../lib/format';
import { AGENT_STATUS_LABEL, AGENT_STATUS_SHAPE as SHAPE, type AgentProjection, type AgentStatus } from '../lib/projection';

export type AgentNodeData = {
  agent: AgentView;
  projection: AgentProjection;
  now: number;
  /** Dimmed when another agent is selected and this one is not a direct neighbor. */
  dim: boolean;
};
export type AgentFlowNode = Node<AgentNodeData, 'orchestrator' | 'agent'>;

/** Pet status for the existing pixel-art animation classes. */
const PET_STATUS: Partial<Record<AgentStatus, string>> = { working: 'working', waiting: 'waiting', paused: 'paused' };

export const AgentNode = memo(function AgentNode({ data, type }: NodeProps<AgentFlowNode>) {
  const { agent, projection, now, dim } = data;
  const { status, currentTask, lastActivity } = projection;
  return (
    <div className={`cv-node cv-${type} cv-st-${status}${dim ? ' is-dim' : ''}`} data-testid="canvas-node" data-agent={agent.name}>
      <Handle type="target" position={Position.Top} isConnectable={false} />
      <div className="cv-node-head">
        <span className="cv-pet">
          <Pet pet={agent.pet} agent={agent} status={PET_STATUS[status] ?? 'idle'} size={28} />
        </span>
        <span className="cv-who">
          <b title={agent.name}>{agent.name}</b>
          <small title={agent.role}>{agent.role}</small>
        </span>
      </div>
      <div className="cv-status" data-testid="node-status">
        <span className="cv-shape" aria-hidden="true">
          {SHAPE[status]}
        </span>
        {AGENT_STATUS_LABEL[status]}
      </div>
      <div className="cv-task" title={currentTask?.title}>
        {currentTask ? currentTask.title : <span className="muted">Sem tarefa</span>}
      </div>
      {lastActivity && (
        <div className="cv-last" title={lastActivity.summary}>
          <span className="cv-last-text">{lastActivity.summary || 'Atividade'}</span>
          <span className="cv-ago">{formatAgo(lastActivity.ts, now)}</span>
        </div>
      )}
      <Handle type="source" position={Position.Bottom} isConnectable={false} />
    </div>
  );
});
