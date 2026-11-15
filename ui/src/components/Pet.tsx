import { memo } from 'react';
import type { Agent, PetConfig } from '../api/types';
import { useAnimationGate } from '../hooks/useAnimationGate';
import { statusStyle } from '../lib/labels';
import { petLook } from '../lib/pets';

interface PetProps {
  pet?: PetConfig;
  agent?: Pick<Agent, 'name' | 'is_orchestrator'>;
  status?: string;
  size: number;
  /** Larger pets get a mouth, feet and a highlight. */
  detailed?: boolean;
}

const EYES = '#16130F';

/** Pixel-art pet. The animation is a CSS class per status; paused is static. */
export const Pet = memo(function Pet({ pet, agent, status, size, detailed = false }: PetProps) {
  const gate = useAnimationGate<SVGSVGElement>();
  const { color, species } = petLook(pet, agent);
  const antenna = species === 'antenna' || species === 'both';
  const ears = species === 'ears' || species === 'both';
  return (
    <svg
      ref={gate}
      className={`pet pet-${statusStyle(status).key}`}
      width={size}
      height={size}
      viewBox="0 0 16 16"
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {antenna && (
        <>
          <rect x="7" y="0" width="3" height="1" fill={color.light} />
          <rect x="8" y="1" width="1" height="2" fill={color.dark} />
        </>
      )}
      {ears && (
        <>
          <rect x="3" y="2" width="2" height="2" fill={color.base} />
          <rect x="11" y="2" width="2" height="2" fill={color.base} />
        </>
      )}
      <rect x="4" y="3" width="8" height="1" fill={color.base} />
      <rect x="3" y="4" width="10" height="8" fill={color.base} />
      {detailed && <rect x="4" y="4" width="2" height="1" fill={color.light} />}
      <rect x="3" y="10" width="10" height="2" fill={color.dark} />
      <rect x="4" y="12" width="8" height="1" fill={color.dark} />
      <g className="eyes">
        <rect x="5" y="6" width="2" height="2" fill={EYES} />
        <rect x="9" y="6" width="2" height="2" fill={EYES} />
      </g>
      {detailed && (
        <>
          <rect x="6" y="9" width="4" height="1" fill={EYES} fillOpacity="0.55" />
          <rect x="4" y="13" width="2" height="1" fill={color.dark} />
          <rect x="10" y="13" width="2" height="1" fill={color.dark} />
        </>
      )}
    </svg>
  );
});
