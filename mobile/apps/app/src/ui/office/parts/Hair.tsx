import React from 'react';
import { Circle, Ellipse, G, Path, Rect } from 'react-native-svg';
import { WHITE } from '../avatarPalette';

/**
 * Ten flat masses of hair or headwear on a head of centre (32,29) and radius about 15.5: what is
 * behind the head and what is in front. No outlines, no highlights.
 * 0 half-round, 1 sleek, 2 bun, 3 bob, 4 curly, 5 ponytail, 6 beanie, 7 hijab, 8 turban, 9 buzz.
 */
export function HairBack({ style, fill }: { style: number; fill: string }) {
  switch (style) {
    case 3:
      return (
        <Path d="M14 42 Q11 9 32 9 Q53 9 50 42 L43.5 42 L43.5 26 L20.5 26 L20.5 42 Z" fill={fill} />
      );
    case 4:
      return (
        <G fill={fill}>
          <Circle cx={17.5} cy={22} r={7.5} />
          <Circle cx={25.5} cy={14.5} r={8} />
          <Circle cx={38.5} cy={14.5} r={8} />
          <Circle cx={46.5} cy={22} r={7.5} />
          <Circle cx={15.5} cy={31} r={5.5} />
          <Circle cx={48.5} cy={31} r={5.5} />
        </G>
      );
    case 5:
      return <Ellipse cx={51.5} cy={37} rx={4.8} ry={10} fill={fill} />;
    case 7: // hijab
      return (
        <Path
          d="M11.5 33 Q9.5 7 32 7 Q54.5 7 52.5 33 Q53 49 45 53 L19 53 Q11 49 11.5 33 Z"
          fill={fill}
        />
      );
    default:
      return null;
  }
}

export function HairFront({ style, fill }: { style: number; fill: string }) {
  switch (style) {
    case 1: // sleek, swept
      return (
        <Path
          d="M16.5 27 Q17 11 33 11.5 Q48 12 48.5 24 Q40 19.5 29 20.5 Q21 21.5 16.5 27 Z"
          fill={fill}
        />
      );
    case 3:
      return (
        <Path
          d="M16.5 25 Q19 12 32 12 Q45 12 47.5 25 Q40 19 32 19.5 Q24 19 16.5 25 Z"
          fill={fill}
        />
      );
    case 4:
      return (
        <Path
          d="M18.5 22 Q21 13.5 32 13.5 Q43 13.5 45.5 22 Q39 18 32 18.5 Q25 18 18.5 22 Z"
          fill={fill}
        />
      );
    case 6: // beanie
      return (
        <G fill={fill}>
          <Path d="M15.5 23 Q15 8.5 32 8.5 Q49 8.5 48.5 23 Z" />
          <Rect x={14.5} y={21} width={35} height={6} rx={3} />
          <Circle cx={32} cy={7.5} r={3} />
        </G>
      );
    case 7: // hijab: a plain forehead edge
      return <Path d="M19 22.5 Q32 11 45 22.5 Q32 17 19 22.5 Z" fill={fill} />;
    case 8: // turban
      return (
        <G>
          <Path d="M15.5 24.5 Q15 8 32 8 Q49 8 48.5 24.5 Z" fill={fill} />
          <Path
            d="M16.5 20.5 Q32 9 47.5 20.5"
            fill="none"
            stroke={WHITE}
            strokeOpacity={0.35}
            strokeWidth={3}
          />
          <Path
            d="M20 24 Q36 11 48 17"
            fill="none"
            stroke={WHITE}
            strokeOpacity={0.35}
            strokeWidth={2.4}
          />
        </G>
      );
    case 9: // buzz
      return (
        <Path d="M16.5 24 Q17 13 32 13 Q47 13 47.5 24 Q40 19 32 19 Q24 19 16.5 24 Z" fill={fill} />
      );
    default: // half-round (0), bun (2), ponytail (5)
      return (
        <G fill={fill}>
          <Path d="M15.5 27 Q14.5 9.5 32 9.5 Q49.5 9.5 48.5 27 Q42 19 32 19.5 Q22 19 15.5 27 Z" />
          {style === 2 ? <Circle cx={32} cy={8} r={6} /> : null}
        </G>
      );
  }
}
