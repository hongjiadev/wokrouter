import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const daemonStatusSchema = z
  .object({
    state: z.enum(["running", "stopped"]),
    version: z.string().trim().min(1).max(64),
  })
  .strict();

export type DaemonStatus = z.infer<typeof daemonStatusSchema>;

export async function getDaemonStatus(): Promise<DaemonStatus> {
  const status = await invoke<unknown>("daemon_status");
  const parsed = daemonStatusSchema.safeParse(status);
  if (!parsed.success) {
    throw new Error("Invalid daemon status returned by desktop bridge.", {
      cause: parsed.error,
    });
  }
  return parsed.data;
}

export async function startDaemon(): Promise<void> {
  await invoke("start_daemon");
}
