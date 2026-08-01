import type { CoreStatus } from "./control";

const eligibleUpdateStates = new Set<CoreStatus["state"]>([
  "running",
  "stopped",
  "authorization_required",
  "incompatible",
]);

export function isCoreUpdateEligible(
  status: CoreStatus | undefined,
): boolean {
  return (
    status?.runtime_channel === "production" &&
    eligibleUpdateStates.has(status.state)
  );
}
