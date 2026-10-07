import { NavigationContext } from "expo-router/react-navigation";
import { useContext, useEffect, useState, useSyncExternalStore } from "react";
import { AccessibilityInfo } from "react-native";

// Fail closed until the OS answer arrives. One OS listener while at least one avatar needs it.
let reduceMotion = true;
let generation = 0;
let stopListening: (() => void) | undefined;
const subscribers = new Set<() => void>();

function setReduceMotion(value: boolean) {
  if (value === reduceMotion) return;
  reduceMotion = value;
  for (const notify of subscribers) notify();
}

function subscribe(notify: () => void) {
  subscribers.add(notify);
  if (!stopListening) {
    const current = ++generation;
    const listener = AccessibilityInfo.addEventListener("reduceMotionChanged", (value) => {
      ++generation; // An OS event newer than the initial request wins.
      setReduceMotion(value);
    });
    stopListening = () => listener.remove();
    void AccessibilityInfo.isReduceMotionEnabled().then((value) => {
      if (generation === current && subscribers.size) setReduceMotion(value);
    });
  }
  return () => {
    subscribers.delete(notify);
    if (!subscribers.size) {
      stopListening?.();
      stopListening = undefined;
      ++generation;
    }
  };
}

export function useReduceMotion(): boolean {
  return useSyncExternalStore(subscribe, () => reduceMotion, () => true);
}

// Context-menu previews may sit outside a navigator. A missing context is not permission to
// animate; ordinary avatars use the screen's actual focus/blur signals.
export function useScreenFocused(): boolean {
  const navigation = useContext(NavigationContext);
  const [focused, setFocused] = useState(() => navigation?.isFocused() ?? false);
  useEffect(() => {
    setFocused(navigation?.isFocused() ?? false);
    if (!navigation) return;
    const gained = navigation.addListener("focus", () => setFocused(true));
    const lost = navigation.addListener("blur", () => setFocused(false));
    return () => { gained(); lost(); };
  }, [navigation]);
  return focused;
}
