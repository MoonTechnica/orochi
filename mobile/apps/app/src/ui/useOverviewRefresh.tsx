import React, { useState } from 'react';
import { RefreshControl } from 'react-native';
import { usePrefs } from '../state/prefs';

export function useOverviewRefresh(refresh: () => Promise<void>) {
  const { colors } = usePrefs();
  const [refreshing, setRefreshing] = useState(false);
  return (
    <RefreshControl
      refreshing={refreshing}
      tintColor={colors.text}
      onRefresh={async () => {
        setRefreshing(true);
        try {
          await refresh();
        } finally {
          setRefreshing(false);
        }
      }}
    />
  );
}
