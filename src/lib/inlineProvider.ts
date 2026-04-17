import type { AiProviderConfig } from "../types/ai";
import { INLINE_LOCAL_PROVIDER_ID } from "../types/ai";
import {
  useInlineAiStore,
  type ModelListItem,
  type SidecarState,
} from "../stores/inlineAiStore";

/**
 * Build the virtual "Local (Inline AI)" provider entry the palette + AI
 * assistant show in their dropdowns. Returns `null` when the sidecar
 * isn't in a state that can actually serve requests — caller shouldn't
 * render or dispatch against it in that case.
 *
 * Takes state as explicit arguments so React-side callers can pass them
 * directly to `useMemo` deps without tripping `exhaustive-deps`.
 */
export function buildVirtualInlineProvider(
  enabled: boolean,
  sidecar: SidecarState,
  models: ModelListItem[],
): AiProviderConfig | null {
  if (!enabled) return null;
  if (sidecar.state !== "ready") return null;
  const { modelId } = sidecar;
  const model = models.find((m) => m.id === modelId);
  const label = model?.displayName ?? modelId;
  return {
    id: INLINE_LOCAL_PROVIDER_ID,
    name: `Local (${label})`,
    provider: "inlineLocal",
    apiKey: "",
    model: modelId,
    isDefault: false,
  };
}

/** Snapshot-reading variant for non-React callers (dispatchers). */
export function selectVirtualInlineProvider(): AiProviderConfig | null {
  const s = useInlineAiStore.getState();
  return buildVirtualInlineProvider(s.enabled, s.sidecar, s.models);
}

/** True when the user has asked for the inline sidecar to act as the
 *  default AI provider AND it is actually available right now. */
export function inlineIsEffectiveDefault(): boolean {
  const s = useInlineAiStore.getState();
  return (
    s.useAsDefaultProvider && s.enabled && s.sidecar.state === "ready"
  );
}
