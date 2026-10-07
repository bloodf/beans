import { FlashList, type FlashListProps, type FlashListRef } from "@shopify/flash-list";
import { useCallback, useRef, type Ref } from "react";
import { AvatarRowVisibility, AvatarViewability, notifyAvatarScroll } from "./avatarVisibility";

const VIEWABILITY = { itemVisiblePercentThreshold: 1, minimumViewTime: 0 };

// FlashList can render measurements and retain offscreen/recycled cells. Only actual viewable
// Cell targets expose visible=true to their avatars; tracking keys avoids recycled-index state.
export function AvatarFlashList<T>({ ref, renderItem, onViewableItemsChanged, keyExtractor, onScroll, ...props }: FlashListProps<T> & { ref?: Ref<FlashListRef<T>> }) {
  const tracker = useRef(new AvatarViewability()).current;
  const supplied = useRef(onViewableItemsChanged);
  supplied.current = onViewableItemsChanged;
  const viewable = useCallback<NonNullable<FlashListProps<T>["onViewableItemsChanged"]>>((info) => {
    tracker.update(info.viewableItems.filter((item) => item.isViewable).map((item) => item.key));
    supplied.current?.(info);
  }, [tracker]);
  const render = useCallback<NonNullable<FlashListProps<T>["renderItem"]>>((info) => {
    const key = info.target === "Cell" ? keyExtractor?.(info.item, info.index) ?? String(info.index) : "\u0000measurement";
    return <AvatarRowVisibility tracker={tracker} rowKey={key}>{renderItem?.(info)}</AvatarRowVisibility>;
  }, [tracker, keyExtractor, renderItem]);
  return <FlashList {...props} ref={ref} keyExtractor={keyExtractor} renderItem={render} viewabilityConfig={VIEWABILITY} onViewableItemsChanged={viewable} onScroll={(event) => { notifyAvatarScroll(); onScroll?.(event); }} />;
}
