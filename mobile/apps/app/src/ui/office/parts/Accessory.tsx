import React from 'react';
import { Circle, G, Path, Rect } from 'react-native-svg';
import type { Accessory as Kind } from '@orochi/core';
import { CAP, GEAR, INK, LAPEL, PIN, SCARF, WHITE } from '../avatarPalette';

/** A role marked quietly, or a finishing touch: flat shapes, no outlines. */
export function Accessory({ kind }: { kind: Kind }) {
  switch (kind) {
    case 'blazer': // a shirt collar, lapels and a small pin
      return (
        <G>
          <Path d="M24 48.5 L32 59 L40 48.5 Z" fill={WHITE} />
          <Path d="M21.5 50 L30 65 L22 65 Z" fill={LAPEL} />
          <Path d="M42.5 50 L34 65 L42 65 Z" fill={LAPEL} />
          <Circle cx={23.5} cy={57.5} r={1.7} fill={PIN} />
        </G>
      );
    case 'collar':
      return (
        <G fill={WHITE}>
          <Path d="M25 48.5 L32 55 L30 48.5 Z" />
          <Path d="M39 48.5 L32 55 L34 48.5 Z" />
        </G>
      );
    case 'headphones':
      return (
        <G>
          <Path
            d="M15.5 30 Q14.5 10.5 32 10.5 Q49.5 10.5 48.5 30"
            fill="none"
            stroke={GEAR}
            strokeWidth={2.6}
            strokeLinecap="round"
          />
          <Rect x={11.5} y={27} width={6} height={10} rx={3} fill={GEAR} />
          <Rect x={46.5} y={27} width={6} height={10} rx={3} fill={GEAR} />
        </G>
      );
    case 'cap':
      return (
        <G fill={CAP}>
          <Path d="M16 24.5 Q16 9 32 9 Q48 9 48 24.5 Z" />
          <Rect x={30} y={22} width={25} height={4.2} rx={2.1} />
        </G>
      );
    case 'glasses':
      return (
        <G fill="none" stroke={INK} strokeWidth={1.4}>
          <Circle cx={25.5} cy={31} r={5.4} />
          <Circle cx={38.5} cy={31} r={5.4} />
          <Path d="M30.9 30.5 L33.1 30.5" />
        </G>
      );
    case 'scarf':
      return <Path d="M22.5 46.5 Q32 53 41.5 46.5 L43 51 Q32 57.5 21 51 Z" fill={SCARF} />;
    case 'earring':
      return <Circle cx={16.4} cy={37} r={1.6} fill={PIN} />;
    default:
      return null;
  }
}
