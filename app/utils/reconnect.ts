/** Seconds to wait before each automatic reconnect attempt after a drop. After the last one we stop and tell the user. */
export const RETRY_SECONDS = [2, 4, 8, 16, 30] as const

/** Milliseconds to wait before attempt number `attempt` (0 is the first), or `null` when it is time to give up. */
export function retryDelayMs(attempt: number): number | null {
  const s = RETRY_SECONDS[attempt]
  return s === undefined ? null : s * 1000
}
