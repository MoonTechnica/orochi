import React from 'react';
import { Image } from 'react-native';

/** The desktop's icon art on its own black tile (assets/icon/logo.png), so it reads on any screen. */
export function Logo({ size = 40 }: { size?: number }) {
  return (
    <Image
      source={require('../../assets/icon/logo.png')}
      style={{ width: size, height: size }}
      accessible
      accessibilityRole="image"
      accessibilityLabel="Orochi"
    />
  );
}
