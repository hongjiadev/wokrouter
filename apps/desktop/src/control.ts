import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const coreStatusSchema = z
  .object({
    state: z.enum([
      "missing",
      "stopped",
      "starting",
      "running",
      "draining",
      "authorization_required",
      "incompatible",
      "invalid_runtime",
    ]),
    version: z.string().trim().min(1).max(64).optional(),
    management_api_major: z.number().int().positive().optional(),
    capabilities: z.array(z.string().trim().min(1).max(128)).max(256),
    phase: z
      .enum([
        "starting",
        "running",
        "draining",
        "awaiting_cancellation",
        "stopping",
      ])
      .optional(),
    active_requests: z.number().int().nonnegative().optional(),
    error_code: z.string().trim().min(1).max(128).optional(),
  })
  .strict();

export type CoreStatus = z.infer<typeof coreStatusSchema>;
export const coreStatusQueryKey = ["core-status"] as const;

export async function getCoreStatus(): Promise<CoreStatus> {
  const status = await invoke<unknown>("core_status");
  const parsed = coreStatusSchema.safeParse(status);
  if (!parsed.success) {
    throw new Error("Invalid WokCore status returned by desktop bridge.", {
      cause: parsed.error,
    });
  }
  return parsed.data;
}

export async function startCore(): Promise<void> {
  await invoke("start_core");
}

export async function stopCore(): Promise<void> {
  await invoke("stop_core");
}
