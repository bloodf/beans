import { Alert, type StyleProp, type TextStyle } from "react-native";
import type { ChatSearchResults } from "../core/model";
import { AvatarCluster } from "./Avatar";
import { stamp } from "./format";
import { AvatarFlashList } from "./AvatarFlashList";
import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import { haptic } from "./haptics";
import { Link, Stack, useRouter } from "expo-router";
import Swipeable, { type SwipeableMethods } from "react-native-gesture-handler/ReanimatedSwipeable";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Platform, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { Pressable } from "./Pressable";
import { chatTitle, engine } from "../core/engine";
import { isMuted, type Bot, type Chat, type Section } from "../core/model";
import { markRead, mutate, useBotMap, useStore, useWorkingBotIds } from "../core/store";
import { t, tc, useLanguage } from "../i18n";
import { ChatPeek } from "./ChatPeek";
import { ChatRow } from "./ChatRow";
import { PaneWidth, useSidebarWidth } from "./layout";
import { problemTitle, showRelayProblem } from "./relay";
import { lastActivity, muteSpans } from "./format";

import { SidebarSearch, useSidebarSearchInset } from "./SidebarSearch";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";
import { AndroidIcons } from "./navigation";
import { discardComposerDraft, retainedComposerDrafts, subscribeComposerDrafts } from "../core/updateDrafts";
import { alert, prompt } from "./alert";
import { chatListRows, groupOf, type ChatGroup, type ChatListRow } from "./chatList";

// FlashList keeps the first visible row where it is when rows change, which for a list resting at
// its top means a chat moving to the top pushes the list down by one row: the new first row lands
// under the bar. The list holds its offset instead, so at the top the new first row shows.
const KEEP_OFFSET = { disabled: true };

/// The chat list: the first screen of a narrow window, the sidebar of a wide one. In the sidebar
/// a chat opens in the pane beside the list, in place of the one open there, and its row stays lit.
/// `bar: false` leaves the bar to the screen that shows it (the phone's Bots tab; see ChatsBar).
export function ChatsScreen({ sidebar = false, bar = true }: { sidebar?: boolean; bar?: boolean }) {
  const language = useLanguage();
  const p = usePalette();
  const router = useRouter();
  // Only the sidebar lights the open chat; a phone's list would re-render under the chat it pushes.
  const openChatId = useStore((s) => (sidebar ? s.openChatId : null));
  const sidebarWidth = useSidebarWidth();
  // The sidebar's search field sits at the foot of the list (see SidebarSearch); a phone searches
  // in its Search tab.
  const searchInset = useSidebarSearchInset();
  const chats = useStore((s) => s.chats);
  const sections = useStore(s => s.sections);
  const showsHidden = useStore(s => s.showsHidden);
  const collapsesOthers = useStore(s => s.collapsesOthers);
  const identityId = useStore((s) => s.identityId);
  const running = useStore((s) => s.running);
  const [draftRevision, setDraftRevision] = useState(0);
  useEffect(() => subscribeComposerDrafts(() => setDraftRevision(value => value + 1)), []);
  const retainedDrafts = useMemo(() => retainedComposerDrafts(), [draftRevision]);
  const paused = useStore((s) => s.paused);
  const connecting = useConnecting();
  const updateRequired = useStore((s) => s.relayUpdateRequired);
  const relayError = useStore((s) => s.relayError);
  const relayUrl = useStore((s) => s.relayUrl);
  const deviceId = useStore(s => s.deviceId);
  const paired = useStore(s => s.paired);
  const bots = useBotMap();
  const workingBots = useWorkingBotIds();
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<ChatSearchResults>({ chats: [], messages: [], files: [], history_complete: true });
  const [searching, setSearching] = useState(false);
  const [searchGeneration, setSearchGeneration] = useState(0);
  const searchOwner = useRef<{ active: boolean; query: string } | null>(null);

  function updateQuery(value: string) {
    setQuery(value);
    setMatches({ chats: [], messages: [], files: [], history_complete: true });
    setSearching(!!value.trim());
  }

  useEffect(() => {
    const value = query.trim();
    const source = useStore.getState();
    if (!value || !source.paired || !source.identityId || !source.deviceId) {
      setSearching(false);
      return;
    }
    const owner = { active: true, query: value };
    searchOwner.current = owner;
    // Observe every core authority transition, not only React's eventual render.
    // Once revoked, this request/result owner cannot regain authority after ABA.
    const unsubscribe = useStore.subscribe(state => {
      if (owner.active && (state.identityId !== source.identityId || state.relayUrl !== source.relayUrl
        || state.deviceId !== source.deviceId || state.paired !== source.paired)) {
        owner.active = false;
        setMatches({ chats: [], messages: [], files: [], history_complete: true });
        setSearchGeneration(generation => generation + 1);
      }
    });
    setSearching(true);
    setMatches({ chats: [], messages: [], files: [], history_complete: true });
    const timer = setTimeout(() => {
      if (!owner.active) return;
      void engine
        .searchChats(value)
        .then((results) => {
          if (owner.active) setMatches(results);
        })
        .catch(() => { if (owner.active) setMatches({ chats: [], messages: [], files: [], history_complete: false }); })
        .finally(() => {
          if (owner.active) setSearching(false);
        });
    }, 140);
    return () => {
      owner.active = false;
      unsubscribe();
      clearTimeout(timer);
    };
  }, [query, chats, bots, searchGeneration, identityId, deviceId, paired, relayUrl]);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const visible = chats
      .filter((c) => !q || chatTitle(c).toLowerCase().includes(q) || c.bot_ids.some((id) => bots.get(id)?.name.toLowerCase().includes(q)))
      .sort((a, b) => lastActivity(b) - lastActivity(a));
    return [...visible.filter((c) => c.is_pinned), ...visible.filter((c) => !c.is_pinned)];
  }, [chats, bots, query, language]);

  const ownsSearch = !!searchOwner.current?.active && searchOwner.current.query === query.trim();
  const searchRows = useMemo(() => {
    if (!query.trim() || !ownsSearch) return [];
    const byId = new Map(chats.map((chat) => [chat.id, chat]));
    const seen = new Set<string>();
    const rows: SearchRow[] = [];
    for (const hit of matches.chats) {
      const chat = byId.get(hit.chat_id);
      if (!chat || seen.has(chat.id)) continue;
      seen.add(chat.id);
      rows.push({ key: `chat:${chat.id}`, kind: "chat", chat, snippet: hit.snippet });
    }
    for (const hit of matches.messages) {
      const chat = byId.get(hit.chat_id);
      if (!chat) continue;
      rows.push({ key: `message:${hit.message_id}`, kind: "message", chat, snippet: hit.snippet, createdAt: hit.created_at });
    }
    for (const hit of matches.files) {
      const chat = byId.get(hit.chat_id);
      if (!chat) continue;
      rows.push({ key: `file:${hit.message_id}:${hit.attachment_id}`, kind: "file", chat, snippet: hit.name, createdAt: hit.created_at });
    }
    return rows;
  }, [chats, bots, items, matches, query, language, searchGeneration, ownsSearch]);

  const searchingText = query.trim();
  const rows = useMemo(() => chatListRows(items, sections, showsHidden, collapsesOthers), [items, sections, showsHidden, collapsesOthers]);
  const data: (ChatListRow | SearchRow)[] = searchingText ? searchRows : rows;

  // A chat opened beside the sidebar some other way (a notification, search) shows its row: its
  // group unfolds.
  useEffect(() => {
    const chat = openChatId ? chats.find((each) => each.id === openChatId) : undefined;
    const group = chat && groupOf(chat, sections);
    if (!group) return;
    const folded = group.kind === "section" ? !!sections.find((section) => section.id === group.id)?.collapsed : group.kind === "others" ? collapsesOthers : !showsHidden;
    if (folded) unfold(group);
    // Only a newly opened chat; a group folded around the open chat stays folded.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openChatId]);

  // Chats with a turn in flight: their rows read "Working…" in place of the preview.
  const responding = useMemo(() => new Set(Object.values(running).map((r) => r.chatId)), [running]);

  // The rows are memoized, so what they call back stays the same function across renders; it
  // reads this render's state through the ref.
  const latest = useRef({ sidebar, openChatId });
  latest.current = { sidebar, openChatId };
  const openChat = useCallback((chat: Chat) => {
    const { sidebar, openChatId } = latest.current;
    if (!sidebar) return router.push(`/chat/${chat.id}`);
    if (chat.id === openChatId) return;
    if (openChatId) router.replace(`/chat/${chat.id}`);
    else router.push(`/chat/${chat.id}`);
  }, [router]);

  const confirmDelete = useCallback((chat: Chat) => {
    const { sidebar, openChatId } = latest.current;
    alert(t("Delete “{name}”?", { name: chatTitle(chat) }), t("The chat and its messages are removed from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: async () => {
          try {
            await engine.deleteChat(chat.id);
            if (sidebar && chat.id === openChatId) router.dismissTo("/");
          } catch (error) {
            Alert.alert(t("Delete"), error instanceof Error ? error.message : String(error));
          }
        },
      },
    ]);
  }, [router]);

  const renderItem = useCallback(
    ({ item }: { item: ChatListRow | SearchRow }) => {
      if ("snippet" in item) return <SearchResultRow item={item} bots={bots} query={searchingText} onPress={() => openChat(item.chat)} />;
      if (item.kind === "header") return <GroupHeader group={item.group} collapsed={item.collapsed} sections={sections} width={sidebar ? sidebarWidth : undefined} />;
      const chat = item.chat;
      const working = responding.has(chat.id) || chat.bot_ids.some((id) => workingBots.has(id));
      // A sidebar row has no peek: the chat opens beside it.
      if (Platform.OS === "android" || sidebar)
        return (
          <MenuChatRow
            chat={chat}
            bots={bots}
            working={working}
            responding={responding.has(chat.id)}
            selected={sidebar ? chat.id === openChatId : undefined}
            width={sidebar ? sidebarWidth : undefined}
            sections={sections}
            onOpen={openChat}
            onDelete={confirmDelete}
          />
        );
      return <PeekChatRow chat={chat} bots={bots} working={working} responding={responding.has(chat.id)} sections={sections} onDelete={confirmDelete} />;
    },
    [bots, confirmDelete, openChat, openChatId, responding, searchingText, sections, sidebar, sidebarWidth, workingBots],
  );

  return (
    <>
      {retainedDrafts.map(([key, draft]) => {
        const [identity, relay, chatId] = JSON.parse(key) as [string | null, string | null, string];
        if (identity === identityId && relay === relayUrl && chats.some(chat => chat.id === chatId)) return null;
        const preview = [draft.text, ...draft.attachments.map(file => file.name)].filter(Boolean).join("\n");
        return <Pressable key={key} accessibilityRole="button" accessibilityLabel={`${t("Discard")}: ${preview}`} onPress={() => Alert.alert(t("Discard changes?"), preview, [
          { text: t("Cancel"), style: "cancel" },
          { text: t("Discard"), style: "destructive", onPress: () => discardComposerDraft(key) },
        ])}><Text style={{ color: p.label }}>{t("Draft preview")}: {preview || t("Dictation stopped")}</Text><Text style={{ color: p.secondaryLabel }}>{t("Discard")}</Text></Pressable>;
      })}
      {query.trim() && ownsSearch && !matches.history_complete ? <Text accessibilityRole="text" style={{ color: p.secondaryLabel }}>{t("Search covers downloaded history only")}</Text> : null}
            {sidebar ? null : (
        <Stack.SearchBar
          placeholder={t("Search")}
          onChangeText={(e) => updateQuery(e.nativeEvent.text)}
          onCancelButtonPress={() => updateQuery("")}
          hideWhenScrolling
          autoCapitalize="none"
        />
      )}
      {Platform.OS === "ios" ? (
        <>
          <Stack.Toolbar placement="left">
            <Stack.Toolbar.Button icon="gearshape" accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          </Stack.Toolbar>
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Menu icon="square.and.pencil" accessibilityLabel={t("New")}>
              <Stack.Toolbar.MenuAction icon="person.2.fill" onPress={() => router.push("/new-group")}>
                {t("New Group Chat")}
              </Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="person.badge.plus" onPress={() => router.push("/new-bot")}>
                {t("New Bot")}
              </Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar>
        </>
      ) : (
        <>
          <Stack.Toolbar placement="left" tintColor={p.secondaryLabel}>
            <Stack.Toolbar.Button icon={AndroidIcons.settings} accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          </Stack.Toolbar>
          <Stack.Toolbar placement="right" tintColor={p.secondaryLabel}>
            <Stack.Toolbar.Menu icon={AndroidIcons.add} accessibilityLabel={t("New")}>
              <Stack.Toolbar.MenuAction icon={AndroidIcons.group} onPress={() => router.push("/new-group")}>
                {t("New Group Chat")}
              </Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon={AndroidIcons.personAdd} onPress={() => router.push("/new-bot")}>
                {t("New Bot")}
              </Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar>
        </>
      )}
      {/* The relay status sits in the bar's title slot, so the list never moves. Once a try to
          connect has failed it says so, and a tap shows the error. */}
      {updateRequired ? (
        <Stack.Title asChild>
          <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Update Beans to sync")}>
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {t("Update Beans to sync")}
            </Text>
          </View>
        </Stack.Title>
      ) : connecting && relayError ? (
        <Stack.Title asChild>
          <Pressable
            style={({ pressed }) => [styles.status, pressed && styles.statusPressed]}
            accessibilityRole="button"
            accessibilityLabel={problemTitle(relayError)}
            onPress={() => showRelayProblem(relayError, relayUrl)}
          >
            <Symbol name="exclamationmark.triangle.fill" size={13} color={p.red} />
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {problemTitle(relayError)}
            </Text>
          </Pressable>
        </Stack.Title>
      ) : connecting ? (
        <Stack.Title asChild>
          <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Connecting…")}>
            <ActivityIndicator size="small" color={p.secondaryLabel as any} />
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {t("Connecting…")}
            </Text>
          </View>
        </Stack.Title>
      ) : paused ? (
        <Stack.Title asChild>
          <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Bots paused")}>
            <Symbol name="pause.circle.fill" size={14} color={p.secondaryLabel} />
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>{t("Bots paused")}</Text>
          </View>
        </Stack.Title>
      ) : null}
      <View style={styles.screen}>
        <AvatarFlashList
          style={styles.list}
          data={data}
          keyExtractor={(item) => item.key}
          getItemType={(item) => ("snippet" in item ? "search" : item.kind)}
          contentInsetAdjustmentBehavior="automatic"
          maintainVisibleContentPosition={KEEP_OFFSET}
          keyboardDismissMode="on-drag"
          onScrollBeginDrag={closeOpenRow}
          contentContainerStyle={{ paddingBottom: sidebar ? searchInset + 8 : 24 }}
          ListEmptyComponent={
            <View style={styles.empty}>
              <Symbol name="sparkles" size={36} color={p.tertiaryLabel} />
              <Text style={[styles.emptyTitle, { color: p.label }]}>{query ? (searching ? t("Searching…") : t("No matches")) : t("No chats yet")}</Text>
              <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{query ? t("Try another word.") : t("Your bots and their chats sync from the relay once this phone hears from your Runner.")}</Text>
            </View>
          }
          renderItem={renderItem}
          ItemSeparatorComponent={Separator}
          onRefresh={() => {
            haptic.refresh();
            engine.notify();
          }}
          refreshing={false}
        />
        {sidebar ? <SidebarSearch value={query} placeholder={t("Search")} onChangeText={setQuery} /> : null}
      </View>
    </>
  );
}

/// True once the relay has been unreachable for a moment: a launch or a quick reconnect shows
/// nothing, and neither does the reconnect after coming back to the foreground, which replaces
/// the socket the suspension killed and can take a few seconds on a slow network or a VPN.
/// The chat list's bar: Settings and Attention on the left, New on the right, and the relay's
/// status in the title slot. The list shows it in its own stack's bar; the phone's Bots tab shows
/// it in the bar of the stack around the tabs, the one a chat slides into, so that one bar carries
/// the list's buttons and then the chat's (TabsLayout).
export function ChatsBar() {
  useLanguage();
  const p = usePalette();
  const router = useRouter();
  const connecting = useConnecting();
  const updateRequired = useStore((s) => s.relayUpdateRequired);
  const relayError = useStore((s) => s.relayError);
  const relayUrl = useStore((s) => s.relayUrl);
  // The Attention button shows while something waits on the user.
  const needsAttention = useStore((s) => s.attention.items.length > 0);
  return (
    <>
    {Platform.OS === "ios" ? (
      <>
        <Stack.Toolbar placement="left">
          <Stack.Toolbar.Button icon="gearshape" accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          <Stack.Toolbar.Button hidden={!needsAttention} icon="tray.full" accessibilityLabel={t("Attention")} onPress={() => router.push("/attention")} />
        </Stack.Toolbar>
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Menu icon="square.and.pencil" accessibilityLabel={t("New")}>
            <Stack.Toolbar.MenuAction icon="person.2.fill" onPress={() => router.push("/new-group")}>
              {t("New Group Chat")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="person.badge.plus" onPress={() => router.push("/new-bot")}>
              {t("New Bot")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="square.and.arrow.down" onPress={() => router.push("/template")}>
              {t("New Bot from Template…")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="point.3.connected.trianglepath.dotted" onPress={() => router.push("/workflows")}>
              {t("New Workflow")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon="folder.badge.plus" onPress={() => newSection()}>
              {t("New Section")}
            </Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      </>
    ) : (
      <>
        <Stack.Toolbar placement="left" tintColor={p.secondaryLabel}>
          <Stack.Toolbar.Button icon={AndroidIcons.settings} accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          <Stack.Toolbar.Button hidden={!needsAttention} icon={AndroidIcons.inbox} accessibilityLabel={t("Attention")} onPress={() => router.push("/attention")} />
        </Stack.Toolbar>
        <Stack.Toolbar placement="right" tintColor={p.secondaryLabel}>
          <Stack.Toolbar.Menu icon={AndroidIcons.add} accessibilityLabel={t("New")}>
            <Stack.Toolbar.MenuAction icon={AndroidIcons.group} onPress={() => router.push("/new-group")}>
              {t("New Group Chat")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon={AndroidIcons.personAdd} onPress={() => router.push("/new-bot")}>
              {t("New Bot")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon={AndroidIcons.download} onPress={() => router.push("/template")}>
              {t("New Bot from Template…")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon={AndroidIcons.workflow} onPress={() => router.push("/workflows")}>
              {t("New Workflow")}
            </Stack.Toolbar.MenuAction>
            <Stack.Toolbar.MenuAction icon={AndroidIcons.newFolder} onPress={() => newSection()}>
              {t("New Section")}
            </Stack.Toolbar.MenuAction>
          </Stack.Toolbar.Menu>
        </Stack.Toolbar>
      </>
    )}
    {/* The relay status sits in the bar's title slot, so the list never moves. Once a try to
        connect has failed it says so, and a tap shows the error. */}
    {updateRequired ? (
      <Stack.Title asChild>
        <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Update Lorca to sync")}>
          <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
            {t("Update Lorca to sync")}
          </Text>
        </View>
      </Stack.Title>
    ) : connecting && relayError ? (
      <Stack.Title asChild>
        <Pressable
          style={({ pressed }) => [styles.status, pressed && styles.statusPressed]}
          accessibilityRole="button"
          accessibilityLabel={problemTitle(relayError)}
          onPress={() => showRelayProblem(relayError, relayUrl)}
        >
          <Symbol name="exclamationmark.triangle.fill" size={13} color={p.red} />
          <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
            {problemTitle(relayError)}
          </Text>
        </Pressable>
      </Stack.Title>
    ) : connecting ? (
      <Stack.Title asChild>
        <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Connecting…")}>
          <ActivityIndicator size="small" color={p.secondaryLabel as any} />
          <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
            {t("Connecting…")}
          </Text>
        </View>
      </Stack.Title>
    ) : null}
    </>
  );
}

function useConnecting(): boolean {
  const relayConnected = useStore((s) => s.relayConnected);
  const activeSince = useStore((s) => s.activeSince);
  const [connecting, setConnecting] = useState(false);
  useEffect(() => {
    if (relayConnected) return setConnecting(false);
    const timer = setTimeout(() => setConnecting(true), Math.max(1000, activeSince + 4000 - Date.now()));
    return () => clearTimeout(timer);
  }, [relayConnected, activeSince]);
  return connecting;
}

/// The separator between rows: one component, so the list keeps its separators across renders.
/// A group's header has none above or below it.
function Separator({ leadingItem, trailingItem }: { leadingItem?: ChatListRow | SearchRow; trailingItem?: ChatListRow | SearchRow }) {
  const p = usePalette();
  const header = (item?: ChatListRow | SearchRow) => !!item && !("snippet" in item) && item.kind === "header";
  if (header(leadingItem) || header(trailingItem)) return null;
  return <View style={[styles.separator, { backgroundColor: p.separator }]} />;
}

// MARK: - Groups

function groupTitle(group: ChatGroup, sections: Section[]): string {
  if (group.kind === "section") return sections.find((section) => section.id === group.id)?.name ?? "";
  return group.kind === "others" ? tc("Chats", "no section") : t("Hidden");
}

/// Folds or unfolds a group: a section on every Device, the others on this phone.
function fold(group: ChatGroup, collapsed: boolean) {
  if (group.kind === "section") engine.setSectionCollapsed(group.id, collapsed);
  else if (group.kind === "others") mutate(() => ({ collapsesOthers: collapsed }));
  else mutate(() => ({ showsHidden: !collapsed }));
}

function unfold(group: ChatGroup) {
  fold(group, false);
}

function newSection(chatId?: string) {
  prompt(t("New Section"), {
    placeholder: t("Section name"),
    confirm: t("Create"),
    done: (name) => engine.createSection(name, chatId),
  });
}

function renameSection(section: Section) {
  prompt(t("Rename Section"), {
    value: section.name,
    placeholder: t("Section name"),
    confirm: t("Rename"),
    done: (name) => engine.renameSection(section.id, name),
  });
}

function confirmDeleteSection(section: Section) {
  alert(t("Delete “{name}”?", { name: section.name }), t("Its chats move to {group}.", { group: tc("Chats", "no section") }), [
    { text: t("Cancel"), style: "cancel" },
    { text: t("Delete"), style: "destructive", onPress: () => engine.deleteSection(section.id) },
  ]);
}

/// A group's header: its name and a chevron, as a collapsible list section on each platform. A
/// tap folds it; a long press on a section's header renames, moves, or deletes it, and on the
/// Chats header starts a section.
const GroupHeader = memo(function GroupHeader({ group, collapsed, sections, width: fixedWidth }: { group: ChatGroup; collapsed: boolean; sections: Section[]; width?: number }) {
  useLanguage();
  const p = usePalette();
  const ios = Platform.OS === "ios";
  const menuRef = useRef<MenuComponentRef>(null);
  const { width: windowWidth } = useWindowDimensions();
  // The menu's host takes its size from what it is given, as a chat row's does.
  const width = fixedWidth ?? windowWidth;
  const title = groupTitle(group, sections);
  const place = group.kind === "section" ? sections.findIndex((section) => section.id === group.id) : -1;
  const section = place >= 0 ? sections[place] : undefined;
  const actions: MenuAction[] = section
    ? [
        { id: "rename", title: t("Rename Section…"), image: ios ? "pencil" : undefined },
        { id: "up", title: t("Move Up"), image: ios ? "arrow.up" : undefined, attributes: { disabled: place === 0 } },
        { id: "down", title: t("Move Down"), image: ios ? "arrow.down" : undefined, attributes: { disabled: place === sections.length - 1 } },
        { id: "delete", title: t("Delete Section…"), image: ios ? "trash" : AndroidIcons.delete, attributes: { destructive: true } },
      ]
    : group.kind === "others"
      ? [{ id: "new", title: t("New Section…"), image: ios ? "folder.badge.plus" : AndroidIcons.newFolder }]
      : [];
  const header = (
    <Pressable
      onPress={() => fold(group, !collapsed)}
      onLongPress={!ios && actions.length ? () => menuRef.current?.show() : undefined}
      style={[styles.header, { width }]}
      accessibilityRole="button"
      accessibilityLabel={title}
      accessibilityState={{ expanded: !collapsed }}
    >
      <Text style={[styles.headerTitle, { color: ios ? p.label : p.tint }]} numberOfLines={1}>
        {title}
      </Text>
      <Symbol name={collapsed ? "chevron.right" : "chevron.down"} size={ios ? 15 : 20} color={ios ? p.tint : p.secondaryLabel} weight="semibold" />
    </Pressable>
  );
  if (!actions.length) return header;
  return (
    <MenuView
      ref={menuRef}
      actions={actions}
      shouldOpenOnLongPress
      style={{ width }}
      onOpenMenu={haptic.longPress}
      onPressAction={({ nativeEvent }) => {
        if (nativeEvent.event === "new") newSection();
        if (!section) return;
        if (nativeEvent.event === "rename") renameSection(section);
        else if (nativeEvent.event === "up") engine.moveSection(section.id, place - 1);
        else if (nativeEvent.event === "down") engine.moveSection(section.id, place + 1);
        else if (nativeEvent.event === "delete") confirmDeleteSection(section);
      }}
    >
      {header}
    </MenuView>
  );
});

/// A chat's menu items that quiet and file it, after the desktop app's: Mute and its spans or
/// Unmute, Move to Section and the sections (Move to New Section… while there are none), and Hide
/// or Show in Sidebar.
function sidebarActions(chat: Chat, sections: Section[]): MenuAction[] {
  const ios = Platform.OS === "ios";
  const current = chat.section_id && sections.some((section) => section.id === chat.section_id) ? chat.section_id : "";
  return [
    isMuted(chat)
      ? { id: "unmute", title: t("Unmute"), image: ios ? "bell" : AndroidIcons.unmute }
      : { id: "mute", title: t("Mute"), image: ios ? "bell.slash" : AndroidIcons.mute, subactions: muteSpans().map((span) => ({ id: `mute:${span.seconds}`, title: span.title })) },
    sections.length
      ? {
          id: "sections",
          title: t("Move to Section"),
          image: ios ? "folder" : AndroidIcons.folder,
          subactions: [
            ...sections.map((section) => ({ id: `section:${section.id}`, title: section.name, state: current === section.id ? ("on" as const) : ("off" as const) })),
            { id: "section:", title: tc("Chats", "no section"), state: current === "" ? ("on" as const) : ("off" as const) },
            { id: "new-section", title: t("New Section…"), image: ios ? "folder.badge.plus" : AndroidIcons.newFolder },
          ],
        }
      : { id: "new-section", title: t("Move to New Section…"), image: ios ? "folder.badge.plus" : AndroidIcons.newFolder },
    chat.is_hidden ? { id: "show", title: t("Show in Sidebar"), image: ios ? "eye" : AndroidIcons.show } : { id: "hide", title: t("Hide"), image: ios ? "eye.slash" : AndroidIcons.hide },
  ];
}

/// Does what one of `sidebarActions` names; false for an action it does not know.
function runSidebarAction(chat: Chat, event: string): boolean {
  if (event.startsWith("mute:")) engine.muteChat(chat.id, Number(event.slice(5)));
  else if (event === "unmute") engine.unmuteChat(chat.id);
  else if (event.startsWith("section:")) engine.moveChat(chat.id, event.slice(8) || null);
  else if (event === "new-section") newSection(chat.id);
  else if (event === "hide" || event === "show") engine.hideChat(chat.id, event === "hide");
  else return false;
  return true;
}

/// A row whose long press opens a native menu: every row on Android, a sidebar row on iOS.
/// Memoized: the list sits under an open chat and renders again with every change to any chat.
const MenuChatRow = memo(function MenuChatRow({ chat, bots, working, responding, selected, width: fixedWidth, sections, onOpen, onDelete }: { chat: Chat; bots: Map<string, Bot>; working: boolean; responding: boolean; selected?: boolean; width?: number; sections: Section[]; onOpen: (chat: Chat) => void; onDelete: (chat: Chat) => void }) {
  useLanguage();
  const menuRef = useRef<MenuComponentRef>(null);
  const onPress = useCallback(() => onOpen(chat), [chat, onOpen]);
  const onLongPress = useCallback(() => menuRef.current?.show(), []);
  const { width: windowWidth } = useWindowDimensions();
  const width = fixedWidth ?? windowWidth;
  // iOS says pinned through the Unpin title and glyph, as its Link menu does; Android checks the item.
  const ios = Platform.OS === "ios";
  const actions: MenuAction[] = [
    { id: "pin", title: chat.is_pinned ? t("Unpin") : t("Pin"), image: ios ? (chat.is_pinned ? "pin.slash" : "pin") : AndroidIcons.pin, state: !ios && chat.is_pinned ? "on" : "off" },
    ...sidebarActions(chat, sections),
    ...(chat.unread_count > 0 ? [{ id: "read", title: t("Mark as Read"), image: ios ? "checkmark.circle" : AndroidIcons.read } satisfies MenuAction] : []),
    { id: "delete", title: t("Delete"), image: ios ? "trash" : AndroidIcons.delete, attributes: { destructive: true } },
  ];

  return (
    <MenuView
      ref={menuRef}
      actions={actions}
      shouldOpenOnLongPress
      style={{ width }}
      onOpenMenu={haptic.longPress}
      onPressAction={({ nativeEvent }) => {
        if (nativeEvent.event === "pin") void engine.pinChat(chat.id, !chat.is_pinned);
        else if (runSidebarAction(chat, nativeEvent.event)) return;
        else if (nativeEvent.event === "read") markRead(chat.id);
        else if (nativeEvent.event === "delete") onDelete(chat);
      }}
    >
      <View style={{ width }}>
        <ChatRow chat={chat} bots={bots} title={chatTitle(chat)} working={working} responding={responding} selected={selected} onPress={onPress} onLongPress={ios ? undefined : onLongPress} />
      </View>
    </MenuView>
  );
});

/// An iPhone row, as in Messages: a tap opens the chat, a long press peeks at it with its menu, a
/// swipe to the left offers Pin and Delete and one to the right Mark as Read.
const PeekChatRow = memo(function PeekChatRow({ chat, bots, working, responding, sections, onDelete }: { chat: Chat; bots: Map<string, Bot>; working: boolean; responding: boolean; sections: Section[]; onDelete: (chat: Chat) => void }) {
  useLanguage();
  const router = useRouter();
  const title = chatTitle(chat);
  const onPress = useCallback(() => router.push(`/chat/${chat.id}`), [router, chat.id]);
  const unread = chat.unread_count > 0;
  const swipeRef = useRef<SwipeableMethods>(null);
  const opening = useCallback(() => {
    if (openRow && openRow !== swipeRef.current) openRow.close();
    openRow = swipeRef.current;
  }, []);
  const trailing = useCallback(
    (_progress: unknown, _translation: unknown, swipeable: SwipeableMethods) => (
      <View style={styles.swipeActions}>
        <SwipeAction
          icon={chat.is_pinned ? "pin.slash.fill" : "pin.fill"}
          label={chat.is_pinned ? t("Unpin") : t("Pin")}
          color="#FF9500"
          onPress={() => {
            swipeable.close();
            void engine.pinChat(chat.id, !chat.is_pinned);
          }}
        />
        <SwipeAction
          icon="trash.fill"
          label={t("Delete")}
          color="#FF3B30"
          onPress={() => {
            swipeable.close();
            onDelete(chat);
          }}
        />
      </View>
    ),
    [chat, onDelete],
  );
  const leading = useCallback(
    (_progress: unknown, _translation: unknown, swipeable: SwipeableMethods) => (
      <View style={styles.swipeActions}>
        <SwipeAction
          icon="checkmark.message.fill"
          label={t("Read")}
          color="#007AFF"
          onPress={() => {
            swipeable.close();
            markRead(chat.id);
          }}
        />
      </View>
    ),
    [chat.id],
  );
  return (
    <Swipeable ref={swipeRef} friction={1.6} overshootFriction={8} rightThreshold={40} leftThreshold={40} renderRightActions={trailing} renderLeftActions={unread ? leading : undefined} onSwipeableWillOpen={opening}>
    <Link href={`/chat/${chat.id}`} asChild>
      <Link.Trigger>
        <ChatRow chat={chat} bots={bots} title={title} working={working} responding={responding} onPress={onPress} />
      </Link.Trigger>
      <Link.Preview style={{ width: 340, height: 420 }}>
        <PaneWidth value={340}>
          <ChatPeek chat={chat} bots={bots} title={title} />
        </PaneWidth>
      </Link.Preview>
      <Link.Menu>
        <Link.MenuAction icon={chat.is_pinned ? "pin.slash" : "pin"} onPress={() => engine.pinChat(chat.id, !chat.is_pinned)}>
          {chat.is_pinned ? t("Unpin") : t("Pin")}
        </Link.MenuAction>
        {sidebarActions(chat, sections).map((action) =>
          action.subactions ? (
            <Link.Menu key={action.id} title={action.title} icon={action.image as any}>
              {action.subactions.map((sub) => (
                <Link.MenuAction key={sub.id} icon={sub.image as any} isOn={sub.state === "on"} onPress={() => runSidebarAction(chat, sub.id!)}>
                  {sub.title}
                </Link.MenuAction>
              ))}
            </Link.Menu>
          ) : (
            <Link.MenuAction key={action.id} icon={action.image as any} onPress={() => runSidebarAction(chat, action.id!)}>
              {action.title}
            </Link.MenuAction>
          ),
        )}
        {chat.unread_count > 0 ? (
          <Link.MenuAction icon="checkmark.circle" onPress={() => markRead(chat.id)}>
            {t("Mark as Read")}
          </Link.MenuAction>
        ) : null}
        <Link.MenuAction icon="trash" destructive onPress={() => onDelete(chat)}>
          {t("Delete")}
        </Link.MenuAction>
      </Link.Menu>
    </Link>
    </Swipeable>
  );
});

function closeOpenRow() {
  openRow?.close();
  openRow = null;
}

type SearchRow = {
  key: string;
  kind: "chat" | "message" | "file";
  chat: Chat;
  snippet: string;
  createdAt?: number;
};
/// The row whose actions show: one at a time, closed when the list scrolls, as in Messages.
let openRow: SwipeableMethods | null = null;

/// One button behind a swiped row: a full-height colored cell with its symbol over its name.
function SwipeAction({ icon, label, color, onPress }: { icon: string; label: string; color: string; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} style={[styles.swipeAction, { backgroundColor: color }]} accessibilityRole="button" accessibilityLabel={label}>
      <Symbol name={icon} size={20} color="#FFFFFF" />
      <Text style={styles.swipeLabel} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  searchRow: { flexDirection: "row", alignItems: "center", paddingHorizontal: 16, paddingVertical: 9, gap: 12 },
  searchText: { flex: 1, gap: 3 },
  searchTitleLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8 },
  searchTitle: { flex: 1, fontSize: Font.body, fontWeight: "600" },
  searchStamp: { fontSize: 14 },
  searchSnippet: { fontSize: 15, lineHeight: 20 },
  swipeActions: { flexDirection: "row" },
  swipeAction: { width: 76, alignItems: "center", justifyContent: "center", gap: 4 },
  swipeLabel: { color: "#FFFFFF", fontSize: 13, fontWeight: "500" },
  screen: { flex: 1 },
  list: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 78 },
  // iOS's collapsible list sections head with a bold title and a tinted chevron; Material's with
  // a subheader in the primary color.
  header: { flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 16, paddingTop: Platform.OS === "ios" ? 18 : 16, paddingBottom: Platform.OS === "ios" ? 6 : 8 },
  headerTitle: { flex: 1, fontSize: Platform.OS === "ios" ? 20 : 14, fontWeight: Platform.OS === "ios" ? "700" : "500", letterSpacing: Platform.OS === "ios" ? 0.35 : 0.1 },
  status: { flexDirection: "row", alignItems: "center", gap: 8 },
  statusPressed: { opacity: 0.4 },
  statusText: { fontSize: Font.small, fontWeight: "500" },
  empty: { alignItems: "center", paddingTop: 120, paddingHorizontal: 40, gap: 8 },
  emptyTitle: { fontSize: 20, fontWeight: "600", marginTop: 8 },
  emptyText: { fontSize: 15, textAlign: "center", lineHeight: 21 },
});

function SearchResultRow({ item, bots, query, onPress }: { item: SearchRow; bots: Map<string, Bot>; query: string; onPress: () => void }) {
  useLanguage();
  const p = usePalette();
  const members = item.chat.bot_ids.map((id) => bots.get(id)).filter((bot): bot is Bot => !!bot);
  const at = item.createdAt ?? lastActivity(item.chat);
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={`${(item.kind === "file" ? t("Files") : item.kind === "message" ? t("Messages") : t("Chats"))}, ${chatTitle(item.chat)}, ${item.snippet}`} onPress={onPress} style={({ pressed }) => [styles.searchRow, { backgroundColor: pressed ? p.fill : "transparent" }]}>
      <AvatarCluster bots={members} chatId={item.chat.id} size={44} />
      <View style={styles.searchText}>
        <View style={styles.searchTitleLine}>
          <HighlightedText text={chatTitle(item.chat)} query={query} style={[styles.searchTitle, { color: p.label }]} numberOfLines={1} />
          <Text style={[styles.searchStamp, { color: p.secondaryLabel }]}>{stamp(new Date(at * 1000))}</Text>
        </View>
        <HighlightedText text={item.snippet} query={query} style={[styles.searchSnippet, { color: p.secondaryLabel }]} numberOfLines={2} />
      </View>
      {item.kind !== "chat" ? <Text style={{ color: p.tertiaryLabel }}>{(item.kind === "file" ? t("Files") : t("Messages"))}</Text> : null}
    </Pressable>
  );
}

function HighlightedText({ text, query, style, numberOfLines }: { text: string; query: string; style: StyleProp<TextStyle>; numberOfLines?: number }) {
  const p = usePalette();
  const matchStyle: TextStyle = { color: p.label, backgroundColor: p.secondaryFill, fontWeight: "700" };
  const terms = query.match(/[\p{L}\p{N}]+/gu) ?? [];
  if (!terms.length) return <Text style={style} numberOfLines={numberOfLines}>{text}</Text>;
  const expression = new RegExp(`(${terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "giu");
  const folded = new Set(terms.map((term) => term.toLocaleLowerCase()));
  return (
    <Text style={style} numberOfLines={numberOfLines}>
      {text.split(expression).map((part, index) =>
        folded.has(part.toLocaleLowerCase()) ? (
          <Text key={`${index}:${part}`} style={matchStyle}>
            {part}
          </Text>
        ) : (
          part
        ),
      )}
    </Text>
  );
}
