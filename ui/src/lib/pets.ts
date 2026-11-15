// Pet palette and shapes. Pets are pixel-art SVGs animated only with CSS.

import type { Agent, PetConfig } from '../api/types';

export interface PetColor {
  id: string;
  label: string;
  base: string;
  dark: string;
  light: string;
}

export const PET_COLORS: PetColor[] = [
  { id: 'blue', label: 'Azul', base: '#7FA2F7', dark: '#4C6FC4', light: '#C4D4FB' },
  { id: 'orange', label: 'Laranja', base: '#E8915B', dark: '#B4643A', light: '#F6C3A0' },
  { id: 'purple', label: 'Roxo', base: '#B08CF0', dark: '#7C5BC0', light: '#DCCBF9' },
  { id: 'green', label: 'Verde', base: '#6CC79A', dark: '#3F9468', light: '#BDEBD3' },
  { id: 'yellow', label: 'Amarelo', base: '#ECC56A', dark: '#B8902F', light: '#F7E3AE' },
  { id: 'pink', label: 'Rosa', base: '#EC8FB4', dark: '#B85C84', light: '#F8CADD' },
];

export type PetSpecies = 'antenna' | 'ears' | 'both' | 'none';

export const PET_SPECIES: { id: PetSpecies; label: string }[] = [
  { id: 'antenna', label: 'Antena' },
  { id: 'ears', label: 'Orelhas' },
  { id: 'both', label: 'Os dois' },
  { id: 'none', label: 'Nenhum' },
];

export interface PetLook {
  color: PetColor;
  species: PetSpecies;
}

function hash(text: string): number {
  let value = 0;
  for (let index = 0; index < text.length; index += 1) {
    value = (value * 31 + text.charCodeAt(index)) >>> 0;
  }
  return value;
}

export function petColor(id: string | undefined): PetColor | undefined {
  return PET_COLORS.find((color) => color.id === id);
}

/** Look of a pet: what was saved, or a stable default derived from the agent. */
export function petLook(pet: PetConfig | undefined, agent?: Pick<Agent, 'name' | 'is_orchestrator'>): PetLook {
  const seed = hash(agent?.name ?? '');
  const fallbackColor = agent?.is_orchestrator ? petColor('yellow') : PET_COLORS[seed % PET_COLORS.length];
  const fallbackSpecies: PetSpecies = agent?.is_orchestrator
    ? 'both'
    : ((PET_SPECIES[(seed >>> 3) % PET_SPECIES.length]?.id ?? 'antenna') as PetSpecies);
  const species = PET_SPECIES.find((item) => item.id === pet?.species)?.id ?? fallbackSpecies;
  return { color: petColor(pet?.color) ?? fallbackColor ?? (PET_COLORS[0] as PetColor), species };
}

/** Color square used next to an agent name in the conversation. */
export function agentColor(agent: Pick<Agent, 'name' | 'is_orchestrator' | 'pet'> | undefined): string {
  return agent ? petLook(agent.pet, agent).color.base : '#8A8880';
}
