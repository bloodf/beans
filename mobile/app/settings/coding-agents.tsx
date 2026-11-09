// A Runner's Device › Coding Agents: the agents its bots hand coding work to, in that Runner's
// order of preference, each on or off, with whether it is installed there and its version. Asked
// of the Runner when the screen opens, through the relay when it is another Device. A row's grip
// drags it to another place; the switch turns it on or off. Each change shows at once and sends
// the whole list (`coding.agents.set`), whose answer is the list as it now stands; a refusal puts
// the list back, with an alert.

import { Stack, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { Host as ComposeHost, Switch as ComposeSwitch } from "@expo/ui/jetpack-compose";
import { Platform, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { agentName, type CodingAgent } from "../../src/core/model";
import { t, useLanguage } from "../../src/i18n";
import { AgentLogo } from "../../src/ui/AgentLogo";
import { alert } from "../../src/ui/alert";
import { Row, Section } from "../../src/ui/forms";
import { ReorderableRows } from "../../src/ui/ReorderableRows";
import { Font, usePalette } from "../../src/ui/theme";

// Two lines each: the name, and whether it is installed.
const ROW_HEIGHT = Platform.OS === "android" ? 72 : 60;

function installWord(agent: CodingAgent): string {
  if (!agent.installed) return t("Not installed");
  return agent.version ? t("Installed · {version}", { version: agent.version }) : t("Installed");
}

export default function CodingAgentsScreen() {
  useLanguage();
  const p = usePalette();
  const { runner } = useLocalSearchParams<{ runner: string }>();
  const [agents, setAgents] = useState<CodingAgent[]>();
  const [error, setError] = useState<string>();
  // Every load and change counts here; an answer that a later one overtook is dropped.
  const asks = useRef(0);

  const load = useCallback(() => {
    const ask = ++asks.current;
    engine
      .codingAgents(runner)
      .then((list) => ask === asks.current && (setAgents(list), setError(undefined)))
      .catch((e: unknown) => ask === asks.current && setError(e instanceof Error ? e.message : String(e)));
  }, [runner]);
  useEffect(load, [load]);

  const save = (held: CodingAgent[], next: CodingAgent[]) => {
    const ask = ++asks.current;
    setAgents(next);
    engine
      .setCodingAgents(runner, next)
      .then((list) => ask === asks.current && setAgents(list))
      .catch((e: unknown) => {
        if (ask === asks.current) setAgents(held);
        alert(t("Couldn't save the coding agents"), e instanceof Error ? e.message : String(e));
      });
  };
  const reorder = (list: CodingAgent[], kinds: string[]) => save(list, kinds.map((kind) => list.find((agent) => agent.kind === kind)!).filter(Boolean));
  const toggle = (list: CodingAgent[], kind: string, enabled: boolean) => save(list, list.map((agent) => (agent.kind === kind ? { ...agent, enabled } : agent)));

  return (
    <>
      <Stack.Screen options={{ title: t("Coding Agents") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          footer={`${t("Bots on this Runner hand coding work to these agents. A bot uses the first one that's on and installed, unless you asked for another.")} ${t("Drag an agent by its handle to change the order.")}`}
        >
          {!agents ? (
            <Row title={error ?? t("Loading…")} titleLines={3} />
          ) : (
            <ReorderableRows
              keys={agents.map((agent) => agent.kind)}
              height={ROW_HEIGHT}
              label={(kind) => agents.find((agent) => agent.kind === kind)?.name || agentName(kind)}
              onMove={(kinds) => reorder(agents, kinds)}
              row={(kind) => {
                const agent = agents.find((choice) => choice.kind === kind)!;
                const name = agent.name || agentName(kind);
                return (
                  <View style={styles.row}>
                    <AgentLogo kind={kind} size={24} />
                    <View style={styles.text}>
                      <Text style={[styles.name, { color: p.label }]} numberOfLines={1}>
                        {name}
                      </Text>
                      <Text style={[styles.detail, { color: p.secondaryLabel }]} numberOfLines={1}>
                        {installWord(agent)}
                      </Text>
                    </View>
                    {Platform.OS === "android" ? (
                      <ComposeHost matchContents>
                        <ComposeSwitch value={agent.enabled} onCheckedChange={(on) => toggle(agents, kind, on)} />
                      </ComposeHost>
                    ) : (
                      <Switch value={agent.enabled} onValueChange={(on) => toggle(agents, kind, on)} accessibilityLabel={name} />
                    )}
                  </View>
                );
              }}
            />
          )}
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  row: { flex: 1, flexDirection: "row", alignItems: "center", gap: 12, paddingLeft: 16 },
  text: { flex: 1, gap: 2 },
  name: { fontSize: Font.body },
  detail: { fontSize: Font.small },
});
