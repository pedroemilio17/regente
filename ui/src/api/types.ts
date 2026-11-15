// API entities are generated from shared/schema; these aliases describe UI conveniences.
export type * from './contracts.generated';
import type { Agent, Folder, NewAgentBody as GeneratedNewAgentBody, UsageReport, UsageWindows } from './contracts.generated';

export type CliId = Agent['cli'];
export type FolderAccess = Folder['access'];
export type ProjectFolder = Folder;
export type PetConfig = Agent['pet'];
export type WindowKey = keyof UsageWindows;
export type AccountUsage = UsageReport['accounts'][number];
export type AgentUsage = UsageReport['agents'][number];
/** The daemon names the agent after its engine when `name` is missing. */
export type NewAgentBody = Omit<GeneratedNewAgentBody, 'name'> & { name?: string };
