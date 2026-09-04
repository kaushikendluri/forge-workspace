/**
 * Thin, typed wrapper around `@tauri-apps/api`'s event system.
 *
 * Nothing emits Forge events in M1 — there is no Rust backend yet — so
 * nothing calls this either. It exists so subscriptions written in M2+
 * (e.g. a store subscribing to `agent-run:activity`) are typed against
 * `ForgeEventMap` from the start instead of using raw strings.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ForgeEventMap, ForgeEventName } from "@/types/events";

/**
 * Subscribe to a typed Forge event. Returns the unlisten function Tauri
 * gives back, resolved once the listener is registered.
 */
export async function onForgeEvent<K extends ForgeEventName>(
  event: K,
  handler: (payload: ForgeEventMap[K]) => void,
): Promise<UnlistenFn> {
  return listen<ForgeEventMap[K]>(event, (e) => handler(e.payload));
}
