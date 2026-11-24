import React, { Profiler, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FlatList, Platform, Pressable, RefreshControl, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import {
  filterCounts,
  hueOf,
  isCrowded,
  lookOf,
  officeItems,
  sameRole,
  type OfficeFilter,
  type OfficeItem,
  type Person,
  type Room,
} from '@orochi/core';
import { useOffice } from '../../src/state/office';
import { useOfficeFilter } from '../../src/state/officeView';
import { useOpenThread } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { Icon } from '../../src/ui/Icon';
import { Header } from '../../src/ui/Screen';
import { Avatar } from '../../src/ui/office/Avatar';
import { CompactRow } from '../../src/ui/office/CompactRow';
import { Filters } from '../../src/ui/office/Filters';
import { MotionProvider, MotionRoom, useMotionStore } from '../../src/ui/office/motion';
import { RoomCard } from '../../src/ui/office/RoomCard';
import { RoomSheet } from '../../src/ui/office/RoomSheet';
import { SeatSheet } from '../../src/ui/office/SeatSheet';
import { Strip } from '../../src/ui/office/Strip';

const idle = (provider: string): Person => {
  const who = { role: provider, agent: provider };
  return {
    ...who,
    ...lookOf(who),
    hue: hueOf(who),
    lead: false,
    provider,
    state: 'idle',
    speaking: false,
  } as Person;
};

const EMPTY_FILTER: Record<Exclude<OfficeFilter, 'all'>, string> = {
  needs: 'Nothing needs you right now.',
  working: 'Nobody is working right now.',
};

const VIEWABILITY = { itemVisiblePercentThreshold: 10, minimumViewTime: 100 };

// Dev only: how long the screen takes to put its first rooms on screen, and what React spent on it.
const timing = (globalThis as { __officeTiming?: Record<string, number> }).__officeTiming ?? {};
(globalThis as { __officeTiming?: Record<string, number> }).__officeTiming = timing;

/**
 * The desktop's Office for a phone: projects are floors, conversations are rooms. The page is one
 * flat list of floor headers and room rows, so only the rooms near the screen are mounted; a room
 * shows its first six people, a crowded floor shows one-line rows, and only what is on screen moves.
 */
export default function Office() {
  const { colors, t } = usePrefs();
  const { active, titleOf } = useSession();
  const insets = useSafeAreaInsets();
  const open = useOpenThread();
  const key = active?.key ?? null;
  const { machine, snap, office, icons, client, refreshAll, markSeen } = useOffice(key);
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set());
  const [openRoom, setOpenRoom] = useState<Record<string, string>>({});
  const [closed, setClosed] = useState<ReadonlySet<string>>(new Set());
  const [filter, setFilter] = useOfficeFilter(key);
  const [sheet, setSheet] = useState<Person | null>(null);
  const [listId, setListId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const offline = !snap || snap.state !== 'online';
  const motion = useMotionStore();

  const rooms = useMemo(() => office.floors.flatMap((f) => f.rooms), [office]);
  const items = useMemo(
    () => officeItems(office.floors, { filter, folded, picked: openRoom, closed }),
    [office, filter, folded, openRoom, closed],
  );
  const counts = useMemo(() => filterCounts(office.floors), [office]);
  const listRoom = listId ? (rooms.find((r) => r.id === listId) ?? null) : null;

  const bossOf = (p: Person | null) => {
    if (!p || p.lead) return null;
    const room = rooms.find((r) => r.id === p.thread);
    const boss = p.manager
      ? room?.people.find((o) => o !== p && sameRole(o.role, p.manager))
      : room?.people.find((o) => o.lead);
    return boss ? boss.role || t('lead') : null;
  };
  const show = useCallback(
    (p: Person) => {
      if (p.mail) markSeen(p.mail.seq);
      setSheet(p);
    },
    [markSeen],
  );
  const go = (thread: string) => key && open(key, thread);
  const onMore = useCallback((r: Room) => setListId(r.id), []);
  const fromList = useCallback(
    (p: Person) => {
      setListId(null);
      // A second sheet cannot be presented while the first is still going away.
      setTimeout(() => show(p), 350);
    },
    [show],
  );
  const expand = useCallback((r: Room, floor: string) => {
    setClosed((s) => {
      if (!s.has(r.id)) return s;
      const n = new Set(s);
      n.delete(r.id);
      return n;
    });
    setOpenRoom((o) => ({ ...o, [floor]: r.id }));
  }, []);
  const collapse = useCallback((r: Room, floor: string) => {
    setClosed((s) => new Set(s).add(r.id));
    setOpenRoom((o) => {
      if (o[floor] !== r.id) return o;
      const { [floor]: _gone, ...rest } = o;
      return rest;
    });
  }, []);

  const onViewable = useRef(({ viewableItems }: { viewableItems: { item?: OfficeItem }[] }) => {
    const seen = viewableItems.map((v) => v.item).filter((i): i is OfficeItem => !!i);
    motion.set({
      visible: new Set(seen.flatMap((i) => (i.kind === 'room' ? [i.room.id] : []))),
      crowded: isCrowded(seen),
    });
  }).current;
  const viewability = useRef([
    { viewabilityConfig: VIEWABILITY, onViewableItemsChanged: onViewable },
  ]).current;

  const mountedAt = useRef(performance.now());
  const painted = useRef(false);
  useEffect(() => {
    if (!__DEV__ || painted.current || !items.some((i) => i.kind === 'room')) return;
    painted.current = true;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        timing.firstPaintMs = Math.round(performance.now() - mountedAt.current);
        timing.rooms = rooms.length;
        console.log(`[office] first paint ${timing.firstPaintMs} ms, ${rooms.length} rooms`);
      }),
    );
  }, [items, rooms.length]);

  const renderItem = useCallback(
    ({ item }: { item: OfficeItem }) => {
      if (item.kind === 'floor') {
        const count = t(item.rooms === 1 ? '{n} room' : '{n} rooms').replace(
          '{n}',
          String(item.rooms),
        );
        return (
          <Pressable
            testID={`floor-${item.floor}`}
            accessibilityRole="button"
            accessibilityState={{ expanded: !item.folded }}
            accessibilityLabel={`${item.floor}, ${count}. ${item.folded ? t('Show this floor') : t('Hide this floor')}`}
            onPress={() =>
              setFolded((s) => {
                const n = new Set(s);
                if (!n.delete(item.floor)) n.add(item.floor);
                return n;
              })
            }
            style={{
              flexDirection: 'row',
              alignItems: 'center',
              gap: 8,
              minHeight: 44,
              marginBottom: 4,
            }}
          >
            <Icon
              name={item.folded ? 'chevron-forward' : 'chevron-down'}
              size={18}
              color={colors.sub}
            />
            <Text
              numberOfLines={1}
              style={{
                flex: 1,
                color: colors.sub,
                fontSize: 12,
                fontWeight: '800',
                letterSpacing: 1.2,
              }}
            >
              {item.floor.toUpperCase()}
            </Text>
            {item.asking ? (
              <View
                style={{ width: 9, height: 9, borderRadius: 5, backgroundColor: colors.primary }}
              />
            ) : null}
            <Text style={{ color: colors.muted, fontSize: 12 }}>{count}</Text>
          </Pressable>
        );
      }
      return (
        <MotionRoom id={item.room.id}>
          <View style={{ marginBottom: 10 }}>
            {item.expanded ? (
              <RoomCard
                room={item.room}
                icons={icons}
                onOpen={(r) => go(r.id)}
                onPerson={show}
                onMore={onMore}
                onCollapse={item.compact ? (r) => collapse(r, item.floor) : undefined}
              />
            ) : (
              <CompactRow room={item.room} icons={icons} onOpen={(r) => expand(r, item.floor)} />
            )}
          </View>
        </MotionRoom>
      );
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [colors, t, icons, key, show, onMore, collapse, expand],
  );

  const header = (
    <View style={{ gap: 14, marginBottom: 14 }}>
      {offline ? (
        <Text style={{ color: colors.sub, textAlign: 'center', padding: 24 }}>
          {snap?.error || t('This desktop does not answer.')}
        </Text>
      ) : (
        <>
          <Strip stats={office.stats} filter={filter} onFilter={setFilter} />
          {office.quiet ? null : <Filters value={filter} counts={counts} onChange={setFilter} />}
          {office.quiet ? (
            <View
              testID="office-quiet"
              style={{ alignItems: 'center', gap: 14, paddingVertical: 34 }}
            >
              <View style={{ flexDirection: 'row', gap: 22 }}>
                {['anthropic', 'openai', 'google'].map((p) => (
                  <Avatar key={p} person={idle(p)} size={56} />
                ))}
              </View>
              <Text style={{ color: colors.sub, textAlign: 'center', fontSize: 14, maxWidth: 280 }}>
                {t('The office is quiet. Start a thread to put someone to work.')}
              </Text>
            </View>
          ) : null}
          {!office.quiet && filter !== 'all' && !items.length ? (
            <View
              testID="office-filter-empty"
              style={{ alignItems: 'center', gap: 12, paddingVertical: 28 }}
            >
              <Text style={{ color: colors.sub, textAlign: 'center', fontSize: 14 }}>
                {t(EMPTY_FILTER[filter])}
              </Text>
              <Pressable
                accessibilityRole="button"
                onPress={() => setFilter('all')}
                style={{
                  minHeight: 44,
                  paddingHorizontal: 18,
                  borderRadius: 10,
                  borderWidth: 1,
                  borderColor: colors.line,
                  justifyContent: 'center',
                }}
              >
                <Text style={{ color: colors.text, fontWeight: '700' }}>{t('Show all rooms')}</Text>
              </Pressable>
            </View>
          ) : null}
        </>
      )}
    </View>
  );

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Office')} subtitle={machine ? titleOf(machine) : undefined} />
      <MotionProvider value={motion}>
        <Profiler
          id="office"
          onRender={(_id, _phase, actual) => {
            if (__DEV__) timing.renderMs = Math.round(((timing.renderMs ?? 0) + actual) * 10) / 10;
          }}
        >
          <FlatList
            testID="office-list"
            data={offline ? [] : items}
            keyExtractor={(i) => i.key}
            renderItem={renderItem}
            ListHeaderComponent={header}
            contentContainerStyle={{ padding: 16, paddingBottom: insets.bottom + 40 }}
            initialNumToRender={6}
            maxToRenderPerBatch={4}
            windowSize={3}
            updateCellsBatchingPeriod={40}
            removeClippedSubviews={Platform.OS === 'android'}
            viewabilityConfigCallbackPairs={viewability}
            refreshControl={
              <RefreshControl
                refreshing={refreshing}
                tintColor={colors.text}
                onRefresh={async () => {
                  setRefreshing(true);
                  await refreshAll();
                  setRefreshing(false);
                }}
              />
            }
          />
        </Profiler>
      </MotionProvider>
      <RoomSheet
        room={listRoom}
        icons={icons}
        onClose={() => setListId(null)}
        onPerson={fromList}
      />
      <SeatSheet
        person={sheet}
        icon={sheet ? icons[sheet.provider] : undefined}
        boss={bossOf(sheet)}
        canStop={!!client && !offline}
        onClose={() => setSheet(null)}
        onOpen={(p) => go(p.thread)}
        onStop={async (p) => {
          if (!client || !p.seatId) return;
          await client.stopSeat(p.thread, p.seatId);
          await refreshAll();
        }}
      />
    </View>
  );
}
