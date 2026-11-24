import type { AdapterConfig } from "./config.ts";

/** True only when `requested` is a known level at or below the ceiling. Unknown counts as above (decision 0003.5). */
export function withinCeiling(config: Pick<AdapterConfig, "levels" | "ceiling">, requested: unknown): boolean {
  if (typeof requested !== "string") return false;
  const r = config.levels.indexOf(requested);
  const c = config.levels.indexOf(config.ceiling);
  return r >= 0 && c >= 0 && r <= c;
}
